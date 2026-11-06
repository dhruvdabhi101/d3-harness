use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    User,
    Assistant,
    Tool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    /// Raw JSON string as produced by the model. Parsed by the tool layer.
    pub arguments: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Block {
    Text(String),
    Image(crate::image::ImageAttachment),
    ToolCall(ToolCall),
    ToolResult {
        tool_call_id: String,
        content: String,
        is_error: bool,
    },
    /// Provider-native output items that must be replayed on the next request.
    /// Only the OpenAI Responses provider currently uses this.
    ProviderItems(Vec<serde_json::Value>),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: Role,
    pub blocks: Vec<Block>,
}

impl Message {
    pub fn user(text: impl Into<String>) -> Self {
        Self {
            role: Role::User,
            blocks: vec![Block::Text(text.into())],
        }
    }

    pub fn assistant(text: String, tool_calls: Vec<ToolCall>) -> Self {
        let mut blocks = Vec::new();
        if !text.is_empty() {
            blocks.push(Block::Text(text));
        }
        blocks.extend(tool_calls.into_iter().map(Block::ToolCall));
        Self {
            role: Role::Assistant,
            blocks,
        }
    }

    pub fn tool_result(tool_call_id: String, content: String, is_error: bool) -> Self {
        Self {
            role: Role::Tool,
            blocks: vec![Block::ToolResult {
                tool_call_id,
                content,
                is_error,
            }],
        }
    }

    pub fn text(&self) -> String {
        self.blocks
            .iter()
            .filter_map(|block| match block {
                Block::Text(text) => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
}

/// A plain-text view of the conversation for summarization requests.
pub fn transcript(messages: &[Message], max_chars: usize) -> String {
    let mut out = String::new();
    for message in messages {
        let role = match message.role {
            Role::User => "User",
            Role::Assistant => "Assistant",
            Role::Tool => "Tool",
        };
        for block in &message.blocks {
            match block {
                Block::Text(text) if !text.trim().is_empty() => {
                    out.push_str(&format!("{role}: {}\n\n", text.trim()));
                }
                Block::Image(image) => {
                    out.push_str(&format!(
                        "{role}: [image attachment: {} ({})]\n\n",
                        image.name, image.media_type
                    ));
                }
                Block::ToolCall(call) => {
                    out.push_str(&format!(
                        "{role}: [called tool `{}` with {}]\n\n",
                        call.name, call.arguments
                    ));
                }
                Block::ToolResult {
                    tool_call_id,
                    content,
                    is_error,
                } => {
                    let label = if *is_error {
                        "tool error"
                    } else {
                        "tool result"
                    };
                    out.push_str(&format!(
                        "{label} ({tool_call_id}): {}\n\n",
                        truncate_chars(content, 4_000)
                    ));
                }
                Block::ProviderItems(_) => {}
                Block::Text(_) => {}
            }
        }
    }
    if out.chars().count() <= max_chars {
        return out;
    }
    let kept = out
        .chars()
        .rev()
        .take(max_chars)
        .collect::<String>()
        .chars()
        .rev()
        .collect::<String>();
    format!("[earlier conversation omitted to fit the model context]\n\n{kept}")
}

pub fn to_markdown(messages: &[Message]) -> String {
    let mut out = String::from("# d3 conversation\n\n");
    for message in messages {
        let heading = match message.role {
            Role::User => "User",
            Role::Assistant => "Assistant",
            Role::Tool => "Tool",
        };
        for block in &message.blocks {
            match block {
                Block::Text(text) if !text.trim().is_empty() => {
                    out.push_str(&format!("## {heading}\n\n{}\n\n", text.trim()));
                }
                Block::Image(image) => {
                    out.push_str(&format!(
                        "## {heading}\n\n_Image attachment: {} ({})_\n\n",
                        image.name, image.media_type
                    ));
                }
                Block::ToolCall(call) => {
                    out.push_str(&format!(
                        "### Tool call: `{}`\n\n{}\n\n",
                        call.name,
                        fenced(&call.arguments, "json")
                    ));
                }
                Block::ToolResult {
                    content, is_error, ..
                } => {
                    let label = if *is_error {
                        "Tool error"
                    } else {
                        "Tool result"
                    };
                    out.push_str(&format!("### {label}\n\n{}\n\n", fenced(content, "")));
                }
                Block::ProviderItems(_) => {}
                Block::Text(_) => {}
            }
        }
    }
    out
}

fn fenced(text: &str, language: &str) -> String {
    let fence = if text.contains("```") { "````" } else { "```" };
    format!("{fence}{language}\n{}\n{fence}", text.trim_end())
}

fn truncate_chars(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        text.to_string()
    } else {
        format!(
            "{}… [truncated]",
            text.chars().take(max).collect::<String>()
        )
    }
}
