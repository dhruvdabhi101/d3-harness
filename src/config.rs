use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[derive(Default)]
pub enum Permission {
    Allow,
    #[default]
    Ask,
    Deny,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Permissions {
    pub shell: Option<Permission>,
    pub edit: Option<Permission>,
}

impl Permissions {
    pub fn shell(&self) -> Permission {
        self.shell.unwrap_or(Permission::Ask)
    }

    pub fn edit(&self) -> Permission {
        self.edit.unwrap_or(Permission::Allow)
    }

    fn merge(self, over: Permissions) -> Permissions {
        Permissions {
            shell: over.shell.or(self.shell),
            edit: over.edit.or(self.edit),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[derive(Default)]
pub enum ProviderKind {
    #[serde(
        alias = "openai",
        alias = "chat",
        alias = "openai-compatible",
        alias = "compatible"
    )]
    #[default]
    OpenAi,
    #[serde(alias = "codex", alias = "openai-responses")]
    Responses,
    Anthropic,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ProviderConfig {
    /// Kept so a preset can carry its own name through merging.
    #[serde(skip)]
    pub name: String,
    #[serde(rename = "type")]
    pub kind: Option<ProviderKind>,
    pub base_url: Option<String>,
    pub api_key: Option<String>,
    pub api_key_env: Option<String>,
    pub model: Option<String>,
    pub max_tokens: Option<u32>,
    pub headers: BTreeMap<String, String>,
}

impl ProviderConfig {
    fn merge(self, over: ProviderConfig) -> ProviderConfig {
        ProviderConfig {
            name: if over.name.is_empty() {
                self.name
            } else {
                over.name
            },
            kind: over.kind.or(self.kind),
            base_url: over.base_url.or(self.base_url),
            api_key: over.api_key.or(self.api_key),
            api_key_env: over.api_key_env.or(self.api_key_env),
            model: over.model.or(self.model),
            max_tokens: over.max_tokens.or(self.max_tokens),
            headers: {
                let mut headers = self.headers;
                headers.extend(over.headers);
                headers
            },
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct McpServerConfig {
    pub enabled: Option<bool>,
    #[serde(rename = "type")]
    pub kind: Option<String>,
    /// String form (`"command": "npx"`) or OpenCode's array form.
    pub command: Option<McpCommand>,
    pub args: Option<Vec<String>>,
    pub env: Option<BTreeMap<String, String>>,
    /// OpenCode calls this `environment`.
    pub environment: Option<BTreeMap<String, String>>,
    pub url: Option<String>,
    pub headers: Option<BTreeMap<String, String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum McpCommand {
    One(String),
    Many(Vec<String>),
}

impl McpServerConfig {
    pub fn is_enabled(&self) -> bool {
        self.enabled.unwrap_or(true)
    }

    pub fn is_http(&self) -> bool {
        matches!(
            self.kind.as_deref(),
            Some("http") | Some("remote") | Some("sse")
        ) || self.url.is_some()
    }

    /// Returns the command and its arguments, accepting both our shape and
    /// OpenCode's `"command": ["npx", "-y", "..."]` shape.
    pub fn command_parts(&self) -> Option<(String, Vec<String>)> {
        let mut parts = match self.command.clone()? {
            McpCommand::One(cmd) => vec![cmd],
            McpCommand::Many(parts) => parts,
        };
        if parts.is_empty() {
            return None;
        }
        let cmd = parts.remove(0);
        if let Some(extra) = &self.args {
            parts.extend(extra.iter().cloned());
        }
        Some((cmd, parts))
    }

    pub fn env_map(&self) -> BTreeMap<String, String> {
        let mut map = self.env.clone().unwrap_or_default();
        if let Some(env) = &self.environment {
            map.extend(env.clone());
        }
        map
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct McpConfig {
    pub servers: BTreeMap<String, McpServerConfig>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct SkillConfig {
    pub paths: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub provider: Option<String>,
    pub model: Option<String>,
    pub system_prompt: Option<String>,
    pub system_file: Option<String>,
    pub max_steps: Option<usize>,
    pub auto_approve: bool,
    pub permissions: Permissions,
    pub providers: BTreeMap<String, ProviderConfig>,
    pub mcp: McpConfig,
    pub skills: SkillConfig,
    /// Extra instruction files, relative to the workspace.
    pub instructions: Vec<String>,
}

impl Config {
    pub fn load(explicit: Option<&Path>, workspace: &Path) -> Result<Self> {
        let mut cfg = Config::default();

        if let Some(path) = explicit {
            let path = expand_tilde(path);
            cfg = cfg.merge(Config::from_file(&path)?);
        } else {
            if let Some(global) = global_config_path()
                && global.exists()
            {
                cfg = cfg.merge(Config::from_file(&global)?);
            }
            if let Some(project) = find_project_config(workspace) {
                cfg = cfg.merge(Config::from_file(&project)?);
            }
        }

        if let Ok(provider) = std::env::var("HARNESS_PROVIDER")
            && !provider.is_empty()
        {
            cfg.provider = Some(provider);
        }
        if let Ok(model) = std::env::var("HARNESS_MODEL")
            && !model.is_empty()
        {
            cfg.model = Some(model);
        }
        if let Ok(steps) = std::env::var("HARNESS_MAX_STEPS")
            && let Ok(steps) = steps.parse()
        {
            cfg.max_steps = Some(steps);
        }
        Ok(cfg)
    }

    fn from_file(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("reading config {}", path.display()))?;
        serde_json::from_str(&text).with_context(|| {
            format!(
                "parsing {} (harness.json must be valid JSON)",
                path.display()
            )
        })
    }

    fn merge(self, over: Config) -> Config {
        let mut providers = self.providers;
        for (name, provider) in over.providers {
            match providers.remove(&name) {
                Some(existing) => {
                    providers.insert(name, existing.merge(provider));
                }
                None => {
                    providers.insert(name, provider);
                }
            }
        }
        let mut mcp_servers = self.mcp.servers;
        mcp_servers.extend(over.mcp.servers);

        let mut skill_paths = self.skills.paths;
        skill_paths.extend(over.skills.paths);

        let mut instructions = self.instructions;
        instructions.extend(over.instructions);

        Config {
            provider: over.provider.or(self.provider),
            model: over.model.or(self.model),
            system_prompt: over.system_prompt.or(self.system_prompt),
            system_file: over.system_file.or(self.system_file),
            max_steps: over.max_steps.or(self.max_steps),
            auto_approve: self.auto_approve || over.auto_approve,
            permissions: self.permissions.merge(over.permissions),
            providers,
            mcp: McpConfig {
                servers: mcp_servers,
            },
            skills: SkillConfig { paths: skill_paths },
            instructions,
        }
    }

    pub fn provider_config(&self, name: &str) -> Result<ProviderConfig> {
        let preset = provider_presets().into_iter().find(|p| p.name == name);
        if let Some(over) = self.providers.get(name) {
            return Ok(match preset {
                Some(preset) => preset.merge(over.clone()),
                None => over.clone(),
            });
        }
        if let Some(preset) = preset {
            return Ok(preset);
        }
        let available = provider_presets()
            .iter()
            .map(|p| p.name.as_str())
            .chain(self.providers.keys().map(String::as_str))
            .collect::<Vec<_>>()
            .join(", ");
        bail!("unknown provider '{name}'. Known providers: {available}")
    }
}

fn global_config_path() -> Option<PathBuf> {
    dirs::config_dir().map(|dir| dir.join("harness").join("config.json"))
}

fn find_project_config(workspace: &Path) -> Option<PathBuf> {
    for dir in workspace.ancestors() {
        let direct = dir.join("harness.json");
        if direct.is_file() {
            return Some(direct);
        }
        let nested = dir.join(".harness").join("config.json");
        if nested.is_file() {
            return Some(nested);
        }
    }
    None
}

pub fn expand_tilde(path: impl AsRef<Path>) -> PathBuf {
    let path = path.as_ref();
    if let Ok(rest) = path.strip_prefix("~")
        && let Some(home) = dirs::home_dir()
    {
        return home.join(rest);
    }
    path.to_path_buf()
}

pub fn resolve_api_key(name: &str, cfg: &ProviderConfig) -> Result<String> {
    if let Ok(key) = std::env::var("HARNESS_API_KEY")
        && !key.is_empty()
    {
        return Ok(key);
    }
    if let Some(key) = &cfg.api_key
        && !key.is_empty()
    {
        return Ok(key.clone());
    }
    if let Some(var) = &cfg.api_key_env
        && let Ok(key) = std::env::var(var)
        && !key.is_empty()
    {
        return Ok(key);
    }
    if let Some(key) = key_from_opencode_auth(name) {
        return Ok(key);
    }
    if let Some(base) = &cfg.base_url
        && (base.contains("localhost") || base.contains("127.0.0.1"))
    {
        return Ok("local".to_string());
    }
    let env_name = cfg.api_key_env.as_deref().unwrap_or("HARNESS_API_KEY");
    bail!(
        "missing API key for provider '{name}'. Set {env_name} or add \"api_key\" to harness.json."
    )
}

fn key_from_opencode_auth(provider: &str) -> Option<String> {
    let mut candidates = Vec::new();
    if let Some(data) = dirs::data_dir() {
        candidates.push(data.join("opencode").join("auth.json"));
    }
    if let Some(config) = dirs::config_dir() {
        candidates.push(config.join("opencode").join("auth.json"));
    }
    if let Some(home) = dirs::home_dir() {
        candidates.push(home.join(".opencode").join("auth.json"));
    }
    let names = [provider, "opencode", "zen", "opencode-zen"];
    for path in candidates {
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) else {
            continue;
        };
        for name in names {
            if let Some(entry) = value.get(name)
                && let Some(key) = find_key(entry)
            {
                return Some(key);
            }
        }
    }
    None
}

fn find_key(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::String(s) if !s.is_empty() => Some(s.clone()),
        serde_json::Value::Object(map) => {
            for key in ["key", "apiKey", "api_key", "token"] {
                if let Some(serde_json::Value::String(s)) = map.get(key)
                    && !s.is_empty()
                {
                    return Some(s.clone());
                }
            }
            None
        }
        _ => None,
    }
}

pub fn provider_presets() -> Vec<ProviderConfig> {
    vec![
        preset(
            "opencode",
            ProviderKind::OpenAi,
            "https://opencode.ai/inference/openai/v1",
            "OPENCODE_API_KEY",
            "deepseek-v4.1-flash",
        ),
        preset(
            "openai",
            ProviderKind::Responses,
            "https://api.openai.com/v1",
            "OPENAI_API_KEY",
            "gpt-5.3-codex",
        ),
        preset(
            "codex",
            ProviderKind::Responses,
            "https://api.openai.com/v1",
            "OPENAI_API_KEY",
            "gpt-5.3-codex",
        ),
        preset(
            "anthropic",
            ProviderKind::Anthropic,
            "https://api.anthropic.com/v1",
            "ANTHROPIC_API_KEY",
            "claude-sonnet-4-5",
        ),
        preset(
            "openrouter",
            ProviderKind::OpenAi,
            "https://openrouter.ai/api/v1",
            "OPENROUTER_API_KEY",
            "anthropic/claude-sonnet-4.5",
        ),
        preset(
            "deepseek",
            ProviderKind::OpenAi,
            "https://api.deepseek.com/v1",
            "DEEPSEEK_API_KEY",
            "deepseek-chat",
        ),
        preset(
            "groq",
            ProviderKind::OpenAi,
            "https://api.groq.com/openai/v1",
            "GROQ_API_KEY",
            "llama-3.3-70b-versatile",
        ),
        preset(
            "mistral",
            ProviderKind::OpenAi,
            "https://api.mistral.ai/v1",
            "MISTRAL_API_KEY",
            "mistral-large-latest",
        ),
        preset(
            "xai",
            ProviderKind::OpenAi,
            "https://api.x.ai/v1",
            "XAI_API_KEY",
            "grok-4",
        ),
        preset(
            "ollama",
            ProviderKind::OpenAi,
            "http://localhost:11434/v1",
            "OLLAMA_API_KEY",
            "qwen3-coder",
        ),
        preset(
            "lmstudio",
            ProviderKind::OpenAi,
            "http://localhost:1234/v1",
            "LMSTUDIO_API_KEY",
            "local-model",
        ),
    ]
}

fn preset(
    name: &str,
    kind: ProviderKind,
    base_url: &str,
    api_key_env: &str,
    model: &str,
) -> ProviderConfig {
    ProviderConfig {
        name: name.to_string(),
        kind: Some(kind),
        base_url: Some(base_url.to_string()),
        api_key: None,
        api_key_env: Some(api_key_env.to_string()),
        model: Some(model.to_string()),
        max_tokens: None,
        headers: BTreeMap::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_config_overrides_provider_fields() {
        let base = Config::from_file_str(
            r#"{"provider":"opencode","providers":{"opencode":{"model":"a"}}}"#,
        )
        .unwrap();
        let over =
            Config::from_file_str(r#"{"providers":{"opencode":{"model":"b","max_tokens":123}}}"#)
                .unwrap();
        let merged = base.merge(over);
        let p = &merged.providers["opencode"];
        assert_eq!(p.model.as_deref(), Some("b"));
        assert_eq!(p.max_tokens, Some(123));
    }

    #[test]
    fn permission_defaults_are_safe() {
        let cfg = Config::from_file_str("{}").unwrap();
        assert_eq!(cfg.permissions.shell(), Permission::Ask);
        assert_eq!(cfg.permissions.edit(), Permission::Allow);
    }

    impl Config {
        fn from_file_str(text: &str) -> Result<Self> {
            Ok(serde_json::from_str(text)?)
        }
    }
}
