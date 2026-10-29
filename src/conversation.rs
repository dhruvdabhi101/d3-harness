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
