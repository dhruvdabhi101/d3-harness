use super::{Completion, Provider, ToolDefinition, parse_args, send_streaming, truncate};
use crate::conversation::{Block, Message, Role, ToolCall};
use anyhow::{Result, bail};
use serde_json::{Value, json};
use std::collections::BTreeMap;

const API_VERSION: &str = "2023-06-01";

pub async fn complete<F: FnMut(&str)>(
    provider: &Provider,
    system: &str,
    messages: &[Message],
    tools: &[ToolDefinition],
    on_text: &mut F,
) -> Result<Completion> {
    let url = provider.endpoint("/messages");
    let mut body = json!({
        "model": provider.model(),
        "messages": build_messages(messages),
        "max_tokens": provider.max_tokens().unwrap_or(8192),
        "stream": true,
    });
    if !system.is_empty() {
        body["system"] = json!(system);
    }
    if !tools.is_empty() {
        body["tools"] = Value::Array(
            tools
                .iter()
                .map(|tool| {
                    json!({
                        "name": tool.name,
                        "description": tool.description,
                        "input_schema": tool.schema,
                    })
                })
                .collect(),
        );
    }

    let request = provider
        .json_request(&url, &body)
        .header("x-api-key", provider.api_key())
        .header("anthropic-version", API_VERSION);
    let response = request.send().await?;

    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();

    let mut completion = Completion::default();

    if !content_type.contains("text/event-stream") {
        send_streaming(response, |data| {
            let value: Value = serde_json::from_str(data)?;
            if value.get("type").and_then(Value::as_str) == Some("error") {
                bail!("API error: {}", truncate(&value.to_string(), 1000));
            }
            parse_non_stream(&value, &mut completion);
            if !completion.text.is_empty() {
                on_text(&completion.text);
            }
            Ok(())
        })
        .await?;
    } else {
        let mut blocks: BTreeMap<usize, BlockAcc> = BTreeMap::new();
        send_streaming(response, |data| {
            let value: Value = serde_json::from_str(data)?;
            match value.get("type").and_then(Value::as_str).unwrap_or("") {
                "message_start" => {
                    if let Some(usage) = value.get("message").and_then(|m| m.get("usage")) {
                        completion.usage.input_tokens = usage
                            .get("input_tokens")
                            .and_then(Value::as_u64)
                            .unwrap_or(0);
                    }
                }
                "content_block_start" => {
                    let index = value.get("index").and_then(Value::as_u64).unwrap_or(0) as usize;
                    let block = value.get("content_block").cloned().unwrap_or_default();
                    let kind = block
                        .get("type")
                        .and_then(Value::as_str)
                        .unwrap_or("text")
                        .to_string();
                    let mut acc = BlockAcc {
                        kind,
                        ..Default::default()
                    };
                    if let Some(text) = block.get("text").and_then(Value::as_str) {
                        acc.text.push_str(text);
                    }
                    if let Some(id) = block.get("id").and_then(Value::as_str) {
                        acc.id = id.to_string();
                    }
                    if let Some(name) = block.get("name").and_then(Value::as_str) {
                        acc.name = name.to_string();
                    }
                    blocks.insert(index, acc);
                }
                "content_block_delta" => {
                    let index = value.get("index").and_then(Value::as_u64).unwrap_or(0) as usize;
                    let delta = value.get("delta").cloned().unwrap_or_default();
                    let acc = blocks.entry(index).or_default();
                    match delta.get("type").and_then(Value::as_str).unwrap_or("") {
                        "text_delta" => {
                            if let Some(text) = delta.get("text").and_then(Value::as_str) {
                                acc.text.push_str(text);
                                on_text(text);
                            }
                        }
                        "input_json_delta" => {
                            if let Some(partial) = delta.get("partial_json").and_then(Value::as_str)
                            {
                                acc.arguments.push_str(partial);
                            }
                        }
                        _ => {}
                    }
                }
                "message_delta" => {
                    if let Some(usage) = value.get("usage") {
                        let output = usage
                            .get("output_tokens")
                            .and_then(Value::as_u64)
                            .unwrap_or(0);
                        if output > 0 {
                            completion.usage.output_tokens = output;
                        }
                    }
                }
                "error" => {
                    bail!("API error: {}", truncate(&value.to_string(), 1000));
                }
                _ => {}
            }
            Ok(())
        })
        .await?;

        for acc in blocks.into_values() {
            if acc.kind == "tool_use" {
                completion.tool_calls.push(ToolCall {
                    id: if acc.id.is_empty() {
                        "tool_use".to_string()
                    } else {
                        acc.id
                    },
                    name: acc.name,
                    arguments: parse_args(&acc.arguments),
                });
            } else if !acc.text.is_empty() {
                completion.text.push_str(&acc.text);
            }
        }
    }

    Ok(completion)
}

#[derive(Default)]
struct BlockAcc {
    kind: String,
    text: String,
    id: String,
    name: String,
    arguments: String,
}

fn build_messages(messages: &[Message]) -> Vec<Value> {
    let mut out: Vec<Value> = Vec::new();
    for message in messages {
        let (role, content) = match message.role {
            Role::User => {
                let content = message.blocks.iter().filter_map(|block| match block {
                    Block::Text(text) if !text.is_empty() => Some(json!({"type": "text", "text": text})),
                    Block::Image(image) => Some(json!({"type": "image", "source": {"type": "base64", "media_type": image.media_type, "data": image.data}})),
                    _ => None,
                }).collect();
                ("user", content)
            }
            Role::Assistant => {
                let mut blocks = Vec::new();
                for block in &message.blocks {
                    match block {
                        Block::Text(text) if !text.is_empty() => {
                            blocks.push(json!({"type": "text", "text": text}));
                        }
                        Block::ToolCall(call) => {
                            let input: Value = serde_json::from_str(&call.arguments)
                                .unwrap_or_else(|_| json!({"raw": call.arguments}));
                            blocks.push(json!({
                                "type": "tool_use",
                                "id": call.id,
                                "name": call.name,
                                "input": input,
                            }));
                        }
                        _ => {}
                    }
                }
                if blocks.is_empty() {
                    continue;
                }
                ("assistant", blocks)
            }
            Role::Tool => {
                let mut blocks = Vec::new();
                for block in &message.blocks {
                    if let Block::ToolResult {
                        tool_call_id,
                        content,
                        is_error,
                    } = block
                    {
                        blocks.push(json!({
                            "type": "tool_result",
                            "tool_use_id": tool_call_id,
                            "content": content,
                            "is_error": is_error,
                        }));
                    }
                }
                if blocks.is_empty() {
                    continue;
                }
                ("user", blocks)
            }
        };

        if let Some(last) = out.last_mut()
            && last.get("role").and_then(Value::as_str) == Some(role)
            && let Some(parts) = last.get_mut("content").and_then(Value::as_array_mut)
        {
            parts.extend(content);
            continue;
        }
        out.push(json!({"role": role, "content": content}));
    }
    out
}

fn parse_non_stream(value: &Value, completion: &mut Completion) {
    if let Some(usage) = value.get("usage") {
        completion.usage.input_tokens = usage
            .get("input_tokens")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        completion.usage.output_tokens = usage
            .get("output_tokens")
            .and_then(Value::as_u64)
            .unwrap_or(0);
    }
    let Some(content) = value.get("content").and_then(Value::as_array) else {
        return;
    };
    for block in content {
        match block.get("type").and_then(Value::as_str).unwrap_or("") {
            "text" => {
                if let Some(text) = block.get("text").and_then(Value::as_str) {
                    completion.text.push_str(text);
                }
            }
            "tool_use" => {
                let input = block.get("input").cloned().unwrap_or_else(|| json!({}));
                completion.tool_calls.push(ToolCall {
                    id: block
                        .get("id")
                        .and_then(Value::as_str)
                        .unwrap_or("tool_use")
                        .to_string(),
                    name: block
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string(),
                    arguments: input.to_string(),
                });
            }
            _ => {}
        }
    }
}
