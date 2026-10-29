use super::{Completion, Provider, ToolDefinition, Usage, truncate};
use crate::conversation::{Block, Message, Role, ToolCall};
use anyhow::{Result, bail};
use futures_util::StreamExt;
use serde_json::{Value, json};
use std::collections::BTreeMap;

pub async fn complete<F: FnMut(&str)>(
    provider: &Provider,
    system: &str,
    messages: &[Message],
    tools: &[ToolDefinition],
    on_text: &mut F,
) -> Result<Completion> {
    let url = provider.endpoint("/responses");
    let mut body = json!({
        "model": provider.model(),
        "input": build_input(messages),
        "stream": true,
        "store": false,
        "include": ["reasoning.encrypted_content"],
    });
    if !system.is_empty() {
        body["instructions"] = json!(system);
    }
    if !tools.is_empty() {
        body["tools"] = Value::Array(
            tools
                .iter()
                .map(|tool| {
                    json!({
                        "type": "function",
                        "name": tool.name,
                        "description": tool.description,
                        "parameters": tool.schema,
                        "strict": false,
                    })
                })
                .collect(),
        );
    }
    if let Some(max) = provider.max_tokens() {
        body["max_output_tokens"] = json!(max);
    }

    let response = provider.bearer_request(&url, &body).send().await?;
    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        bail!("API error {status}: {}", truncate(&body, 2000));
    }

    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();

    if !content_type.contains("text/event-stream") {
        let text = response.text().await?;
        let value: Value = serde_json::from_str(&text)?;
        let mut completion = Completion::default();
        parse_response(&value, &mut completion);
        if !completion.text.is_empty() {
            on_text(&completion.text);
        }
        return Ok(completion);
    }

    let mut completion = Completion::default();
    let mut calls: BTreeMap<String, CallAcc> = BTreeMap::new();
    let mut stream = response.bytes_stream();
    let mut buffer = String::new();

    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        buffer.push_str(&String::from_utf8_lossy(&chunk));
        if buffer.contains('\r') {
            buffer = buffer.replace("\r\n", "\n").replace('\r', "");
        }
        while let Some(pos) = buffer.find("\n\n") {
            let event: String = buffer.drain(..pos + 2).collect();
            let Some(data) = event_data(&event) else {
                continue;
            };
            let value: Value = serde_json::from_str(&data)?;
            let event_type = value
                .get("type")
                .and_then(Value::as_str)
                .unwrap_or_default();
            match event_type {
                "response.output_text.delta" => {
                    if let Some(delta) = value.get("delta").and_then(Value::as_str) {
                        completion.text.push_str(delta);
                        on_text(delta);
                    }
                }
                "response.output_item.added" | "response.output_item.done" => {
                    let Some(item) = value.get("item") else {
                        continue;
                    };
                    if item.get("type").and_then(Value::as_str) != Some("function_call") {
                        continue;
                    }
                    let item_id = item
                        .get("id")
                        .and_then(Value::as_str)
                        .map(str::to_string)
                        .or_else(|| {
                            item.get("call_id")
                                .and_then(Value::as_str)
                                .map(str::to_string)
                        })
                        .unwrap_or_default();
                    let acc = calls.entry(item_id).or_default();
                    if let Some(call_id) = item.get("call_id").and_then(Value::as_str) {
                        acc.call_id = call_id.to_string();
                    }
                    if let Some(name) = item.get("name").and_then(Value::as_str) {
                        acc.name = name.to_string();
                    }
                    if let Some(args) = item.get("arguments").and_then(Value::as_str)
                        && !args.is_empty()
                    {
                        acc.arguments = args.to_string();
                    }
                }
                "response.function_call_arguments.delta" => {
                    let item_id = value
                        .get("item_id")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string();
                    let acc = calls.entry(item_id).or_default();
                    if let Some(delta) = value.get("delta").and_then(Value::as_str) {
                        acc.arguments.push_str(delta);
                    }
                }
                "response.function_call_arguments.done" => {
                    let item_id = value
                        .get("item_id")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string();
                    let acc = calls.entry(item_id).or_default();
                    if let Some(arguments) = value.get("arguments").and_then(Value::as_str) {
                        acc.arguments = arguments.to_string();
                    }
                }
                "response.completed" => {
                    if let Some(usage) = value.get("response").and_then(|r| r.get("usage")) {
                        read_usage(usage, &mut completion.usage);
                    }
                }
                "response.failed" | "error" => {
                    bail!("API error: {}", truncate(&value.to_string(), 1000));
                }
                _ => {}
            }
        }
    }

    for acc in calls.into_values() {
        completion.tool_calls.push(ToolCall {
            id: if acc.call_id.is_empty() {
                "call_0".to_string()
            } else {
                acc.call_id
            },
            name: acc.name,
            arguments: if acc.arguments.trim().is_empty() {
                "{}".to_string()
            } else {
                acc.arguments
            },
        });
    }

    Ok(completion)
}

#[derive(Default)]
struct CallAcc {
    call_id: String,
    name: String,
    arguments: String,
}

fn event_data(event: &str) -> Option<String> {
    let mut data = Vec::new();
    for line in event.lines() {
        let line = line.trim_start();
        if let Some(rest) = line.strip_prefix("data:") {
            data.push(rest.strip_prefix(' ').unwrap_or(rest));
        }
    }
    if data.is_empty() {
        None
    } else {
        Some(data.join("\n"))
    }
}

fn build_input(messages: &[Message]) -> Vec<Value> {
    let mut out = Vec::new();
    for message in messages {
        match message.role {
            Role::User => {
                out.push(json!({"role": "user", "content": message.text()}));
            }
            Role::Assistant => {
                let provider_items = message.blocks.iter().find_map(|block| match block {
                    Block::ProviderItems(items) => Some(items),
                    _ => None,
                });
                if let Some(items) = provider_items {
                    out.extend(items.iter().cloned());
                    continue;
                }
                let text = message.text();
                if !text.is_empty() {
                    out.push(json!({"role": "assistant", "content": text}));
                }
                for block in &message.blocks {
                    if let Block::ToolCall(call) = block {
                        out.push(json!({
                            "type": "function_call",
                            "call_id": call.id,
                            "name": call.name,
                            "arguments": call.arguments,
                        }));
                    }
                }
            }
            Role::Tool => {
                for block in &message.blocks {
                    if let Block::ToolResult {
                        tool_call_id,
                        content,
                        is_error,
                    } = block
                    {
                        let output = if *is_error {
                            format!("Error: {content}")
                        } else {
                            content.clone()
                        };
                        out.push(json!({
                            "type": "function_call_output",
                            "call_id": tool_call_id,
                            "output": output,
                        }));
                    }
                }
            }
        }
    }
    out
}

fn parse_response(value: &Value, completion: &mut Completion) {
    if let Some(usage) = value.get("usage") {
        read_usage(usage, &mut completion.usage);
    }
    let Some(output) = value.get("output").and_then(Value::as_array) else {
        return;
    };
    completion.provider_items = output.clone();
    for item in output {
        match item.get("type").and_then(Value::as_str).unwrap_or("") {
            "message" => {
                if let Some(content) = item.get("content").and_then(Value::as_array) {
                    for part in content {
                        if let Some(text) = part.get("text").and_then(Value::as_str) {
                            completion.text.push_str(text);
                        }
                    }
                }
            }
            "function_call" => {
                completion.tool_calls.push(ToolCall {
                    id: item
                        .get("call_id")
                        .and_then(Value::as_str)
                        .unwrap_or("call_0")
                        .to_string(),
                    name: item
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string(),
                    arguments: item
                        .get("arguments")
                        .and_then(Value::as_str)
                        .unwrap_or("{}")
                        .to_string(),
                });
            }
            _ => {}
        }
    }
}

fn read_usage(usage: &Value, out: &mut Usage) {
    out.input_tokens = usage
        .get("input_tokens")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    out.output_tokens = usage
        .get("output_tokens")
        .and_then(Value::as_u64)
        .unwrap_or(0);
}
