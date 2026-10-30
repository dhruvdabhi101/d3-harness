use clap::Parser;
use std::path::PathBuf;

/// A small, honest coding-agent harness.
///
/// Run with a prompt for one-shot use, or without one for an interactive
/// session. Project context comes from AGENTS.md, skills, and MCP servers.
#[derive(Debug, Parser)]
#[command(name = "d3", version, about, long_about = None)]
pub struct Cli {
    /// Prompt to run. Omit it to start an interactive session. Use "-" for stdin.
    #[arg(value_name = "PROMPT", trailing_var_arg = true)]
    pub prompt: Vec<String>,

    /// Model id, for example deepseek-v4.1-flash or claude-sonnet-4-5.
    #[arg(short = 'm', long)]
    pub model: Option<String>,

    /// Provider name, for example opencode, openai, anthropic, or a custom one.
    #[arg(short = 'p', long)]
    pub provider: Option<String>,

    /// Replace the built-in system prompt with this text.
    #[arg(long, value_name = "TEXT")]
    pub system_prompt: Option<String>,

    /// Replace the built-in system prompt with a file's contents.
    #[arg(long, value_name = "PATH")]
    pub system_file: Option<PathBuf>,

    /// Workspace directory. Defaults to the current directory.
    #[arg(short = 'C', long, value_name = "DIR")]
    pub cwd: Option<PathBuf>,

    /// Read configuration from this file instead of searching for harness.json.
    #[arg(long, value_name = "PATH")]
    pub config: Option<PathBuf>,

    /// Approve shell commands and edits without asking.
    #[arg(
        short = 'y',
        long = "yes",
        alias = "auto-approve",
        visible_alias = "skip-approval"
    )]
    pub yes: bool,

    /// Continue the latest saved conversation in this workspace.
    #[arg(short = 'c', long = "continue", conflicts_with = "resume")]
    pub r#continue: bool,

    /// Resume a session by ID, or the latest session when no ID is given.
    #[arg(long, value_name = "ID", num_args = 0..=1, default_missing_value = "")]
    pub resume: Option<String>,

    /// List saved conversations in this workspace.
    #[arg(long)]
    pub list_sessions: bool,

    /// Maximum number of model turns per user message (default: 1000).
    #[arg(long, value_name = "N")]
    pub max_steps: Option<usize>,

    /// Print the assembled system prompt, tools, and skills as JSON, then exit.
    #[arg(long)]
    pub dump_context: bool,

    /// List built-in and configured providers, then exit.
    #[arg(long)]
    pub list_providers: bool,

    /// Fetch the selected provider's models and print them, then exit.
    #[arg(long)]
    pub list_models: bool,

    /// Sign in (or verify the existing ChatGPT login), then exit.
    #[arg(long)]
    pub login: bool,

    /// Hide the startup banner.
    #[arg(short = 'q', long)]
    pub quiet: bool,

    /// Use the plain line-based REPL instead of the full-screen TUI.
    #[arg(long)]
    pub no_tui: bool,

    /// Disable colored output.
    #[arg(long)]
    pub no_color: bool,
}
