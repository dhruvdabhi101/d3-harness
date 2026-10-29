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

#[derive(Debug, Clone)]
pub struct ModelInfo {
    pub id: String,
    pub name: Option<String>,
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

#[derive(Debug, Clone)]
pub struct ProviderAuth {
    pub key: String,
    pub account_id: Option<String>,
}

pub struct Provider {
    pub name: String,
    config: ProviderConfig,
    model: String,
    api_key: String,
    account_id: Option<String>,
    client: reqwest::Client,
}

impl Provider {
    pub fn new(
        name: String,
        config: ProviderConfig,
        model: String,
        auth: ProviderAuth,
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
            api_key: auth.key,
            account_id: auth.account_id,
            client,
        })
    }

    pub fn model(&self) -> &str {
        &self.model
    }

    pub fn set_model(&mut self, model: String) {
        self.model = model;
    }

    pub async fn list_models(&self) -> Result<Vec<ModelInfo>> {
        if self.config.kind == Some(ProviderKind::ChatGpt) {
            return codex_models();
        }
        let url = self
            .config
            .models_url
            .clone()
            .unwrap_or_else(|| self.endpoint("/models"));
        let mut request = self.client.get(&url);
        for (key, value) in &self.config.headers {
            request = request.header(key, value);
        }
        request = match self.config.kind.unwrap_or_default() {
            ProviderKind::Anthropic => request
                .header("x-api-key", &self.api_key)
                .header("anthropic-version", "2023-06-01"),
            _ => request.bearer_auth(&self.api_key),
        };
        let response = request.send().await?;
        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            bail!(
                "could not list models ({status}): {}",
                truncate(&body, 1000)
            );
        }
        let value: Value = response.json().await.context("parsing model list")?;
        Ok(parse_models(&value))
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
            ProviderKind::Responses | ProviderKind::ChatGpt => {
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
        let mut req = self.client.post(url).bearer_auth(&self.api_key).json(body);
        if self.config.kind == Some(ProviderKind::ChatGpt)
            && let Some(account_id) = &self.account_id
        {
            req = req
                .header("chatgpt-account-id", account_id)
                .header("originator", "codex_cli_rs");
        }
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

fn codex_models() -> Result<Vec<ModelInfo>> {
    let home = dirs::home_dir().context("could not find the home directory")?;
    let codex_home = std::env::var("CODEX_HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| home.join(".codex"));
    let path = codex_home.join("models_cache.json");
    let text = std::fs::read_to_string(&path).with_context(|| {
        format!(
            "no Codex model cache at {}. Run `codex login` and try again.",
            path.display()
        )
    })?;
    let value: Value = serde_json::from_str(&text)?;
    Ok(parse_models(&value))
}

pub(super) fn parse_models(value: &Value) -> Vec<ModelInfo> {
    let items = value
        .get("data")
        .or_else(|| value.get("models"))
        .and_then(Value::as_array)
        .or_else(|| value.as_array());
    let Some(items) = items else {
        return Vec::new();
    };
    let mut models = items
        .iter()
        .filter_map(|item| {
            let id = item
                .get("id")
                .or_else(|| item.get("slug"))
                .or_else(|| item.get("name"))
                .and_then(Value::as_str)?
                .to_string();
            let name = item
                .get("display_name")
                .or_else(|| item.get("name"))
                .and_then(Value::as_str)
                .filter(|name| *name != id)
                .map(str::to_string);
            Some(ModelInfo { id, name })
        })
        .collect::<Vec<_>>();
    models.sort_by(|a, b| a.id.cmp(&b.id));
    models.dedup_by(|a, b| a.id == b.id);
    models
}

pub(super) fn parse_args(raw: &str) -> String {
    if raw.trim().is_empty() {
        "{}".to_string()
    } else {
        raw.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_openai_and_anthropic_model_lists() {
        let openai = json!({"object":"list","data":[{"id":"b"},{"id":"a"}]});
        let models = parse_models(&openai);
        assert_eq!(models.len(), 2);
        assert_eq!(models[0].id, "a");

        let anthropic = json!({"data":[{"id":"claude-x","display_name":"Claude X"}]});
        let models = parse_models(&anthropic);
        assert_eq!(models[0].id, "claude-x");
        assert_eq!(models[0].name.as_deref(), Some("Claude X"));
    }
}
