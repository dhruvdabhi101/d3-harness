use crate::agent::Agent;
use crate::ui;
use anyhow::Result;

pub async fn run(agent: &mut Agent) -> Result<()> {
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

        if let Err(error) = agent.run_turn(input).await {
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
        "/model" => {
            if argument.is_empty() {
                ui::info(&format!("model: {}", agent.model()));
            } else {
                agent.set_model(argument.to_string());
                ui::info(&format!("model: {argument}"));
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
        "/diff" | "/cost" => {
            ui::info("no changes or costs are tracked beyond the current session");
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
        "/model [id]        show or change the model",
        "/provider          show the current provider",
        "/system            how to change the system prompt",
        "/tools             list available tools",
        "/skills            list available skills",
        "/mcp               list connected MCP servers",
        "/context           message and token counts",
        "/approve [on|off]  toggle auto-approval",
        "/exit              leave",
    ]
    .join("\n")
}
