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
pub mod ui;

use anyhow::{Context, Result};
use cli::Cli;
use config::{Config, ProviderKind};
use provider::Provider;
use std::io::IsTerminal;
use tools::ToolRegistry;

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
    let skills = skills::SkillSet::discover(&cwd, &cfg.skills.paths);
    let mcp = mcp::McpManager::connect(&cfg.mcp, &cwd).await;
    let tools = ToolRegistry::new(
        cwd.clone(),
        skills.clone(),
        mcp,
        cfg.permissions.clone(),
        cfg.auto_approve,
        std::io::stdin().is_terminal(),
    );

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

    let api_key = config::resolve_api_key(&provider_name, &provider_cfg)?;
    let provider = Provider::new(
        provider_name.clone(),
        provider_cfg.clone(),
        model.clone(),
        api_key,
    )?;

    let interactive = std::io::stdin().is_terminal();
    let prompt_text = prompt_from_args(&cli.prompt, interactive)?;

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
        cfg.max_steps.unwrap_or(40),
    );

    match prompt_text {
        Some(text) if !text.trim().is_empty() => {
            agent.run_turn(text.trim()).await?;
        }
        _ => {
            if !interactive {
                anyhow::bail!(
                    "no prompt given. Pass a prompt, or pipe one in:\n  harness \"explain this repo\"\n  echo \"fix the tests\" | harness"
                );
            }
            repl::run(&mut agent).await?;
        }
    }

    Ok(())
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

fn kind_name(kind: ProviderKind) -> &'static str {
    match kind {
        ProviderKind::OpenAi => "openai",
        ProviderKind::Responses => "responses",
        ProviderKind::Anthropic => "anthropic",
    }
}
