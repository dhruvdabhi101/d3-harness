use clap::{Args, Parser, Subcommand};
use std::path::PathBuf;

/// A small, honest coding-agent harness.
///
/// Run with a prompt for one-shot use, or without one for an interactive
/// session. Project context comes from AGENTS.md, skills, and MCP servers.
#[derive(Debug, Parser)]
#[command(name = "d3", version, about, long_about = None)]
pub struct Cli {
    /// Management command. Omit it to start a coding session.
    #[command(subcommand)]
    pub command: Option<Command>,

    /// Prompt to run. Omit it to start an interactive session. Use "-" for stdin.
    #[arg(value_name = "PROMPT", trailing_var_arg = true)]
    pub prompt: Vec<String>,

    /// Model id, for example deepseek-v4.1-flash or claude-sonnet-4-5.
    #[arg(short = 'm', long, global = true)]
    pub model: Option<String>,

    /// Provider name, for example opencode, openai, anthropic, or a custom one.
    #[arg(short = 'p', long, global = true)]
    pub provider: Option<String>,

    /// Replace the built-in system prompt with this text.
    #[arg(long, value_name = "TEXT", global = true)]
    pub system_prompt: Option<String>,

    /// Replace the built-in system prompt with a file's contents.
    #[arg(long, value_name = "PATH", global = true)]
    pub system_file: Option<PathBuf>,

    /// Workspace directory. Defaults to the current directory.
    #[arg(short = 'C', long, value_name = "DIR", global = true)]
    pub cwd: Option<PathBuf>,

    /// Read configuration from this file instead of searching for harness.json.
    #[arg(long, value_name = "PATH", global = true)]
    pub config: Option<PathBuf>,

    /// Approve shell commands and edits without asking.
    #[arg(
        short = 'y',
        long = "yes",
        alias = "auto-approve",
        visible_alias = "skip-approval",
        global = true
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
    #[arg(long, value_name = "N", global = true)]
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

    /// Sign in and verify the credential, then exit. Alias for `d3 auth login`.
    #[arg(long, visible_alias = "auth-login")]
    pub login: bool,

    /// Remove the stored credential for the selected provider, then exit.
    #[arg(long, visible_alias = "auth-logout")]
    pub logout: bool,

    /// Verify the selected provider credential locally, then exit.
    #[arg(long, visible_alias = "auth-check")]
    pub check_auth: bool,

    /// Hide the startup banner.
    #[arg(short = 'q', long, global = true)]
    pub quiet: bool,

    /// Use the plain line-based REPL instead of the full-screen TUI.
    #[arg(long)]
    pub no_tui: bool,

    /// Disable colored output.
    #[arg(long, global = true)]
    pub no_color: bool,
}

#[derive(Debug, Clone, Subcommand)]
pub enum Command {
    /// Manage provider credentials.
    Auth {
        #[command(subcommand)]
        action: AuthAction,
    },
    /// Fetch the provider catalog and list available models.
    Models {
        /// Case-insensitive text to filter model ids and names.
        #[arg(value_name = "SEARCH")]
        search: Option<String>,
        /// Print the catalog as JSON.
        #[arg(long)]
        json: bool,
    },
    /// List saved conversations in this workspace.
    Sessions {
        /// Print sessions as JSON.
        #[arg(long)]
        json: bool,
    },
    /// Print the resolved configuration and credential locations.
    Config {
        /// Print the configuration as JSON.
        #[arg(long)]
        json: bool,
    },
    /// Connect to configured MCP servers and list their tools.
    Mcp {
        /// Print the MCP summary as JSON.
        #[arg(long)]
        json: bool,
    },
    /// Create a starter AGENTS.md in the workspace.
    Init {
        /// Overwrite an existing AGENTS.md.
        #[arg(long)]
        force: bool,
    },
    /// Check GitHub for a newer stable release.
    Update,
    /// Run the built-in harness benchmark against the configured model.
    Bench(BenchArgs),
    /// Print version and build information.
    Version,
}

#[derive(Debug, Clone, Subcommand)]
pub enum AuthAction {
    /// Sign in to a provider, verify the credential, and save it.
    Login {
        /// Provider name. Omit to choose from a list.
        #[arg(value_name = "PROVIDER")]
        provider: Option<String>,
        /// Prompt for an API key instead of an account sign-in flow.
        #[arg(long)]
        api_key: bool,
        /// Do not open a browser; use the provider's device-code flow.
        #[arg(long)]
        no_browser: bool,
    },
    /// Remove the stored credential for a provider.
    Logout {
        /// Provider name. Omit to choose from stored credentials.
        #[arg(value_name = "PROVIDER")]
        provider: Option<String>,
        /// Remove every stored provider credential.
        #[arg(long)]
        all: bool,
    },
    /// Report whether a provider credential is available.
    Check {
        /// Provider name. Omit to check every configured provider.
        #[arg(value_name = "PROVIDER")]
        provider: Option<String>,
        /// Print structured JSON.
        #[arg(long)]
        json: bool,
        /// Call the provider's model catalog to verify the credential.
        #[arg(long)]
        verify: bool,
    },
    /// List providers and whether each has a usable credential.
    List,
}

#[derive(Debug, Clone, Args)]
pub struct BenchArgs {
    /// Benchmark suite: smoke, tools, or all.
    #[arg(long, default_value = "all")]
    pub suite: String,
    /// Print structured JSON instead of a table.
    #[arg(long)]
    pub json: bool,
    /// Keep the temporary benchmark workspace for inspection.
    #[arg(long)]
    pub keep: bool,
    /// Run only tasks whose name contains this text. Repeatable.
    #[arg(long, value_name = "TEXT")]
    pub filter: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subcommands_parse_without_stealing_normal_prompts() {
        let cli = Cli::try_parse_from(["d3", "explain", "this", "repo"]).unwrap();
        assert!(cli.command.is_none());
        assert_eq!(cli.prompt, ["explain", "this", "repo"]);

        let cli = Cli::try_parse_from(["d3", "auth", "login", "opencode"]).unwrap();
        assert!(matches!(
            cli.command,
            Some(Command::Auth {
                action: AuthAction::Login {
                    provider: Some(ref provider),
                    ..
                }
            }) if provider == "opencode"
        ));

        let cli = Cli::try_parse_from(["d3", "bench", "--suite", "smoke", "--json"]).unwrap();
        assert!(matches!(
            cli.command,
            Some(Command::Bench(BenchArgs { ref suite, json: true, .. })) if suite == "smoke"
        ));
    }
}
