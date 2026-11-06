use crate::agent::Agent;
use crate::ui;
use anyhow::Result;

pub async fn run(agent: &mut Agent) -> Result<()> {
    if !agent.messages().is_empty() {
        for message in agent.messages() {
            let text = message.text();
            if text.is_empty() {
                continue;
            }
            match message.role {
                crate::conversation::Role::User => eprintln!("{} {text}", ui::bold("›")),
                crate::conversation::Role::Assistant => eprintln!("{text}"),
                crate::conversation::Role::Tool => {}
            }
        }
        ui::info("resumed saved conversation");
    }
    loop {
        let Some(line) = ui::read_line(&format!("{} ", ui::bold("›")))? else {
            eprintln!();
            break;
        };
        let input = line.trim();
        if input.is_empty() {
            continue;
        }
        if input.starts_with('/') {
            match command(agent, input).await {
                Ok(true) => break,
                Ok(false) => continue,
                Err(error) => {
                    ui::error(&format!("{error:#}"));
                    continue;
                }
            }
        }

        if let Some(command) = input.strip_prefix('!') {
            let include = !command.starts_with('!');
            let command = command.trim_start_matches('!').trim();
            if command.is_empty() {
                ui::warn("usage: !<shell command> or !!<command> to hide its output");
                continue;
            }
            match crate::tools::shell::run(command, agent.workspace(), 300).await {
                Ok(output) => {
                    eprintln!("{}", output.content);
                    if include
                        && let Err(error) = agent.append_context(format!(
                            "Output of `!{command}` (run by the user, not by you):\n\n{}",
                            output.content
                        ))
                    {
                        ui::warn(&format!("could not save the command output: {error:#}"));
                    }
                }
                Err(error) => ui::error(&format!("{error:#}")),
            }
            continue;
        }

        let input = expand_mentions(input, agent.workspace());
        if let Err(error) = agent.run_turn(&input).await {
            ui::error(&format!("{error:#}"));
        }
        eprintln!();
    }
    Ok(())
}

async fn command(agent: &mut Agent, input: &str) -> Result<bool> {
    let mut parts = input.splitn(2, char::is_whitespace);
    let command = parts.next().unwrap_or("");
    let argument = parts.next().unwrap_or("").trim();

    match command {
        "/exit" | "/quit" | "/q" => return Ok(true),
        "/help" | "/?" => {
            eprintln!("{}", help());
        }
        "/new" | "/clear" => {
            agent.clear();
            ui::info("conversation cleared");
        }
        "/resume" if argument.is_empty() => {
            for session in agent.list_sessions()? {
                eprintln!("{}\t{}", session.id, session.title);
            }
            ui::info("Use /resume <ID> or /continue for the latest conversation.");
        }
        "/resume" | "/continue" => {
            let messages = agent.resume_session(if argument.is_empty() {
                None
            } else {
                Some(argument)
            })?;
            for message in messages {
                let text = message.text();
                if !text.is_empty() {
                    eprintln!("{text}");
                }
            }
            ui::info("resumed project conversation");
        }
        "/model" => {
            if argument.is_empty() {
                ui::info(&format!("model: {}", agent.model()));
            } else {
                agent.set_model(argument.to_string());
                ui::info(&format!("model: {argument}"));
            }
        }
        "/models" => {
            let models = agent.list_models().await?;
            if models.is_empty() {
                ui::warn("the provider returned no models");
                return Ok(false);
            }
            if !argument.is_empty() {
                if models.iter().any(|model| model.id == argument) {
                    agent.set_model(argument.to_string());
                    ui::info(&format!("model: {argument}"));
                } else {
                    ui::warn(&format!(
                        "'{argument}' is not in the provider's list; run /models to pick one"
                    ));
                }
                return Ok(false);
            }
            for (index, model) in models.iter().enumerate() {
                let marker = if model.id == agent.model() { "*" } else { " " };
                let name = model.name.as_deref().unwrap_or("");
                let suffix = if name.is_empty() {
                    String::new()
                } else {
                    format!(" {}", ui::dim(name))
                };
                eprintln!("{marker} {:>3}. {:<34}{suffix}", index + 1, model.id);
            }
            let Some(choice) = ui::read_line("select model [number or id, empty to cancel]: ")?
            else {
                return Ok(false);
            };
            let choice = choice.trim();
            if choice.is_empty() {
                ui::info("model unchanged");
                return Ok(false);
            }
            let selected = if let Ok(index) = choice.parse::<usize>() {
                models
                    .get(index.saturating_sub(1))
                    .map(|model| model.id.clone())
            } else if models.iter().any(|model| model.id == choice) {
                Some(choice.to_string())
            } else {
                None
            };
            match selected {
                Some(model) => {
                    agent.set_model(model.clone());
                    ui::info(&format!("model: {model}"));
                }
                None => ui::warn("that is not one of the listed models"),
            }
        }
        "/provider" => {
            ui::info(&format!(
                "provider: {}. To switch providers, restart with --provider <name>.",
                agent.provider_name()
            ));
        }
        "/system" => {
            if argument.is_empty() {
                ui::info(
                    "restart with --system-file <path> or --system-prompt <text> to change the system prompt",
                );
            } else {
                ui::info(&format!(
                    "system prompts are fixed for the session; restart with --system-file {argument}"
                ));
            }
        }
        "/tools" => {
            let names = agent.tool_names();
            for chunk in names.chunks(4) {
                eprintln!("  {}", chunk.join("  "));
            }
        }
        "/skills" => {
            let names = agent.skill_names();
            if names.is_empty() {
                ui::info("no skills found");
            } else {
                eprintln!("  {}", names.join("  "));
            }
        }
        "/mcp" => {
            let summaries = agent.mcp_summaries();
            if summaries.is_empty() {
                ui::info("no MCP servers connected");
            } else {
                for summary in summaries {
                    eprintln!("  {}", summary);
                }
            }
        }
        "/context" => {
            let usage = agent.usage();
            ui::info(&format!(
                "{} messages, {} input / {} output tokens this session",
                agent.message_count(),
                usage.input_tokens,
                usage.output_tokens
            ));
        }
        "/cost" => {
            let usage = agent.usage();
            ui::info(&format!(
                "{} input / {} output tokens this session; pricing is not tracked",
                usage.input_tokens, usage.output_tokens
            ));
        }
        "/status" => {
            let usage = agent.usage();
            ui::info(&format!(
                "provider {} · model {} · {} messages · {} in / {} out tokens · workspace {}",
                agent.provider_name(),
                agent.model(),
                agent.message_count(),
                usage.input_tokens,
                usage.output_tokens,
                agent.workspace().display()
            ));
        }
        "/compact" => {
            let info = agent
                .compact((!argument.is_empty()).then_some(argument))
                .await?;
            ui::info(&format!(
                "compacted {} messages into {}; {} input / {} output tokens",
                info.before_messages, info.after_messages, info.input_tokens, info.output_tokens
            ));
        }
        "/diff" => {
            let output = crate::tools::shell::run(
                "git status --short; git diff --stat; git diff -- . ':!Cargo.lock'",
                agent.workspace(),
                60,
            )
            .await?;
            eprintln!("{}", output.content);
        }
        "/export" => {
            let path = if argument.is_empty() {
                let stamp = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs();
                agent
                    .workspace()
                    .join(format!("d3-conversation-{stamp}.md"))
            } else {
                let path = std::path::PathBuf::from(argument);
                if path.is_absolute() {
                    path
                } else {
                    agent.workspace().join(path)
                }
            };
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(&path, agent.export_markdown())?;
            ui::info(&format!("exported conversation to {}", path.display()));
        }
        "/copy" => {
            let text = agent
                .messages()
                .iter()
                .rev()
                .find(|message| message.role == crate::conversation::Role::Assistant)
                .map(|message| message.text())
                .unwrap_or_default();
            if text.trim().is_empty() {
                ui::warn("there is no assistant answer to copy");
            } else {
                match crate::image::copy_text(&text) {
                    Ok(()) => ui::info("copied the last answer to the clipboard"),
                    Err(error) => ui::warn(&format!("could not copy: {error:#}")),
                }
            }
        }
        "/permissions" | "/permission" => {
            let mut parts = argument.split_whitespace();
            match (parts.next(), parts.next()) {
                (None, _) => ui::info("use /permissions shell|edit ask|allow|deny"),
                (Some(target), Some(value)) => {
                    let value = match value {
                        "ask" => crate::config::Permission::Ask,
                        "allow" | "on" | "yes" => crate::config::Permission::Allow,
                        "deny" | "off" | "no" => crate::config::Permission::Deny,
                        _ => {
                            ui::warn("permission must be ask, allow, or deny");
                            return Ok(false);
                        }
                    };
                    agent.set_permission(target, value)?;
                    ui::info(&format!("permission {target}: {value:?}"));
                }
                _ => ui::warn("usage: /permissions shell|edit ask|allow|deny"),
            }
        }
        "/init" => {
            let path = agent.workspace().join("AGENTS.md");
            if path.exists() {
                ui::warn(&format!("{} already exists", path.display()));
            } else {
                std::fs::write(&path, crate::prompt::AGENTS_TEMPLATE)?;
                ui::info(&format!("wrote {}", path.display()));
            }
        }
        "/version" => ui::info(&format!("d3 {}", env!("CARGO_PKG_VERSION"))),
        "/login" | "/auth" => ui::info(&format!(
            "run `d3 auth login {}` in a terminal, or use /login inside the TUI",
            agent.provider_name()
        )),
        "/logout" => {
            let name = if argument.is_empty() {
                agent.provider_name()
            } else {
                argument
            };
            match crate::config::logout(name) {
                Ok(true) => ui::info(&format!("removed the stored credential for {name}")),
                Ok(false) => ui::info(&format!(
                    "no stored credential for {name}; environment and harness.json values are unchanged"
                )),
                Err(error) => ui::warn(&format!("could not remove the credential: {error:#}")),
            }
        }
        "/approve" | "/auto" => {
            let enabled = match argument {
                "" => agent.toggle_auto_approve(),
                "on" | "yes" | "true" => {
                    agent.set_auto_approve(true);
                    true
                }
                "off" | "no" | "false" => {
                    agent.set_auto_approve(false);
                    false
                }
                _ => {
                    ui::warn("usage: /approve [on|off]");
                    return Ok(false);
                }
            };
            ui::info(&format!(
                "auto-approve: {}",
                if enabled { "on" } else { "off" }
            ));
        }
        other => {
            ui::warn(&format!("unknown command {other}; try /help"));
        }
    }
    Ok(false)
}

fn help() -> String {
    [
        "/help              show this help",
        "/new               clear the conversation",
        "/resume [id]       list or resume project conversations",
        "/continue          resume the latest project conversation",
        "/compact [focus]   summarize history to free context",
        "/status            provider, model, messages, and tokens",
        "/context           message and token counts",
        "/cost              token usage for this session",
        "/diff              show Git status and the working diff",
        "/export [path]     save this conversation as Markdown",
        "/copy              copy the last answer to the clipboard",
        "/model [id]        show or change the model",
        "/models            fetch models and choose one",
        "/provider          show the current provider",
        "/login             how to sign in",
        "/logout [provider] remove a stored credential",
        "/system            how to change the system prompt",
        "/tools             list available tools",
        "/skills            list available skills",
        "/mcp               list connected MCP servers",
        "/init              create a starter AGENTS.md",
        "/permissions       change shell/edit permissions",
        "/approve [on|off]  toggle auto-approval",
        "/version           show the harness version",
        "/exit              leave",
        "",
        "!command           run a shell command and add its output to context",
        "!!command          run a shell command without adding output to context",
        "@path              attach a file to your next prompt",
    ]
    .join("\n")
}

fn expand_mentions(text: &str, workspace: &std::path::Path) -> String {
    use std::sync::OnceLock;
    static MENTION: OnceLock<regex::Regex> = OnceLock::new();
    let regex = MENTION
        .get_or_init(|| regex::Regex::new(r"(?:^|\s)@([^\s]+)").expect("file mention regex"));
    let mut context = String::new();
    for capture in regex.captures_iter(text) {
        let Some(raw) = capture.get(1).map(|found| found.as_str()) else {
            continue;
        };
        let raw = raw.trim_matches(['"', '\'', ',', ';', ')', ']']);
        let Ok(path) = workspace.join(raw).canonicalize() else {
            ui::warn(&format!("@ mention not found: {raw}"));
            continue;
        };
        if !path.starts_with(workspace) || !path.is_file() {
            ui::warn(&format!("@ mention is not a workspace file: {raw}"));
            continue;
        }
        let Ok(bytes) = std::fs::read(&path) else {
            ui::warn(&format!("could not read @ mention: {raw}"));
            continue;
        };
        if bytes.contains(&0) {
            ui::warn(&format!("@ mention is a binary file: {raw}"));
            continue;
        }
        let file = String::from_utf8_lossy(&bytes);
        let file = if file.chars().count() > 50_000 {
            format!(
                "{}… [file truncated at 50,000 characters]",
                file.chars().take(50_000).collect::<String>()
            )
        } else {
            file.into_owned()
        };
        context.push_str(&format!(
            "\n\nAttached file `{}`:\n```\n{}\n```",
            raw,
            file.trim_end()
        ));
    }
    if context.is_empty() {
        text.to_string()
    } else {
        format!("{text}{context}")
    }
}
