pub mod agent;
pub mod bench;
pub mod cli;
pub mod config;
pub mod conversation;
pub mod image;
pub mod mcp;
pub mod prompt;
pub mod provider;
pub mod repl;
pub mod session;
pub mod skills;
pub mod tools;
pub mod tui;
pub mod ui;
pub mod ui_bridge;
mod update;

use anyhow::{Context, Result, bail};
use cli::{AuthAction, Cli, Command};
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

    if cli.list_sessions {
        let sessions = session::Session::new(&cwd)?.list()?;
        for session in sessions {
            println!("{}\t{}", session.id, session.title);
        }
        return Ok(());
    }

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

    if let Some(command) = cli.command.clone() {
        return run_command(command, &cli, &cfg, &cwd).await;
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
        return auth_login(
            &provider_name,
            &provider_cfg,
            &model,
            interactive,
            false,
            false,
        )
        .await;
    }

    if cli.logout {
        return auth_logout(Some(&provider_name), false);
    }

    if cli.check_auth {
        return auth_check(Some(&provider_name), &cfg, false, false).await;
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
                        "the provider rejected that API key, so it was not kept. Run `d3 --login -p {provider_name}` to try again."
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

    let prompt_text = prompt_from_args(&cli.prompt, interactive)?;
    let max_steps = cfg.max_steps.unwrap_or(1000);
    let mut session = session::Session::new(&cwd)?;
    let messages = if let Some(id) = &cli.resume {
        session.resume(if id.is_empty() { None } else { Some(id) })?
    } else if cli.r#continue {
        session.resume_latest()?
    } else {
        Vec::new()
    };

    if let Some(text) = prompt_text.filter(|text| !text.trim().is_empty()) {
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
                            "the provider rejected the API key. Run `d3 --login -p {provider_name}` to try again."
                        );
                    }
                }
            }
        }
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
        agent.set_session(session, messages);
        agent.run_turn(text.trim()).await?;
        return Ok(());
    }

    if !interactive {
        anyhow::bail!(
            "no prompt given. Pass a prompt, or pipe one in:\n  d3 \"explain this repo\"\n  echo \"fix the tests\" | d3"
        );
    }

    if !cli.no_tui {
        let setup = tui::setup();
        let update_ui = setup.agent_ui.clone();
        tokio::spawn(async move {
            if let Some(version) = update::check().await {
                update_ui.info(&format!(
                    "d3 {version} is available. Run the install command in the README to update."
                ));
            }
        });
        let tools = ToolRegistry::new(
            cwd.clone(),
            skills,
            mcp,
            cfg.permissions.clone(),
            cfg.auto_approve,
            setup.tool_ui.clone(),
        );
        let tool_count = tools.specs().len();
        let connected = provider_connected(&provider_name, &provider_cfg);
        let auth = if connected {
            provider_auth(&provider_name, &provider_cfg, false)?.0
        } else {
            ProviderAuth {
                key: String::new(),
                account_id: None,
            }
        };
        let provider = Provider::new(
            provider_name.clone(),
            provider_cfg.clone(),
            model.clone(),
            auth,
        )?;
        let tool_names = tools
            .specs()
            .iter()
            .map(|spec| spec.name.clone())
            .collect::<Vec<_>>();
        let mcp_servers = tools
            .mcp_summaries()
            .iter()
            .filter_map(|summary| {
                summary
                    .get("name")
                    .and_then(|name| name.as_str())
                    .map(str::to_string)
            })
            .collect::<Vec<_>>();
        let info = tui::app::TuiInfo {
            provider: provider_name.clone(),
            model: model.clone(),
            workspace: cwd.clone(),
            branch: git_branch(&cwd),
            skills: tools.skill_names(),
            tool_names,
            mcp: mcp_count,
            mcp_servers,
            tools: tool_count,
            auto_approve: cfg.auto_approve,
            connected,
            shell_permission: cfg.permissions.shell(),
            edit_permission: cfg.permissions.edit(),
            providers: provider_choices(&cfg),
            ui: cfg.ui.clone(),
        };
        let mut agent = agent::Agent::new(
            provider,
            model,
            core,
            system,
            tools,
            max_steps,
            setup.agent_ui.clone(),
        );
        agent.set_session(session, messages);
        return tui::run(agent, setup, info).await;
    }

    let (auth, _) = provider_auth(&provider_name, &provider_cfg, interactive)?;
    let provider = Provider::new(
        provider_name.clone(),
        provider_cfg.clone(),
        model.clone(),
        auth,
    )?;
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
    agent.set_session(session, messages);
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

async fn run_command(
    command: Command,
    _cli: &Cli,
    cfg: &Config,
    cwd: &std::path::Path,
) -> Result<()> {
    match command {
        Command::Auth { action } => run_auth(action, cfg).await,
        Command::Models { search, json } => {
            let (name, provider_cfg, model) = selected_provider(cfg)?;
            list_models_command(&name, &provider_cfg, &model, search.as_deref(), json).await
        }
        Command::Sessions { json } => list_sessions_command(cwd, json),
        Command::Config { json } => print_config(cfg, cwd, json),
        Command::Mcp { json } => list_mcp_command(cfg, cwd, json).await,
        Command::Init { force } => init_agents_md(cwd, force),
        Command::Update => {
            match update::check().await {
                Some(version) => {
                    println!("d3 {version} is available; re-run the install command to update.")
                }
                None => println!("d3 {} is up to date.", env!("CARGO_PKG_VERSION")),
            }
            Ok(())
        }
        Command::Bench(args) => bench_command(cfg, &args).await,
        Command::Version => {
            println!("d3 {}", env!("CARGO_PKG_VERSION"));
            println!(
                "target: {} {}",
                std::env::consts::OS,
                std::env::consts::ARCH
            );
            println!("workspace: {}", cwd.display());
            Ok(())
        }
    }
}

async fn run_auth(action: AuthAction, cfg: &Config) -> Result<()> {
    match action {
        AuthAction::Login {
            provider,
            api_key,
            no_browser: _,
        } => {
            let interactive = std::io::stdin().is_terminal();
            let name = match provider {
                Some(name) => name,
                None => choose_provider(cfg, interactive, "Sign in to which provider?")?,
            };
            let provider_cfg = cfg
                .provider_config(&name)
                .with_context(|| format!("unknown provider '{name}'"))?;
            let model = cfg
                .model
                .clone()
                .or_else(|| provider_cfg.model.clone())
                .unwrap_or_else(|| default_model_name(&provider_cfg));
            auth_login(&name, &provider_cfg, &model, interactive, api_key, false).await
        }
        AuthAction::Logout { provider, all } => {
            if all {
                let providers = config::stored_providers();
                if providers.is_empty() {
                    ui::info("no stored provider credentials");
                    return Ok(());
                }
                for provider in providers {
                    auth_logout(Some(&provider), true)?;
                }
                return Ok(());
            }
            let name = match provider {
                Some(name) => name,
                None => cfg
                    .provider
                    .clone()
                    .unwrap_or_else(|| "opencode".to_string()),
            };
            auth_logout(Some(&name), false)
        }
        AuthAction::Check {
            provider,
            json,
            verify,
        } => auth_check(provider.as_deref(), cfg, json, verify).await,
        AuthAction::List => auth_list(cfg),
    }
}

async fn auth_login(
    name: &str,
    provider_cfg: &config::ProviderConfig,
    model: &str,
    interactive: bool,
    force_api_key: bool,
    _no_browser: bool,
) -> Result<()> {
    if provider_cfg.kind == Some(ProviderKind::ChatGpt) && !force_api_key {
        let existing = config::codex_auth().ok();
        let auth = if !interactive {
            match existing {
                Some(auth) => {
                    ui::info("using the ChatGPT login saved by Codex CLI");
                    auth
                }
                None => {
                    bail!("ChatGPT is not signed in. Run `d3 auth login chatgpt` in a terminal.")
                }
            }
        } else {
            ui::info("starting the ChatGPT device sign-in…");
            match std::process::Command::new("codex")
                .args(["login", "--device-auth"])
                .status()
            {
                Ok(status) if status.success() => config::codex_auth()?,
                Ok(_) => bail!("ChatGPT sign-in did not complete"),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => match existing {
                    Some(auth) => {
                        ui::warn(
                            "the `codex` CLI is not installed; using the ChatGPT login it left on disk",
                        );
                        auth
                    }
                    None => bail!(
                        "the `codex` CLI is required for ChatGPT sign-in. Install it with `npm install -g @openai/codex`."
                    ),
                },
                Err(error) => {
                    return Err(error).context("starting `codex login --device-auth`");
                }
            }
        };
        let auth = ProviderAuth {
            key: auth.access_token,
            account_id: Some(auth.account_id),
        };
        let provider = Provider::new(
            name.to_string(),
            provider_cfg.clone(),
            model.to_string(),
            auth,
        )?;
        match provider.list_models().await {
            Ok(models) => ui::info(&format!(
                "signed in to {name}; {} models are available (current: {model})",
                models.len()
            )),
            Err(error) => ui::warn(&format!(
                "signed in, but the model list could not be verified: {error:#}"
            )),
        }
        return Ok(());
    }

    let key = if interactive {
        config::prompt_key(name)?
    } else {
        config::find_api_key(name, provider_cfg).with_context(|| {
            format!(
                "missing API key for provider '{name}'. Set the provider environment variable or run `d3 auth login {name}` in a terminal."
            )
        })?
    };
    let provider = Provider::new(
        name.to_string(),
        provider_cfg.clone(),
        model.to_string(),
        ProviderAuth {
            key: key.clone(),
            account_id: provider_cfg.account_id.clone(),
        },
    )?;
    match provider.list_models().await {
        Ok(models) => {
            let path = config::store_api_key(name, &key)?;
            ui::info(&format!(
                "signed in to {name}; {} models are available (current: {model})",
                models.len()
            ));
            ui::info(&format!("credential saved to {}", path.display()));
        }
        Err(error) => {
            let message = format!("{error:#}");
            if is_auth_error(&message) {
                bail!(
                    "the provider rejected that API key, so it was not saved. Check the key and run `d3 auth login {name}` again."
                );
            }
            let path = config::store_api_key(name, &key)?;
            ui::warn(&format!(
                "credential saved to {}, but it could not be verified: {message}",
                path.display()
            ));
        }
    }
    Ok(())
}

fn auth_logout(provider: Option<&str>, quiet: bool) -> Result<()> {
    let name = provider.context("no provider selected")?;
    let removed = config::logout(name)?;
    let kind = config::provider_presets()
        .into_iter()
        .find(|preset| preset.name == name)
        .and_then(|preset| preset.kind);
    if kind == Some(ProviderKind::ChatGpt) {
        ui::info(
            "ChatGPT credentials are managed by Codex CLI; run `codex logout` to remove them.",
        );
    }
    if removed {
        if !quiet {
            ui::info(&format!("removed the stored credential for {name}"));
        }
    } else if !quiet {
        ui::info(&format!(
            "no stored credential for {name}; environment and harness.json credentials are unchanged"
        ));
    }
    Ok(())
}

async fn auth_check(provider: Option<&str>, cfg: &Config, json: bool, verify: bool) -> Result<()> {
    let names = match provider {
        Some(name) => vec![name.to_string()],
        None => provider_names(cfg),
    };
    let mut checks = Vec::new();
    let mut missing = Vec::new();
    for name in names {
        let provider_cfg = match cfg.provider_config(&name) {
            Ok(config) => config,
            Err(_) => {
                if provider.is_some() {
                    return Err(anyhow::anyhow!("unknown provider '{name}'"));
                }
                continue;
            }
        };
        let mut status = config::auth_status(&name, &provider_cfg);
        if verify && status.ready {
            let auth = if provider_cfg.kind == Some(ProviderKind::ChatGpt) {
                match config::codex_auth() {
                    Ok(auth) => Some(ProviderAuth {
                        key: auth.access_token,
                        account_id: Some(auth.account_id),
                    }),
                    Err(_) => None,
                }
            } else {
                config::find_api_key(&name, &provider_cfg).map(|key| ProviderAuth {
                    key,
                    account_id: None,
                })
            };
            if let Some(auth) = auth {
                let model = provider_cfg
                    .model
                    .clone()
                    .unwrap_or_else(|| default_model_name(&provider_cfg));
                match Provider::new(name.clone(), provider_cfg.clone(), model, auth) {
                    Ok(provider) => match provider.list_models().await {
                        Ok(models) => {
                            status.ready = true;
                            checks.push(serde_json::json!({
                                "provider": name,
                                "kind": kind_name(status.kind),
                                "ready": true,
                                "source": status.source.map(|source| source.label()),
                                "models": models.len(),
                            }));
                            continue;
                        }
                        Err(error) => {
                            status.ready = false;
                            checks.push(serde_json::json!({
                                "provider": name,
                                "kind": kind_name(status.kind),
                                "ready": false,
                                "source": status.source.map(|source| source.label()),
                                "error": format!("{error:#}"),
                            }));
                            continue;
                        }
                    },
                    Err(error) => {
                        status.ready = false;
                        checks.push(serde_json::json!({
                            "provider": name,
                            "kind": kind_name(status.kind),
                            "ready": false,
                            "error": format!("{error:#}"),
                        }));
                        continue;
                    }
                }
            }
        }
        if !status.ready {
            missing.push(name.clone());
        }
        checks.push(serde_json::json!({
            "provider": name,
            "kind": kind_name(status.kind),
            "ready": status.ready,
            "source": status.source.map(|source| source.label()),
            "stored": status.stored,
        }));
    }

    if json {
        println!("{}", serde_json::to_string_pretty(&checks)?);
    } else {
        println!("{:<14} {:<10} {:<9} SOURCE", "PROVIDER", "API", "STATUS");
        for check in &checks {
            println!(
                "{:<14} {:<10} {:<9} {}",
                check["provider"].as_str().unwrap_or(""),
                check["kind"].as_str().unwrap_or(""),
                if check["ready"].as_bool().unwrap_or(false) {
                    "ready"
                } else {
                    "missing"
                },
                check["source"].as_str().unwrap_or("-"),
            );
        }
    }
    if provider.is_some() && !missing.is_empty() {
        bail!(
            "provider '{}' is not signed in. Run `d3 auth login {}`.",
            missing[0],
            missing[0]
        );
    }
    Ok(())
}

fn auth_list(cfg: &Config) -> Result<()> {
    println!("{:<14} {:<10} {:<9} SOURCE", "PROVIDER", "API", "STATUS");
    for name in provider_names(cfg) {
        let Ok(provider_cfg) = cfg.provider_config(&name) else {
            continue;
        };
        let status = config::auth_status(&name, &provider_cfg);
        println!(
            "{:<14} {:<10} {:<9} {}",
            name,
            kind_name(status.kind),
            if status.ready { "ready" } else { "missing" },
            status.source.map(|source| source.label()).unwrap_or("-"),
        );
    }
    Ok(())
}

async fn list_models_command(
    name: &str,
    provider_cfg: &config::ProviderConfig,
    model: &str,
    search: Option<&str>,
    json: bool,
) -> Result<()> {
    let interactive = std::io::stdin().is_terminal();
    let (auth, had_key) = provider_auth(name, provider_cfg, interactive)?;
    let provider = Provider::new(
        name.to_string(),
        provider_cfg.clone(),
        model.to_string(),
        auth,
    )?;
    let models = match provider.list_models().await {
        Ok(models) => models,
        Err(error) => {
            let message = format!("{error:#}");
            if !had_key && is_auth_error(&message) {
                let _ = config::remove_api_key(name);
                bail!(
                    "the provider rejected that API key, so it was not kept. Run `d3 auth login {name}` to try again."
                );
            }
            return Err(error);
        }
    };
    let models = models
        .into_iter()
        .filter(|entry| match search {
            Some(search) => {
                let search = search.to_ascii_lowercase();
                entry.id.to_ascii_lowercase().contains(&search)
                    || entry
                        .name
                        .as_deref()
                        .is_some_and(|name| name.to_ascii_lowercase().contains(&search))
            }
            None => true,
        })
        .collect::<Vec<_>>();
    if json {
        let value = models
            .iter()
            .map(|entry| {
                serde_json::json!({
                    "id": entry.id,
                    "name": entry.name,
                    "image_input": entry.image_input,
                })
            })
            .collect::<Vec<_>>();
        println!("{}", serde_json::to_string_pretty(&value)?);
        return Ok(());
    }
    print_models(&models, model);
    Ok(())
}

fn list_sessions_command(cwd: &std::path::Path, json: bool) -> Result<()> {
    let sessions = session::Session::new(cwd)?.list()?;
    if json {
        let value = sessions
            .iter()
            .map(|session| serde_json::json!({"id": session.id, "title": session.title}))
            .collect::<Vec<_>>();
        println!("{}", serde_json::to_string_pretty(&value)?);
    } else {
        for session in sessions {
            println!("{}\t{}", session.id, session.title);
        }
    }
    Ok(())
}

fn print_config(cfg: &Config, cwd: &std::path::Path, json: bool) -> Result<()> {
    let auth = config::auth_path().map(|path| path.display().to_string());
    let selection = config::selection_path().map(|path| path.display().to_string());
    let global = config::global_config_path().map(|path| path.display().to_string());
    let project = config::find_project_config(cwd).map(|path| path.display().to_string());
    if json {
        let value = serde_json::json!({
            "workspace": cwd.display().to_string(),
            "provider": cfg.provider,
            "model": cfg.model,
            "max_steps": cfg.max_steps,
            "auto_approve": cfg.auto_approve,
            "permissions": {
                "shell": cfg.permissions.shell(),
                "edit": cfg.permissions.edit(),
            },            "ui": cfg.ui,
            "providers": cfg.providers.keys().collect::<Vec<_>>(),
            "skills": cfg.skills,
            "instructions": cfg.instructions,
            "auth_path": auth,
            "selection_path": selection,
            "global_config": global,
            "project_config": project,
        });
        println!("{}", serde_json::to_string_pretty(&value)?);
        return Ok(());
    }
    println!("workspace       {}", cwd.display());
    println!(
        "provider        {}",
        cfg.provider.as_deref().unwrap_or("opencode (default)")
    );
    println!(
        "model           {}",
        cfg.model.as_deref().unwrap_or("provider default")
    );
    println!("max steps       {}", cfg.max_steps.unwrap_or(1000));
    println!("auto approve    {}", cfg.auto_approve);
    println!(
        "permissions     shell={:?} edit={:?}",
        cfg.permissions.shell(),
        cfg.permissions.edit()
    );
    println!(
        "custom providers {}",
        if cfg.providers.is_empty() {
            "none".to_string()
        } else {
            cfg.providers.keys().cloned().collect::<Vec<_>>().join(", ")
        }
    );
    for (label, path) in [
        ("global config", global),
        ("project config", project),
        ("selection", selection),
        ("auth", auth),
    ] {
        if let Some(path) = path {
            println!("{label:<15} {path}");
        }
    }
    Ok(())
}

async fn list_mcp_command(cfg: &Config, cwd: &std::path::Path, json: bool) -> Result<()> {
    let mcp = mcp::McpManager::connect(&cfg.mcp, cwd).await;
    let summaries = mcp.summaries();
    if json {
        println!("{}", serde_json::to_string_pretty(&summaries)?);
    } else if summaries.is_empty() {
        println!("No MCP servers configured.");
    } else {
        for summary in summaries {
            println!("{}", serde_json::to_string(&summary).unwrap_or_default());
        }
    }
    Ok(())
}

fn init_agents_md(cwd: &std::path::Path, force: bool) -> Result<()> {
    let path = cwd.join("AGENTS.md");
    if path.exists() && !force {
        bail!(
            "{} already exists. Pass --force to overwrite it.",
            path.display()
        );
    }
    std::fs::write(&path, prompt::AGENTS_TEMPLATE)
        .with_context(|| format!("writing {}", path.display()))?;
    println!("wrote {}", path.display());
    Ok(())
}

async fn bench_command(cfg: &Config, args: &cli::BenchArgs) -> Result<()> {
    let (name, provider_cfg, model) = selected_provider(cfg)?;
    let interactive = std::io::stdin().is_terminal();
    let (auth, _) = provider_auth(&name, &provider_cfg, interactive)?;
    let report = bench::run(cfg, &name, &provider_cfg, &model, auth, args).await?;
    bench::print_report(&report, args.json)?;
    if report.passed != report.total {
        bail!(
            "benchmark failed: {}/{} tasks passed",
            report.passed,
            report.total
        );
    }
    Ok(())
}

fn selected_provider(cfg: &Config) -> Result<(String, config::ProviderConfig, String)> {
    let name = cfg
        .provider
        .clone()
        .unwrap_or_else(|| "opencode".to_string());
    let provider_cfg = cfg
        .provider_config(&name)
        .with_context(|| format!("unknown provider '{name}'"))?;
    let model = cfg
        .model
        .clone()
        .or_else(|| provider_cfg.model.clone())
        .unwrap_or_else(|| default_model_name(&provider_cfg));
    Ok((name, provider_cfg, model))
}

fn default_model_name(provider_cfg: &config::ProviderConfig) -> String {
    match provider_cfg.kind.unwrap_or_default() {
        ProviderKind::ChatGpt => "gpt-5.6-sol".to_string(),
        ProviderKind::Anthropic => "claude-sonnet-4-5".to_string(),
        _ => "deepseek-v4.1-flash".to_string(),
    }
}

fn provider_names(cfg: &Config) -> Vec<String> {
    let mut names = config::provider_presets()
        .into_iter()
        .map(|preset| preset.name)
        .collect::<Vec<_>>();
    for name in cfg.providers.keys() {
        if !names.contains(name) {
            names.push(name.clone());
        }
    }
    names
}

fn choose_provider(cfg: &Config, interactive: bool, prompt: &str) -> Result<String> {
    let names = provider_names(cfg);
    if !interactive {
        bail!("{prompt} Pass a provider name, for example `-p opencode`.");
    }
    eprintln!("{}", prompt);
    for (index, name) in names.iter().enumerate() {
        let marker = if cfg.provider.as_deref() == Some(name) {
            "*"
        } else {
            " "
        };
        eprintln!("{marker} {:>2}. {name}", index + 1);
    }
    let Some(answer) = ui::read_line("provider [number or name]: ")? else {
        bail!("no provider selected");
    };
    let answer = answer.trim();
    if answer.is_empty() {
        bail!("no provider selected");
    }
    if let Ok(index) = answer.parse::<usize>()
        && index >= 1
        && index <= names.len()
    {
        return Ok(names[index - 1].clone());
    }
    if names.iter().any(|name| name == answer) {
        return Ok(answer.to_string());
    }
    bail!("unknown provider '{answer}'")
}

fn is_auth_error(message: &str) -> bool {
    let message = message.to_ascii_lowercase();
    message.contains(" 401")
        || message.contains(" 403")
        || message.contains("401:")
        || message.contains("403:")
        || message.contains("unauthorized")
        || message.contains("invalid api key")
        || message.contains("incorrect api key")
}

pub fn parse_and_run() -> Result<()> {
    let cli = <Cli as clap::Parser>::parse();
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(run_cli(cli))
}

fn provider_connected(name: &str, config: &config::ProviderConfig) -> bool {
    if config.kind == Some(ProviderKind::ChatGpt) {
        return config::codex_auth().is_ok();
    }
    if config::find_api_key(name, config).is_some() {
        return true;
    }
    config.api_key.is_none()
        && config.api_key_env.is_none()
        && config
            .base_url
            .as_deref()
            .is_some_and(|base| base.contains("localhost") || base.contains("127.0.0.1"))
}

fn provider_choices(cfg: &Config) -> Vec<tui::app::ProviderChoice> {
    let mut names = config::provider_presets()
        .into_iter()
        .map(|preset| preset.name)
        .collect::<Vec<_>>();
    for name in cfg.providers.keys() {
        if !names.contains(name) {
            names.push(name.clone());
        }
    }
    names
        .into_iter()
        .filter(|name| name != "openai" && name != "codex")
        .filter_map(|name| {
            let config = cfg.provider_config(&name).ok()?;
            let model = cfg
                .providers
                .get(&name)
                .and_then(|provider| provider.model.clone())
                .or_else(|| config.model.clone())
                .unwrap_or_default();
            Some(tui::app::ProviderChoice {
                name,
                model,
                config,
            })
        })
        .collect()
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
