mod fs_tools;
mod shell;

use crate::config::{Permission, Permissions};
use crate::conversation::ToolCall;
use crate::mcp::McpManager;
use crate::provider::ToolDefinition;
use crate::skills::SkillSet;
use crate::ui;
use crate::ui_bridge::ToolUi;
use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

#[derive(Debug, Clone)]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    pub schema: Value,
    pub kind: ToolKind,
}

#[derive(Debug, Clone)]
pub enum ToolKind {
    Read,
    Write,
    Edit,
    Bash,
    Glob,
    Grep,
    AskUser,
    Todo,
    Skill,
    Mcp { server: String, tool: String },
}

pub struct ToolOutcome {
    pub content: String,
    pub summary: String,
    pub is_error: bool,
}

impl ToolOutcome {
    pub fn ok(content: impl Into<String>, summary: impl Into<String>) -> Self {
        Self {
            content: content.into(),
            summary: summary.into(),
            is_error: false,
        }
    }

    pub fn err(content: impl Into<String>) -> Self {
        Self {
            content: content.into(),
            summary: String::new(),
            is_error: true,
        }
    }
}

pub struct ToolRegistry {
    root: PathBuf,
    specs: Vec<ToolSpec>,
    skills: SkillSet,
    mcp: McpManager,
    permissions: Permissions,
    auto_approve: bool,
    ui: Arc<dyn ToolUi>,
    todos: Vec<TodoItem>,
}

#[derive(Debug, Clone)]
struct TodoItem {
    content: String,
    status: String,
}

impl ToolRegistry {
    pub fn new(
        root: PathBuf,
        skills: SkillSet,
        mcp: McpManager,
        permissions: Permissions,
        auto_approve: bool,
        ui: Arc<dyn ToolUi>,
    ) -> Self {
        let mut specs = builtin_specs(&skills);
        for tool in mcp.tools() {
            specs.push(ToolSpec {
                name: tool.exposed_name,
                description: format!("{} (MCP server: {})", tool.description, tool.server),
                schema: tool.schema,
                kind: ToolKind::Mcp {
                    server: tool.server,
                    tool: tool.tool_name,
                },
            });
        }
        Self {
            root,
            specs,
            skills,
            mcp,
            permissions,
            auto_approve,
            ui,
            todos: Vec::new(),
        }
    }

    pub fn specs(&self) -> &[ToolSpec] {
        &self.specs
    }

    pub fn definitions(&self) -> Vec<ToolDefinition> {
        self.specs
            .iter()
            .map(|spec| ToolDefinition {
                name: spec.name.clone(),
                description: spec.description.clone(),
                schema: spec.schema.clone(),
            })
            .collect()
    }

    pub fn mcp_summaries(&self) -> Vec<Value> {
        self.mcp.summaries()
    }

    pub fn skill_names(&self) -> Vec<String> {
        self.skills.names()
    }

    pub fn set_auto_approve(&mut self, value: bool) {
        self.auto_approve = value;
    }

    pub fn auto_approve(&self) -> bool {
        self.auto_approve
    }

    pub fn print_startup(&self) {
        let mut names = self
            .specs
            .iter()
            .filter(|spec| !matches!(spec.kind, ToolKind::Mcp { .. }))
            .map(|spec| spec.name.as_str())
            .collect::<Vec<_>>();
        names.sort();
        let mcp_count = self
            .specs
            .iter()
            .filter(|spec| matches!(spec.kind, ToolKind::Mcp { .. }))
            .count();
        ui::info(&format!(
            "tools: {}  ·  skills: {}  ·  mcp tools: {}",
            names.join(", "),
            self.skills.iter().count(),
            mcp_count
        ));
        if !self.skills.is_empty() {
            let names = self.skills.names();
            let shown = names.iter().take(8).cloned().collect::<Vec<_>>().join(", ");
            let suffix = if names.len() > 8 {
                format!(", and {} more (use /skills)", names.len() - 8)
            } else {
                String::new()
            };
            ui::info(&format!("skills: {shown}{suffix}"));
        }
        eprintln!();
    }

    pub async fn execute(&mut self, call: &ToolCall) -> ToolOutcome {
        let spec = self
            .specs
            .iter()
            .find(|spec| spec.name == call.name)
            .cloned();
        let Some(spec) = spec else {
            return ToolOutcome::err(format!(
                "Unknown tool `{}`. Available tools: {}",
                call.name,
                self.specs
                    .iter()
                    .map(|spec| spec.name.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        };
        let arguments: Value = match serde_json::from_str(&call.arguments) {
            Ok(value) => value,
            Err(error) => {
                return ToolOutcome::err(format!(
                    "Invalid JSON arguments for `{}`: {error}. Received: {}",
                    call.name, call.arguments
                ));
            }
        };

        match spec.kind {
            ToolKind::Read => self.read(&arguments, &spec),
            ToolKind::Write => self.write(&arguments, &spec).await,
            ToolKind::Edit => self.edit(&arguments, &spec).await,
            ToolKind::Bash => self.bash(&arguments).await,
            ToolKind::Glob => self.glob(&arguments),
            ToolKind::Grep => self.grep(&arguments),
            ToolKind::AskUser => self.ask_user(&arguments).await,
            ToolKind::Todo => self.write_todos(&arguments),
            ToolKind::Skill => self.load_skill(&arguments),
            ToolKind::Mcp { server, tool } => self.call_mcp(server, tool, arguments).await,
        }
    }

    fn read(&self, args: &Value, spec: &ToolSpec) -> ToolOutcome {
        match fs_tools::read(&self.root, args) {
            Ok((content, summary)) => ToolOutcome::ok(content, summary),
            Err(error) => ToolOutcome::err(format!("{} failed: {error:#}", spec.name)),
        }
    }

    async fn write(&mut self, args: &Value, spec: &ToolSpec) -> ToolOutcome {
        let path = match args.get("path").and_then(Value::as_str) {
            Some(path) => path,
            None => return ToolOutcome::err("write needs a `path` string"),
        };
        let content = args
            .get("content")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let path = match self.resolve_path(path) {
            Ok(path) => path,
            Err(error) => return ToolOutcome::err(format!("{error:#}")),
        };
        let old = std::fs::read_to_string(&path).unwrap_or_default();
        self.ui.diff(&self.display_path(&path), &old, content);
        if let Err(error) = self
            .authorize(self.permissions.edit(), "write this file?")
            .await
        {
            return ToolOutcome::err(format!("{error:#}"));
        }
        if let Some(parent) = path.parent()
            && let Err(error) = std::fs::create_dir_all(parent)
        {
            return ToolOutcome::err(format!(
                "{} failed: creating {}: {error}",
                spec.name,
                parent.display()
            ));
        }
        match std::fs::write(&path, content) {
            Ok(()) => {
                let rel = self.display_path(&path);
                let bytes = content.len();
                ToolOutcome::ok(
                    format!("Wrote {bytes} bytes to {rel}."),
                    format!("{rel} ({bytes} bytes)"),
                )
            }
            Err(error) => ToolOutcome::err(format!(
                "{} failed: writing {}: {error}",
                spec.name,
                path.display()
            )),
        }
    }

    async fn edit(&mut self, args: &Value, spec: &ToolSpec) -> ToolOutcome {
        let path = match args.get("path").and_then(Value::as_str) {
            Some(path) => path,
            None => return ToolOutcome::err("edit needs a `path` string"),
        };
        let old_string = match args.get("old_string").and_then(Value::as_str) {
            Some(text) if !text.is_empty() => text,
            _ => return ToolOutcome::err("edit needs a non-empty `old_string`"),
        };
        let new_string = args
            .get("new_string")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let replace_all = args
            .get("replace_all")
            .and_then(Value::as_bool)
            .unwrap_or(false);

        let path = match self.resolve_path(path) {
            Ok(path) => path,
            Err(error) => return ToolOutcome::err(format!("{error:#}")),
        };
        let original = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(error) => {
                return ToolOutcome::err(format!(
                    "{} failed: reading {}: {error}",
                    spec.name,
                    path.display()
                ));
            }
        };
        let count = original.matches(old_string).count();
        if count == 0 {
            return ToolOutcome::err(
                "old_string was not found. Read the file again and copy the exact text, including whitespace.",
            );
        }
        if count > 1 && !replace_all {
            return ToolOutcome::err(format!(
                "old_string appears {count} times. Add surrounding context to make it unique, or set `replace_all` to true."
            ));
        }
        let updated = if replace_all {
            original.replace(old_string, new_string)
        } else {
            original.replacen(old_string, new_string, 1)
        };
        let rel = self.display_path(&path);
        self.ui.diff(&rel, &original, &updated);
        if let Err(error) = self
            .authorize(self.permissions.edit(), "apply this edit?")
            .await
        {
            return ToolOutcome::err(format!("{error:#}"));
        }
        match std::fs::write(&path, &updated) {
            Ok(()) => ToolOutcome::ok(
                format!(
                    "Edited {rel}: replaced {} occurrence{}.",
                    count,
                    if count == 1 { "" } else { "s" }
                ),
                format!(
                    "{rel} ({} replacement{})",
                    count,
                    if count == 1 { "" } else { "s" }
                ),
            ),
            Err(error) => ToolOutcome::err(format!(
                "{} failed: writing {}: {error}",
                spec.name,
                path.display()
            )),
        }
    }

    async fn bash(&mut self, args: &Value) -> ToolOutcome {
        let command = match args.get("command").and_then(Value::as_str) {
            Some(command) if !command.trim().is_empty() => command,
            _ => return ToolOutcome::err("bash needs a non-empty `command` string"),
        };
        let timeout = args
            .get("timeout")
            .and_then(Value::as_u64)
            .unwrap_or(120)
            .clamp(1, 600);
        let cwd = match args.get("cwd").and_then(Value::as_str) {
            Some(cwd) => match self.resolve_path(cwd) {
                Ok(path) => path,
                Err(error) => return ToolOutcome::err(format!("{error:#}")),
            },
            None => self.root.clone(),
        };

        match self.permissions.shell() {
            Permission::Deny => {
                return ToolOutcome::err("Shell commands are disabled by configuration.");
            }
            Permission::Ask => {
                if !self.auto_approve {
                    if !self.ui.is_interactive() {
                        return ToolOutcome::err(
                            "Shell approval is required, but this run is not interactive. Re-run with --yes to allow shell commands.",
                        );
                    }
                    let approved = self
                        .ui
                        .confirm(&format!("run `{}`?", truncate(command, 160)), command)
                        .await;
                    if !approved {
                        return ToolOutcome::err("The user declined to run this command.");
                    }
                }
            }
            Permission::Allow => {}
        }

        match shell::run(command, &cwd, timeout).await {
            Ok(output) => {
                let summary = summarise_output(&output.content, 24);
                ToolOutcome::ok(output.content, summary)
            }
            Err(error) => ToolOutcome::err(format!("bash failed: {error:#}")),
        }
    }

    fn glob(&self, args: &Value) -> ToolOutcome {
        match fs_tools::glob(&self.root, args) {
            Ok((content, summary)) => ToolOutcome::ok(content, summary),
            Err(error) => ToolOutcome::err(format!("glob failed: {error:#}")),
        }
    }

    fn grep(&self, args: &Value) -> ToolOutcome {
        match fs_tools::grep(&self.root, args) {
            Ok((content, summary)) => ToolOutcome::ok(content, summary),
            Err(error) => ToolOutcome::err(format!("grep failed: {error:#}")),
        }
    }

    async fn ask_user(&self, args: &Value) -> ToolOutcome {
        let question = match args.get("question").and_then(Value::as_str) {
            Some(question) if !question.trim().is_empty() => question,
            _ => return ToolOutcome::err("ask_user needs a non-empty `question` string"),
        };
        let options = args
            .get("options")
            .and_then(Value::as_array)
            .map(|options| {
                options
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();

        if !self.ui.is_interactive() {
            return ToolOutcome::ok(
                "The user is not available to answer in this run. State a reasonable assumption explicitly and continue, or stop and report exactly what you need.",
                "user unavailable; agent will state an assumption",
            );
        }

        let answer = self.ui.ask(question, &options).await;
        match answer {
            Some(answer) => {
                ToolOutcome::ok(format!("User answer: {answer}"), format!("user: {answer}"))
            }
            None => ToolOutcome::ok("The user did not answer.", ""),
        }
    }

    fn write_todos(&mut self, args: &Value) -> ToolOutcome {
        let Some(items) = args.get("todos").and_then(Value::as_array) else {
            return ToolOutcome::err("todo_write needs a `todos` array");
        };
        let mut todos = Vec::new();
        for item in items {
            let content = item
                .get("content")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .trim();
            if content.is_empty() {
                continue;
            }
            let status = match item
                .get("status")
                .and_then(Value::as_str)
                .unwrap_or("pending")
            {
                "in_progress" | "in-progress" | "active" => "in_progress",
                "completed" | "done" => "completed",
                _ => "pending",
            };
            todos.push(TodoItem {
                content: content.to_string(),
                status: status.to_string(),
            });
        }
        self.todos = todos;
        if self.todos.is_empty() {
            return ToolOutcome::ok("Todo list cleared.", "");
        }
        let done = self
            .todos
            .iter()
            .filter(|todo| todo.status == "completed")
            .count();
        let mut lines = Vec::new();
        for todo in &self.todos {
            let marker = match todo.status.as_str() {
                "completed" => "[x]",
                "in_progress" => "[~]",
                _ => "[ ]",
            };
            lines.push(format!("{marker} {}", todo.content));
        }
        let content = format!(
            "Todo list updated ({done}/{} complete):\n{}",
            self.todos.len(),
            lines.join("\n")
        );
        ToolOutcome::ok(content, lines.join("\n"))
    }

    fn load_skill(&self, args: &Value) -> ToolOutcome {
        let Some(name) = args.get("name").and_then(Value::as_str) else {
            return ToolOutcome::err("skill needs a `name` string");
        };
        match self.skills.get(name) {
            Some(skill) => ToolOutcome::ok(
                format!(
                    "# Skill: {}\n\nSource: {}\n\n{}\n\nIf this skill references relative files or scripts, read them from the skill directory before using them.",
                    skill.name,
                    skill.path.display(),
                    skill.body.trim()
                ),
                format!("loaded skill `{}`", skill.name),
            ),
            None => ToolOutcome::err(format!(
                "Unknown skill `{name}`. Available skills: {}",
                if self.skills.is_empty() {
                    "none".to_string()
                } else {
                    self.skills.names().join(", ")
                }
            )),
        }
    }

    async fn call_mcp(&mut self, server: String, tool: String, arguments: Value) -> ToolOutcome {
        let arguments = if arguments.is_null() {
            json!({})
        } else {
            arguments
        };
        match self.mcp.call(&server, &tool, arguments).await {
            Ok((content, is_error)) => {
                let summary = summarise_output(&content, 12);
                if is_error {
                    ToolOutcome {
                        content,
                        summary,
                        is_error: true,
                    }
                } else {
                    ToolOutcome::ok(content, summary)
                }
            }
            Err(error) => ToolOutcome::err(format!("MCP call failed: {error:#}")),
        }
    }

    async fn authorize(&self, configured: Permission, question: &str) -> Result<()> {
        match configured {
            Permission::Allow => Ok(()),
            Permission::Deny => bail!("edits are disabled by configuration"),
            Permission::Ask => {
                if self.auto_approve {
                    Ok(())
                } else if !self.ui.is_interactive() {
                    bail!(
                        "edit approval is required, but this run is not interactive. Re-run with --yes to allow edits"
                    )
                } else if self.ui.confirm(question, "").await {
                    Ok(())
                } else {
                    bail!("the user declined the edit")
                }
            }
        }
    }

    fn resolve_path(&self, raw: &str) -> Result<PathBuf> {
        resolve_path_for(&self.root, raw)
    }

    fn display_path(&self, path: &Path) -> String {
        path.strip_prefix(&self.root)
            .unwrap_or(path)
            .display()
            .to_string()
    }
}

pub(super) fn resolve_path_for(root: &Path, raw: &str) -> Result<PathBuf> {
    if raw.trim().is_empty() {
        bail!("path is empty");
    }
    let candidate = Path::new(raw);
    let joined = if candidate.is_absolute() {
        candidate.to_path_buf()
    } else {
        root.join(candidate)
    };
    let normalized = normalize(&joined);
    let resolved = canonicalize_maybe(&normalized)?;
    if !resolved.starts_with(root) {
        bail!(
            "path '{}' is outside the workspace ({})",
            raw,
            root.display()
        );
    }
    Ok(resolved)
}

fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

fn canonicalize_maybe(path: &Path) -> Result<PathBuf> {
    if path.exists() {
        return path
            .canonicalize()
            .with_context(|| format!("resolving {}", path.display()));
    }
    let mut missing = Vec::new();
    let mut current = path;
    while !current.exists() {
        let Some(name) = current.file_name() else {
            bail!("cannot resolve {}", path.display());
        };
        missing.push(name.to_os_string());
        let Some(parent) = current.parent() else {
            bail!("cannot resolve {}", path.display());
        };
        current = parent;
    }
    let mut resolved = current
        .canonicalize()
        .with_context(|| format!("resolving {}", current.display()))?;
    for name in missing.into_iter().rev() {
        resolved.push(name);
    }
    Ok(resolved)
}

fn summarise_output(text: &str, max_lines: usize) -> String {
    let lines = text.lines().take(max_lines).collect::<Vec<_>>();
    let total = text.lines().count();
    let mut summary = lines.join("\n");
    if total > max_lines {
        summary.push_str(&format!("\n… {} more lines", total - max_lines));
    }
    summary
}

fn truncate(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        text.to_string()
    } else {
        format!("{}…", text.chars().take(max).collect::<String>())
    }
}

fn builtin_specs(skills: &SkillSet) -> Vec<ToolSpec> {
    let mut specs = vec![
        ToolSpec {
            name: "read".to_string(),
            description: "Read a text file from the workspace. Returns line-numbered content. Read a file before editing it.".to_string(),
            schema: json!({
                "type": "object",
                "properties": {
                    "path": {"type": "string", "description": "Path relative to the workspace."},
                    "offset": {"type": "integer", "description": "First line to read, 1-based. Optional."},
                    "limit": {"type": "integer", "description": "Maximum lines to read. Default 2000."}
                },
                "required": ["path"]
            }),
            kind: ToolKind::Read,
        },
        ToolSpec {
            name: "write".to_string(),
            description: "Write a complete file. Creates parent directories. Prefer `edit` for existing files so unrelated content is preserved.".to_string(),
            schema: json!({
                "type": "object",
                "properties": {
                    "path": {"type": "string"},
                    "content": {"type": "string"}
                },
                "required": ["path", "content"]
            }),
            kind: ToolKind::Write,
        },
        ToolSpec {
            name: "edit".to_string(),
            description: "Replace an exact string in a file. old_string must match exactly once unless replace_all is true. Read the file first.".to_string(),
            schema: json!({
                "type": "object",
                "properties": {
                    "path": {"type": "string"},
                    "old_string": {"type": "string"},
                    "new_string": {"type": "string"},
                    "replace_all": {"type": "boolean", "default": false}
                },
                "required": ["path", "old_string", "new_string"]
            }),
            kind: ToolKind::Edit,
        },
        ToolSpec {
            name: "bash".to_string(),
            description: "Run a shell command in the workspace. Use for builds, tests, and inspection. Output is truncated.".to_string(),
            schema: json!({
                "type": "object",
                "properties": {
                    "command": {"type": "string"},
                    "cwd": {"type": "string", "description": "Optional working directory relative to the workspace."},
                    "timeout": {"type": "integer", "description": "Seconds before the command is killed. Default 120, max 600."}
                },
                "required": ["command"]
            }),
            kind: ToolKind::Bash,
        },
        ToolSpec {
            name: "glob".to_string(),
            description: "Find files by glob pattern. Respects .gitignore. A pattern without / is matched recursively by basename.".to_string(),
            schema: json!({
                "type": "object",
                "properties": {
                    "pattern": {"type": "string", "description": "For example **/*.rs or src/**/*.ts."},
                    "path": {"type": "string", "description": "Directory to search. Defaults to the workspace."},
                    "limit": {"type": "integer", "description": "Maximum results. Default 200."}
                },
                "required": ["pattern"]
            }),
            kind: ToolKind::Glob,
        },
        ToolSpec {
            name: "grep".to_string(),
            description: "Search file contents with a regular expression. Respects .gitignore and skips binary files.".to_string(),
            schema: json!({
                "type": "object",
                "properties": {
                    "pattern": {"type": "string"},
                    "path": {"type": "string", "description": "Directory or file to search. Defaults to the workspace."},
                    "include": {"type": "string", "description": "Optional glob filter, for example *.rs."},
                    "literal": {"type": "boolean", "default": false},
                    "case_sensitive": {"type": "boolean", "default": true},
                    "limit": {"type": "integer", "description": "Maximum matches. Default 100."}
                },
                "required": ["pattern"]
            }),
            kind: ToolKind::Grep,
        },
        ToolSpec {
            name: "ask_user".to_string(),
            description: "Ask the user a question when ambiguity would change the outcome. Offer concrete options when possible. Do not use this to ask permission for obvious work.".to_string(),
            schema: json!({
                "type": "object",
                "properties": {
                    "question": {"type": "string"},
                    "options": {"type": "array", "items": {"type": "string"}, "description": "Optional choices. The user can still type a free-form answer."}
                },
                "required": ["question"]
            }),
            kind: ToolKind::AskUser,
        },
        ToolSpec {
            name: "todo_write".to_string(),
            description: "Replace the visible todo list. Use it to keep the user oriented during multi-step work.".to_string(),
            schema: json!({
                "type": "object",
                "properties": {
                    "todos": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {
                                "content": {"type": "string"},
                                "status": {"type": "string", "enum": ["pending", "in_progress", "completed"]}
                            },
                            "required": ["content", "status"]
                        }
                    }
                },
                "required": ["todos"]
            }),
            kind: ToolKind::Todo,
        },
    ];

    if !skills.is_empty() {
        specs.push(ToolSpec {
            name: "skill".to_string(),
            description: "Load the full instructions for an available skill. Use when the skill's description matches the task.".to_string(),
            schema: json!({
                "type": "object",
                "properties": {
                    "name": {"type": "string", "enum": skills.names()}
                },
                "required": ["name"]
            }),
            kind: ToolKind::Skill,
        });
    }
    specs
}
