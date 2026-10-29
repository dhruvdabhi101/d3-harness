pub mod agent;
pub mod cli;
pub mod config;
pub mod conversation;
pub mod mcp;
pub mod prompt;
pub mod provider;
pub mod repl;
pub mod skills;
pub mod tools;
pub mod tui;
pub mod ui;
pub mod ui_bridge;

use anyhow::{Context, Result};
use cli::Cli;
use config::{Config, ProviderKind};
use provider::{Provider, ProviderAuth};
use std::io::IsTerminal;
use std::sync::Arc;
use tools::ToolRegistry;
use ui_bridge::{ConsoleAgentUi, ConsoleToolUi};

pub async fn run_cli(cli: Cli) -> Result<()> {
    ui::init(cli.no_color);

    let cwd = match &cli.cwd {
        Some(dir) => dir.clone(),
        None => std::env::current_dir().context("reading current directory")?,
    };
    let cwd = cwd
        .canonicalize()
        .with_context(|| format!("opening workspace {}", cwd.display()))?;
    std::env::set_current_dir(&cwd)
        .with_context(|| format!("switching to workspace {}", cwd.display()))?;

    let mut cfg = Config::load(cli.config.as_deref(), &cwd)?;
    if let Some(provider) = &cli.provider {
        cfg.provider = Some(provider.clone());
    }
    if let Some(model) = &cli.model {
        cfg.model = Some(model.clone());
    }
    if let Some(steps) = cli.max_steps {
        cfg.max_steps = Some(steps);
    }
    if cli.yes {
        cfg.auto_approve = true;
    }

    if cli.list_providers {
        print_providers(&cfg);
        return Ok(());
    }

    let provider_name = cfg
        .provider
        .clone()
        .unwrap_or_else(|| "opencode".to_string());
    let provider_cfg = cfg
        .provider_config(&provider_name)
        .with_context(|| format!("unknown provider '{provider_name}'"))?;
    let model = cfg
        .model
        .clone()
        .or_else(|| provider_cfg.model.clone())
        .unwrap_or_else(|| "deepseek-v4.1-flash".to_string());
    let interactive = std::io::stdin().is_terminal();

    if cli.login {
        let auth = if provider_cfg.kind == Some(ProviderKind::ChatGpt) {
            let auth = config::codex_auth()?;
            ui::info("using the ChatGPT login saved by Codex CLI");
            ProviderAuth {
                key: auth.access_token,
                account_id: Some(auth.account_id),
            }
        } else {
            let key = config::prompt_and_store_key(&provider_name, interactive)?;
            ProviderAuth {
                key,
                account_id: None,
            }
        };
        let provider = Provider::new(
            provider_name.clone(),
            provider_cfg.clone(),
            model.clone(),
            auth,
        )?;
        match provider.list_models().await {
            Ok(models) => ui::info(&format!(
                "key verified; {} models are available",
                models.len()
            )),
            Err(error) => {
                let message = format!("{error:#}");
                if message.contains("401") || message.contains("403") {
                    let _ = config::remove_api_key(&provider_name);
                    anyhow::bail!(
                        "the provider rejected that API key, so it was not kept. Run `harness --login -p {provider_name}` to try again."
                    );
                }
                ui::warn(&format!(
                    "key saved, but the model list could not be verified: {message}"
                ));
            }
        }
        return Ok(());
    }

    if cli.list_models {
        let (auth, had_key) = provider_auth(&provider_name, &provider_cfg, interactive)?;
        let provider = Provider::new(
            provider_name.clone(),
            provider_cfg.clone(),
            model.clone(),
            auth,
        )?;
        let models = match provider.list_models().await {
            Ok(models) => models,
            Err(error) => {
                let message = format!("{error:#}");
                if !had_key && (message.contains("401") || message.contains("403")) {
                    let _ = config::remove_api_key(&provider_name);
                    anyhow::bail!(
                        "the provider rejected that API key, so it was not kept. Run `harness --login -p {provider_name}` to try again."
                    );
                }
                return Err(error);
            }
        };
        print_models(&models, &model);
        return Ok(());
    }

    let skills = skills::SkillSet::discover(&cwd, &cfg.skills.paths);
    let mcp = mcp::McpManager::connect(&cfg.mcp, &cwd).await;
    let mcp_count = mcp.summaries().len();

    let core = resolve_core_prompt(&cli, &cfg)?;
    let system = prompt::build_system_prompt(&prompt::PromptContext {
        core: &core,
        workspace: &cwd,
        model: &model,
        provider: &provider_name,
        skills: &skills,
        extra_instructions: &cfg.instructions,
        shell_permission: cfg.permissions.shell(),
    });

    if cli.dump_context {
        let tools = ToolRegistry::new(
            cwd.clone(),
            skills.clone(),
            mcp,
            cfg.permissions.clone(),
            cfg.auto_approve,
            Arc::new(ConsoleToolUi::new(false)),
        );
        let dump = serde_json::json!({
            "provider": provider_name,
            "model": model,
            "workspace": cwd,
            "system": system,
            "skills": skills.summaries(),
            "tools": tools.specs().iter().map(|s| serde_json::json!({
                "name": s.name,
                "description": s.description,
                "schema": s.schema,
            })).collect::<Vec<_>>(),
            "mcp_servers": tools.mcp_summaries(),
        });
        println!("{}", serde_json::to_string_pretty(&dump)?);
        return Ok(());
    }

    let (auth, had_key) = provider_auth(&provider_name, &provider_cfg, interactive)?;
    let provider = Provider::new(
        provider_name.clone(),
        provider_cfg.clone(),
        model.clone(),
        auth,
    )?;
    if !had_key && interactive {
        match provider.list_models().await {
            Ok(_) => ui::info("API key accepted"),
            Err(error) => {
                let message = format!("{error:#}");
                if message.contains("401") || message.contains("403") {
                    let _ = config::remove_api_key(&provider_name);
                    anyhow::bail!(
                        "the provider rejected the API key. Run `harness --login -p {provider_name}` to try again."
                    );
                }
            }
        }
    }

    let prompt_text = prompt_from_args(&cli.prompt, interactive)?;
    let max_steps = cfg.max_steps.unwrap_or(40);

    if let Some(text) = prompt_text.filter(|text| !text.trim().is_empty()) {
        let tools = ToolRegistry::new(
            cwd.clone(),
            skills,
            mcp,
            cfg.permissions.clone(),
            cfg.auto_approve,
            Arc::new(ConsoleToolUi::new(interactive)),
        );
        if !cli.quiet {
            ui::banner(&provider_name, &model, &cwd);
            tools.print_startup();
        }
        let mut agent = agent::Agent::new(
            provider,
            model,
            core,
            system,
            tools,
            max_steps,
            Arc::new(ConsoleAgentUi),
        );
        agent.run_turn(text.trim()).await?;
        return Ok(());
    }

    if !interactive {
        anyhow::bail!(
            "no prompt given. Pass a prompt, or pipe one in:\n  harness \"explain this repo\"\n  echo \"fix the tests\" | harness"
        );
    }

    if !cli.no_tui {
        let setup = tui::setup();
        let tools = ToolRegistry::new(
            cwd.clone(),
            skills,
            mcp,
            cfg.permissions.clone(),
            cfg.auto_approve,
            setup.tool_ui.clone(),
        );
        let tool_count = tools.specs().len();
        let info = tui::app::TuiInfo {
            provider: provider_name.clone(),
            model: model.clone(),
            workspace: cwd.clone(),
            branch: git_branch(&cwd),
            skills: tools.skill_names().len(),
            mcp: mcp_count,
            tools: tool_count,
            auto_approve: cfg.auto_approve,
        };
        let agent = agent::Agent::new(
            provider,
            model,
            core,
            system,
            tools,
            max_steps,
            setup.agent_ui.clone(),
        );
        return tui::run(agent, setup, info).await;
    }

    let tools = ToolRegistry::new(
        cwd.clone(),
        skills,
        mcp,
        cfg.permissions.clone(),
        cfg.auto_approve,
        Arc::new(ConsoleToolUi::new(true)),
    );
    if !cli.quiet {
        ui::banner(&provider_name, &model, &cwd);
        tools.print_startup();
    }
    let mut agent = agent::Agent::new(
        provider,
        model,
        core,
        system,
        tools,
        max_steps,
        Arc::new(ConsoleAgentUi),
    );
    repl::run(&mut agent).await?;
    Ok(())
}

fn git_branch(workspace: &std::path::Path) -> Option<String> {
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(workspace)
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let branch = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if branch.is_empty() || branch == "HEAD" {
        None
    } else {
        Some(branch)
    }
}

fn resolve_core_prompt(cli: &Cli, cfg: &Config) -> Result<String> {
    if let Some(text) = &cli.system_prompt {
        return Ok(text.clone());
    }
    if let Some(path) = &cli.system_file {
        return read_prompt_file(path);
    }
    if let Some(text) = &cfg.system_prompt {
        return Ok(text.clone());
    }
    if let Some(path) = &cfg.system_file {
        let path = config::expand_tilde(path);
        return read_prompt_file(&path);
    }
    Ok(prompt::DEFAULT_SYSTEM_PROMPT.to_string())
}

fn read_prompt_file(path: &std::path::Path) -> Result<String> {
    std::fs::read_to_string(path)
        .with_context(|| format!("reading system prompt {}", path.display()))
}

fn prompt_from_args(args: &[String], interactive: bool) -> Result<Option<String>> {
    if args.is_empty() {
        if !interactive {
            let mut input = String::new();
            use std::io::Read;
            std::io::stdin().read_to_string(&mut input)?;
            if !input.trim().is_empty() {
                return Ok(Some(input));
            }
        }
        return Ok(None);
    }
    if args.len() == 1 && args[0] == "-" {
        let mut input = String::new();
        use std::io::Read;
        std::io::stdin().read_to_string(&mut input)?;
        return Ok(Some(input));
    }
    Ok(Some(args.join(" ")))
}

fn print_providers(cfg: &Config) {
    println!(
        "{:<14} {:<10} {:<44} DEFAULT MODEL",
        "PROVIDER", "API", "BASE URL"
    );
    for preset in config::provider_presets() {
        let kind = kind_name(preset.kind.unwrap_or_default());
        let default = cfg
            .providers
            .get(&preset.name)
            .and_then(|p| p.model.clone())
            .or(preset.model.clone())
            .unwrap_or_default();
        println!(
            "{:<14} {:<10} {:<44} {}",
            preset.name,
            kind,
            preset.base_url.clone().unwrap_or_default(),
            default
        );
    }
    if !cfg.providers.is_empty() {
        println!("\nFrom harness.json:");
        for (name, p) in &cfg.providers {
            println!(
                "  {name} ({}) {}",
                kind_name(p.kind.unwrap_or_default()),
                p.base_url.clone().unwrap_or_default()
            );
        }
    }
}

pub fn parse_and_run() -> Result<()> {
    let cli = <Cli as clap::Parser>::parse();
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(run_cli(cli))
}

fn provider_auth(
    name: &str,
    config: &config::ProviderConfig,
    interactive: bool,
) -> anyhow::Result<(ProviderAuth, bool)> {
    if config.kind == Some(ProviderKind::ChatGpt) {
        if let (Some(key), Some(account_id)) = (&config.api_key, &config.account_id) {
            return Ok((
                ProviderAuth {
                    key: key.clone(),
                    account_id: Some(account_id.clone()),
                },
                true,
            ));
        }
        let auth = config::codex_auth()?;
        return Ok((
            ProviderAuth {
                key: auth.access_token,
                account_id: Some(auth.account_id),
            },
            true,
        ));
    }
    let had_key = config::find_api_key(name, config).is_some();
    let key = config::resolve_api_key(name, config, interactive)?;
    Ok((
        ProviderAuth {
            key,
            account_id: None,
        },
        had_key,
    ))
}

fn print_models(models: &[provider::ModelInfo], current: &str) {
    if models.is_empty() {
        println!("No models returned by the provider.");
        return;
    }
    println!("{:<36} NAME", "MODEL ID");
    for model in models {
        let marker = if model.id == current { "*" } else { " " };
        let name = model.name.as_deref().unwrap_or("");
        println!("{marker} {:<34} {}", model.id, name);
    }
    println!();
    println!("Current model is marked with *. Use -m <id> or /models to change it.");
}

fn kind_name(kind: ProviderKind) -> &'static str {
    match kind {
        ProviderKind::OpenAi => "openai",
        ProviderKind::Responses => "responses",
        ProviderKind::Anthropic => "anthropic",
        ProviderKind::ChatGpt => "chatgpt",
    }
}
