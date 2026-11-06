use crate::conversation::{Block, Message, ToolCall};
use crate::provider::{Provider, Usage};
use crate::tools::ToolRegistry;
use crate::ui_bridge::AgentUi;
use anyhow::{Context, Result, bail};
use std::sync::Arc;

pub struct Agent {
    provider: Provider,
    model: String,
    system: String,
    tools: ToolRegistry,
    messages: Vec<Message>,
    max_steps: usize,
    usage: Usage,
    ui: Arc<dyn AgentUi>,
    session: Option<crate::session::Session>,
}

pub struct CompactInfo {
    pub before_messages: usize,
    pub after_messages: usize,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub summary: String,
}

impl Agent {
    pub fn new(
        provider: Provider,
        model: String,
        _core: String,
        system: String,
        tools: ToolRegistry,
        max_steps: usize,
        ui: Arc<dyn AgentUi>,
    ) -> Self {
        Self {
            provider,
            model,
            system,
            tools,
            messages: Vec::new(),
            max_steps,
            usage: Usage::default(),
            ui,
            session: None,
        }
    }

    pub fn set_session(&mut self, session: crate::session::Session, messages: Vec<Message>) {
        self.session = Some(session);
        self.messages = messages;
    }

    pub fn messages(&self) -> &[Message] {
        &self.messages
    }

    pub fn list_sessions(&self) -> Result<Vec<crate::session::SessionSummary>> {
        self.session
            .as_ref()
            .context("conversation storage is unavailable")?
            .list()
    }

    pub fn resume_session(&mut self, id: Option<&str>) -> Result<Vec<Message>> {
        let messages = self
            .session
            .as_mut()
            .context("conversation storage is unavailable")?
            .resume(id)?;
        self.messages = messages;
        self.usage = Usage::default();
        Ok(self.messages.clone())
    }

    fn save(&mut self) -> Result<()> {
        if let Some(session) = &mut self.session {
            session.save(&self.messages)?;
        }
        Ok(())
    }

    pub fn model(&self) -> &str {
        &self.model
    }

    pub fn provider_name(&self) -> &str {
        &self.provider.name
    }

    pub fn set_provider(&mut self, provider: Provider, model: String) {
        let old = format!("- Model: {}/{}\n", self.provider.name, self.model);
        let new = format!("- Model: {}/{}\n", provider.name, model);
        self.system = self.system.replace(&old, &new);
        self.provider = provider;
        self.model = model;
    }

    pub fn set_model(&mut self, model: String) {
        let old = format!("- Model: {}/{}\n", self.provider.name, self.model);
        let new = format!("- Model: {}/{}\n", self.provider.name, model);
        self.system = self.system.replace(&old, &new);
        self.provider.set_model(model.clone());
        self.model = model;
    }

    pub async fn list_models(&self) -> Result<Vec<crate::provider::ModelInfo>> {
        self.provider.list_models().await
    }

    pub fn clear(&mut self) {
        self.messages.clear();
        self.usage = Usage::default();
        if let Some(session) = &mut self.session {
            session.new_conversation();
        }
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

    pub fn workspace(&self) -> &std::path::Path {
        self.tools.workspace()
    }

    pub fn set_permission(&mut self, target: &str, value: crate::config::Permission) -> Result<()> {
        self.tools.set_permission(target, value)
    }

    /// Record context without calling the model. Used by the `!` shell command.
    pub fn append_context(&mut self, text: String) -> Result<()> {
        if text.trim().is_empty() {
            return Ok(());
        }
        self.messages.push(Message::user(text));
        self.save()
    }

    pub fn export_markdown(&self) -> String {
        crate::conversation::to_markdown(&self.messages)
    }

    pub async fn compact(&mut self, instructions: Option<&str>) -> Result<CompactInfo> {
        if self.messages.is_empty() {
            bail!("there is no conversation to compact");
        }
        let before_messages = self.messages.len();
        let transcript = crate::conversation::transcript(&self.messages, 120_000);
        let focus = instructions
            .map(str::trim)
            .filter(|text| !text.is_empty())
            .map(|text| format!("\n\nThe user wants the summary to focus on: {text}"))
            .unwrap_or_default();
        let system = format!(
            "You compact coding-agent conversations. Write a dense, factual summary that preserves the user's goal, decisions, constraints, files and paths touched, commands run, test results, open problems, and the next concrete step. Omit pleasantries and repetition. Do not invent details. Use short headings and bullets. The summary will replace the older conversation, so it must be self-contained.{focus}"
        );
        let request = vec![Message::user(format!(
            "Summarize this conversation:\n\n{transcript}"
        ))];
        let completion = self
            .provider
            .complete(&system, &request, &[], &mut |_| {})
            .await?;
        let summary = completion.text.trim().to_string();
        if summary.is_empty() {
            bail!("the model returned an empty summary; the conversation was not compacted");
        }
        self.messages = vec![
            Message::user(format!(
                "[Compacted conversation summary. Older messages were replaced to save context.]\n\n{summary}"
            )),
            Message::assistant(
                "I have the compacted context. Continue from the summary above.".to_string(),
                Vec::new(),
            ),
        ];
        self.usage.input_tokens += completion.usage.input_tokens;
        self.usage.output_tokens += completion.usage.output_tokens;
        self.save()?;
        Ok(CompactInfo {
            before_messages,
            after_messages: self.messages.len(),
            input_tokens: completion.usage.input_tokens,
            output_tokens: completion.usage.output_tokens,
            summary,
        })
    }

    pub fn toggle_auto_approve(&mut self) -> bool {
        let value = !self.tools.auto_approve();
        self.tools.set_auto_approve(value);
        value
    }

    pub fn set_auto_approve(&mut self, value: bool) {
        self.tools.set_auto_approve(value);
    }

    pub fn cancel_turn(&mut self) -> Result<()> {
        let completed: std::collections::HashSet<_> = self
            .messages
            .iter()
            .flat_map(|message| &message.blocks)
            .filter_map(|block| match block {
                Block::ToolResult { tool_call_id, .. } => Some(tool_call_id.clone()),
                _ => None,
            })
            .collect();
        let pending: Vec<_> = self
            .messages
            .iter()
            .flat_map(|message| &message.blocks)
            .filter_map(|block| match block {
                Block::ToolCall(call) if !completed.contains(&call.id) => Some(call.id.clone()),
                _ => None,
            })
            .collect();
        for id in pending {
            self.messages.push(Message::tool_result(id, "Stopped by the user. This tool may have partially executed; inspect its effects before retrying.".into(), true));
        }
        self.save()
    }

    pub async fn run_turn(&mut self, input: &str) -> Result<()> {
        self.run_message(Message::user(input)).await
    }

    pub async fn run_message(&mut self, message: Message) -> Result<()> {
        if message
            .blocks
            .iter()
            .any(|block| matches!(block, Block::Image(_)))
        {
            self.provider.validate_image_input()?;
        }
        self.messages.push(message);
        self.save()?;
        let mut turn_usage = Usage::default();

        for _step in 0..self.max_steps {
            let definitions = self.tools.definitions();
            let mut printed = false;
            let ui = self.ui.clone();
            let completion = self
                .provider
                .complete(&self.system, &self.messages, &definitions, &mut |delta| {
                    ui.text(delta);
                    printed = true;
                })
                .await?;
            if printed {
                ui.text_done();
            }

            turn_usage.input_tokens += completion.usage.input_tokens;
            turn_usage.output_tokens += completion.usage.output_tokens;

            if completion.tool_calls.is_empty() {
                if !completion.text.trim().is_empty() {
                    self.messages
                        .push(Message::assistant(completion.text, Vec::new()));
                } else {
                    self.ui.warn("the model returned no text and no tool calls");
                }
                self.save()?;
                self.usage.input_tokens += turn_usage.input_tokens;
                self.usage.output_tokens += turn_usage.output_tokens;
                self.ui
                    .usage(turn_usage.input_tokens, turn_usage.output_tokens);
                return Ok(());
            }

            for call in &completion.tool_calls {
                self.ui.tool_call(&call.id, &call.name, &call_detail(call));
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
                if call.name == "todo_write" && !outcome.is_error {
                    let lines = outcome
                        .summary
                        .lines()
                        .map(str::to_string)
                        .collect::<Vec<_>>();
                    self.ui.todos(&lines);
                }
                let summary = if outcome.is_error {
                    first_line(&outcome.content)
                } else {
                    outcome.summary.clone()
                };
                self.ui.tool_result(&call.id, &summary, outcome.is_error);
                self.ui.tool_output(&call.id, &outcome.content);
                self.messages.push(Message::tool_result(
                    call.id,
                    outcome.content,
                    outcome.is_error,
                ));
            }
            self.save()?;
        }

        self.usage.input_tokens += turn_usage.input_tokens;
        self.usage.output_tokens += turn_usage.output_tokens;
        bail!(
            "stopped after {} model turns without finishing. Ask me to continue, or raise --max-steps.",
            self.max_steps
        )
    }
}

fn call_detail(call: &ToolCall) -> String {
    serde_json::from_str::<serde_json::Value>(&call.arguments)
        .ok()
        .map(|args| describe_call(&call.name, &args))
        .unwrap_or_default()
}

fn describe_call(name: &str, args: &serde_json::Value) -> String {
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
