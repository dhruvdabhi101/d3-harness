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
    #[serde(alias = "chat-gpt", alias = "openai-chatgpt", alias = "codex-chatgpt")]
    ChatGpt,
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
    /// Optional endpoint that lists models. Defaults to `<base_url>/models`.
    pub models_url: Option<String>,
    pub api_key: Option<String>,
    pub api_key_env: Option<String>,
    /// ChatGPT account id paired with a manually supplied access token.
    pub account_id: Option<String>,
    pub model: Option<String>,
    pub max_tokens: Option<u32>,
    pub image_input: Option<bool>,
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
            models_url: over.models_url.or(self.models_url),
            api_key: over.api_key.or(self.api_key),
            api_key_env: over.api_key_env.or(self.api_key_env),
            account_id: over.account_id.or(self.account_id),
            model: over.model.or(self.model),
            max_tokens: over.max_tokens.or(self.max_tokens),
            image_input: over.image_input.or(self.image_input),
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
#[serde(default, deny_unknown_fields)]
pub struct UiConfig {
    pub accent: Option<String>,
    pub user_background: Option<String>,
    pub assistant_background: Option<String>,
    pub tool_output_expanded: Option<bool>,
    pub editor_max_height: Option<u16>,
}

impl UiConfig {
    fn merge(self, over: Self) -> Self {
        Self {
            accent: over.accent.or(self.accent),
            user_background: over.user_background.or(self.user_background),
            assistant_background: over.assistant_background.or(self.assistant_background),
            tool_output_expanded: over.tool_output_expanded.or(self.tool_output_expanded),
            editor_max_height: over.editor_max_height.or(self.editor_max_height),
        }
    }

    pub fn validate(&self) -> Result<()> {
        for color in [
            &self.accent,
            &self.user_background,
            &self.assistant_background,
        ]
        .into_iter()
        .flatten()
        {
            if color.len() != 7
                || !color.starts_with('#')
                || !color[1..].bytes().all(|byte| byte.is_ascii_hexdigit())
            {
                bail!("UI colors must use #RRGGBB, got '{color}'");
            }
        }
        if self
            .editor_max_height
            .is_some_and(|height| !(1..=20).contains(&height))
        {
            bail!("ui.editor_max_height must be between 1 and 20");
        }
        Ok(())
    }
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
    pub ui: UiConfig,
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
            if let Some(path) = selection_path()
                && path.exists()
            {
                cfg = cfg.merge(Config::from_file(&path)?);
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
        cfg.ui.validate()?;
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
            ui: self.ui.merge(over.ui),
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

fn selection_path() -> Option<PathBuf> {
    dirs::config_dir().map(|dir| dir.join("harness").join("selection.json"))
}

pub fn store_selection(provider: &str, model: &str) -> Result<()> {
    let path = selection_path().context("finding user config directory")?;
    write_selection(&path, provider, model)
}

fn write_selection(path: &Path, provider: &str, model: &str) -> Result<()> {
    write_private_json(
        path,
        &serde_json::json!({"provider": provider, "model": model}),
    )
    .with_context(|| format!("saving selection to {}", path.display()))
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

pub fn resolve_api_key(name: &str, cfg: &ProviderConfig, interactive: bool) -> Result<String> {
    if let Some(key) = find_api_key(name, cfg) {
        return Ok(key);
    }
    if cfg.api_key.is_none()
        && cfg.api_key_env.is_none()
        && let Some(base) = &cfg.base_url
        && (base.contains("localhost") || base.contains("127.0.0.1"))
    {
        return Ok("local".to_string());
    }
    if interactive {
        return prompt_and_store_key(name, interactive);
    }
    let env_name = cfg.api_key_env.as_deref().unwrap_or("HARNESS_API_KEY");
    bail!(
        "missing API key for provider '{name}'. Set {env_name}, add \"api_key\" to harness.json, or run `d3 --login -p {name}`."
    )
}

pub fn find_api_key(name: &str, cfg: &ProviderConfig) -> Option<String> {
    if let Ok(key) = std::env::var("HARNESS_API_KEY")
        && !key.is_empty()
    {
        return Some(key);
    }
    if let Some(key) = &cfg.api_key
        && !key.is_empty()
    {
        return Some(key.clone());
    }
    if let Some(var) = &cfg.api_key_env
        && let Ok(key) = std::env::var(var)
        && !key.is_empty()
    {
        return Some(key);
    }
    if let Some(key) = stored_api_key(name) {
        return Some(key);
    }
    if matches!(name, "opencode" | "zen" | "opencode-zen")
        && let Some(key) = key_from_opencode_auth(name)
    {
        return Some(key);
    }
    None
}

pub fn prompt_and_store_key(name: &str, interactive: bool) -> Result<String> {
    if !interactive {
        bail!("no terminal available to ask for an API key");
    }
    let hint = match name {
        "opencode" => "Get a key at https://opencode.ai/auth.",
        "openai" | "codex" => "Get a key at https://platform.openai.com/api-keys.",
        "anthropic" => "Get a key at https://console.anthropic.com/settings/keys.",
        "openrouter" => "Get a key at https://openrouter.ai/keys.",
        _ => "Check your provider's dashboard for an API key.",
    };
    eprintln!();
    eprintln!(
        "{} {}",
        crate::ui::yellow("No API key found for"),
        crate::ui::bold(name)
    );
    eprintln!("{hint}");
    let key = crate::ui::prompt_secret("Paste the API key (hidden): ")?;
    let key = key.trim().to_string();
    if key.is_empty() {
        bail!("no API key entered");
    }
    match store_api_key(name, &key) {
        Ok(path) => crate::ui::info(&format!("saved key to {}", path.display())),
        Err(error) => crate::ui::warn(&format!("could not save the key: {error:#}")),
    }
    Ok(key)
}

pub fn auth_path() -> Option<PathBuf> {
    dirs::config_dir().map(|dir| dir.join("harness").join("auth.json"))
}

pub fn stored_api_key(provider: &str) -> Option<String> {
    let path = auth_path()?;
    let text = std::fs::read_to_string(path).ok()?;
    let value = serde_json::from_str::<serde_json::Value>(&text).ok()?;
    value.get(provider).and_then(find_key)
}

pub fn store_api_key(provider: &str, key: &str) -> Result<PathBuf> {
    let path = auth_path().context("could not determine a config directory")?;
    write_auth_key(&path, provider, key)?;
    Ok(path)
}

fn write_auth_key(path: &Path, provider: &str, key: &str) -> Result<()> {
    let mut value = read_auth_value(path);
    value[provider] = serde_json::json!({"type": "api", "key": key});
    write_private_json(path, &value)
}

pub fn remove_api_key(provider: &str) -> Result<()> {
    let Some(path) = auth_path() else {
        return Ok(());
    };
    if !path.is_file() {
        return Ok(());
    }
    let mut value = read_auth_value(&path);
    if let Some(map) = value.as_object_mut() {
        map.remove(provider);
    }
    write_private_json(&path, &value)
}

fn read_auth_value(path: &Path) -> serde_json::Value {
    let value = std::fs::read_to_string(path)
        .ok()
        .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
        .unwrap_or_else(|| serde_json::json!({}));
    if value.is_object() {
        value
    } else {
        serde_json::json!({})
    }
}

fn write_private_json(path: &Path, value: &serde_json::Value) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700));
        }
    }
    let temp = path.with_extension(format!("tmp.{}", std::process::id()));
    std::fs::write(&temp, serde_json::to_string_pretty(value)?)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&temp, std::fs::Permissions::from_mode(0o600))?;
    }
    std::fs::rename(&temp, path)?;
    Ok(())
}

pub struct ChatGptAuth {
    pub access_token: String,
    pub account_id: String,
}

pub fn codex_auth() -> Result<ChatGptAuth> {
    if let (Ok(token), Ok(account)) = (
        std::env::var("CODEX_ACCESS_TOKEN"),
        std::env::var("CODEX_ACCOUNT_ID"),
    ) && !token.is_empty()
        && !account.is_empty()
    {
        return Ok(ChatGptAuth {
            access_token: token,
            account_id: account,
        });
    }

    let home = dirs::home_dir().context("could not find the home directory")?;
    let codex_home = std::env::var("CODEX_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| home.join(".codex"));
    let path = codex_home.join("auth.json");
    let text = std::fs::read_to_string(&path).with_context(|| {
        format!(
            "Codex is not logged in ({} not found). Run `codex login`, or use `-p openai` with an API key.",
            path.display()
        )
    })?;
    let value: serde_json::Value =
        serde_json::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
    let tokens = value
        .get("tokens")
        .context("Codex auth has no ChatGPT tokens. Run `codex login`, or use `-p openai`.")?;
    let access_token = tokens
        .get("access_token")
        .and_then(serde_json::Value::as_str)
        .filter(|token| !token.is_empty())
        .context("Codex auth has no access token. Run `codex login` again.")?
        .to_string();
    let account_id = tokens
        .get("account_id")
        .and_then(serde_json::Value::as_str)
        .filter(|account| !account.is_empty())
        .context("Codex auth has no ChatGPT account id. Run `codex login` again.")?
        .to_string();
    Ok(ChatGptAuth {
        access_token,
        account_id,
    })
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
        {
            let mut opencode = preset(
                "opencode",
                ProviderKind::OpenAi,
                "https://opencode.ai/inference/openai/v1",
                "OPENCODE_API_KEY",
                "deepseek-v4.1-flash",
            );
            opencode.models_url = Some("https://opencode.ai/inference/v1/models".to_string());
            opencode
        },
        preset(
            "chatgpt",
            ProviderKind::ChatGpt,
            "https://chatgpt.com/backend-api/codex",
            "",
            "gpt-5.6-sol",
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
            "",
            "qwen3-coder",
        ),
        preset(
            "lmstudio",
            ProviderKind::OpenAi,
            "http://localhost:1234/v1",
            "",
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
        models_url: None,
        api_key: None,
        account_id: None,
        api_key_env: if api_key_env.is_empty() {
            None
        } else {
            Some(api_key_env.to_string())
        },
        model: Some(model.to_string()),
        max_tokens: None,
        image_input: None,
        headers: BTreeMap::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ui_settings_merge_individually_and_reject_invalid_values() {
        let base: Config = serde_json::from_value(
            serde_json::json!({"ui": {"accent": "#123456", "tool_output_expanded": true}}),
        )
        .unwrap();
        let over: Config = serde_json::from_value(
            serde_json::json!({"ui": {"editor_max_height": 3, "tool_output_expanded": false}}),
        )
        .unwrap();
        let merged = base.merge(over);
        assert_eq!(merged.ui.accent.as_deref(), Some("#123456"));
        assert_eq!(merged.ui.tool_output_expanded, Some(false));
        assert_eq!(merged.ui.editor_max_height, Some(3));
        assert!(merged.ui.validate().is_ok());
        for ui in [
            serde_json::json!({"accent": "orange"}),
            serde_json::json!({"editor_max_height": 0}),
            serde_json::json!({"editor_max_height": 21}),
        ] {
            assert!(
                serde_json::from_value::<UiConfig>(ui)
                    .unwrap()
                    .validate()
                    .is_err()
            );
        }
    }

    #[test]
    fn selection_round_trips_and_overrides_global_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("selection.json");
        write_selection(&path, "openai", "chosen-model").unwrap();
        let global = Config::from_file_str(r#"{"provider":"opencode","model":"default"}"#).unwrap();
        let loaded = global.merge(Config::from_file(&path).unwrap());
        assert_eq!(loaded.provider.as_deref(), Some("openai"));
        assert_eq!(loaded.model.as_deref(), Some("chosen-model"));
        let project = Config::from_file_str(r#"{"model":"project-model"}"#).unwrap();
        assert_eq!(
            loaded.merge(project).model.as_deref(),
            Some("project-model")
        );
    }

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
    fn auth_file_is_private_and_preserves_other_providers() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("harness/auth.json");
        write_auth_key(&path, "opencode", "one").unwrap();
        write_auth_key(&path, "openai", "two").unwrap();
        let value: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(value["opencode"]["key"], "one");
        assert_eq!(value["openai"]["key"], "two");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
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
