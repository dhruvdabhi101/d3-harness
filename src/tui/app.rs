use super::input::InputBuffer;
use super::{AgentNotice, LoginEvent, ToolRequest, WorkerCommand, WorkerEvent};
use crate::config::{self, ProviderConfig, ProviderKind};
use crate::provider::{ModelInfo, Provider, ProviderAuth};
use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseEventKind};
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{
        Block, BorderType, Clear, List, ListItem, ListState, Paragraph, Scrollbar,
        ScrollbarOrientation, ScrollbarState, Wrap,
    },
};
use similar::TextDiff;
use std::path::PathBuf;
use tokio::sync::{mpsc, oneshot};
use tui_textarea::{TextArea, WrapMode};

const ACCENT: Color = Color::Rgb(255, 153, 102);
const ACCENT_DIM: Color = Color::Rgb(160, 100, 70);
const TOOL: Color = Color::Rgb(120, 180, 255);
const GREEN: Color = Color::Rgb(120, 210, 150);
const RED: Color = Color::Rgb(240, 120, 120);
const YELLOW: Color = Color::Rgb(240, 200, 120);
const DIM: Color = Color::Rgb(135, 135, 145);
const FAINT: Color = Color::Rgb(80, 80, 90);
const USER_BG: Color = Color::Rgb(35, 43, 56);
const ASSISTANT_BG: Color = Color::Rgb(26, 29, 35);
const CODE_BG: Color = Color::Rgb(36, 38, 46);
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
    pub skills: Vec<String>,
    pub mcp: usize,
    pub tools: usize,
    pub auto_approve: bool,
    pub connected: bool,
    pub providers: Vec<ProviderChoice>,
    pub ui: config::UiConfig,
}

pub struct App {
    info: TuiInfo,
    save_selection: fn(&str, &str) -> anyhow::Result<()>,
    model: String,
    items: Vec<Item>,
    input: TextArea<'static>,
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
    tool_hits: Vec<(Rect, usize)>,
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
        expanded: bool,
        output: String,
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
    Skills,
    Models,
    Providers,
    Sessions,
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
        name: "/resume",
        description: "choose a saved conversation in this project",
    },
    CommandSpec {
        name: "/continue",
        description: "continue the latest conversation in this project",
    },
    CommandSpec {
        name: "/ui",
        description: "show UI customization settings",
    },
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
            save_selection: config::store_selection,
            model,
            items: Vec::new(),
            input: TextArea::default(),
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
            tool_hits: Vec::new(),
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
                "Type a request, or / for commands. /resume opens project conversations.",
            );
        } else {
            app.status = "choose provider".to_string();
            app.push_info("Choose a provider, then sign in. Everything stays inside this screen.");
            app.open_provider_palette();
        }
        app
    }

    pub fn restore_messages(&mut self, messages: &[crate::conversation::Message]) {
        use crate::conversation::{Block, Role};
        for message in messages {
            for block in &message.blocks {
                match block {
                    Block::Text(text) if message.role == Role::User => {
                        self.history.push(text.clone());
                        self.items.push(Item::User(text.clone()));
                        self.messages += 1;
                    }
                    Block::Text(text) if message.role == Role::Assistant => {
                        self.items.push(Item::Assistant {
                            text: text.clone(),
                            streaming: false,
                        });
                    }
                    Block::ToolCall(call) => self.handle_notice(AgentNotice::ToolCall {
                        id: call.id.clone(),
                        name: call.name.clone(),
                        detail: call.arguments.clone(),
                    }),
                    Block::ToolResult {
                        tool_call_id,
                        content,
                        is_error,
                    } => {
                        self.handle_notice(AgentNotice::ToolResult {
                            id: tool_call_id.clone(),
                            summary: String::new(),
                            is_error: *is_error,
                        });
                        self.handle_notice(AgentNotice::ToolOutput {
                            id: tool_call_id.clone(),
                            content: content.clone(),
                        });
                    }
                    _ => {}
                }
            }
        }
        if !messages.is_empty() {
            self.push_info(
                "Resumed project conversation. Click a tool row to inspect its saved output.",
            );
        }
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

        self.input.set_wrap_mode(WrapMode::WordOrGlyph);
        self.input.set_block(Block::bordered());
        let input_height = self
            .input
            .measure(area.width)
            .content_rows
            .clamp(1, self.info.ui.editor_max_height.unwrap_or(6));
        let approval_height = self.approval.as_ref().map(approval_height).unwrap_or(0);
        let bottom_height = if approval_height > 0 {
            approval_height
        } else {
            input_height + 2
        };
        let chunks = Layout::vertical([
            Constraint::Min(3),
            Constraint::Length(bottom_height),
            Constraint::Length(1),
        ])
        .split(area);

        self.draw_chat(frame, chunks[0]);
        self.draw_status(frame, chunks[2]);

        if let Some(approval) = &self.approval {
            self.draw_approval(frame, approval, chunks[1]);
        } else {
            self.draw_input(frame, chunks[1]);
            if let Some(palette) = &self.palette {
                self.draw_palette(frame, palette, chunks[1]);
            }
        }
        if let Some(modal) = &self.modal {
            self.draw_modal(frame, modal, area);
        }
        let colors = [
            (ACCENT, self.info.ui.accent.as_deref()),
            (USER_BG, self.info.ui.user_background.as_deref()),
            (ASSISTANT_BG, self.info.ui.assistant_background.as_deref()),
        ];
        let colors: Vec<_> = colors
            .into_iter()
            .filter_map(|(original, configured)| {
                configured.map(|value| {
                    let rgb = u32::from_str_radix(&value[1..], 16).expect("validated UI color");
                    (
                        original,
                        Color::Rgb((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8),
                    )
                })
            })
            .collect();
        for cell in &mut frame.buffer_mut().content {
            if let Some((_, color)) = colors.iter().find(|(original, _)| *original == cell.fg) {
                cell.fg = *color;
            }
            if let Some((_, color)) = colors.iter().find(|(original, _)| *original == cell.bg) {
                cell.bg = *color;
            }
        }
    }

    fn draw_status(&self, frame: &mut Frame, area: Rect) {
        let approval = if self.info.auto_approve {
            "auto"
        } else {
            "ask"
        };
        let branch = self.info.branch.as_deref().unwrap_or("no branch");
        let status = format!(
            " {}/{} · {} · approval {} · {}",
            self.info.provider, self.model, branch, approval, self.status
        );
        frame.render_widget(Paragraph::new(status).style(Style::default().fg(DIM)), area);
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
        let paragraph = Paragraph::new(Text::from(self.chat_lines())).wrap(Wrap { trim: false });
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
        self.tool_hits.clear();
        let mut start = 1usize;
        for (index, item) in self.items.iter().enumerate() {
            let item_lines = self.item_lines(item);
            let end = start
                + Paragraph::new(Text::from(item_lines))
                    .wrap(Wrap { trim: false })
                    .line_count(inner.width);
            if matches!(item, Item::Tool { .. }) {
                let top = start.max(self.scroll as usize);
                let bottom = end.min(self.scroll as usize + inner.height as usize);
                if bottom > top {
                    self.tool_hits.push((
                        Rect::new(
                            inner.x,
                            inner.y + (top - self.scroll as usize) as u16,
                            inner.width,
                            (bottom - top) as u16,
                        ),
                        index,
                    ));
                }
            }
            start = end;
        }
        let paragraph = paragraph.scroll((self.scroll, 0));
        frame.render_widget(paragraph, inner);
        let buffer = frame.buffer_mut();
        for y in inner.top()..inner.bottom() {
            let background = buffer[(inner.left(), y)].bg;
            if matches!(background, USER_BG | ASSISTANT_BG | CODE_BG) {
                for x in inner.left()..inner.right() {
                    let cell = &mut buffer[(x, y)];
                    if cell.bg == Color::Reset {
                        cell.bg = background;
                    }
                }
            }
        }

        if total > inner.height as usize && area.width > 6 {
            let bar_area = Rect {
                x: area.x + area.width.saturating_sub(2),
                y: inner.y,
                width: 1,
                height: inner.height,
            };
            let mut state = ScrollbarState::new(total)
                .position(self.scroll as usize)
                .viewport_content_length(inner.height as usize);
            let bar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
                .thumb_symbol("┃")
                .track_symbol(Some("│"))
                .style(Style::default().fg(FAINT))
                .thumb_style(Style::default().fg(ACCENT_DIM));
            frame.render_stateful_widget(bar, bar_area, &mut state);
        }
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

    fn draw_input(&mut self, frame: &mut Frame, area: Rect) {
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
        self.input.set_block(block);
        self.input.set_style(Style::default().fg(LIGHT).bg(USER_BG));
        self.input.set_cursor_line_style(Style::default());
        self.input
            .set_cursor_style(Style::default().fg(Color::Rgb(20, 20, 24)).bg(ACCENT));
        self.input.set_placeholder_text(if self.busy {
            "Waiting for the current turn…"
        } else {
            "Ask anything, or type / for commands"
        });
        self.input.set_placeholder_style(Style::default().fg(FAINT));
        self.input.clear_custom_highlight();
        let lines = self.input.lines().to_vec();
        for (row, line) in lines.iter().enumerate() {
            for (start, _) in line.match_indices('$') {
                let name: String = line[start + 1..]
                    .chars()
                    .take_while(|ch| ch.is_alphanumeric() || *ch == '-' || *ch == '_')
                    .collect();
                if self.info.skills.contains(&name) {
                    let col = line[..start].chars().count();
                    self.input.custom_highlight(
                        ((row, col), (row, col + name.chars().count() + 1)),
                        Style::default().fg(ACCENT),
                        1,
                    );
                }
            }
        }
        frame.render_widget(&self.input, area);
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
            PaletteKind::Skills => " skills ",
            PaletteKind::Models => " models ",
            PaletteKind::Providers => " providers ",
            PaletteKind::Sessions => " project conversations ",
        };
        let block = Block::bordered()
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(ACCENT_DIM))
            .title(Line::from(Span::styled(title, Style::default().fg(ACCENT))));
        let inner = block.inner(area);
        frame.render_widget(block, area);

        let items = palette
            .items
            .iter()
            .map(|item| {
                ListItem::new(Line::from(vec![
                    Span::styled(format!("  {:<20} ", item.label), Style::default().fg(LIGHT)),
                    Span::styled(truncate(&item.description, 46), Style::default().fg(DIM)),
                ]))
            })
            .collect::<Vec<_>>();
        let list = List::new(items)
            .highlight_style(
                Style::default()
                    .fg(Color::Rgb(20, 20, 24))
                    .bg(ACCENT)
                    .add_modifier(Modifier::BOLD),
            )
            .highlight_symbol("› ");
        let mut state = ListState::default()
            .with_selected(Some(palette.selected))
            .with_offset(palette.selected.saturating_sub(visible.saturating_sub(1)));
        frame.render_stateful_widget(list, inner, &mut state);
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
                        Span::raw(input.split_at_cursor().0.to_string()),
                        Span::styled("▌", Style::default().fg(ACCENT)),
                        Span::raw(input.split_at_cursor().1.to_string()),
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
                        Span::raw(input.split_at_cursor().0.to_string()),
                        Span::styled("▌", Style::default().fg(ACCENT)),
                        Span::raw(input.split_at_cursor().1.to_string()),
                    ]));
                }
                lines.push(Line::default());
                lines.push(Line::from(Span::styled(
                    "Enter submit · ↑/↓ choose · ←/→ edit · Esc skip",
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
            lines.extend(self.item_lines(item));
        }
        lines
    }

    fn item_lines(&self, item: &Item) -> Vec<Line<'static>> {
        let mut lines = Vec::new();
        match item {
            Item::User(text) => {
                let mut first = true;
                for line in text.lines() {
                    let prefix = if first { "› " } else { "  " };
                    first = false;
                    lines.push(
                        Line::from(vec![
                            Span::styled(
                                format!(" {prefix}"),
                                Style::default().fg(ACCENT).bg(USER_BG),
                            ),
                            Span::styled(
                                format!("{line} "),
                                Style::default().fg(LIGHT).bg(USER_BG),
                            ),
                        ])
                        .style(Style::default().bg(USER_BG)),
                    );
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
                expanded,
                output,
                state,
                ..
            } => {
                let (marker, color) = match state {
                    ToolState::Running => (spinner(self.spinner), YELLOW),
                    ToolState::Done => ("●", GREEN),
                    ToolState::Error => ("●", RED),
                };
                lines.push(Line::from(vec![
                    Span::styled(
                        format!("  {} {marker} ", if *expanded { "▾" } else { "▸" }),
                        Style::default().fg(color),
                    ),
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
                let visible = if *expanded {
                    if output.is_empty() {
                        summary.as_str()
                    } else {
                        output.as_str()
                    }
                } else {
                    ""
                };
                for line in visible.lines() {
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
        lines
    }

    pub fn handle_event(&mut self, event: Event) -> bool {
        match event {
            Event::Key(key) if key.kind != KeyEventKind::Release => self.handle_key(key),
            Event::Mouse(mouse) => match mouse.kind {
                MouseEventKind::Down(crossterm::event::MouseButton::Left)
                    if self.modal.is_none()
                        && self.approval.is_none()
                        && self.palette.is_none() =>
                {
                    if let Some((_, index)) = self
                        .tool_hits
                        .iter()
                        .find(|(rect, _)| rect.contains((mouse.column, mouse.row).into()))
                        && let Item::Tool { expanded, .. } = &mut self.items[*index]
                    {
                        *expanded = !*expanded;
                    }
                }
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
                KeyCode::Char('a') | KeyCode::Char('e') => {
                    self.input.input(key);
                }
                KeyCode::Char('u') => {
                    self.input.clear();
                    self.refresh_palette();
                }
                KeyCode::Char('w') => {
                    self.input.input(key);
                    self.refresh_palette();
                }
                KeyCode::Char('d') => {
                    if self.input.is_empty() {
                        self.should_quit = true;
                    }
                }
                KeyCode::Up => self.scroll_up(4),
                KeyCode::Down => self.scroll_down(4),
                KeyCode::Home => {
                    self.auto_scroll = false;
                    self.scroll = 0;
                }
                KeyCode::End => self.scroll_to_bottom(),
                KeyCode::Char('j') | KeyCode::Enter => {
                    self.input.insert_str("\n");
                }
                _ => {}
            }
            return;
        }

        match key.code {
            KeyCode::Enter
                if key.modifiers.contains(KeyModifiers::SHIFT)
                    || key.modifiers.contains(KeyModifiers::ALT) =>
            {
                self.input.insert_str("\n");
            }
            KeyCode::Enter => self.submit(),
            KeyCode::Tab => {
                if self.input_text().starts_with('/') {
                    self.open_command_palette();
                } else {
                    self.input.insert_str("    ");
                }
            }
            KeyCode::Backspace | KeyCode::Delete => {
                self.input.input(key);
                self.refresh_palette();
            }
            KeyCode::Left | KeyCode::Right | KeyCode::Home | KeyCode::End => {
                self.input.input(key);
            }
            KeyCode::Up => {
                if self.input.lines().len() <= 1 && !self.history.is_empty() {
                    self.history_prev();
                } else {
                    self.input.input(key);
                }
            }
            KeyCode::Down => {
                if self.history_pos.is_some() {
                    self.history_next();
                } else {
                    self.input.input(key);
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
                self.input.input(key);
                if was_empty && ch == '/' {
                    self.open_command_palette();
                } else if ch == '$' {
                    self.open_skill_palette();
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
                if palette.items.is_empty() && palette.kind == PaletteKind::Skills {
                    self.palette = None;
                    if key.code == KeyCode::Enter {
                        self.submit();
                    }
                    return;
                }
                if let Some(item) = palette.items.get(palette.selected).cloned() {
                    let kind = palette.kind;
                    self.palette = None;
                    match kind {
                        PaletteKind::Commands => {
                            self.input.clear();
                            self.run_command(&item.value);
                        }
                        PaletteKind::Skills => {
                            let (row, col) = self.input.cursor();
                            let line = &self.input.lines()[row];
                            let before: String = line.chars().take(col).collect();
                            let query_len = before
                                .rsplit_once('$')
                                .map_or(0, |(_, q)| q.chars().count() + 1);
                            for _ in 0..query_len {
                                self.input.delete_char();
                            }
                            self.input.insert_str(format!("${} ", item.value));
                        }
                        PaletteKind::Models => {
                            self.input.clear();
                            self.set_model(&item.value);
                        }
                        PaletteKind::Sessions => {
                            self.input.clear();
                            self.busy = true;
                            let _ = self.commands.send(WorkerCommand::Resume(Some(item.value)));
                        }
                        PaletteKind::Providers => {
                            self.input.clear();
                            self.switch_provider(&item.value);
                        }
                    }
                }
            }
            KeyCode::Backspace => {
                self.input.input(key);
                self.refresh_palette();
            }
            KeyCode::Char(_) => {
                self.input.input(key);
                self.refresh_palette();
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
                    KeyCode::Left
                    | KeyCode::Right
                    | KeyCode::Home
                    | KeyCode::End
                    | KeyCode::Delete => {
                        match key.code {
                            KeyCode::Left => input.left(),
                            KeyCode::Right => input.right(),
                            KeyCode::Home => input.home(),
                            KeyCode::End => input.end(),
                            KeyCode::Delete => input.delete(),
                            _ => unreachable!(),
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
        let text = self.input_text().trim().to_string();
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
            "/ui" => self.push_info("Customize ui in harness.json or ~/.config/harness/harness.json. Settings: accent, user_background, assistant_background (#RRGGBB), tool_output_expanded (boolean), editor_max_height (1..20). Restart to apply."),
            "/resume" | "/continue" => {
                if self.busy { self.push_warn("wait for the current turn to finish first"); return; }
                self.busy = true;
                self.status = "loading conversations".to_string();
                let command = if command == "/continue" { WorkerCommand::Resume(None) }
                    else if argument.is_empty() { WorkerCommand::ListSessions }
                    else { WorkerCommand::Resume(Some(argument.to_string())) };
                let _ = self.commands.send(command);
            }
            "/new" | "/clear" => {
                if self.busy {
                    self.push_warn("wait for the current turn to finish first");
                    return;
                }
                self.items.clear();
                self.history.clear();
                self.history_pos = None;
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
            "/skills" => {
                self.set_input_text("$");
                self.open_skill_palette();
            },
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
        if let Err(error) = (self.save_selection)(&self.info.provider, model) {
            self.push_warn(format!("could not save provider and model: {error:#}"));
        }
        self.push_info(format!("model: {model}"));
    }

    fn input_text(&self) -> String {
        self.input.lines().join("\n")
    }

    fn set_input_text(&mut self, text: impl AsRef<str>) {
        self.input.clear();
        self.input.insert_str(text.as_ref());
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
                if let Err(error) = (self.save_selection)(&choice.name, &choice.model) {
                    self.push_warn(format!("could not save provider and model: {error:#}"));
                }
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

    fn open_skill_palette(&mut self) {
        self.palette = Some(Palette {
            kind: PaletteKind::Skills,
            items: Vec::new(),
            selected: 0,
        });
        self.refresh_palette();
    }

    fn refresh_palette(&mut self) {
        if self
            .palette
            .as_ref()
            .is_some_and(|p| p.kind == PaletteKind::Skills)
        {
            let (row, col) = self.input.cursor();
            let before: String = self.input.lines()[row].chars().take(col).collect();
            let Some((_, query)) = before.rsplit_once('$') else {
                self.palette = None;
                return;
            };
            if query.chars().any(char::is_whitespace) {
                self.palette = None;
                return;
            }
            let query = query.to_ascii_lowercase();
            let palette = self.palette.as_mut().unwrap();
            palette.items = self
                .info
                .skills
                .iter()
                .filter(|name| name.to_ascii_lowercase().contains(&query))
                .map(|name| PaletteItem {
                    label: name.clone(),
                    description: String::new(),
                    value: name.clone(),
                })
                .collect();
            palette.selected = palette.selected.min(palette.items.len().saturating_sub(1));
            return;
        }
        let text = self.input_text();
        let Some(palette) = &mut self.palette else {
            return;
        };
        if palette.kind != PaletteKind::Commands {
            return;
        }
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
                    expanded: self.info.ui.tool_output_expanded.unwrap_or(false),
                    output: String::new(),
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
                        expanded: self.info.ui.tool_output_expanded.unwrap_or(false),
                        output: String::new(),
                        id,
                        name: "tool".to_string(),
                        detail: String::new(),
                        summary,
                        state,
                    });
                }
            }
            AgentNotice::ToolOutput { id, content } => {
                if let Some(Item::Tool { output, .. }) =
                    self.items.iter_mut().rev().find(
                        |item| matches!(item, Item::Tool { id: tool_id, .. } if *tool_id == id),
                    )
                {
                    *output = content;
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
            WorkerEvent::Sessions(result) => {
                self.busy = false;
                self.status = "ready".to_string();
                match result {
                    Ok(sessions) if sessions.is_empty() => {
                        self.push_info("No saved conversations in this project.")
                    }
                    Ok(sessions) => {
                        self.palette = Some(Palette {
                            kind: PaletteKind::Sessions,
                            selected: 0,
                            items: sessions
                                .into_iter()
                                .map(|session| PaletteItem {
                                    label: session.title,
                                    description: session.id.clone(),
                                    value: session.id,
                                })
                                .collect(),
                        })
                    }
                    Err(error) => self.push_warn(error),
                }
            }
            WorkerEvent::Resumed(result) => {
                self.busy = false;
                self.status = "ready".to_string();
                match result {
                    Ok(messages) => {
                        self.items.clear();
                        self.history.clear();
                        self.history_pos = None;
                        self.messages = 0;
                        self.input_tokens = 0;
                        self.output_tokens = 0;
                        self.todos.clear();
                        self.auto_scroll = true;
                        self.restore_messages(&messages);
                    }
                    Err(error) => self.push_warn(error),
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
        self.set_input_text(self.history[index].clone());
    }

    fn history_next(&mut self) {
        let Some(index) = self.history_pos else {
            return;
        };
        if index + 1 < self.history.len() {
            let next = index + 1;
            self.history_pos = Some(next);
            self.set_input_text(self.history[next].clone());
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
    let rendered = tui_markdown::from_str(text);
    let mut lines = Vec::new();
    let mut in_code = false;
    for (index, mut line) in rendered.lines.into_iter().enumerate() {
        let plain = line
            .spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect::<String>();
        if plain.trim_start().starts_with("```") {
            in_code = !in_code;
            continue;
        }
        if let Some(first) = line.spans.first() {
            let marker = first.content.as_ref();
            if marker.contains('#') && marker.chars().all(|ch| ch == '#' || ch == ' ') {
                line.spans.remove(0);
            } else if marker == "- " || marker == "* " {
                line.spans[0] = Span::styled("• ", Style::default().fg(ACCENT));
            } else if marker == ">" {
                line.spans[0] = Span::styled("│ ", Style::default().fg(ACCENT));
                if line.spans.get(1).is_some_and(|span| span.content == " ") {
                    line.spans.remove(1);
                }
            }
        }
        let prefix = if index == 0 { " ◆ " } else { "   " };
        let mut spans = vec![Span::styled(prefix, Style::default().fg(ACCENT))];
        spans.extend(
            line.spans
                .into_iter()
                .map(|span| Span::styled(span.content.into_owned(), span.style)),
        );
        spans.push(Span::styled(" ", Style::default()));
        let background = if in_code { CODE_BG } else { ASSISTANT_BG };
        lines.push(Line::from(spans).style(Style::default().fg(LIGHT).bg(background)));
    }
    if streaming {
        lines.push(Line::from(Span::styled(
            format!("   {} ", spinner(frame_index)),
            Style::default().fg(ACCENT).bg(ASSISTANT_BG),
        )));
    }
    lines
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
        "Ctrl+Enter / Shift+Enter / Alt+Enter / Ctrl+J  newline (terminal support varies)",
        "/                  command palette",
        "/resume [id]       choose a project conversation",
        "/continue          resume the latest project conversation",
        "/ui                show customization settings",
        "Click tool row     expand or collapse output",
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
            skills: vec!["review".to_string(), "teach".to_string()],
            mcp: 0,
            tools: 9,
            auto_approve: false,
            connected: true,
            providers: Vec::new(),
            ui: config::UiConfig::default(),
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
    fn tool_output_expands_and_collapses_on_click_after_wrapping_and_scrolling() {
        let (mut app, _rx) = app();
        app.items.push(Item::User("wrapped text ".repeat(40)));
        app.handle_notice(AgentNotice::ToolCall {
            id: "g".into(),
            name: "grep".into(),
            detail: "query ".repeat(30),
        });
        app.handle_notice(AgentNotice::ToolResult {
            id: "g".into(),
            summary: "one match".into(),
            is_error: false,
        });
        app.handle_notice(AgentNotice::ToolOutput {
            id: "g".into(),
            content: "hidden-result-line\n".repeat(50),
        });
        assert!(!text(&mut app).contains("hidden-result-line"));
        let hit = app.tool_hits[0].0;
        let click = Event::Mouse(crossterm::event::MouseEvent {
            kind: MouseEventKind::Down(crossterm::event::MouseButton::Left),
            column: hit.x,
            row: hit.y,
            modifiers: KeyModifiers::NONE,
        });
        app.handle_event(click.clone());
        assert!(text(&mut app).contains("hidden-result-line"));
        app.scroll_up(10);
        text(&mut app);
        let hit = app.tool_hits[0].0;
        app.handle_event(Event::Mouse(crossterm::event::MouseEvent {
            kind: MouseEventKind::Down(crossterm::event::MouseButton::Left),
            column: hit.x,
            row: hit.y,
            modifiers: KeyModifiers::NONE,
        }));
        assert!(!text(&mut app).contains("hidden-result-line"));
    }

    #[test]
    fn resume_picker_replaces_chat_and_restores_tool_results() {
        let (mut app, mut rx) = app();
        app.run_command("/resume");
        assert!(matches!(rx.try_recv(), Ok(WorkerCommand::ListSessions)));
        app.handle_worker_event(WorkerEvent::Sessions(Ok(vec![
            crate::session::SessionSummary {
                id: "saved".into(),
                title: "old task".into(),
            },
        ])));
        assert!(text(&mut app).contains("old task"));
        key(&mut app, KeyCode::Enter);
        assert!(matches!(rx.try_recv(), Ok(WorkerCommand::Resume(Some(id))) if id == "saved"));
        app.handle_worker_event(WorkerEvent::Resumed(Ok(vec![
            crate::conversation::Message::user("old task"),
            crate::conversation::Message::assistant(
                String::new(),
                vec![crate::conversation::ToolCall {
                    id: "g".into(),
                    name: "grep".into(),
                    arguments: "{}".into(),
                }],
            ),
            crate::conversation::Message::tool_result("g".into(), "saved output".into(), false),
        ])));
        assert_eq!(app.history, vec!["old task"]);
        assert!(text(&mut app).contains("grep"));
        assert!(
            matches!(&app.items[1], Item::Tool { output, expanded: false, .. } if output == "saved output")
        );
        assert!(!app.busy);
    }

    #[test]
    fn status_is_at_bottom_without_sidebar_or_key_hints_and_theme_applies() {
        let (mut app, _rx) = app();
        app.info.ui.accent = Some("#123456".into());
        let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();
        terminal.draw(|frame| app.draw(frame)).unwrap();
        let buffer = terminal.backend().buffer();
        let bottom: String = (0..120).map(|x| buffer[(x, 39)].symbol()).collect();
        assert!(bottom.contains("test/test-model · main · approval ask"));
        assert!(
            buffer
                .content()
                .iter()
                .any(|cell| cell.fg == Color::Rgb(18, 52, 86))
        );
        let screen = text(&mut app);
        assert!(!screen.contains("Enter send"));
        assert!(!screen.contains("session ─"));
        assert!(!screen.contains("Ctrl+B"));
    }

    #[test]
    fn question_answer_can_be_edited_with_arrow_keys() {
        let (mut app, _rx) = app();
        let (tx, rx) = oneshot::channel();
        app.handle_tool_request(ToolRequest::Ask {
            question: "Choose".into(),
            options: vec!["first".into(), "second".into()],
            respond: tx,
        });
        key(&mut app, KeyCode::Down);
        key(&mut app, KeyCode::Up);
        for ch in "ac".chars() {
            key(&mut app, KeyCode::Char(ch));
        }
        key(&mut app, KeyCode::Left);
        key(&mut app, KeyCode::Char('b'));
        key(&mut app, KeyCode::Enter);
        assert_eq!(rx.blocking_recv().unwrap(), Some("abc".into()));
    }

    #[test]
    fn input_text_is_not_underlined() {
        let (mut app, _rx) = app();
        app.set_input_text("hello");
        let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();
        terminal.draw(|frame| app.draw(frame)).unwrap();
        let cell = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .find(|cell| cell.symbol() == "h" && cell.fg == LIGHT)
            .unwrap();
        assert!(!cell.modifier.contains(Modifier::UNDERLINED));
    }

    #[test]
    fn skills_palette_selects_and_unmatched_text_is_sent() {
        let (mut app, mut rx) = app();
        for ch in "/skills".chars() {
            key(&mut app, KeyCode::Char(ch));
        }
        key(&mut app, KeyCode::Tab);
        assert_eq!(app.palette.as_ref().unwrap().items.len(), 2);
        for ch in "rev".chars() {
            key(&mut app, KeyCode::Char(ch));
        }
        assert_eq!(app.palette.as_ref().unwrap().items.len(), 1);
        key(&mut app, KeyCode::Enter);
        assert_eq!(app.input_text(), "$review ");
        let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();
        terminal.draw(|frame| app.draw(frame)).unwrap();
        assert!(
            terminal
                .backend()
                .buffer()
                .content()
                .iter()
                .any(|cell| cell.symbol() == "$" && cell.fg == ACCENT)
        );
        key(&mut app, KeyCode::Enter);
        assert!(matches!(rx.try_recv(), Ok(WorkerCommand::Run(text)) if text == "$review"));
        app.busy = false;
        app.input.clear();
        for ch in "hello $unknown".chars() {
            key(&mut app, KeyCode::Char(ch));
        }
        key(&mut app, KeyCode::Enter);
        assert!(matches!(rx.try_recv(), Ok(WorkerCommand::Run(text)) if text == "hello $unknown"));
    }

    #[test]
    fn ctrl_enter_inserts_newline() {
        let (mut app, _rx) = app();
        key(&mut app, KeyCode::Char('a'));
        app.handle_event(Event::Key(KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::CONTROL,
        )));
        key(&mut app, KeyCode::Char('b'));
        assert_eq!(app.input_text(), "a\nb");
    }

    #[test]
    fn long_input_wraps_and_grows() {
        let (mut app, _rx) = app();
        app.set_input_text(format!("{} {}", "a".repeat(120), "secondrowmarker"));
        let screen = text(&mut app);
        assert!(screen.contains("secondrowmarker"), "{screen}");
        assert_eq!(app.input.lines().len(), 1);
        assert!(app.input.measure(120).content_rows > 1);
    }

    #[test]
    fn slash_opens_and_filters_the_command_palette() {
        let (mut app, _rx) = app();
        key(&mut app, KeyCode::Char('/'));
        let screen = text(&mut app);
        assert!(screen.contains("fetch and choose"), "{screen}");
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
    fn mouse_wheel_scrolls_conversation_without_recalling_input() {
        use crossterm::event::{MouseEvent, MouseEventKind};

        let (mut app, _rx) = app();
        app.history.push("old input".into());
        app.items.push(Item::Assistant {
            text: (0..40).map(|i| format!("answer line {i}\n")).collect(),
            streaming: false,
        });
        let mut terminal = Terminal::new(TestBackend::new(60, 20)).unwrap();
        terminal.draw(|frame| app.draw(frame)).unwrap();
        let bottom = app.scroll;
        assert!(bottom > 0);
        app.handle_event(Event::Mouse(MouseEvent {
            kind: MouseEventKind::ScrollUp,
            column: 10,
            row: 10,
            modifiers: KeyModifiers::NONE,
        }));
        terminal.draw(|frame| app.draw(frame)).unwrap();
        assert!(app.scroll < bottom);
        assert!(app.input_text().is_empty());
        app.handle_event(Event::Key(KeyEvent::new(
            KeyCode::Home,
            KeyModifiers::CONTROL,
        )));
        terminal.draw(|frame| app.draw(frame)).unwrap();
        assert_eq!(app.scroll, 0);
    }

    #[test]
    fn assistant_background_fills_wrapped_rows() {
        let (mut app, _rx) = app();
        app.items.push(Item::Assistant {
            text: format!("{} ending", "word ".repeat(30)),
            streaming: true,
        });
        let mut terminal = Terminal::new(TestBackend::new(60, 20)).unwrap();
        terminal.draw(|frame| app.draw(frame)).unwrap();
        let buffer = terminal.backend().buffer();
        for y in 0..20 {
            let has_assistant = (2..56).any(|x| buffer[(x, y)].bg == ASSISTANT_BG);
            if has_assistant {
                for x in 2..56 {
                    assert_eq!(buffer[(x, y)].bg, ASSISTANT_BG, "cell {x},{y}");
                }
            }
        }
    }

    #[test]
    fn markdown_answers_render_headings_lists_and_code() {
        let lines = assistant_lines("# Heading\n- item one\n```rust\nlet x = 1;\n```", false, 0);
        let text = lines
            .iter()
            .flat_map(|line| line.spans.iter())
            .map(|span| span.content.as_ref())
            .collect::<String>();
        assert!(text.contains("Heading"), "{text}");
        assert!(text.contains("item one"), "{text}");
        assert!(text.contains("let x = 1;"), "{text}");
        assert!(!text.contains("##"), "{text}");
        assert!(!text.contains("```"), "{text}");
        assert!(text.contains("• item one"), "{text}");
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
            skills: Vec::new(),
            mcp: 0,
            tools: 9,
            auto_approve: false,
            connected: false,
            ui: config::UiConfig::default(),
            providers: vec![choice],
        };
        let mut app = App::new(info, tx, login_tx);
        app.save_selection = |_, _| Ok(());
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
