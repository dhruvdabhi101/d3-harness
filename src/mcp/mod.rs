use crate::config::{McpConfig, McpServerConfig};
use crate::ui;
use anyhow::{Context, Result, bail};
use futures_util::StreamExt;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::path::Path;
use std::process::Stdio;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};

const PROTOCOL_VERSION: &str = "2025-06-18";
const REQUEST_TIMEOUT: Duration = Duration::from_secs(60);

#[derive(Debug, Clone)]
pub struct McpTool {
    pub server: String,
    pub exposed_name: String,
    pub tool_name: String,
    pub description: String,
    pub schema: Value,
}

pub struct McpManager {
    servers: Vec<McpServer>,
}

struct McpServer {
    name: String,
    client: Client,
    tools: Vec<McpTool>,
}

enum Client {
    Stdio(Box<StdioClient>),
    Http(HttpClient),
}

impl Client {
    async fn request(&mut self, method: &str, params: Value) -> Result<Value> {
        match self {
            Client::Stdio(client) => client.request(method, params).await,
            Client::Http(client) => client.request(method, params).await,
        }
    }

    async fn notify(&mut self, method: &str, params: Value) -> Result<()> {
        match self {
            Client::Stdio(client) => client.notify(method, params).await,
            Client::Http(client) => client.notify(method, params).await,
        }
    }
}

impl McpManager {
    pub async fn connect(config: &McpConfig, cwd: &Path) -> Self {
        let mut servers = Vec::new();
        for (name, server_config) in &config.servers {
            if !server_config.is_enabled() {
                continue;
            }
            match connect_server(name, server_config, cwd).await {
                Ok(server) => servers.push(server),
                Err(error) => {
                    ui::warn(&format!("MCP server '{name}' failed to start: {error:#}"));
                }
            }
        }
        Self { servers }
    }

    pub fn is_empty(&self) -> bool {
        self.servers.is_empty()
    }

    pub fn tools(&self) -> Vec<McpTool> {
        self.servers
            .iter()
            .flat_map(|server| server.tools.iter().cloned())
            .collect()
    }

    pub fn summaries(&self) -> Vec<Value> {
        self.servers
            .iter()
            .map(|server| {
                json!({
                    "name": server.name,
                    "tools": server.tools.iter().map(|tool| &tool.tool_name).collect::<Vec<_>>(),
                })
            })
            .collect()
    }

    pub async fn call(
        &mut self,
        server: &str,
        tool: &str,
        arguments: Value,
    ) -> Result<(String, bool)> {
        let server_state = self
            .servers
            .iter_mut()
            .find(|state| state.name == server)
            .with_context(|| format!("MCP server '{server}' is not connected"))?;
        if !server_state.tools.iter().any(|t| t.tool_name == tool) {
            bail!("MCP server '{server}' has no tool '{tool}'");
        }
        let result = server_state
            .client
            .request("tools/call", json!({"name": tool, "arguments": arguments}))
            .await?;
        let is_error = result
            .get("isError")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let mut parts = Vec::new();
        if let Some(content) = result.get("content").and_then(Value::as_array) {
            for item in content {
                match item.get("type").and_then(Value::as_str).unwrap_or("") {
                    "text" => {
                        if let Some(text) = item.get("text").and_then(Value::as_str) {
                            parts.push(text.to_string());
                        }
                    }
                    "resource" => {
                        if let Some(resource) = item.get("resource") {
                            parts.push(resource.to_string());
                        }
                    }
                    other => parts.push(format!("[{other} content]")),
                }
            }
        } else if let Some(value) = result.get("structuredContent") {
            parts.push(value.to_string());
        }
        if parts.is_empty() {
            parts.push(result.to_string());
        }
        Ok((parts.join("\n"), is_error))
    }
}

async fn connect_server(name: &str, config: &McpServerConfig, cwd: &Path) -> Result<McpServer> {
    let mut client = if config.is_http() {
        let url = config.url.clone().context("MCP HTTP server needs a url")?;
        let headers = config
            .headers
            .clone()
            .unwrap_or_default()
            .into_iter()
            .map(|(key, value)| (key, expand_env(&value)))
            .collect();
        Client::Http(HttpClient::new(expand_env(&url), headers))
    } else {
        let (command, args) = config
            .command_parts()
            .context("MCP stdio server needs a command")?;
        let command = expand_env(&command);
        let args = args.iter().map(|arg| expand_env(arg)).collect::<Vec<_>>();
        let env = config
            .env_map()
            .into_iter()
            .map(|(key, value)| (key, expand_env(&value)))
            .collect();
        Client::Stdio(Box::new(
            StdioClient::spawn(&command, &args, env, cwd).await?,
        ))
    };

    let init = client
        .request(
            "initialize",
            json!({
                "protocolVersion": PROTOCOL_VERSION,
                "capabilities": {},
                "clientInfo": {
                    "name": "oc-harness",
                    "version": env!("CARGO_PKG_VERSION"),
                }
            }),
        )
        .await
        .with_context(|| format!("initializing MCP server '{name}'"))?;
    let _ = init;
    client
        .notify("notifications/initialized", json!({}))
        .await
        .with_context(|| format!("acknowledging MCP server '{name}'"))?;

    let mut tools = Vec::new();
    let mut cursor: Option<String> = None;
    loop {
        let params = match &cursor {
            Some(cursor) => json!({"cursor": cursor}),
            None => json!({}),
        };
        let result = client
            .request("tools/list", params)
            .await
            .with_context(|| format!("listing tools from MCP server '{name}'"))?;
        if let Some(list) = result.get("tools").and_then(Value::as_array) {
            for tool in list {
                let tool_name = tool
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                if tool_name.is_empty() {
                    continue;
                }
                let schema = tool
                    .get("inputSchema")
                    .or_else(|| tool.get("input_schema"))
                    .filter(|schema| schema.is_object())
                    .cloned()
                    .unwrap_or_else(|| json!({"type": "object", "properties": {}}));
                tools.push(McpTool {
                    server: name.to_string(),
                    exposed_name: format!("mcp__{}__{}", sanitize(name), sanitize(&tool_name)),
                    tool_name,
                    description: tool
                        .get("description")
                        .and_then(Value::as_str)
                        .unwrap_or("MCP tool")
                        .to_string(),
                    schema,
                });
            }
        }
        cursor = result
            .get("nextCursor")
            .and_then(Value::as_str)
            .filter(|cursor| !cursor.is_empty())
            .map(str::to_string);
        if cursor.is_none() {
            break;
        }
    }

    Ok(McpServer {
        name: name.to_string(),
        client,
        tools,
    })
}

fn expand_env(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut chars = value.char_indices().peekable();
    while let Some((_, ch)) = chars.next() {
        if ch != '$' {
            out.push(ch);
            continue;
        }
        let braced = matches!(chars.peek(), Some((_, '{')));
        if braced {
            chars.next();
        }
        let mut name = String::new();
        for (_, next) in chars.by_ref() {
            if next == '}' && braced {
                break;
            }
            if next.is_ascii_alphanumeric() || next == '_' {
                name.push(next);
            } else {
                if !braced {
                    out.push('$');
                    out.push_str(&name);
                    out.push(next);
                    name.clear();
                }
                break;
            }
        }
        if name.is_empty() {
            out.push('$');
            if braced {
                out.push('{');
            }
        } else if let Ok(found) = std::env::var(&name) {
            out.push_str(&found);
        } else if braced {
            out.push_str("${");
            out.push_str(&name);
            out.push('}');
        } else {
            out.push('$');
            out.push_str(&name);
        }
    }
    out
}

fn sanitize(name: &str) -> String {
    name.chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' {
                ch
            } else {
                '_'
            }
        })
        .collect()
}

struct StdioClient {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    next_id: u64,
}

impl StdioClient {
    async fn spawn(
        command: &str,
        args: &[String],
        env: BTreeMap<String, String>,
        cwd: &Path,
    ) -> Result<Self> {
        let mut cmd = Command::new(command);
        cmd.args(args)
            .current_dir(cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .kill_on_drop(true);
        for (key, value) in env {
            cmd.env(key, value);
        }
        let mut child = cmd
            .spawn()
            .with_context(|| format!("spawning MCP command '{command}'"))?;
        let stdin = child.stdin.take().context("MCP stdin unavailable")?;
        let stdout = child.stdout.take().context("MCP stdout unavailable")?;
        Ok(Self {
            child,
            stdin,
            stdout: BufReader::new(stdout),
            next_id: 1,
        })
    }

    async fn notify(&mut self, method: &str, params: Value) -> Result<()> {
        let message = json!({"jsonrpc": "2.0", "method": method, "params": params});
        let mut line = serde_json::to_string(&message)?;
        line.push('\n');
        self.stdin.write_all(line.as_bytes()).await?;
        self.stdin.flush().await?;
        Ok(())
    }

    async fn request(&mut self, method: &str, params: Value) -> Result<Value> {
        let id = self.next_id;
        self.next_id += 1;
        let message = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        });
        let mut line = serde_json::to_string(&message)?;
        line.push('\n');
        self.stdin.write_all(line.as_bytes()).await?;
        self.stdin.flush().await?;
        tokio::time::timeout(REQUEST_TIMEOUT, self.read_response(id))
            .await
            .context("MCP request timed out")?
    }

    async fn read_response(&mut self, id: u64) -> Result<Value> {
        let mut line = String::new();
        loop {
            line.clear();
            let read = self.stdout.read_line(&mut line).await?;
            if read == 0 {
                bail!("MCP server closed its stdout");
            }
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            let Ok(value) = serde_json::from_str::<Value>(trimmed) else {
                continue;
            };
            if value.get("id").and_then(Value::as_u64) != Some(id) {
                continue;
            }
            if let Some(error) = value.get("error") {
                let message = error
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown MCP error");
                bail!("MCP error: {message}");
            }
            return Ok(value.get("result").cloned().unwrap_or(Value::Null));
        }
    }
}

impl Drop for StdioClient {
    fn drop(&mut self) {
        let _ = self.child.start_kill();
    }
}

struct HttpClient {
    client: reqwest::Client,
    url: String,
    headers: BTreeMap<String, String>,
    session: Option<String>,
    next_id: u64,
}

impl HttpClient {
    fn new(url: String, headers: BTreeMap<String, String>) -> Self {
        Self {
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(120))
                .build()
                .unwrap_or_default(),
            url,
            headers,
            session: None,
            next_id: 1,
        }
    }

    fn request_builder(&self, body: &Value) -> reqwest::RequestBuilder {
        let mut req = self
            .client
            .post(&self.url)
            .header("accept", "application/json, text/event-stream")
            .header("mcp-protocol-version", PROTOCOL_VERSION)
            .json(body);
        for (key, value) in &self.headers {
            req = req.header(key, value);
        }
        if let Some(session) = &self.session {
            req = req.header("mcp-session-id", session);
        }
        req
    }

    async fn notify(&mut self, method: &str, params: Value) -> Result<()> {
        let body = json!({"jsonrpc": "2.0", "method": method, "params": params});
        let response = self.request_builder(&body).send().await?;
        self.capture_session(&response);
        let _ = response.bytes().await;
        Ok(())
    }

    async fn request(&mut self, method: &str, params: Value) -> Result<Value> {
        let id = self.next_id;
        self.next_id += 1;
        let body = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        });
        let response = self.request_builder(&body).send().await?;
        self.capture_session(&response);
        let status = response.status();
        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string();
        if !status.is_success() {
            let text = response.text().await.unwrap_or_default();
            bail!("MCP HTTP {status}: {text}");
        }
        if status == reqwest::StatusCode::ACCEPTED {
            return Ok(Value::Null);
        }

        if content_type.contains("text/event-stream") {
            let mut stream = response.bytes_stream();
            let mut buffer = String::new();
            while let Some(chunk) = stream.next().await {
                buffer.push_str(&String::from_utf8_lossy(&chunk?));
                if buffer.contains('\r') {
                    buffer = buffer.replace("\r\n", "\n").replace('\r', "");
                }
                while let Some(pos) = buffer.find("\n\n") {
                    let event: String = buffer.drain(..pos + 2).collect();
                    if let Some(value) = parse_sse_event(&event, id)? {
                        return Ok(value);
                    }
                }
            }
            if let Some(value) = parse_sse_event(&buffer, id)? {
                return Ok(value);
            }
            bail!("MCP SSE stream ended without a response");
        }

        let text = response.text().await?;
        let value: Value = serde_json::from_str(&text)
            .with_context(|| format!("MCP returned invalid JSON: {}", truncate(&text, 500)))?;
        extract_result(&value, id)
    }

    fn capture_session(&mut self, response: &reqwest::Response) {
        if let Some(session) = response
            .headers()
            .get("mcp-session-id")
            .and_then(|value| value.to_str().ok())
        {
            self.session = Some(session.to_string());
        }
    }
}

fn parse_sse_event(event: &str, id: u64) -> Result<Option<Value>> {
    let mut data = Vec::new();
    for line in event.lines() {
        let line = line.trim_start();
        if let Some(rest) = line.strip_prefix("data:") {
            data.push(rest.strip_prefix(' ').unwrap_or(rest));
        }
    }
    if data.is_empty() {
        return Ok(None);
    }
    let value: Value = serde_json::from_str(&data.join("\n"))?;
    if let Some(array) = value.as_array() {
        if array
            .iter()
            .any(|item| item.get("id").and_then(Value::as_u64) == Some(id))
        {
            return extract_result(&value, id).map(Some);
        }
        return Ok(None);
    }
    if value.get("id").and_then(Value::as_u64) == Some(id) {
        return extract_result(&value, id).map(Some);
    }
    Ok(None)
}

fn extract_result(value: &Value, id: u64) -> Result<Value> {
    if let Some(array) = value.as_array() {
        for item in array {
            if item.get("id").and_then(Value::as_u64) == Some(id) {
                return extract_result(item, id);
            }
        }
        bail!("MCP response did not contain id {id}");
    }
    if value.get("id").and_then(Value::as_u64) != Some(id) {
        bail!("unexpected MCP response id");
    }
    if let Some(error) = value.get("error") {
        let message = error
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("unknown MCP error");
        bail!("MCP error: {message}");
    }
    Ok(value.get("result").cloned().unwrap_or(Value::Null))
}

fn truncate(text: &str, max: usize) -> String {
    text.chars().take(max).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitizes_tool_names() {
        assert_eq!(sanitize("my.server/tool"), "my_server_tool");
    }

    #[test]
    fn expands_env_values() {
        // SAFETY: this test owns the variable for its duration.
        unsafe { std::env::set_var("HARNESS_MCP_TEST_TOKEN", "secret") };
        assert_eq!(
            expand_env("Bearer ${HARNESS_MCP_TEST_TOKEN}"),
            "Bearer secret"
        );
        assert_eq!(
            expand_env("Bearer $HARNESS_MCP_TEST_TOKEN"),
            "Bearer secret"
        );
        assert_eq!(
            expand_env("${HARNESS_MCP_MISSING}"),
            "${HARNESS_MCP_MISSING}"
        );
        assert_eq!(expand_env("plain"), "plain");
    }
}
