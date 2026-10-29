mod anthropic;
mod openai;
mod responses;

use crate::config::{ProviderConfig, ProviderKind};
use crate::conversation::{Message, ToolCall};
use anyhow::{Context, Result, bail};
use futures_util::StreamExt;
use reqwest::Response;
use serde_json::Value;
use std::time::Duration;

#[derive(Debug, Clone)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub schema: Value,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct Usage {
    pub input_tokens: u64,
    pub output_tokens: u64,
}

#[derive(Debug, Clone, Default)]
pub struct Completion {
    pub text: String,
    pub tool_calls: Vec<ToolCall>,
    pub usage: Usage,
    /// Raw provider output items to replay verbatim on the next turn.
    pub provider_items: Vec<Value>,
}

pub struct Provider {
    pub name: String,
    config: ProviderConfig,
    model: String,
    api_key: String,
    client: reqwest::Client,
}

impl Provider {
    pub fn new(
        name: String,
        config: ProviderConfig,
        model: String,
        api_key: String,
    ) -> Result<Self> {
        if config.base_url.is_none() {
            bail!("provider '{name}' has no base_url");
        }
        let client = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(20))
            .build()
            .context("building HTTP client")?;
        Ok(Self {
            name,
            config,
            model,
            api_key,
            client,
        })
    }

    pub fn model(&self) -> &str {
        &self.model
    }

    pub fn set_model(&mut self, model: String) {
        self.model = model;
    }

    pub async fn complete<F: FnMut(&str)>(
        &self,
        system: &str,
        messages: &[Message],
        tools: &[ToolDefinition],
        on_text: &mut F,
    ) -> Result<Completion> {
        let kind = self.config.kind.unwrap_or_default();
        match kind {
            ProviderKind::OpenAi => openai::complete(self, system, messages, tools, on_text).await,
            ProviderKind::Anthropic => {
                anthropic::complete(self, system, messages, tools, on_text).await
            }
            ProviderKind::Responses => {
                responses::complete(self, system, messages, tools, on_text).await
            }
        }
    }

    pub(super) fn api_key(&self) -> &str {
        &self.api_key
    }

    pub(super) fn endpoint(&self, suffix: &str) -> String {
        let base = self
            .config
            .base_url
            .as_deref()
            .unwrap_or_default()
            .trim_end_matches('/');
        if base.ends_with(suffix) {
            base.to_string()
        } else {
            format!("{base}{suffix}")
        }
    }

    fn apply_headers(&self, mut req: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        for (key, value) in &self.config.headers {
            req = req.header(key, value);
        }
        req
    }

    pub(super) fn json_request(&self, url: &str, body: &Value) -> reqwest::RequestBuilder {
        let req = self.client.post(url).json(body);
        self.apply_headers(req)
    }

    pub(super) fn bearer_request(&self, url: &str, body: &Value) -> reqwest::RequestBuilder {
        let req = self.client.post(url).bearer_auth(&self.api_key).json(body);
        self.apply_headers(req)
    }

    pub(super) fn max_tokens(&self) -> Option<u32> {
        self.config.max_tokens
    }
}

pub(super) async fn send_streaming<F>(response: Response, mut on_data: F) -> Result<()>
where
    F: FnMut(&str) -> Result<()>,
{
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
        let body = response.text().await.context("reading API response")?;
        on_data(&body)?;
        return Ok(());
    }

    let mut stream = response.bytes_stream();
    let mut buffer = String::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.context("reading API stream")?;
        buffer.push_str(&String::from_utf8_lossy(&chunk));
        if buffer.contains('\r') {
            buffer = buffer.replace("\r\n", "\n").replace('\r', "");
        }
        while let Some(pos) = buffer.find("\n\n") {
            let event: String = buffer.drain(..pos + 2).collect();
            if let Some(data) = event_data(&event) {
                on_data(&data)?;
            }
        }
    }
    if !buffer.trim().is_empty()
        && let Some(data) = event_data(&buffer)
    {
        on_data(&data)?;
    }
    Ok(())
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
        let joined = data.join("\n");
        if joined.trim() == "[DONE]" {
            None
        } else {
            Some(joined)
        }
    }
}

pub(super) fn truncate(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let cut: String = text.chars().take(max).collect();
    format!("{cut}\n… truncated")
}

pub(super) fn parse_args(raw: &str) -> String {
    if raw.trim().is_empty() {
        "{}".to_string()
    } else {
        raw.to_string()
    }
}
