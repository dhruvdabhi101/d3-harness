# harness

A small, honest coding-agent harness for the terminal, written in Rust.

It opens a full-screen TUI with a message box, a slash-command palette, model
picking, approval dialogs, and live tool diffs. Behind it is a tool-calling
agent loop that loads the context a real project needs: `AGENTS.md`, skills, and
MCP servers. It talks to OpenCode, ChatGPT/Codex, OpenAI, Anthropic, and any
OpenAI-compatible endpoint.

The default system prompt is part of the product. It asks the agent to write
code a careful human would review, to ask when ambiguity matters, to keep the
user oriented, and to verify what it claims.

## Install and update

Install the latest GitHub release on Linux or macOS:

```bash
curl -fsSL https://raw.githubusercontent.com/dhruvdabhi101/d3-harness/main/install.sh | sh
```

The installer verifies the archive checksum and installs `d3` to `~/.local/bin`.
Set `D3_INSTALL_DIR` to choose another directory. Add that directory to your
`PATH`. Run the same command to update. The TUI checks GitHub for a newer stable
release at startup and displays a notice when one is available. The check
fails silently when offline.

To publish a release, set the version in `Cargo.toml`, update `Cargo.lock`, and
push a matching tag such as `v0.1.1`. GitHub Actions builds Linux and macOS
assets for x86-64 and ARM64. The installer requires a published release; a
local build or an untagged push does not create one.

## Resume a conversation

Conversations are saved automatically in the local data directory under
`d3/sessions`. History is scoped to the canonical Git working-tree root, or the
canonical current folder outside Git. Subfolders of one working tree share
history. Separate repositories and Git worktrees do not. Run `d3 --continue` (or `d3 -c`) in the same project to
resume its most recently updated conversation. Add a prompt after the options
to continue in one-shot mode, for example `d3 -c "continue the task"`.
To select an older conversation, run `d3 --list-sessions`, then pass its ID to
`d3 --resume <ID>`. Add a prompt after the ID for one-shot use. Bare `--resume`
continues the latest conversation. Inside chat, `/resume` opens a project
conversation picker, `/resume <ID>` selects a conversation, and `/continue`
resumes the latest. The plain REPL lists IDs with `/resume` instead of opening
a picker. `/new` starts a separate conversation
without deleting the previous one. Each resumed process saves a separate
continuation, so concurrent runs do not overwrite one another. `--continue`
selects the most recently updated continuation.

Saved conversations contain prompts, responses, and tool results, which may
include sensitive data. The session directory and files are private to your
user on Unix. Delete the files to remove saved conversations. The current
provider, model, permissions, and system prompt are loaded again on restart;
use the same provider when resuming provider-specific tool history.

Older snapshots created from a repository subfolder before v0.2.0 stay under
their previous folder key. They are not automatically migrated or shown in the
repository-root history. Root-folder snapshots remain available.

If the process stops during a tool call, the last complete checkpoint is
resumed. The tool may already have changed files or run commands, so inspect
those effects before retrying.

## Build

```bash
cargo build --release
./target/release/d3 --help
```

Install the `d3` command from the repository directory:

```bash
./scripts/build-install.sh
```

Then run `d3` from any project directory. Cargo installs the command in
`~/.cargo/bin`; make sure that directory is on your `PATH`. Existing harness
configuration and credentials remain in their current locations.

## Quick start with OpenCode

Run it:

```bash
d3 "explain what this repository does"
```

If no key is configured, the harness asks for one, hides the input, saves it to
the user config directory, and uses it immediately. The next run reuses it. You
can also set `OPENCODE_API_KEY` yourself.

The default provider is `opencode` and the default model is
`deepseek-v4.1-flash`. Any model available through OpenCode Console can be
selected:

```bash
d3 -m gpt-5.3-codex "fix the failing test"
d3 -m claude-sonnet-4-5 "review this diff"
d3 --list-models
```

Run without a prompt for the full-screen interface:

```bash
d3
```

The TUI handles first-run setup itself. If the selected provider has no
credentials, it opens a provider picker and a sign-in screen. ChatGPT uses the
official device-code flow: harness displays the URL and one-time code, waits for
approval, then connects without leaving the UI. API-key providers ask for the key
inside a masked modal. Credentials are saved with `0600` permissions and reused
on the next run.

Options must come before the prompt. `d3 "prompt" --yes` treats `--yes` as
part of the prompt.

## TUI

The interactive mode is a real terminal application, not a line reader.

```text
    › find references to fetch

    ▸ ● grep  fetch

    Found the references in src/client.rs.

 ╭ message ─────────────────────────────────────────────────────────────────╮
 │ Ask anything, or type / for commands                                      │
 ╰──────────────────────────────────────────────────────────────────────────╯
 chatgpt/gpt-5.6-sol · main · approval ask · ready
```

Features:

- Streamed assistant text with light markdown and code styling.
- Distinct backgrounds for your prompt and the assistant's answer.
- Tool calls as collapsed rows with a spinner and success or error status. Click
  a row to expand the full model-visible output, then click again to collapse.
  Restored conversations include the saved tool calls and results.
- Color-coded edit diffs remain visible for review.
- Approval bar docked at the bottom, not a centered dialog, with the command and
  `[y] allow / [n] deny` in reach.
- Slash-command palette that opens as soon as you type `/`.
- Provider picker via `/provider`, with sign-in without leaving the TUI.
- ChatGPT device-code sign-in via `/login`, including the URL and one-time code.
- Live model picker via `/models`, fetched from the provider catalog.
- Bottom approval bar for shell commands, with the exact command shown.
- Question modal for `ask_user`, with options and free-form input.
- Bottom status line with provider, model, Git branch, and approval mode.
- `/context` shows message and token counts without a sidebar.
- Scrollback with PageUp/PageDown and auto-follow that pauses when you scroll up.
  Mouse capture enables tool-row clicks. Hold your terminal's selection modifier,
  commonly Shift, to select and copy text.
- Multi-line editor with history, bracketed paste, and proper cursor movement.

Keybindings:

| Key | Action |
| --- | --- |
| `Enter` | send |
| `Ctrl+Enter` / `Shift+Enter` / `Alt+Enter` / `Ctrl+J` | newline (modified Enter depends on terminal support) |
| `/` | open the command palette |
| `Tab` | complete/open the command palette |
| `Up` / `Down` | history or move through the palette |
| `PageUp` / `PageDown` | scroll the conversation |
| Click a tool row | expand or collapse its output |
| `Ctrl+U` / `Ctrl+W` | clear line / delete word |
| `Ctrl+C` twice | quit while a turn is running |
| `Ctrl+D` | quit when the input is empty |
| `Esc` | close palette/modal, deny approval, or clear input |
| `o` in ChatGPT sign-in | open the verification URL in your browser |
| `y` / `n` on approval | allow / deny the command |

The TUI is built on maintained crates:

- [`ratatui`](https://ratatui.rs) for the layout, lists, scrollbar, and widgets.
- [`tui-textarea-2`](https://crates.io/crates/tui-textarea-2) for the message editor,
  cursor movement, undo, word movement, and bracketed paste.
- [`tui-markdown`](https://crates.io/crates/tui-markdown) for headings, lists,
  tables, inline code, and syntax-highlighted code blocks.
- `crossterm` for the terminal backend.

The plain line-based REPL is still available with `--no-tui`.

### Customize the UI

Add a `ui` object to the global `~/.config/harness/harness.json` or project
`harness.json`, then restart. Project settings override individual global
settings. `/ui` lists the available settings. No plugins or executable theme
files are loaded.

```json
{
  "ui": {
    "accent": "#FF9966",
    "user_background": "#232B38",
    "assistant_background": "#1A1D23",
    "tool_output_expanded": false,
    "editor_max_height": 6
  }
}
```

Colors accept `#RRGGBB`. `editor_max_height` accepts 1 through 20 content rows.
Tool output starts collapsed unless `tool_output_expanded` is true. The default
editor limit is six rows. Enter still sends and modified Enter adds a newline;
the footer no longer repeats those hints. The sidebar and Ctrl+B toggle are
removed.


## What it does

- **Agent loop**: streams model text, executes tool calls, feeds results back,
  and stops when the model answers without tools.
- **Tools**: `read`, `write`, `edit`, `bash`, `glob`, `grep`, `ask_user`,
  `todo_write`, and `skill`, plus every tool exposed by configured MCP servers.
- **Project context**: loads `AGENTS.md` from the workspace and its parents,
  global OpenCode/Harness instructions, and any extra instruction files.
- **Skills**: discovers `SKILL.md` packages and loads their full text on demand
  through the `skill` tool.
- **MCP**: connects to stdio and streamable-HTTP MCP servers, lists their tools,
  and exposes them as `mcp__<server>__<tool>`.
- **Human-in-the-loop behavior**: prints a diff for every edit and asks before
  running shell commands by default.
- **Providers**: OpenCode, OpenAI/Codex (Responses API), Anthropic Messages,
  OpenRouter, DeepSeek, Groq, Mistral, xAI, Ollama, LM Studio, and custom
  OpenAI-compatible endpoints.
- **First-run setup**: asks for a missing API key in the terminal, hides the
  input, stores it with `0600` permissions, and reuses it next time.
- **Model discovery**: fetches the provider's model catalog and supports
  `d3 --list-models` and the interactive `/models` picker.
- **Full-screen TUI**: streaming messages, tool diffs, plan sidebar, command
  palette, model picker, and approval/question modals.
- **ChatGPT authentication**: reuse the login saved by Codex CLI, including
  model discovery from its local cache.
- **Streaming**: SSE streaming for Chat Completions, Anthropic Messages, and
  OpenAI Responses. Non-streaming JSON responses also work.

## Providers

Built-in presets:

| Provider | API | Key | Default model |
| --- | --- | --- | --- |
| `opencode` | OpenAI Chat Completions | `OPENCODE_API_KEY` | `deepseek-v4.1-flash` |
| `chatgpt` | ChatGPT/Codex backend | Codex CLI login | `gpt-5.6-sol` |
| `openai` | OpenAI Responses | `OPENAI_API_KEY` | `gpt-5.3-codex` |
| `codex` | OpenAI Responses | `OPENAI_API_KEY` | `gpt-5.3-codex` |
| `anthropic` | Anthropic Messages | `ANTHROPIC_API_KEY` | `claude-sonnet-4-5` |
| `openrouter` | OpenAI compatible | `OPENROUTER_API_KEY` | `anthropic/claude-sonnet-4.5` |
| `deepseek` | OpenAI compatible | `DEEPSEEK_API_KEY` | `deepseek-chat` |
| `groq` | OpenAI compatible | `GROQ_API_KEY` | `llama-3.3-70b-versatile` |
| `mistral` | OpenAI compatible | `MISTRAL_API_KEY` | `mistral-large-latest` |
| `xai` | OpenAI compatible | `XAI_API_KEY` | `grok-4` |
| `ollama` | OpenAI compatible | none (uses `local`) | `qwen3-coder` |
| `lmstudio` | OpenAI compatible | none (uses `local`) | `local-model` |

List them with `d3 --list-providers`, and fetch the selected provider's
models with `d3 --list-models`.

### Custom providers

`harness.json` can define or override providers:

```json
{
  "provider": "my-gateway",
  "model": "my-model",
  "providers": {
    "my-gateway": {
      "type": "openai",
      "base_url": "https://gateway.example.com/v1",
      "api_key_env": "MY_GATEWAY_KEY",
      "model": "my-model",
      "max_tokens": 8192,
      "headers": {
        "X-Project": "demo"
      }
    }
  }
}
```

`type` is one of `openai` (Chat Completions), `responses` (OpenAI Responses),
or `anthropic` (Messages). `base_url` should be the API root; the harness appends
`/chat/completions`, `/responses`, or `/messages`.

A provider can also set `"api_key"` directly, or point at a non-standard model
catalog with `"models_url"`. `HARNESS_API_KEY` overrides every provider key. For
`opencode`, the harness falls back to OpenCode's `auth.json` if it can find it.

OpenCode's model list lives at a different path from its Chat Completions
endpoint, so the built-in `opencode` preset sets:

```json
"models_url": "https://opencode.ai/inference/v1/models"
```

The default for other providers is `<base_url>/models`.

If you already use Codex models through OpenCode, this works too because
OpenCode's gateway translates text models across its APIs:

```bash
d3 -p opencode -m gpt-5.3-codex "implement the endpoint"
```

## Configuration

The harness reads configuration in this order:

1. `--config <path>` if given (no other config files are merged).
2. `~/.config/harness/config.json`.
3. The last provider and model selected in the TUI, stored in
   `~/.config/harness/selection.json` (or under `$XDG_CONFIG_HOME`).
4. The nearest `harness.json` or `.harness/config.json` from the workspace up
   to the filesystem root. Project values override global and saved selections.

An explicit `--config` bypasses the saved selection. Environment variables and
CLI options override it as well. The message editor wraps long input and grows
to six rows; longer input scrolls within the editor.

Environment overrides: `HARNESS_PROVIDER`, `HARNESS_MODEL`,
`HARNESS_MAX_STEPS`, `HARNESS_API_KEY`.

### API keys

If a key is missing and the run is interactive, the harness asks for it:

```text
No API key found for opencode
Get a key at https://opencode.ai/auth.
Paste the API key (hidden):
```

The key is saved to `~/.config/harness/auth.json` (or
`$XDG_CONFIG_HOME/harness/auth.json`) with `0600` permissions and reused on the
next run. It is checked against the provider's model catalog before the session
starts; a key the provider rejects is removed instead of being kept. Run
`d3 --login -p opencode` to force the prompt again, for example to rotate a
key or retry after an invalid one. When stdin is not a terminal, the harness
never prompts; it fails with the environment variable and config options
instead.

Keys can also come from the provider's environment variable, an `api_key` in
`harness.json`, a custom `api_key_env`, or `HARNESS_API_KEY`.

### ChatGPT / OpenAI account authentication

OpenAI in the TUI uses account authentication, not an API key:

- The `chatgpt` provider signs in with ChatGPT using the official device-code
  flow. The TUI shows `https://auth.openai.com/codex/device` and a one-time code,
  waits for browser approval, then connects.
- It drives the official `codex login --device-auth` command, so the `codex` CLI
  must be installed (`npm install -g @openai/codex`). No separate `codex login`
  is required; the TUI starts it and keeps the code and status on screen.
- After sign-in, harness uses the tokens stored by Codex at runtime and calls the
  ChatGPT Codex backend. It never copies those tokens into harness config.
- Models come from the Codex model list, fetched by the provider or read from
  `models_cache.json` when offline.
- If an existing Codex login is present, choosing `chatgpt` connects immediately.
  Use `/login` to force a fresh device sign-in.

OpenAI API-key providers (`openai`, `codex`) remain available to the CLI and
custom `harness.json` setups, but they are intentionally hidden from the TUI
provider picker. The TUI path is the account sign-in flow.

The ChatGPT provider speaks the same Responses wire format as the OpenAI
provider, including reasoning-item replay for tool calls.

A complete example is in [`harness.example.json`](harness.example.json).

### Permissions

```json
{
  "permissions": {
    "shell": "ask",
    "edit": "allow"
  },
  "auto_approve": false
}
```

- `shell`: `ask` (default), `allow`, or `deny`.
- `edit`: `allow` (default), `ask`, or `deny`.
- `--yes` (also `--skip-approval` or `--auto-approve`) automatically approves
  `ask` permissions. Explicit `deny` permissions remain in effect.
- In a non-interactive run, `ask` behaves like `deny`. That is deliberate: a
  command should not be silently approved because stdin is a pipe.

File tools are confined to the workspace. Absolute paths and `..` escapes are
rejected. `bash` runs with the workspace as its working directory, but it is a
shell, not a sandbox.

## AGENTS.md

The harness loads every `AGENTS.md` from the filesystem root down to the
workspace, plus:

- `~/.config/harness/AGENTS.md`
- `~/.config/opencode/AGENTS.md`
- files listed under `"instructions"` in `harness.json`

All are injected into the system prompt with their paths. The nearest file is
listed last and wins on conflict. Each file is capped at 50,000 characters.

This means an existing OpenCode project works without changes.

## Skills

Skills are directories containing a `SKILL.md` file with YAML frontmatter:

```markdown
---
name: review
description: Reviews a diff for correctness, regressions, and missing tests.
---

# Review

Read the diff. Check behavior, not just style. Report concrete findings with
file and line references. Run the tests if they are available.
```

Search paths:

- `<workspace>/.harness/skills/<name>/SKILL.md`
- `<workspace>/.agents/skills/<name>/SKILL.md`
- `<workspace>/.opencode/skills/<name>/SKILL.md`
- `<workspace>/.opencode/skill/<name>/SKILL.md`
- `<workspace>/.claude/skills/<name>/SKILL.md`
- `~/.agents/skills/<name>/SKILL.md`
- `~/.agent/skills/<name>/SKILL.md`
- `~/.config/harness/skills/...`
- `~/.config/opencode/skills/...`
- `~/.claude/skills/...`
- Additional paths under `"skills": {"paths": [...]}` in `harness.json`

Skills are discovered at startup; restart the harness after installing new skills.
When names collide, the first search path above takes precedence.

Only the name and description go into the system prompt. The body is loaded
when the model calls the `skill` tool, which keeps the context small. Skills can
reference files next to `SKILL.md`; the tool returns the source path so the agent
can read them.

## MCP

Configure servers in `harness.json`:

```json
{
  "mcp": {
    "servers": {
      "filesystem": {
        "type": "stdio",
        "command": "npx",
        "args": ["-y", "@modelcontextprotocol/server-filesystem", "."]
      },
      "remote": {
        "type": "http",
        "url": "https://mcp.example.com/mcp",
        "headers": {
          "Authorization": "Bearer ${MCP_TOKEN}"
        }
      }
    }
  }
}
```

For stdio, `command` can be a string plus `args`, or OpenCode's array form:

```json
{
  "type": "local",
  "command": ["npx", "-y", "@modelcontextprotocol/server-filesystem", "."],
  "environment": {"DEBUG": "1"},
  "enabled": true
}
```

For HTTP, the harness sends JSON-RPC over POST, follows `Mcp-Session-Id`, and
parses both JSON and SSE responses. `enabled: false` skips a server.

Tools are listed in `--dump-context` and called as `mcp__<server>__<tool>`.
A server that fails to start produces a warning and does not stop the session.

## CLI

```text
d3 [OPTIONS] [PROMPT]...

  -m, --model <MODEL>          model id
  -p, --provider <PROVIDER>    provider name
      --system-prompt <TEXT>   replace the built-in core prompt
      --system-file <PATH>     replace it with a file
  -C, --cwd <DIR>              workspace directory
      --config <PATH>          explicit config file
  -y, --yes, --skip-approval   auto-approve shell commands and edits
  -c, --continue               resume the latest workspace conversation
      --list-sessions          list saved workspace conversations
      --resume [ID]            resume a conversation by ID, or the latest
      --max-steps <N>          model turns per user message (default 1000)
      --dump-context           print prompt, tools, skills, and MCP as JSON
      --list-providers         list providers and exit
      --list-models            fetch the provider's models and exit
      --login                  ask for an API key, save it, verify it, and exit
  -q, --quiet                  hide the startup banner
      --no-tui                 use the plain line-based REPL
      --no-color               disable color
```

A custom system prompt replaces the core instructions, but the harness still
appends the environment, `AGENTS.md`, skills catalog, and harness notes.

One-shot examples:

```bash
d3 "add a --json flag to the CLI"
d3 -C ../other-project "why does this test fail?"
echo "summarize the changes" | d3
git diff | d3 "review this diff for bugs"
```

Interactive commands:

```text
/help              show commands
/new               clear the conversation
/model [id]        show or change the model
/models            fetch the provider's models and choose one
/provider          choose and switch provider
/login             sign in with ChatGPT or update a key
/system            how to change the system prompt
/tools             list available tools
/skills            select an available skill (also type $ in a prompt)
/mcp               list connected MCP servers
/context           message and token counts
/approve [on|off]  toggle auto-approval
/exit              leave
```

## Verification

```bash
cargo test
```

The test suite covers:

- the full agent loop against a mock streaming Chat Completions API, including
  writing a file, sending the tool result back, and injecting `AGENTS.md` and a
  skill description;
- Anthropic Messages streaming and tool-result serialization;
- OpenAI Responses streaming, function calls, and `function_call_output`;
- write/edit/glob/grep/shell behavior and workspace path confinement;
- skill loading and visible todos;
- stdio MCP tool discovery and calls;
- HTTP MCP initialization, session headers, SSE responses, and calls;
- provider model-list fetching and sorted output;
- API-key storage permissions and preservation of other provider keys;
- TUI command-palette filtering, turn submission, streaming, and approval modals;
- multi-line input editing and cursor movement at UTF-8 boundaries.

To inspect exactly what the model will receive without spending tokens:

```bash
d3 --dump-context | jq '.system'
d3 --dump-context | jq '.tools[].name'
d3 --list-models
```

For a live smoke test:

```bash
export OPENCODE_API_KEY="..."
d3 -q "Reply with exactly: harness is alive"
```

## Design notes

The harness is deliberately small. It uses one provider abstraction, one
sequential agent loop, and one tool registry. The CLI and TUI share the same
`AgentUi` and `ToolUi` traits, so tool execution never knows which interface is
drawing it. There is no hidden agent orchestration: every tool call and result
is in the conversation, and every file change is shown as a diff.

Default prompt principles:

- solve the user's actual problem, not a guessed feature;
- ask before guessing when the answer changes the design;
- keep changes small and match the surrounding code;
- write comments only for non-obvious reasons;
- verify with real commands and report failures honestly;
- keep the user oriented so a mistake is visible while it is still cheap to fix.

Not included yet: context compaction, image input, subagents, LSP integration,
and sandboxing. Those are deliberate omissions for
a first version, not hidden claims.
