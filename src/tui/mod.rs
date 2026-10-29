pub mod app;
mod input;

use crate::agent::Agent;
use crate::provider::ModelInfo;
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
use std::time::Duration;
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
    SetModel(String),
    SetAutoApprove(bool),
    ListModels,
    Quit,
}

pub enum WorkerEvent {
    TurnDone(Result<(), String>),
    Models(Result<Vec<ModelInfo>, String>),
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
    let mut app = App::new(info, command_tx);
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
            _ = ticker.tick() => app.tick(),
        }
        if app.should_quit {
            break;
        }
    }

    Ok(())
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
            DisableMouseCapture,
            DisableBracketedPaste,
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
            EnableMouseCapture,
            EnableBracketedPaste
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
