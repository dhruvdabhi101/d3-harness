pub mod app;
mod input;

use crate::agent::Agent;
use crate::provider::{ModelInfo, Provider};
use crate::ui_bridge::{AgentUi, ToolUi};
use anyhow::{Context, Result};
use app::{App, TuiInfo};
use async_trait::async_trait;
use crossterm::{
    cursor::Show,
    event::{
        self, DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
    },
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{Terminal, backend::CrosstermBackend};
use std::io::{Stdout, stdout};
use std::sync::Arc;
use std::sync::OnceLock;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::sync::{mpsc, oneshot};

pub enum AgentNotice {
    Text(String),
    TextDone,
    ToolCall {
        id: String,
        name: String,
        detail: String,
    },
    ToolResult {
        id: String,
        summary: String,
        is_error: bool,
    },
    ToolOutput {
        id: String,
        content: String,
    },
    Todos(Vec<String>),
    Usage {
        input: u64,
        output: u64,
    },
    Info(String),
    Warn(String),
}

pub enum ToolRequest {
    Diff {
        path: String,
        old: String,
        new: String,
    },
    Confirm {
        title: String,
        body: String,
        respond: oneshot::Sender<bool>,
    },
    Ask {
        question: String,
        options: Vec<String>,
        respond: oneshot::Sender<Option<String>>,
    },
}

pub enum WorkerCommand {
    Run(String),
    Clear,
    ListSessions,
    Resume(Option<String>),
    SetProvider(Box<Provider>, String),
    SetModel(String),
    SetAutoApprove(bool),
    ListModels,
    Quit,
}

pub enum LoginEvent {
    Code { url: String, code: String },
    Success,
    Failed(String),
}

pub enum WorkerEvent {
    TurnDone(Result<(), String>),
    Models(Result<Vec<ModelInfo>, String>),
    Sessions(Result<Vec<crate::session::SessionSummary>, String>),
    Resumed(Result<Vec<crate::conversation::Message>, String>),
}

pub struct TuiSetup {
    pub agent_ui: Arc<dyn AgentUi>,
    pub tool_ui: Arc<dyn ToolUi>,
    notices: mpsc::UnboundedReceiver<AgentNotice>,
    requests: mpsc::UnboundedReceiver<ToolRequest>,
}

pub fn setup() -> TuiSetup {
    let (notice_tx, notices) = mpsc::unbounded_channel();
    let (request_tx, requests) = mpsc::unbounded_channel();
    TuiSetup {
        agent_ui: Arc::new(TuiAgentUi {
            tx: notice_tx.clone(),
        }),
        tool_ui: Arc::new(TuiToolUi { tx: request_tx }),
        notices,
        requests,
    }
}

struct TuiAgentUi {
    tx: mpsc::UnboundedSender<AgentNotice>,
}

impl AgentUi for TuiAgentUi {
    fn text(&self, delta: &str) {
        let _ = self.tx.send(AgentNotice::Text(delta.to_string()));
    }

    fn text_done(&self) {
        let _ = self.tx.send(AgentNotice::TextDone);
    }

    fn tool_call(&self, id: &str, name: &str, detail: &str) {
        let _ = self.tx.send(AgentNotice::ToolCall {
            id: id.to_string(),
            name: name.to_string(),
            detail: detail.to_string(),
        });
    }

    fn tool_result(&self, id: &str, summary: &str, is_error: bool) {
        let _ = self.tx.send(AgentNotice::ToolResult {
            id: id.to_string(),
            summary: summary.to_string(),
            is_error,
        });
    }

    fn tool_output(&self, id: &str, content: &str) {
        let _ = self.tx.send(AgentNotice::ToolOutput {
            id: id.to_string(),
            content: content.to_string(),
        });
    }

    fn todos(&self, lines: &[String]) {
        let _ = self.tx.send(AgentNotice::Todos(lines.to_vec()));
    }

    fn usage(&self, input: u64, output: u64) {
        let _ = self.tx.send(AgentNotice::Usage { input, output });
    }

    fn info(&self, text: &str) {
        let _ = self.tx.send(AgentNotice::Info(text.to_string()));
    }

    fn warn(&self, text: &str) {
        let _ = self.tx.send(AgentNotice::Warn(text.to_string()));
    }
}

struct TuiToolUi {
    tx: mpsc::UnboundedSender<ToolRequest>,
}

#[async_trait]
impl ToolUi for TuiToolUi {
    fn diff(&self, path: &str, old: &str, new: &str) {
        let _ = self.tx.send(ToolRequest::Diff {
            path: path.to_string(),
            old: old.to_string(),
            new: new.to_string(),
        });
    }

    fn is_interactive(&self) -> bool {
        true
    }

    async fn confirm(&self, title: &str, body: &str) -> bool {
        let (respond, answer) = oneshot::channel();
        if self
            .tx
            .send(ToolRequest::Confirm {
                title: title.to_string(),
                body: body.to_string(),
                respond,
            })
            .is_err()
        {
            return false;
        }
        answer.await.unwrap_or(false)
    }

    async fn ask(&self, question: &str, options: &[String]) -> Option<String> {
        let (respond, answer) = oneshot::channel();
        if self
            .tx
            .send(ToolRequest::Ask {
                question: question.to_string(),
                options: options.to_vec(),
                respond,
            })
            .is_err()
        {
            return None;
        }
        answer.await.unwrap_or(None)
    }
}

pub async fn run(agent: Agent, setup: TuiSetup, info: TuiInfo) -> Result<()> {
    let messages = agent.messages().to_vec();
    let (command_tx, command_rx) = mpsc::unbounded_channel();
    let (worker_tx, mut worker_rx) = mpsc::unbounded_channel();

    tokio::spawn(worker(agent, command_rx, worker_tx));

    let mut terminal = TuiTerminal::new()?;
    let (input_tx, mut input_rx) = mpsc::unbounded_channel();
    std::thread::spawn(move || {
        while let Ok(event) = event::read() {
            if input_tx.send(event).is_err() {
                break;
            }
        }
    });

    let TuiSetup {
        notices: mut notice_rx,
        requests: mut request_rx,
        ..
    } = setup;
    let (login_tx, mut login_rx) = mpsc::unbounded_channel();
    let mut app = App::new(info, command_tx, login_tx);
    app.restore_messages(&messages);
    let mut ticker = tokio::time::interval(Duration::from_millis(90));

    loop {
        if app.dirty {
            terminal
                .terminal
                .draw(|frame| app.draw(frame))
                .context("drawing the terminal")?;
        }
        tokio::select! {
            Some(event) = input_rx.recv() => {
                if app.handle_event(event) {
                    break;
                }
            }
            Some(request) = request_rx.recv() => app.handle_tool_request(request),
            Some(notice) = notice_rx.recv() => app.handle_notice(notice),
            Some(event) = worker_rx.recv() => app.handle_worker_event(event),
            Some(event) = login_rx.recv() => app.handle_login_event(event),
            _ = ticker.tick() => app.tick(),
        }
        if app.should_quit {
            break;
        }
    }

    Ok(())
}

pub(super) async fn codex_device_login(tx: mpsc::UnboundedSender<LoginEvent>) {
    let auth_path = codex_auth_path();
    let mut backup = AuthBackup::new(auth_path.clone());
    let mut child = match Command::new("codex")
        .args(["login", "--device-auth"])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
    {
        Ok(child) => child,
        Err(error) => {
            let _ = tx.send(LoginEvent::Failed(format!(
                "could not start `codex login --device-auth`: {error}. Install Codex CLI first."
            )));
            return;
        }
    };
    let Some(stdout) = child.stdout.take() else {
        let _ = tx.send(LoginEvent::Failed("codex login has no stdout".to_string()));
        return;
    };
    let Some(stderr) = child.stderr.take() else {
        let _ = tx.send(LoginEvent::Failed("codex login has no stderr".to_string()));
        return;
    };
    let mut out_lines = BufReader::new(stdout).lines();
    let mut err_lines = BufReader::new(stderr).lines();
    let mut out_done = false;
    let mut err_done = false;
    let mut last_error = String::new();
    while !out_done || !err_done {
        tokio::select! {
            result = out_lines.next_line(), if !out_done => {
                match result {
                    Ok(Some(line)) => parse_login_line(&line, &tx),
                    _ => out_done = true,
                }
            }
            result = err_lines.next_line(), if !err_done => {
                match result {
                    Ok(Some(line)) => {
                        let clean = strip_ansi(&line);
                        if clean.trim().is_empty() {
                            continue;
                        }
                        if clean.to_ascii_lowercase().contains("error") {
                            last_error = clean.trim().to_string();
                        }
                        parse_login_line(&clean, &tx);
                    }
                    _ => err_done = true,
                }
            }
        }
    }
    match child.wait().await {
        Ok(status) if status.success() => {
            backup.keep();
            let _ = tx.send(LoginEvent::Success);
        }
        Ok(status) => {
            let message = if last_error.is_empty() {
                format!("codex login exited with {status}")
            } else {
                last_error
            };
            let _ = tx.send(LoginEvent::Failed(message));
        }
        Err(error) => {
            let _ = tx.send(LoginEvent::Failed(format!(
                "waiting for codex login: {error}"
            )));
        }
    }
}

fn codex_auth_path() -> std::path::PathBuf {
    let home = dirs::home_dir().unwrap_or_default();
    std::env::var("CODEX_HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| home.join(".codex"))
        .join("auth.json")
}

struct AuthBackup {
    path: std::path::PathBuf,
    content: Option<Vec<u8>>,
    keep: bool,
}

impl AuthBackup {
    fn new(path: std::path::PathBuf) -> Self {
        let content = std::fs::read(&path).ok();
        Self {
            path,
            content,
            keep: false,
        }
    }

    fn keep(&mut self) {
        self.keep = true;
    }
}

impl Drop for AuthBackup {
    fn drop(&mut self) {
        if self.keep {
            return;
        }
        match &self.content {
            Some(content) => {
                if let Some(parent) = self.path.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                let _ = std::fs::write(&self.path, content);
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    let _ = std::fs::set_permissions(
                        &self.path,
                        std::fs::Permissions::from_mode(0o600),
                    );
                }
            }
            None => {
                let _ = std::fs::remove_file(&self.path);
            }
        }
    }
}

fn parse_login_line(line: &str, tx: &mpsc::UnboundedSender<LoginEvent>) {
    let clean = strip_ansi(line);
    let Some(url) = clean
        .split_whitespace()
        .find(|word| word.contains("auth.openai.com/codex/device"))
        .map(|word| {
            word.trim_matches(|ch: char| !ch.is_ascii_graphic())
                .to_string()
        })
    else {
        if let Some(code) = extract_device_code(&clean) {
            let url = "https://auth.openai.com/codex/device".to_string();
            let _ = tx.send(LoginEvent::Code { url, code });
        }
        return;
    };
    let code = extract_device_code(&clean).unwrap_or_default();
    let _ = tx.send(LoginEvent::Code { url, code });
}

fn extract_device_code(line: &str) -> Option<String> {
    static CODE: OnceLock<regex::Regex> = OnceLock::new();
    let regex = CODE
        .get_or_init(|| regex::Regex::new(r"[A-Z0-9]{4}-[A-Z0-9]{5}").expect("device code regex"));
    regex.find(line).map(|found| found.as_str().to_string())
}

fn strip_ansi(line: &str) -> String {
    static ANSI: OnceLock<regex::Regex> = OnceLock::new();
    let regex =
        ANSI.get_or_init(|| regex::Regex::new(r"\x1b\[[0-9;?]*[ -/]*[@-~]").expect("ansi regex"));
    regex.replace_all(line, "").to_string()
}

async fn worker(
    mut agent: Agent,
    mut commands: mpsc::UnboundedReceiver<WorkerCommand>,
    events: mpsc::UnboundedSender<WorkerEvent>,
) {
    while let Some(command) = commands.recv().await {
        match command {
            WorkerCommand::Run(text) => {
                let result = agent
                    .run_turn(&text)
                    .await
                    .map_err(|error| format!("{error:#}"));
                let _ = events.send(WorkerEvent::TurnDone(result));
            }
            WorkerCommand::Clear => agent.clear(),
            WorkerCommand::ListSessions => {
                let result = agent.list_sessions().map_err(|error| format!("{error:#}"));
                let _ = events.send(WorkerEvent::Sessions(result));
            }
            WorkerCommand::Resume(id) => {
                let result = agent
                    .resume_session(id.as_deref())
                    .map_err(|error| format!("{error:#}"));
                let _ = events.send(WorkerEvent::Resumed(result));
            }
            WorkerCommand::SetProvider(provider, model) => agent.set_provider(*provider, model),
            WorkerCommand::SetModel(model) => agent.set_model(model),
            WorkerCommand::SetAutoApprove(value) => agent.set_auto_approve(value),
            WorkerCommand::ListModels => {
                let result = agent
                    .list_models()
                    .await
                    .map_err(|error| format!("{error:#}"));
                let _ = events.send(WorkerEvent::Models(result));
            }
            WorkerCommand::Quit => break,
        }
    }
}

struct TuiTerminal {
    terminal: Terminal<CrosstermBackend<Stdout>>,
    _guard: TerminalGuard,
}

struct TerminalGuard;

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(
            stdout(),
            LeaveAlternateScreen,
            DisableBracketedPaste,
            DisableMouseCapture,
            Show
        );
    }
}

impl TuiTerminal {
    fn new() -> Result<Self> {
        enable_raw_mode().context("enabling raw mode")?;
        let guard = TerminalGuard;
        let mut out = stdout();
        execute!(
            out,
            EnterAlternateScreen,
            EnableBracketedPaste,
            EnableMouseCapture
        )
        .context("entering the alternate screen")?;
        let backend = CrosstermBackend::new(out);
        let terminal = Terminal::new(backend)?;
        Ok(Self {
            terminal,
            _guard: guard,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auth_backup_restores_cancelled_login() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("auth.json");
        std::fs::write(&path, "old").unwrap();
        {
            let _backup = AuthBackup::new(path.clone());
            std::fs::write(&path, "partial").unwrap();
        }
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "old");
    }

    #[test]
    fn auth_backup_keeps_successful_login() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("auth.json");
        std::fs::write(&path, "old").unwrap();
        {
            let mut backup = AuthBackup::new(path.clone());
            std::fs::write(&path, "new").unwrap();
            backup.keep();
        }
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "new");
    }

    #[test]
    fn parses_device_code_lines() {
        assert_eq!(
            extract_device_code("   O9UI-A5U6J\u{1b}[0m"),
            Some("O9UI-A5U6J".to_string())
        );
        assert_eq!(
            strip_ansi("\u{1b}[94mhttps://auth.openai.com/codex/device\u{1b}[0m"),
            "https://auth.openai.com/codex/device"
        );
    }
}
