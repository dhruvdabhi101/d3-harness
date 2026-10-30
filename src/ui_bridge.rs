use crate::ui;
use async_trait::async_trait;
use std::io::Write;

/// Events the agent produces while a turn is running.
pub trait AgentUi: Send + Sync {
    fn text(&self, delta: &str);
    fn text_done(&self);
    fn tool_call(&self, id: &str, name: &str, detail: &str);
    fn tool_result(&self, id: &str, summary: &str, is_error: bool);
    fn tool_output(&self, _id: &str, _content: &str) {}
    fn todos(&self, lines: &[String]);
    fn usage(&self, input: u64, output: u64);
    fn info(&self, text: &str);
    fn warn(&self, text: &str);
}

pub struct ConsoleAgentUi;

impl AgentUi for ConsoleAgentUi {
    fn text(&self, delta: &str) {
        print!("{delta}");
        let _ = std::io::stdout().flush();
    }

    fn text_done(&self) {
        println!();
        let _ = std::io::stdout().flush();
    }

    fn tool_call(&self, _id: &str, name: &str, detail: &str) {
        ui::tool_call(name, detail);
    }

    fn tool_result(&self, _id: &str, summary: &str, is_error: bool) {
        if is_error {
            if !summary.is_empty() {
                ui::error(summary.lines().next().unwrap_or(summary));
            }
        } else {
            ui::tool_done(summary);
        }
    }

    fn todos(&self, _lines: &[String]) {}

    fn usage(&self, input: u64, output: u64) {
        ui::print_usage(input, output);
    }

    fn info(&self, text: &str) {
        ui::info(text);
    }

    fn warn(&self, text: &str) {
        ui::warn(text);
    }
}

/// Requests from tools that need a decision or a question answered.
#[async_trait]
pub trait ToolUi: Send + Sync {
    fn diff(&self, path: &str, old: &str, new: &str);
    fn is_interactive(&self) -> bool;
    async fn confirm(&self, title: &str, body: &str) -> bool;
    async fn ask(&self, question: &str, options: &[String]) -> Option<String>;
}

pub struct ConsoleToolUi {
    interactive: bool,
}

impl ConsoleToolUi {
    pub fn new(interactive: bool) -> Self {
        Self { interactive }
    }
}

#[async_trait]
impl ToolUi for ConsoleToolUi {
    fn diff(&self, path: &str, old: &str, new: &str) {
        ui::print_diff(path, old, new);
    }

    fn is_interactive(&self) -> bool {
        self.interactive
    }

    async fn confirm(&self, title: &str, body: &str) -> bool {
        if !body.trim().is_empty() {
            eprintln!();
            for line in body.lines() {
                eprintln!("  {line}");
            }
        }
        ui::confirm(title)
    }

    async fn ask(&self, question: &str, options: &[String]) -> Option<String> {
        eprintln!();
        eprintln!("{} {}", ui::yellow("?"), ui::bold(question));
        for (index, option) in options.iter().enumerate() {
            eprintln!("  {}. {}", index + 1, option);
        }
        let answer = match ui::read_line("  answer: ") {
            Ok(Some(answer)) => answer.trim().to_string(),
            _ => String::new(),
        };
        if answer.is_empty() {
            return None;
        }
        if let Ok(index) = answer.parse::<usize>()
            && index >= 1
            && index <= options.len()
        {
            return Some(options[index - 1].clone());
        }
        Some(answer)
    }
}
