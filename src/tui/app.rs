use super::input::InputBuffer;
use super::{AgentNotice, LoginEvent, ToolRequest, WorkerCommand, WorkerEvent};
use crate::config::{self, ProviderConfig, ProviderKind};
use crate::provider::{ModelInfo, Provider, ProviderAuth};
use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseEventKind};
use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, BorderType, Clear, Paragraph, Wrap},
};
use similar::TextDiff;
use std::path::PathBuf;
use tokio::sync::{mpsc, oneshot};

const ACCENT: Color = Color::Rgb(255, 153, 102);
const ACCENT_DIM: Color = Color::Rgb(160, 100, 70);
const TOOL: Color = Color::Rgb(120, 180, 255);
const GREEN: Color = Color::Rgb(120, 210, 150);
const RED: Color = Color::Rgb(240, 120, 120);
const YELLOW: Color = Color::Rgb(240, 200, 120);
const DIM: Color = Color::Rgb(135, 135, 145);
const FAINT: Color = Color::Rgb(80, 80, 90);
const CODE_BG: Color = Color::Rgb(36, 36, 42);
const USER_BG: Color = Color::Rgb(35, 43, 56);
const ASSISTANT_BG: Color = Color::Rgb(26, 29, 35);
const APPROVAL_BG: Color = Color::Rgb(46, 37, 24);
const LIGHT: Color = Color::Rgb(232, 234, 240);

#[derive(Clone)]
pub struct ProviderChoice {
    pub name: String,
    pub model: String,
    pub config: ProviderConfig,
}

pub struct TuiInfo {
    pub provider: String,
    pub model: String,
    pub workspace: PathBuf,
    pub branch: Option<String>,
    pub skills: usize,
    pub mcp: usize,
    pub tools: usize,
    pub auto_approve: bool,
    pub connected: bool,
    pub providers: Vec<ProviderChoice>,
}

pub struct App {
    info: TuiInfo,
    model: String,
    items: Vec<Item>,
    input: InputBuffer,
    palette: Option<Palette>,
    modal: Option<Modal>,
    approval: Option<ApprovalPrompt>,
    history: Vec<String>,
    history_pos: Option<usize>,
    scroll: u16,
    max_scroll: u16,
    auto_scroll: bool,
    busy: bool,
    spinner: usize,
    status: String,
    ctrl_c_pending: bool,
    show_sidebar: bool,
    messages: usize,
    input_tokens: u64,
    output_tokens: u64,
    todos: Vec<String>,
    commands: mpsc::UnboundedSender<WorkerCommand>,
    login_tx: mpsc::UnboundedSender<super::LoginEvent>,
    login_task: Option<tokio::task::AbortHandle>,
    connected: bool,
    providers: Vec<ProviderChoice>,
    pub should_quit: bool,
    pub dirty: bool,
}

enum Item {
    User(String),
    Assistant {
        text: String,
        streaming: bool,
    },
    Tool {
        id: String,
        name: String,
        detail: String,
        summary: String,
        state: ToolState,
    },
    Diff {
        path: String,
        old: String,
        new: String,
    },
    Info(String),
    Warn(String),
    Error(String),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ToolState {
    Running,
    Done,
    Error,
}

struct Palette {
    kind: PaletteKind,
    items: Vec<PaletteItem>,
    selected: usize,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum PaletteKind {
    Commands,
    Models,
    Providers,
}

#[derive(Clone)]
struct PaletteItem {
    label: String,
    description: String,
    value: String,
}

enum Modal {
    Ask {
        question: String,
        options: Vec<String>,
        selected: usize,
        input: InputBuffer,
        response: Option<oneshot::Sender<Option<String>>>,
    },
    ApiKey {
        provider: ProviderChoice,
        input: InputBuffer,
    },
    Login {
        provider: ProviderChoice,
        url: Option<String>,
        code: Option<String>,
        status: String,
        failed: bool,
    },
}

struct ApprovalPrompt {
    title: String,
    body: String,
    response: Option<oneshot::Sender<bool>>,
}

struct CommandSpec {
    name: &'static str,
    description: &'static str,
}

const COMMANDS: &[CommandSpec] = &[
    CommandSpec {
        name: "/help",
        description: "show commands and shortcuts",
    },
    CommandSpec {
        name: "/new",
        description: "start a fresh conversation",
    },
    CommandSpec {
        name: "/model",
        description: "show or change the model",
    },
    CommandSpec {
        name: "/models",
        description: "fetch and choose from the provider catalog",
    },
    CommandSpec {
        name: "/provider",
        description: "choose and switch provider",
    },
    CommandSpec {
        name: "/tools",
        description: "list available tools",
    },
    CommandSpec {
        name: "/skills",
        description: "list available skills",
    },
    CommandSpec {
        name: "/mcp",
        description: "list connected MCP servers",
    },
    CommandSpec {
        name: "/context",
        description: "show messages and token usage",
    },
    CommandSpec {
        name: "/approve",
        description: "toggle auto-approval",
    },
    CommandSpec {
        name: "/system",
        description: "how to change the system prompt",
    },
    CommandSpec {
        name: "/login",
        description: "sign in with ChatGPT or update a key",
    },
    CommandSpec {
        name: "/exit",
        description: "leave harness",
    },
];

impl App {
    pub fn new(
        info: TuiInfo,
        commands: mpsc::UnboundedSender<WorkerCommand>,
        login_tx: mpsc::UnboundedSender<super::LoginEvent>,
    ) -> Self {
        let model = info.model.clone();
        let connected = info.connected;
        let providers = info.providers.clone();
        let mut app = Self {
            info,
            model,
            items: Vec::new(),
            input: InputBuffer::new(),
            palette: None,
            modal: None,
            approval: None,
            history: Vec::new(),
            history_pos: None,
            scroll: 0,
            max_scroll: 0,
            auto_scroll: true,
            busy: false,
            spinner: 0,
            status: "ready".to_string(),
            ctrl_c_pending: false,
            show_sidebar: true,
            messages: 0,
            input_tokens: 0,
            output_tokens: 0,
            todos: Vec::new(),
            commands,
            login_tx,
            login_task: None,
            connected,
            providers,
            should_quit: false,
            dirty: true,
        };
        if connected {
            app.push_info(
                "Welcome. Type a request, or / for the command palette. Shift+Enter adds a newline.",
            );
        } else {
            app.status = "choose provider".to_string();
            app.push_info("Choose a provider, then sign in. Everything stays inside this screen.");
            app.open_provider_palette();
        }
        app
    }

    pub fn draw(&mut self, frame: &mut Frame) {
        self.dirty = false;
        let area = frame.area();
        if area.width < 30 || area.height < 8 {
            frame.render_widget(
                Paragraph::new("terminal is too small for harness").style(Style::default().fg(RED)),
                area,
            );
            return;
        }

        let input_height = self.input.line_count().clamp(1, 6) as u16;
        let approval_height = self.approval.as_ref().map(approval_height).unwrap_or(0);
        let bottom_height = if approval_height > 0 {
            approval_height
        } else {
            input_height + 3
        };
        let chunks = Layout::vertical([
            Constraint::Length(1),
            Constraint::Min(3),
            Constraint::Length(bottom_height),
        ])
        .split(area);

        self.draw_header(frame, chunks[0]);

        let (chat_area, sidebar_area) = if self.show_sidebar && area.width >= 108 {
            let split =
                Layout::horizontal([Constraint::Min(40), Constraint::Length(38)]).split(chunks[1]);
            (split[0], Some(split[1]))
        } else {
            (chunks[1], None)
        };
        self.draw_chat(frame, chat_area);
        if let Some(sidebar) = sidebar_area {
            self.draw_sidebar(frame, sidebar);
        }

        if let Some(approval) = &self.approval {
            self.draw_approval(frame, approval, chunks[2]);
        } else {
            let input_chunks =
                Layout::vertical([Constraint::Length(input_height + 2), Constraint::Length(1)])
                    .split(chunks[2]);
            self.draw_input(frame, input_chunks[0], input_chunks[1], input_height);
            if let Some(palette) = &self.palette {
                self.draw_palette(frame, palette, input_chunks[0]);
            }
        }
        if let Some(modal) = &self.modal {
            self.draw_modal(frame, modal, area);
        }
    }

    fn draw_header(&self, frame: &mut Frame, area: Rect) {
        let area = Rect {
            x: area.x + 1,
            width: area.width.saturating_sub(2),
            ..area
        };
        let halves = Layout::horizontal([Constraint::Min(20), Constraint::Length(46)]).split(area);
        let mut left = vec![
            Span::styled("◆ ", Style::default().fg(ACCENT)),
            Span::styled(
                "harness",
                Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!(" {}", env!("CARGO_PKG_VERSION")),
                Style::default().fg(FAINT),
            ),
        ];
        if let Some(name) = self
            .info
            .workspace
            .file_name()
            .and_then(|name| name.to_str())
        {
            left.push(Span::styled(format!("  {name}"), Style::default().fg(DIM)));
        }
        if let Some(branch) = &self.info.branch {
            left.push(Span::styled(
                format!("  {branch}"),
                Style::default().fg(Color::Rgb(150, 170, 220)),
            ));
        }
        frame.render_widget(Paragraph::new(Line::from(left)), halves[0]);

        let status = if self.busy {
            format!("{} {} ", spinner(self.spinner), self.status)
        } else {
            format!("{} ", self.status)
        };
        let model = truncate(&format!(" {}/{} ", self.info.provider, self.model), 46);
        let right = Line::from(vec![
            Span::styled(
                status,
                Style::default().fg(if self.busy { YELLOW } else { DIM }),
            ),
            Span::styled(
                model,
                Style::default()
                    .fg(Color::Rgb(20, 20, 24))
                    .bg(ACCENT)
                    .add_modifier(Modifier::BOLD),
            ),
        ]);
        frame.render_widget(Paragraph::new(right).alignment(Alignment::Right), halves[1]);
    }

    fn draw_chat(&mut self, frame: &mut Frame, area: Rect) {
        let inner = Rect {
            x: area.x + 2,
            y: area.y + 1,
            width: area.width.saturating_sub(4),
            height: area.height.saturating_sub(1),
        };
        if inner.width == 0 || inner.height == 0 {
            return;
        }
        let lines = self.chat_lines();
        let paragraph = Paragraph::new(Text::from(lines)).wrap(Wrap { trim: false });
        let total = paragraph.line_count(inner.width);
        self.max_scroll = total
            .saturating_sub(inner.height as usize)
            .try_into()
            .unwrap_or(u16::MAX);
        if self.auto_scroll {
            self.scroll = self.max_scroll;
        } else {
            self.scroll = self.scroll.min(self.max_scroll);
        }
        let paragraph = paragraph.scroll((self.scroll, 0));
        frame.render_widget(paragraph, inner);
    }

    fn draw_sidebar(&self, frame: &mut Frame, area: Rect) {
        let block = Block::bordered()
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(FAINT))
            .title(Line::from(Span::styled(
                " session ",
                Style::default().fg(ACCENT),
            )));
        let inner = block.inner(area);
        frame.render_widget(block, area);
        if inner.height == 0 {
            return;
        }

        let mut lines = vec![
            kv("provider", &self.info.provider, DIM),
            kv("model", &self.model, ACCENT),
        ];
        if let Some(branch) = &self.info.branch {
            lines.push(kv("branch", branch, Color::Rgb(150, 170, 220)));
        }
        lines.push(kv(
            "approval",
            if self.info.auto_approve {
                "auto"
            } else {
                "ask"
            },
            if self.info.auto_approve { YELLOW } else { DIM },
        ));
        lines.push(Line::default());
        lines.push(Line::from(Span::styled(
            "PLAN",
            Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
        )));
        if self.todos.is_empty() {
            lines.push(Line::from(Span::styled(
                "  no plan yet",
                Style::default().fg(FAINT),
            )));
        } else {
            for todo in self.todos.iter().take(10) {
                let color = if todo.starts_with("[x]") {
                    GREEN
                } else if todo.starts_with("[~]") {
                    ACCENT
                } else {
                    DIM
                };
                lines.push(Line::from(Span::styled(
                    format!("  {}", truncate(todo, 30)),
                    Style::default().fg(color),
                )));
            }
        }
        lines.push(Line::default());
        lines.push(Line::from(Span::styled(
            "CONTEXT",
            Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
        )));
        lines.push(Line::from(vec![
            Span::styled("  messages  ", Style::default().fg(FAINT)),
            Span::styled(self.messages.to_string(), Style::default().fg(DIM)),
        ]));
        lines.push(Line::from(vec![
            Span::styled("  tokens    ", Style::default().fg(FAINT)),
            Span::styled(
                format!(
                    "{} in / {} out",
                    short(self.input_tokens),
                    short(self.output_tokens)
                ),
                Style::default().fg(DIM),
            ),
        ]));
        lines.push(Line::from(vec![
            Span::styled("  tools     ", Style::default().fg(FAINT)),
            Span::styled(
                format!(
                    "{} · skills {} · mcp {}",
                    self.info.tools, self.info.skills, self.info.mcp
                ),
                Style::default().fg(DIM),
            ),
        ]));
        lines.push(Line::default());
        lines.push(Line::from(Span::styled(
            "  ctrl+b hides this panel",
            Style::default().fg(FAINT),
        )));

        frame.render_widget(
            Paragraph::new(Text::from(lines)).wrap(Wrap { trim: false }),
            inner,
        );
    }

    fn draw_approval(&self, frame: &mut Frame, approval: &ApprovalPrompt, area: Rect) {
        frame.render_widget(Clear, area);
        let block = Block::bordered()
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(ACCENT))
            .title(Line::from(Span::styled(
                " permission required ",
                Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
            )))
            .style(Style::default().bg(APPROVAL_BG));
        let inner = block.inner(area);
        frame.render_widget(block, area);
        if inner.height == 0 {
            return;
        }
        let mut lines = vec![Line::from(Span::styled(
            approval.title.clone(),
            Style::default()
                .fg(LIGHT)
                .bg(APPROVAL_BG)
                .add_modifier(Modifier::BOLD),
        ))];
        if !approval.body.trim().is_empty() {
            for line in approval.body.lines() {
                lines.push(Line::from(Span::styled(
                    format!("  {line}"),
                    Style::default()
                        .fg(Color::Rgb(220, 210, 190))
                        .bg(APPROVAL_BG),
                )));
            }
        }
        lines.push(Line::from(vec![
            Span::styled("  [y] allow", Style::default().fg(GREEN).bg(APPROVAL_BG)),
            Span::styled("    [n] deny", Style::default().fg(RED).bg(APPROVAL_BG)),
            Span::styled("    Esc deny", Style::default().fg(FAINT).bg(APPROVAL_BG)),
        ]));
        frame.render_widget(
            Paragraph::new(Text::from(lines)).wrap(Wrap { trim: false }),
            inner,
        );
    }

    fn draw_input(&self, frame: &mut Frame, area: Rect, hint_area: Rect, input_height: u16) {
        let focused = !self.busy && self.modal.is_none() && self.approval.is_none();
        let border = if focused { ACCENT } else { ACCENT_DIM };
        let title = if self.busy {
            " message · working "
        } else {
            " message "
        };
        let block = Block::bordered()
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(border))
            .title(Line::from(Span::styled(
                title,
                Style::default().fg(if focused { ACCENT } else { DIM }),
            )));
        let inner = block.inner(area);
        frame.render_widget(block, area);

        let lines = self.input.lines();
        let (cursor_row, cursor_col) = self.input.line_col();
        let start = if cursor_row >= input_height as usize {
            cursor_row + 1 - input_height as usize
        } else {
            0
        };
        let visible = lines
            .iter()
            .skip(start)
            .take(input_height as usize)
            .cloned()
            .collect::<Vec<_>>();
        let text = if self.input.is_empty() && !self.busy {
            Text::from(Line::from(Span::styled(
                "Ask anything, or type / for commands",
                Style::default().fg(FAINT),
            )))
        } else {
            Text::from(
                visible
                    .into_iter()
                    .map(|line| Line::from(Span::raw(line)))
                    .collect::<Vec<_>>(),
            )
        };
        frame.render_widget(Paragraph::new(text), inner);

        if focused && self.modal.is_none() {
            let x = inner.x + cursor_col.min(inner.width.saturating_sub(1) as usize) as u16;
            let y = inner.y + (cursor_row - start) as u16;
            if y < inner.y + inner.height {
                frame.set_cursor_position((x, y));
            }
        }

        let left = if self.busy {
            format!(
                "{} working · Ctrl+C twice to quit · / commands",
                spinner(self.spinner)
            )
        } else {
            "Enter send · Shift+Enter newline · / commands · Ctrl+B sidebar".to_string()
        };
        let right = if self.busy { "" } else { "ready" };
        let halves =
            Layout::horizontal([Constraint::Min(20), Constraint::Length(8)]).split(hint_area);
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(left, Style::default().fg(FAINT)))),
            halves[0],
        );
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(right, Style::default().fg(FAINT))))
                .alignment(Alignment::Right),
            halves[1],
        );
    }

    fn draw_palette(&self, frame: &mut Frame, palette: &Palette, input_area: Rect) {
        if palette.items.is_empty() {
            return;
        }
        let visible = palette.items.len().min(10);
        let height = (visible + 2) as u16;
        let width = input_area.width.saturating_sub(4).clamp(34, 78);
        let x = input_area.x + 2;
        let y = input_area.y.saturating_sub(height).max(1);
        let area = Rect {
            x,
            y,
            width,
            height,
        };
        frame.render_widget(Clear, area);
        let title = match palette.kind {
            PaletteKind::Commands => " commands ",
            PaletteKind::Models => " models ",
            PaletteKind::Providers => " providers ",
        };
        let block = Block::bordered()
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(ACCENT_DIM))
            .title(Line::from(Span::styled(title, Style::default().fg(ACCENT))));
        let inner = block.inner(area);
        frame.render_widget(block, area);

        let start = if palette.selected >= visible {
            palette.selected + 1 - visible
        } else {
            0
        };
        let mut lines = Vec::new();
        for (index, item) in palette.items.iter().enumerate().skip(start).take(visible) {
            let selected = index == palette.selected;
            let base = if selected {
                Style::default()
                    .fg(Color::Rgb(20, 20, 24))
                    .bg(ACCENT)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };
            let desc = if selected {
                base
            } else {
                Style::default().fg(DIM)
            };
            lines.push(Line::from(vec![
                Span::styled(format!("  {:<18} ", item.label), base),
                Span::styled(truncate(&item.description, 48), desc),
            ]));
        }
        frame.render_widget(Paragraph::new(Text::from(lines)), inner);
    }

    fn draw_modal(&self, frame: &mut Frame, modal: &Modal, area: Rect) {
        let width = area.width.saturating_sub(8).min(84);
        let height = area.height.saturating_sub(6).min(20);
        let x = area.x + (area.width.saturating_sub(width)) / 2;
        let y = area.y + (area.height.saturating_sub(height)) / 2;
        let rect = Rect {
            x,
            y,
            width,
            height,
        };
        frame.render_widget(Clear, rect);
        let title = match modal {
            Modal::Ask { .. } => "question",
            Modal::ApiKey { .. } => "sign in",
            Modal::Login { .. } => "ChatGPT sign-in",
        };
        let block = Block::bordered()
            .border_type(BorderType::Double)
            .border_style(Style::default().fg(ACCENT))
            .title(Line::from(Span::styled(
                format!(" {title} "),
                Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
            )));
        let inner = block.inner(rect);
        frame.render_widget(block, rect);

        let mut lines = Vec::new();
        match modal {
            Modal::Ask {
                question,
                options,
                selected,
                input,
                ..
            } => {
                for line in question.lines() {
                    lines.push(Line::from(Span::styled(
                        line.to_string(),
                        Style::default().fg(Color::Rgb(225, 225, 230)),
                    )));
                }
                lines.push(Line::default());
                if options.is_empty() {
                    lines.push(Line::from(vec![
                        Span::styled("> ", Style::default().fg(ACCENT)),
                        Span::raw(input.text().to_string()),
                        Span::styled("▌", Style::default().fg(ACCENT)),
                    ]));
                } else {
                    for (index, option) in options.iter().enumerate() {
                        let style = if index == *selected {
                            Style::default()
                                .fg(Color::Rgb(20, 20, 24))
                                .bg(ACCENT)
                                .add_modifier(Modifier::BOLD)
                        } else {
                            Style::default().fg(DIM)
                        };
                        lines.push(Line::from(Span::styled(
                            format!("  {}. {}", index + 1, option),
                            style,
                        )));
                    }
                    lines.push(Line::default());
                    lines.push(Line::from(vec![
                        Span::styled("> ", Style::default().fg(ACCENT)),
                        Span::raw(input.text().to_string()),
                        Span::styled("▌", Style::default().fg(ACCENT)),
                    ]));
                }
                lines.push(Line::default());
                lines.push(Line::from(Span::styled(
                    "Enter submit · ↑/↓ choose · Esc skip",
                    Style::default().fg(FAINT),
                )));
            }
            Modal::ApiKey { provider, input } => {
                lines.push(Line::from(Span::styled(
                    format!("Sign in to {}", provider.name),
                    Style::default().fg(Color::Rgb(225, 225, 230)),
                )));
                lines.push(Line::default());
                lines.push(Line::from(Span::styled(
                    "Paste the provider's key. Input is hidden.",
                    Style::default().fg(DIM),
                )));
                lines.push(Line::default());
                let masked = "•".repeat(input.text().chars().count());
                lines.push(Line::from(vec![
                    Span::styled("> ", Style::default().fg(ACCENT)),
                    Span::styled(masked, Style::default().fg(GREEN)),
                    Span::styled("▌", Style::default().fg(ACCENT)),
                ]));
                lines.push(Line::default());
                lines.push(Line::from(Span::styled(
                    "Enter sign in · Esc cancel",
                    Style::default().fg(FAINT),
                )));
            }
            Modal::Login {
                provider,
                url,
                code,
                status,
                failed,
            } => {
                lines.push(Line::from(Span::styled(
                    format!("Sign in to {} with your ChatGPT account.", provider.name),
                    Style::default().fg(Color::Rgb(225, 225, 230)),
                )));
                lines.push(Line::default());
                if let Some(url) = url {
                    lines.push(Line::from(vec![
                        Span::styled("Open  ", Style::default().fg(DIM)),
                        Span::styled(url.clone(), Style::default().fg(TOOL)),
                    ]));
                }
                if let Some(code) = code.as_ref().filter(|code| !code.is_empty()) {
                    lines.push(Line::from(vec![
                        Span::styled("Code  ", Style::default().fg(DIM)),
                        Span::styled(
                            code.clone(),
                            Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
                        ),
                    ]));
                }
                lines.push(Line::default());
                lines.push(Line::from(Span::styled(
                    status.clone(),
                    Style::default().fg(if *failed { RED } else { YELLOW }),
                )));
                lines.push(Line::default());
                lines.push(Line::from(Span::styled(
                    "o open browser · Esc cancel",
                    Style::default().fg(FAINT),
                )));
            }
        }
        frame.render_widget(
            Paragraph::new(Text::from(lines)).wrap(Wrap { trim: false }),
            inner,
        );
    }

    fn chat_lines(&self) -> Vec<Line<'static>> {
        let mut lines = vec![Line::default()];
        for item in &self.items {
            match item {
                Item::User(text) => {
                    let mut first = true;
                    for line in text.lines() {
                        let prefix = if first { "› " } else { "  " };
                        first = false;
                        lines.push(Line::from(vec![
                            Span::styled(
                                format!(" {prefix}"),
                                Style::default().fg(ACCENT).bg(USER_BG),
                            ),
                            Span::styled(
                                format!("{line} "),
                                Style::default().fg(LIGHT).bg(USER_BG),
                            ),
                        ]));
                    }
                    lines.push(Line::default());
                }
                Item::Assistant { text, streaming } => {
                    lines.extend(assistant_lines(text, *streaming, self.spinner));
                    lines.push(Line::default());
                }
                Item::Tool {
                    name,
                    detail,
                    summary,
                    state,
                    ..
                } => {
                    let (marker, color) = match state {
                        ToolState::Running => (spinner(self.spinner), YELLOW),
                        ToolState::Done => ("●", GREEN),
                        ToolState::Error => ("●", RED),
                    };
                    lines.push(Line::from(vec![
                        Span::styled(format!("  {marker} "), Style::default().fg(color)),
                        Span::styled(
                            name.clone(),
                            Style::default().fg(TOOL).add_modifier(Modifier::BOLD),
                        ),
                        Span::styled(
                            if detail.is_empty() {
                                String::new()
                            } else {
                                format!("  {detail}")
                            },
                            Style::default().fg(DIM),
                        ),
                    ]));
                    for line in summary.lines().take(3) {
                        lines.push(Line::from(Span::styled(
                            format!("    {line}"),
                            Style::default().fg(if *state == ToolState::Error {
                                RED
                            } else {
                                FAINT
                            }),
                        )));
                    }
                    lines.push(Line::default());
                }
                Item::Diff { path, old, new } => {
                    lines.extend(diff_lines(path, old, new));
                    lines.push(Line::default());
                }
                Item::Info(text) => {
                    for (index, line) in text.lines().enumerate() {
                        let prefix = if index == 0 { "  • " } else { "    " };
                        lines.push(Line::from(Span::styled(
                            format!("{prefix}{line}"),
                            Style::default().fg(DIM),
                        )));
                    }
                    lines.push(Line::default());
                }
                Item::Warn(text) => {
                    for line in text.lines() {
                        lines.push(Line::from(Span::styled(
                            format!("  ! {line}"),
                            Style::default().fg(YELLOW),
                        )));
                    }
                    lines.push(Line::default());
                }
                Item::Error(text) => {
                    for line in text.lines() {
                        lines.push(Line::from(Span::styled(
                            format!("  ! {line}"),
                            Style::default().fg(RED),
                        )));
                    }
                    lines.push(Line::default());
                }
            }
        }
        lines
    }

    pub fn handle_event(&mut self, event: Event) -> bool {
        match event {
            Event::Key(key) if key.kind != KeyEventKind::Release => self.handle_key(key),
            Event::Mouse(mouse) => match mouse.kind {
                MouseEventKind::ScrollUp => self.scroll_up(3),
                MouseEventKind::ScrollDown => self.scroll_down(3),
                _ => {}
            },
            Event::Paste(text) => {
                if let Some(Modal::Ask { input, .. }) = &mut self.modal {
                    input.insert_str(&text);
                } else {
                    self.input.insert_str(&text);
                    self.refresh_palette();
                }
            }
            _ => {}
        }
        self.dirty = true;
        self.should_quit
    }

    fn handle_key(&mut self, key: KeyEvent) {
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            if self.busy && !self.ctrl_c_pending {
                self.ctrl_c_pending = true;
                self.push_warn("press Ctrl+C again to quit while a turn is running");
            } else {
                self.should_quit = true;
            }
            return;
        }
        self.ctrl_c_pending = false;

        if self.approval.is_some() {
            self.handle_approval_key(key);
            return;
        }
        if self.modal.is_some() {
            self.handle_modal_key(key);
            return;
        }
        if self.palette.is_some() {
            self.handle_palette_key(key);
            return;
        }

        if key.modifiers.contains(KeyModifiers::CONTROL) {
            match key.code {
                KeyCode::Char('a') => self.input.home(),
                KeyCode::Char('e') => self.input.end(),
                KeyCode::Char('u') => {
                    self.input.clear();
                    self.refresh_palette();
                }
                KeyCode::Char('w') => {
                    self.input.delete_word();
                    self.refresh_palette();
                }
                KeyCode::Char('b') => self.show_sidebar = !self.show_sidebar,
                KeyCode::Char('d') => {
                    if self.input.is_empty() {
                        self.should_quit = true;
                    }
                }
                KeyCode::Up => self.scroll_up(4),
                KeyCode::Down => self.scroll_down(4),
                KeyCode::Home => self.scroll = 0,
                KeyCode::End => self.scroll_to_bottom(),
                KeyCode::Char('j') => self.input.insert_char('\n'),
                _ => {}
            }
            return;
        }

        match key.code {
            KeyCode::Enter
                if key.modifiers.contains(KeyModifiers::SHIFT)
                    || key.modifiers.contains(KeyModifiers::ALT) =>
            {
                self.input.insert_char('\n');
            }
            KeyCode::Enter => self.submit(),
            KeyCode::Tab => {
                if self.input.text().starts_with('/') {
                    self.open_command_palette();
                } else {
                    self.input.insert_str("    ");
                }
            }
            KeyCode::Backspace => {
                self.input.backspace();
                self.refresh_palette();
            }
            KeyCode::Delete => {
                self.input.delete();
                self.refresh_palette();
            }
            KeyCode::Left => self.input.left(),
            KeyCode::Right => self.input.right(),
            KeyCode::Home => self.input.home(),
            KeyCode::End => self.input.end(),
            KeyCode::Up => {
                if self.input.up() {
                } else {
                    self.history_prev();
                }
            }
            KeyCode::Down => {
                if self.input.down() {
                } else {
                    self.history_next();
                }
            }
            KeyCode::PageUp => self.scroll_up(12),
            KeyCode::PageDown => self.scroll_down(12),
            KeyCode::Esc => {
                if !self.input.is_empty() {
                    self.input.clear();
                }
            }
            KeyCode::Char(ch) => {
                let was_empty = self.input.is_empty();
                self.input.insert_char(ch);
                if was_empty && ch == '/' {
                    self.open_command_palette();
                } else {
                    self.refresh_palette();
                }
            }
            _ => {}
        }
    }

    fn handle_palette_key(&mut self, key: KeyEvent) {
        let Some(palette) = &mut self.palette else {
            return;
        };
        match key.code {
            KeyCode::Esc => {
                self.palette = None;
                self.input.clear();
            }
            KeyCode::Up => {
                if palette.selected == 0 {
                    palette.selected = palette.items.len().saturating_sub(1);
                } else {
                    palette.selected -= 1;
                }
            }
            KeyCode::Down => {
                if !palette.items.is_empty() {
                    palette.selected = (palette.selected + 1) % palette.items.len();
                }
            }
            KeyCode::Enter | KeyCode::Tab => {
                if let Some(item) = palette.items.get(palette.selected).cloned() {
                    let kind = palette.kind;
                    self.palette = None;
                    match kind {
                        PaletteKind::Commands => {
                            self.input.clear();
                            self.run_command(&item.value);
                        }
                        PaletteKind::Models => {
                            self.input.clear();
                            self.set_model(&item.value);
                        }
                        PaletteKind::Providers => {
                            self.input.clear();
                            self.switch_provider(&item.value);
                        }
                    }
                }
            }
            KeyCode::Backspace => {
                self.input.backspace();
                if self.input.text().starts_with('/') && !self.input.text().contains(' ') {
                    self.refresh_palette();
                } else {
                    self.palette = None;
                }
            }
            KeyCode::Char(ch) => {
                self.input.insert_char(ch);
                if self.input.text().starts_with('/') && !self.input.text().contains(' ') {
                    self.refresh_palette();
                } else {
                    self.palette = None;
                }
            }
            _ => {}
        }
    }

    fn handle_approval_key(&mut self, key: KeyEvent) {
        let Some(mut approval) = self.approval.take() else {
            return;
        };
        let answer = match key.code {
            KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter => Some(true),
            KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => Some(false),
            _ => None,
        };
        if let Some(answer) = answer {
            if let Some(sender) = approval.response.take() {
                let _ = sender.send(answer);
            }
            self.status = "working".to_string();
        } else {
            self.approval = Some(approval);
        }
    }

    fn handle_modal_key(&mut self, key: KeyEvent) {
        let Some(mut modal) = self.modal.take() else {
            return;
        };
        match &mut modal {
            Modal::Ask {
                options,
                selected,
                input,
                response,
                ..
            } => {
                let mut send = |value: Option<String>, this: &mut Self| {
                    if let Some(sender) = response.take() {
                        let _ = sender.send(value);
                    }
                    this.status = "working".to_string();
                };
                match key.code {
                    KeyCode::Esc => send(None, self),
                    KeyCode::Up => {
                        if !options.is_empty() {
                            *selected = selected.saturating_sub(1);
                        }
                        self.modal = Some(modal);
                    }
                    KeyCode::Down => {
                        if !options.is_empty() {
                            *selected = (*selected + 1) % options.len();
                        }
                        self.modal = Some(modal);
                    }
                    KeyCode::Enter => {
                        let value = if !input.is_empty() {
                            Some(input.text().to_string())
                        } else if !options.is_empty() {
                            options.get(*selected).cloned()
                        } else {
                            None
                        };
                        send(value, self);
                    }
                    KeyCode::Char(ch)
                        if !options.is_empty() && input.is_empty() && ch.is_ascii_digit() =>
                    {
                        let index = ch.to_digit(10).unwrap_or(0) as usize;
                        if index >= 1 && index <= options.len() {
                            send(Some(options[index - 1].clone()), self);
                        } else {
                            self.modal = Some(modal);
                        }
                    }
                    KeyCode::Char(ch) => {
                        input.insert_char(ch);
                        self.modal = Some(modal);
                    }
                    KeyCode::Backspace => {
                        input.backspace();
                        self.modal = Some(modal);
                    }
                    _ => self.modal = Some(modal),
                }
            }
            Modal::ApiKey { provider, input } => match key.code {
                KeyCode::Esc => {}
                KeyCode::Enter => {
                    let key = input.text().trim().to_string();
                    if key.is_empty() {
                        self.modal = Some(modal);
                        return;
                    }
                    let provider = provider.clone();
                    if let Err(error) = config::store_api_key(&provider.name, &key) {
                        self.push_warn(format!("could not save the key: {error:#}"));
                    }
                    self.complete_connect(
                        provider,
                        ProviderAuth {
                            key,
                            account_id: None,
                        },
                    );
                }
                KeyCode::Char(ch) => {
                    input.insert_char(ch);
                    self.modal = Some(modal);
                }
                KeyCode::Backspace => {
                    input.backspace();
                    self.modal = Some(modal);
                }
                _ => self.modal = Some(modal),
            },
            Modal::Login { url, .. } => {
                if key.code == KeyCode::Esc {
                    if let Some(task) = self.login_task.take() {
                        task.abort();
                    }
                } else {
                    if key.code == KeyCode::Char('o')
                        && let Some(url) = url.as_deref()
                    {
                        open_browser(url);
                    }
                    self.modal = Some(modal);
                }
            }
        }
    }

    pub fn handle_login_event(&mut self, event: LoginEvent) {
        match event {
            LoginEvent::Code { url, code } => {
                if let Some(Modal::Login {
                    url: current_url,
                    code: current_code,
                    status,
                    ..
                }) = &mut self.modal
                {
                    *current_url = Some(url);
                    if !code.is_empty() {
                        *current_code = Some(code);
                    }
                    *status = "waiting for approval in the browser…".to_string();
                }
            }
            LoginEvent::Success => {
                self.login_task = None;
                if let Some(Modal::Login { provider, .. }) = self.modal.take() {
                    self.finish_login(provider);
                }
            }
            LoginEvent::Failed(error) => {
                self.login_task = None;
                if let Some(Modal::Login { status, failed, .. }) = &mut self.modal {
                    *status = error;
                    *failed = true;
                }
            }
        }
        self.dirty = true;
    }

    fn submit(&mut self) {
        let text = self.input.text().trim().to_string();
        if text.is_empty() {
            return;
        }
        if !self.connected && !text.starts_with('/') {
            self.push_warn("choose a provider and sign in first");
            self.open_provider_palette();
            return;
        }
        if !text.starts_with('/') && self.busy {
            self.push_warn("a turn is already running; wait for it to finish");
            return;
        }
        self.history.push(text.clone());
        self.history_pos = None;
        self.input.clear();
        self.palette = None;
        if text.starts_with('/') {
            self.run_command(&text);
            return;
        }
        self.items.push(Item::User(text.clone()));
        self.messages += 1;
        self.busy = true;
        self.status = "thinking".to_string();
        self.auto_scroll = true;
        let _ = self.commands.send(WorkerCommand::Run(text));
    }

    fn run_command(&mut self, text: &str) {
        let mut parts = text.splitn(2, char::is_whitespace);
        let command = parts.next().unwrap_or("");
        let argument = parts.next().unwrap_or("").trim();
        match command {
            "/help" => self.push_info(help_text()),
            "/new" | "/clear" => {
                if self.busy {
                    self.push_warn("wait for the current turn to finish first");
                    return;
                }
                self.items.clear();
                self.messages = 0;
                self.input_tokens = 0;
                self.output_tokens = 0;
                self.todos.clear();
                self.auto_scroll = true;
                let _ = self.commands.send(WorkerCommand::Clear);
                self.push_info("conversation cleared");
            }
            "/model" => {
                if argument.is_empty() {
                    self.push_info(format!("model: {}", self.model));
                } else {
                    self.set_model(argument);
                }
            }
            "/models" => {
                if !self.connected {
                    self.push_warn("choose a provider and sign in first");
                    self.open_provider_palette();
                    return;
                }
                if self.busy {
                    self.push_warn("wait for the current turn to finish first");
                    return;
                }
                self.status = "loading models".to_string();
                let _ = self.commands.send(WorkerCommand::ListModels);
            }
            "/provider" => self.open_provider_palette(),
            "/tools" => self.push_info(format!(
                "{} tools: read, write, edit, bash, glob, grep, ask_user, todo_write, skill",
                self.info.tools
            )),
            "/skills" => self.push_info(format!(
                "{} skills discovered. The agent loads one with the skill tool when it matches.",
                self.info.skills
            )),
            "/mcp" => self.push_info(format!("{} MCP servers connected.", self.info.mcp)),
            "/context" => self.push_info(format!(
                "{} messages · {} input / {} output tokens",
                self.messages, self.input_tokens, self.output_tokens
            )),
            "/approve" | "/auto" => {
                let value = match argument {
                    "on" | "yes" | "true" => true,
                    "off" | "no" | "false" => false,
                    "" => !self.info.auto_approve,
                    _ => {
                        self.push_warn("usage: /approve [on|off]");
                        return;
                    }
                };
                self.info.auto_approve = value;
                let _ = self.commands.send(WorkerCommand::SetAutoApprove(value));
                self.push_info(format!("auto-approve: {}", if value { "on" } else { "off" }));
            }
            "/system" => self.push_info(
                "restart with --system-file <path> or --system-prompt <text> to change the core prompt",
            ),
            "/login" => {
                if self.busy {
                    self.push_warn("wait for the current turn to finish first");
                    return;
                }
                let Some(choice) = self
                    .providers
                    .iter()
                    .find(|provider| provider.name == self.info.provider)
                    .cloned()
                else {
                    self.push_warn("no provider selected");
                    return;
                };
                if choice.config.kind == Some(ProviderKind::ChatGpt) {
                    self.start_login(choice);
                } else {
                    self.modal = Some(Modal::ApiKey {
                        provider: choice,
                        input: InputBuffer::new(),
                    });
                    self.status = "sign in".to_string();
                }
            }
            "/exit" | "/quit" => self.should_quit = true,
            other => self.push_warn(format!("unknown command {other}; try /help")),
        }
    }

    fn set_model(&mut self, model: &str) {
        let model = model.trim();
        if model.is_empty() {
            return;
        }
        self.model = model.to_string();
        let _ = self
            .commands
            .send(WorkerCommand::SetModel(model.to_string()));
        self.push_info(format!("model: {model}"));
    }

    fn open_provider_palette(&mut self) {
        self.palette = Some(Palette {
            kind: PaletteKind::Providers,
            items: self
                .providers
                .iter()
                .map(|provider| PaletteItem {
                    label: provider.name.clone(),
                    description: provider.model.clone(),
                    value: provider.name.clone(),
                })
                .collect(),
            selected: self
                .providers
                .iter()
                .position(|provider| provider.name == self.info.provider)
                .unwrap_or(0),
        });
    }

    fn switch_provider(&mut self, name: &str) {
        if self.busy {
            self.push_warn("wait for the current turn to finish before switching providers");
            return;
        }
        let Some(choice) = self
            .providers
            .iter()
            .find(|provider| provider.name == name)
            .cloned()
        else {
            self.push_warn(format!("unknown provider {name}"));
            return;
        };
        if choice.config.kind == Some(ProviderKind::ChatGpt) {
            match config::codex_auth() {
                Ok(auth) => {
                    self.complete_connect(
                        choice,
                        ProviderAuth {
                            key: auth.access_token,
                            account_id: Some(auth.account_id),
                        },
                    );
                }
                Err(_) => self.start_login(choice),
            }
            return;
        }
        if let Some(key) = provider_key(&choice) {
            self.complete_connect(
                choice,
                ProviderAuth {
                    key,
                    account_id: None,
                },
            );
        } else {
            self.modal = Some(Modal::ApiKey {
                provider: choice,
                input: InputBuffer::new(),
            });
            self.status = "sign in".to_string();
        }
    }

    fn complete_connect(&mut self, choice: ProviderChoice, auth: ProviderAuth) {
        match Provider::new(
            choice.name.clone(),
            choice.config.clone(),
            choice.model.clone(),
            auth,
        ) {
            Ok(provider) => {
                let _ = self.commands.send(WorkerCommand::SetProvider(
                    Box::new(provider),
                    choice.model.clone(),
                ));
                self.info.provider = choice.name.clone();
                self.model = choice.model.clone();
                self.connected = true;
                self.status = "ready".to_string();
                self.palette = None;
                self.push_info(format!("connected to {}/{}", choice.name, choice.model));
            }
            Err(error) => {
                self.push_warn(format!("could not connect to {}: {error:#}", choice.name))
            }
        }
    }

    fn start_login(&mut self, provider: ProviderChoice) {
        if let Some(task) = self.login_task.take() {
            task.abort();
        }
        self.modal = Some(Modal::Login {
            provider,
            url: None,
            code: None,
            status: "starting device sign-in…".to_string(),
            failed: false,
        });
        self.status = "sign in".to_string();
        let tx = self.login_tx.clone();
        let task = tokio::spawn(super::codex_device_login(tx));
        self.login_task = Some(task.abort_handle());
    }

    fn finish_login(&mut self, provider: ProviderChoice) {
        match config::codex_auth() {
            Ok(auth) => self.complete_connect(
                provider,
                ProviderAuth {
                    key: auth.access_token,
                    account_id: Some(auth.account_id),
                },
            ),
            Err(error) => self.push_warn(format!("sign-in did not produce credentials: {error:#}")),
        }
    }

    fn open_command_palette(&mut self) {
        self.palette = Some(Palette {
            kind: PaletteKind::Commands,
            items: COMMANDS
                .iter()
                .map(|command| PaletteItem {
                    label: command.name.to_string(),
                    description: command.description.to_string(),
                    value: command.name.to_string(),
                })
                .collect(),
            selected: 0,
        });
        self.refresh_palette();
    }

    fn refresh_palette(&mut self) {
        let Some(palette) = &mut self.palette else {
            return;
        };
        if palette.kind != PaletteKind::Commands {
            return;
        }
        let text = self.input.text();
        if !text.starts_with('/') || text.contains(' ') {
            self.palette = None;
            return;
        }
        let query = text.trim_start_matches('/').to_ascii_lowercase();
        palette.items = COMMANDS
            .iter()
            .filter(|command| {
                query.is_empty() || command.name[1..].to_ascii_lowercase().contains(&query)
            })
            .map(|command| PaletteItem {
                label: command.name.to_string(),
                description: command.description.to_string(),
                value: command.name.to_string(),
            })
            .collect();
        palette.selected = palette.selected.min(palette.items.len().saturating_sub(1));
    }

    fn show_models(&mut self, models: Vec<ModelInfo>) {
        if models.is_empty() {
            self.push_warn("the provider returned no models");
            self.status = "ready".to_string();
            return;
        }
        let current = self.model.clone();
        let items = models
            .into_iter()
            .map(|model| PaletteItem {
                label: if model.id == current {
                    format!("* {}", model.id)
                } else {
                    model.id.clone()
                },
                description: model.name.unwrap_or_default(),
                value: model.id,
            })
            .collect::<Vec<_>>();
        let selected = items
            .iter()
            .position(|item| item.value == current)
            .unwrap_or(0);
        self.palette = Some(Palette {
            kind: PaletteKind::Models,
            items,
            selected,
        });
        self.status = "ready".to_string();
    }

    pub fn handle_notice(&mut self, notice: AgentNotice) {
        match notice {
            AgentNotice::Text(delta) => {
                if let Some(Item::Assistant { text, streaming }) = self.items.last_mut()
                    && *streaming
                {
                    text.push_str(&delta);
                    return;
                }
                self.items.push(Item::Assistant {
                    text: delta,
                    streaming: true,
                });
            }
            AgentNotice::TextDone => {
                if let Some(Item::Assistant { streaming, .. }) = self.items.last_mut() {
                    *streaming = false;
                }
            }
            AgentNotice::ToolCall { id, name, detail } => {
                self.items.push(Item::Tool {
                    id,
                    name,
                    detail,
                    summary: String::new(),
                    state: ToolState::Running,
                });
            }
            AgentNotice::ToolResult {
                id,
                summary,
                is_error,
            } => {
                let state = if is_error {
                    ToolState::Error
                } else {
                    ToolState::Done
                };
                let mut found = false;
                for item in self.items.iter_mut().rev() {
                    if let Item::Tool {
                        id: tool_id,
                        summary: tool_summary,
                        state: tool_state,
                        ..
                    } = item
                        && *tool_id == id
                    {
                        *tool_summary = summary.clone();
                        *tool_state = state;
                        found = true;
                        break;
                    }
                }
                if !found {
                    self.items.push(Item::Tool {
                        id,
                        name: "tool".to_string(),
                        detail: String::new(),
                        summary,
                        state,
                    });
                }
            }
            AgentNotice::Todos(lines) => self.todos = lines,
            AgentNotice::Usage { input, output } => {
                self.input_tokens += input;
                self.output_tokens += output;
            }
            AgentNotice::Info(text) => self.push_info(text),
            AgentNotice::Warn(text) => self.push_warn(text),
        }
        self.dirty = true;
    }

    pub fn handle_tool_request(&mut self, request: ToolRequest) {
        match request {
            ToolRequest::Diff { path, old, new } => {
                self.items.push(Item::Diff { path, old, new });
            }
            ToolRequest::Confirm {
                title,
                body,
                respond,
            } => {
                self.approval = Some(ApprovalPrompt {
                    title,
                    body,
                    response: Some(respond),
                });
                self.status = "approval needed".to_string();
            }
            ToolRequest::Ask {
                question,
                options,
                respond,
            } => {
                self.modal = Some(Modal::Ask {
                    question,
                    options,
                    selected: 0,
                    input: InputBuffer::new(),
                    response: Some(respond),
                });
                self.status = "question".to_string();
            }
        }
        self.dirty = true;
    }

    pub fn handle_worker_event(&mut self, event: WorkerEvent) {
        match event {
            WorkerEvent::TurnDone(result) => {
                self.busy = false;
                self.ctrl_c_pending = false;
                self.status = "ready".to_string();
                if let Err(error) = result {
                    self.items.push(Item::Error(error));
                }
                if let Some(Item::Assistant { streaming, .. }) = self.items.last_mut() {
                    *streaming = false;
                }
            }
            WorkerEvent::Models(result) => match result {
                Ok(models) => self.show_models(models),
                Err(error) => {
                    self.status = "ready".to_string();
                    self.push_warn(error);
                }
            },
        }
        self.dirty = true;
    }

    pub fn tick(&mut self) {
        self.spinner = self.spinner.wrapping_add(1);
        if self.busy {
            self.dirty = true;
        }
    }

    fn push_info(&mut self, text: impl Into<String>) {
        self.items.push(Item::Info(text.into()));
    }

    fn push_warn(&mut self, text: impl Into<String>) {
        self.items.push(Item::Warn(text.into()));
    }

    fn scroll_up(&mut self, amount: u16) {
        self.auto_scroll = false;
        self.scroll = self.scroll.saturating_sub(amount);
    }

    fn scroll_down(&mut self, amount: u16) {
        self.scroll = self.scroll.saturating_add(amount).min(self.max_scroll);
        if self.scroll >= self.max_scroll {
            self.auto_scroll = true;
        }
    }

    fn scroll_to_bottom(&mut self) {
        self.auto_scroll = true;
        self.scroll = self.max_scroll;
    }

    fn history_prev(&mut self) {
        if self.history.is_empty() {
            return;
        }
        let index = match self.history_pos {
            Some(0) => 0,
            Some(index) => index - 1,
            None => self.history.len() - 1,
        };
        self.history_pos = Some(index);
        self.input.set_text(self.history[index].clone());
    }

    fn history_next(&mut self) {
        let Some(index) = self.history_pos else {
            return;
        };
        if index + 1 < self.history.len() {
            let next = index + 1;
            self.history_pos = Some(next);
            self.input.set_text(self.history[next].clone());
        } else {
            self.history_pos = None;
            self.input.clear();
        }
    }
}

fn approval_height(prompt: &ApprovalPrompt) -> u16 {
    (prompt.body.lines().count() as u16 + 5).clamp(6, 10)
}

fn open_browser(url: &str) {
    #[cfg(target_os = "macos")]
    let command = "open";
    #[cfg(target_os = "windows")]
    let command = "explorer";
    #[cfg(all(unix, not(target_os = "macos")))]
    let command = "xdg-open";
    let _ = std::process::Command::new(command)
        .arg(url)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
}

fn provider_key(choice: &ProviderChoice) -> Option<String> {
    if let Some(key) = config::find_api_key(&choice.name, &choice.config) {
        return Some(key);
    }
    if choice.config.api_key.is_none()
        && choice.config.api_key_env.is_none()
        && let Some(base) = &choice.config.base_url
        && (base.contains("localhost") || base.contains("127.0.0.1"))
    {
        return Some("local".to_string());
    }
    None
}

fn assistant_lines(text: &str, streaming: bool, frame_index: usize) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    let mut code = false;
    let mut first = true;
    let base = Style::default().fg(LIGHT).bg(ASSISTANT_BG);
    for raw in text.lines() {
        let clean = raw.trim_end();
        if clean.trim_start().starts_with("```") {
            code = !code;
            continue;
        }
        let prefix = if first { " ◆ " } else { "   " };
        first = false;
        let prefix_span = Span::styled(prefix, Style::default().fg(ACCENT).bg(ASSISTANT_BG));
        if code {
            lines.push(Line::from(vec![
                prefix_span,
                Span::styled(
                    format!(" {clean} "),
                    Style::default().fg(Color::Rgb(190, 210, 230)).bg(CODE_BG),
                ),
            ]));
            continue;
        }
        if let Some(heading) = clean.strip_prefix('#') {
            lines.push(Line::from(vec![
                prefix_span,
                Span::styled(
                    format!("{} ", heading.trim_start_matches('#').trim()),
                    base.fg(ACCENT).add_modifier(Modifier::BOLD),
                ),
            ]));
            continue;
        }
        if let Some(bullet) = clean
            .strip_prefix("- ")
            .or_else(|| clean.strip_prefix("* "))
        {
            let mut spans = vec![
                prefix_span,
                Span::styled("• ", Style::default().fg(ACCENT).bg(ASSISTANT_BG)),
            ];
            spans.extend(inline_spans(bullet, base));
            spans.push(Span::styled(" ", Style::default().bg(ASSISTANT_BG)));
            lines.push(Line::from(spans));
            continue;
        }
        let mut spans = vec![prefix_span];
        spans.extend(inline_spans(clean, base));
        spans.push(Span::styled(" ", Style::default().bg(ASSISTANT_BG)));
        lines.push(Line::from(spans));
    }
    if streaming {
        lines.push(Line::from(Span::styled(
            format!("   {} ", spinner(frame_index)),
            Style::default().fg(ACCENT).bg(ASSISTANT_BG),
        )));
    }
    lines
}

fn inline_spans(text: &str, base: Style) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find('`') {
        if start > 0 {
            spans.push(Span::styled(rest[..start].to_string(), base));
        }
        let after = &rest[start + 1..];
        if let Some(end) = after.find('`') {
            spans.push(Span::styled(
                after[..end].to_string(),
                base.fg(Color::Rgb(150, 205, 255)).bg(CODE_BG),
            ));
            rest = &after[end + 1..];
        } else {
            spans.push(Span::styled(rest[start..].to_string(), base));
            return spans;
        }
    }
    if !rest.is_empty() {
        spans.push(Span::styled(rest.to_string(), base));
    }
    spans
}

fn diff_lines(path: &str, old: &str, new: &str) -> Vec<Line<'static>> {
    let diff = TextDiff::from_lines(old, new);
    let rendered = diff
        .unified_diff()
        .context_radius(2)
        .header(&format!("a/{path}"), &format!("b/{path}"))
        .to_string();
    let mut lines = vec![Line::from(vec![
        Span::styled("  ┌ ", Style::default().fg(FAINT)),
        Span::styled(path.to_string(), Style::default().fg(TOOL)),
    ])];
    for line in rendered.lines().skip(2).take(80) {
        let style = if line.starts_with('+') {
            Style::default().fg(GREEN)
        } else if line.starts_with('-') {
            Style::default().fg(RED)
        } else if line.starts_with("@@") {
            Style::default().fg(TOOL)
        } else {
            Style::default().fg(DIM)
        };
        lines.push(Line::from(Span::styled(format!("  │ {line}"), style)));
    }
    if rendered.lines().count() > 82 {
        lines.push(Line::from(Span::styled(
            "  │ … diff truncated",
            Style::default().fg(FAINT),
        )));
    }
    lines
}

fn kv(key: &str, value: &str, color: Color) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!("{key:<10}"), Style::default().fg(FAINT)),
        Span::styled(truncate(value, 21), Style::default().fg(color)),
    ])
}

fn spinner(index: usize) -> &'static str {
    const FRAMES: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
    FRAMES[index % FRAMES.len()]
}

fn truncate(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        text.to_string()
    } else {
        format!("{}…", text.chars().take(max).collect::<String>())
    }
}

fn short(value: u64) -> String {
    if value >= 1_000_000 {
        format!("{:.1}m", value as f64 / 1_000_000.0)
    } else if value >= 1_000 {
        format!("{:.1}k", value as f64 / 1_000.0)
    } else {
        value.to_string()
    }
}

fn help_text() -> String {
    [
        "Commands",
        "/help              show this help",
        "/new               start a fresh conversation",
        "/model [id]        show or change the model",
        "/models            fetch and choose from the provider catalog",
        "/provider          show the current provider",
        "/tools             list available tools",
        "/skills            list available skills",
        "/mcp               list connected MCP servers",
        "/context           show messages and token usage",
        "/approve [on|off]  toggle auto-approval",
        "/system            how to change the system prompt",
        "/login             how to change API credentials",
        "/exit              leave harness",
        "",
        "Keys",
        "Enter              send",
        "Shift+Enter        newline",
        "/                  command palette",
        "Ctrl+B             toggle the sidebar",
        "PageUp/PageDown    scroll the conversation",
        "Ctrl+C twice       quit while a turn is running",
    ]
    .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn app() -> (App, mpsc::UnboundedReceiver<WorkerCommand>) {
        let (tx, rx) = mpsc::unbounded_channel();
        let (login_tx, _login_rx) = mpsc::unbounded_channel();
        let info = TuiInfo {
            provider: "test".to_string(),
            model: "test-model".to_string(),
            workspace: PathBuf::from("/tmp/test-workspace"),
            branch: Some("main".to_string()),
            skills: 2,
            mcp: 0,
            tools: 9,
            auto_approve: false,
            connected: true,
            providers: Vec::new(),
        };
        (App::new(info, tx, login_tx), rx)
    }

    fn key(app: &mut App, code: KeyCode) {
        app.handle_event(Event::Key(KeyEvent::new(code, KeyModifiers::NONE)));
    }

    fn text(app: &mut App) -> String {
        let backend = TestBackend::new(120, 40);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| app.draw(frame)).unwrap();
        terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect()
    }

    #[test]
    fn slash_opens_and_filters_the_command_palette() {
        let (mut app, _rx) = app();
        key(&mut app, KeyCode::Char('/'));
        eprintln!("input after slash: {:?}", app.input.text());
        let screen = text(&mut app);
        assert!(screen.contains("commands"), "{screen}");
        assert!(screen.contains("/models"), "{screen}");
        key(&mut app, KeyCode::Char('m'));
        key(&mut app, KeyCode::Char('o'));
        let screen = text(&mut app);
        assert!(screen.contains("/model"), "{screen}");
        assert!(!screen.contains("/help"), "{screen}");
        key(&mut app, KeyCode::Esc);
        let screen = text(&mut app);
        assert!(!screen.contains("fetch and choose"), "{screen}");
    }

    #[test]
    fn submitting_sends_a_turn_and_streams_the_reply() {
        let (mut app, mut rx) = app();
        for ch in "hello".chars() {
            key(&mut app, KeyCode::Char(ch));
        }
        key(&mut app, KeyCode::Enter);
        match rx.try_recv().unwrap() {
            WorkerCommand::Run(text) => assert_eq!(text, "hello"),
            _ => panic!("expected a run command"),
        }
        app.handle_notice(AgentNotice::Text("hi there".to_string()));
        let screen = text(&mut app);
        assert!(screen.contains("hi there"), "{screen}");
        app.handle_worker_event(WorkerEvent::TurnDone(Ok(())));
    }

    #[test]
    fn switching_providers_rebuilds_the_agent_provider() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let (login_tx, _login_rx) = mpsc::unbounded_channel();
        let choice = ProviderChoice {
            name: "alt".to_string(),
            model: "alt-model".to_string(),
            config: ProviderConfig {
                base_url: Some("http://127.0.0.1:1/v1".to_string()),
                api_key: Some("test-key".to_string()),
                kind: Some(ProviderKind::OpenAi),
                model: Some("alt-model".to_string()),
                ..Default::default()
            },
        };
        let info = TuiInfo {
            provider: "alt".to_string(),
            model: "alt-model".to_string(),
            workspace: PathBuf::from("/tmp/test-workspace"),
            branch: None,
            skills: 0,
            mcp: 0,
            tools: 9,
            auto_approve: false,
            connected: false,
            providers: vec![choice],
        };
        let mut app = App::new(info, tx, login_tx);
        app.switch_provider("alt");
        assert!(app.connected);
        assert_eq!(app.model, "alt-model");
        assert!(matches!(
            rx.try_recv().unwrap(),
            WorkerCommand::SetProvider(_, _)
        ));
    }

    #[test]
    fn approval_modal_answers_the_tool() {
        let (mut app, _rx) = app();
        let (tx, mut rx) = oneshot::channel();
        app.handle_tool_request(ToolRequest::Confirm {
            title: "run?".to_string(),
            body: "printf hi".to_string(),
            respond: tx,
        });
        let screen = text(&mut app);
        assert!(screen.contains("permission required"), "{screen}");
        assert!(screen.contains("run?"), "{screen}");
        assert!(screen.contains("allow"), "{screen}");
        assert!(app.modal.is_none());
        assert!(app.approval.is_some());
        key(&mut app, KeyCode::Char('y'));
        assert!(rx.try_recv().unwrap());
    }
}
