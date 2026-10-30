use super::{Completion, Provider, ToolDefinition, Usage, parse_args, send_streaming, truncate};
use crate::conversation::{Block, Message, Role, ToolCall};
use anyhow::{Result, bail};
use serde_json::{Value, json};
use std::collections::BTreeMap;

pub async fn complete<F: FnMut(&str)>(
    provider: &Provider,
    system: &str,
    messages: &[Message],
    tools: &[ToolDefinition],
    on_text: &mut F,
) -> Result<Completion> {
    let url = provider.endpoint("/chat/completions");
    let mut body = json!({
        "model": provider.model(),
        "messages": build_messages(system, messages),
        "stream": true,
    });
    if !tools.is_empty() {
        body["tools"] = Value::Array(
            tools
                .iter()
                .map(|tool| {
                    json!({
                        "type": "function",
                        "function": {
                            "name": tool.name,
                            "description": tool.description,
                            "parameters": tool.schema,
                        }
                    })
                })
                .collect(),
        );
        body["tool_choice"] = json!("auto");
    }
    if let Some(max) = provider.max_tokens() {
        body["max_tokens"] = json!(max);
    }

    let response = provider.bearer_request(&url, &body).send().await?;
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();

    let mut completion = Completion::default();
    let mut calls: BTreeMap<usize, CallAcc> = BTreeMap::new();

    if !content_type.contains("text/event-stream") {
        send_streaming(response, |data| {
            let value: Value = serde_json::from_str(data)?;
            if let Some(error) = value.get("error") {
                bail!("API error: {}", truncate(&error.to_string(), 1000));
            }
            parse_non_stream(&value, &mut completion);
            if !completion.text.is_empty() {
                on_text(&completion.text);
            }
            Ok(())
        })
        .await?;
    } else {
        send_streaming(response, |data| {
            let value: Value = serde_json::from_str(data)?;
            if let Some(error) = value.get("error") {
                bail!("API error: {}", truncate(&error.to_string(), 1000));
            }
            if let Some(usage) = value.get("usage") {
                read_usage(usage, &mut completion.usage);
            }
            let Some(choices) = value.get("choices").and_then(Value::as_array) else {
                return Ok(());
            };
            for choice in choices {
                let Some(delta) = choice.get("delta") else {
                    continue;
                };
                if let Some(text) = delta.get("content").and_then(Value::as_str)
                    && !text.is_empty()
                {
                    completion.text.push_str(text);
                    on_text(text);
                }
                let Some(tool_calls) = delta.get("tool_calls").and_then(Value::as_array) else {
                    continue;
                };
                for call in tool_calls {
                    let index = call.get("index").and_then(Value::as_u64).unwrap_or(0) as usize;
                    let acc = calls.entry(index).or_default();
                    if let Some(id) = call.get("id").and_then(Value::as_str)
                        && !id.is_empty()
                    {
                        acc.id = id.to_string();
                    }
                    if let Some(function) = call.get("function") {
                        if let Some(name) = function.get("name").and_then(Value::as_str)
                            && !name.is_empty()
                        {
                            acc.name.push_str(name);
                        }
                        if let Some(args) = function.get("arguments").and_then(Value::as_str) {
                            acc.arguments.push_str(args);
                        }
                    }
                }
            }
            Ok(())
        })
        .await?;
    }

    for (index, call) in calls {
        completion.tool_calls.push(ToolCall {
            id: if call.id.is_empty() {
                format!("call_{index}")
            } else {
                call.id
            },
            name: call.name,
            arguments: parse_args(&call.arguments),
        });
    }
    Ok(completion)
}

#[derive(Default)]
struct CallAcc {
    id: String,
    name: String,
    arguments: String,
}

fn build_messages(system: &str, messages: &[Message]) -> Vec<Value> {
    let mut out = Vec::new();
    if !system.is_empty() {
        out.push(json!({"role": "system", "content": system}));
    }
    for message in messages {
        match message.role {
            Role::User => {
                let content = if message
                    .blocks
                    .iter()
                    .any(|block| matches!(block, Block::Image(_)))
                {
                    Value::Array(message.blocks.iter().filter_map(|block| match block {
                        Block::Text(text) => Some(json!({"type": "text", "text": text})),
                        Block::Image(image) => Some(json!({"type": "image_url", "image_url": {"url": image.data_url()}})),
                        _ => None,
                    }).collect())
                } else {
                    json!(message.text())
                };
                out.push(json!({"role": "user", "content": content}));
            }
            Role::Assistant => {
                let text = message.text();
                let calls: Vec<Value> = message
                    .blocks
                    .iter()
                    .filter_map(|block| match block {
                        Block::ToolCall(call) => Some(json!({
                            "id": call.id,
                            "type": "function",
                            "function": {
                                "name": call.name,
                                "arguments": call.arguments,
                            }
                        })),
                        _ => None,
                    })
                    .collect();
                if text.is_empty() && calls.is_empty() {
                    continue;
                }
                let mut value = json!({
                    "role": "assistant",
                    "content": if text.is_empty() { Value::Null } else { json!(text) },
                });
                if !calls.is_empty() {
                    value["tool_calls"] = Value::Array(calls);
                }
                out.push(value);
            }
            Role::Tool => {
                for block in &message.blocks {
                    if let Block::ToolResult {
                        tool_call_id,
                        content,
                        is_error,
                    } = block
                    {
                        let content = if *is_error {
                            format!("Error: {content}")
                        } else {
                            content.clone()
                        };
                        out.push(json!({
                            "role": "tool",
                            "tool_call_id": tool_call_id,
                            "content": content,
                        }));
                    }
                }
            }
        }
    }
    out
}

fn parse_non_stream(value: &Value, completion: &mut Completion) {
    if let Some(usage) = value.get("usage") {
        read_usage(usage, &mut completion.usage);
    }
    let Some(message) = value
        .get("choices")
        .and_then(Value::as_array)
        .and_then(|choices| choices.first())
        .and_then(|choice| choice.get("message"))
    else {
        return;
    };
    if let Some(content) = message.get("content") {
        completion.text = match content {
            Value::String(text) => text.clone(),
            Value::Array(parts) => parts
                .iter()
                .filter_map(|part| part.get("text").and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join(""),
            _ => String::new(),
        };
    }
    if let Some(calls) = message.get("tool_calls").and_then(Value::as_array) {
        for (index, call) in calls.iter().enumerate() {
            let function = call.get("function").cloned().unwrap_or_default();
            completion.tool_calls.push(ToolCall {
                id: call
                    .get("id")
                    .and_then(Value::as_str)
                    .map(str::to_string)
                    .unwrap_or_else(|| format!("call_{index}")),
                name: function
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                arguments: parse_args(
                    function
                        .get("arguments")
                        .and_then(Value::as_str)
                        .unwrap_or("{}"),
                ),
            });
        }
    }
}

fn read_usage(usage: &Value, out: &mut Usage) {
    out.input_tokens = usage
        .get("prompt_tokens")
        .or_else(|| usage.get("input_tokens"))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    out.output_tokens = usage
        .get("completion_tokens")
        .or_else(|| usage.get("output_tokens"))
        .and_then(Value::as_u64)
        .unwrap_or(0);
}
