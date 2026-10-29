use crate::conversation::{Block, Message, ToolCall};
use crate::provider::{Provider, Usage};
use crate::tools::ToolRegistry;
use crate::ui;
use anyhow::{Result, bail};
use std::io::Write;

pub struct Agent {
    provider: Provider,
    model: String,
    system: String,
    tools: ToolRegistry,
    messages: Vec<Message>,
    max_steps: usize,
    usage: Usage,
}

impl Agent {
    pub fn new(
        provider: Provider,
        model: String,
        _core: String,
        system: String,
        tools: ToolRegistry,
        max_steps: usize,
    ) -> Self {
        Self {
            provider,
            model,
            system,
            tools,
            messages: Vec::new(),
            max_steps,
            usage: Usage::default(),
        }
    }

    pub fn model(&self) -> &str {
        &self.model
    }

    pub fn provider_name(&self) -> &str {
        &self.provider.name
    }

    pub fn set_model(&mut self, model: String) {
        self.provider.set_model(model.clone());
        self.model = model;
    }

    pub fn clear(&mut self) {
        self.messages.clear();
        self.usage = Usage::default();
    }

    pub fn message_count(&self) -> usize {
        self.messages.len()
    }

    pub fn tool_names(&self) -> Vec<String> {
        self.tools
            .specs()
            .iter()
            .map(|spec| spec.name.clone())
            .collect()
    }

    pub fn mcp_summaries(&self) -> Vec<serde_json::Value> {
        self.tools.mcp_summaries()
    }

    pub fn skill_names(&self) -> Vec<String> {
        self.tools.skill_names()
    }

    pub fn usage(&self) -> Usage {
        self.usage
    }

    pub fn toggle_auto_approve(&mut self) -> bool {
        let value = !self.tools.auto_approve();
        self.tools.set_auto_approve(value);
        value
    }

    pub fn set_auto_approve(&mut self, value: bool) {
        self.tools.set_auto_approve(value);
    }

    pub async fn run_turn(&mut self, input: &str) -> Result<()> {
        self.messages.push(Message::user(input));
        let mut turn_usage = Usage::default();

        for _step in 0..self.max_steps {
            let definitions = self.tools.definitions();
            let mut printed = false;
            let completion = self
                .provider
                .complete(&self.system, &self.messages, &definitions, &mut |delta| {
                    print!("{delta}");
                    let _ = std::io::stdout().flush();
                    printed = true;
                })
                .await?;
            if printed {
                println!();
                let _ = std::io::stdout().flush();
            }

            turn_usage.input_tokens += completion.usage.input_tokens;
            turn_usage.output_tokens += completion.usage.output_tokens;

            if completion.tool_calls.is_empty() {
                if !completion.text.trim().is_empty() {
                    self.messages
                        .push(Message::assistant(completion.text, Vec::new()));
                } else {
                    ui::warn("the model returned no text and no tool calls");
                }
                self.usage.input_tokens += turn_usage.input_tokens;
                self.usage.output_tokens += turn_usage.output_tokens;
                ui::print_usage(turn_usage.input_tokens, turn_usage.output_tokens);
                return Ok(());
            }

            for call in &completion.tool_calls {
                print_call(call);
            }
            let mut assistant = Message::assistant(completion.text, completion.tool_calls.clone());
            if !completion.provider_items.is_empty() {
                assistant
                    .blocks
                    .push(Block::ProviderItems(completion.provider_items));
            }
            self.messages.push(assistant);

            for call in completion.tool_calls {
                let outcome = self.tools.execute(&call).await;
                if outcome.is_error {
                    ui::error(&first_line(&outcome.content));
                } else if !outcome.summary.is_empty() {
                    ui::tool_done(&outcome.summary);
                }
                self.messages.push(Message::tool_result(
                    call.id,
                    outcome.content,
                    outcome.is_error,
                ));
            }
        }

        self.usage.input_tokens += turn_usage.input_tokens;
        self.usage.output_tokens += turn_usage.output_tokens;
        bail!(
            "stopped after {} model turns without finishing. Ask me to continue, or raise --max-steps.",
            self.max_steps
        )
    }
}

fn print_call(call: &ToolCall) {
    let detail = serde_json::from_str::<serde_json::Value>(&call.arguments)
        .ok()
        .map(|args| call_detail(&call.name, &args))
        .unwrap_or_default();
    ui::tool_call(&call.name, &detail);
}

fn call_detail(name: &str, args: &serde_json::Value) -> String {
    let text = |key: &str| {
        args.get(key)
            .and_then(serde_json::Value::as_str)
            .map(str::to_string)
    };
    match name {
        "read" | "write" | "edit" => text("path").unwrap_or_default(),
        "bash" => truncate(&text("command").unwrap_or_default(), 140),
        "glob" => {
            let pattern = text("pattern").unwrap_or_default();
            match text("path") {
                Some(path) => format!("{pattern} in {path}"),
                None => pattern,
            }
        }
        "grep" => {
            let pattern = text("pattern").unwrap_or_default();
            match text("include") {
                Some(include) => format!("{pattern} ({include})"),
                None => pattern,
            }
        }
        "skill" => text("name").unwrap_or_default(),
        "ask_user" => truncate(&text("question").unwrap_or_default(), 140),
        "todo_write" => {
            let count = args
                .get("todos")
                .and_then(serde_json::Value::as_array)
                .map(Vec::len)
                .unwrap_or(0);
            format!("{count} item{}", if count == 1 { "" } else { "s" })
        }
        _ => {
            if name.starts_with("mcp__") {
                truncate(&args.to_string(), 140)
            } else {
                String::new()
            }
        }
    }
}

fn first_line(text: &str) -> String {
    let line = text.lines().next().unwrap_or(text);
    truncate(line, 240)
}

fn truncate(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        text.to_string()
    } else {
        format!("{}…", text.chars().take(max).collect::<String>())
    }
}
