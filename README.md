# harness

A small, honest coding-agent harness for the terminal, written in Rust.

It opens a full-screen TUI with a message box, a slash-command palette, model
picking, approval dialogs, and live tool diffs. Behind it is a tool-calling
agent loop that loads the context a real project needs: `AGENTS.md`, skills, and
MCP servers. It talks to OpenCode, OpenCode Go, ChatGPT/Codex, OpenAI, Anthropic,
and any OpenAI-compatible endpoint.

The harness also has a real management CLI: provider sign-in and logout,
credential checks, model discovery, session listing, config inspection, MCP
inspection, `AGENTS.md` scaffolding, updates, and a built-in benchmark that runs
the actual agent loop and tools. In chat, `/compact` reclaims context, `/export`
saves a transcript, `/diff` shows the working tree, `!command` runs a shell
command, and `@path` attaches a file.

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

## Sign in and manage credentials

Credentials are managed by `d3 auth`, or by `/login` inside the TUI. The
interactive flow never leaves the screen: choose a provider, paste an API key
(masked and verified before it is saved), or approve a ChatGPT device code.
`/logout` removes a stored key, and `/login` can rotate one.

```bash
d3 auth list                    # every provider, readiness, and key source
d3 auth login opencode          # prompt, verify, and save a key
d3 auth login                   # choose a provider from a list
d3 auth login chatgpt           # device-code account sign-in
d3 auth login opencode-go       # OpenCode Go subscription key
d3 auth login opencode --api-key
d3 auth logout opencode         # remove only the stored credential
d3 auth logout --all            # remove every stored harness credential
d3 auth check opencode          # local readiness, no network call
d3 auth check opencode --verify # call the model catalog
d3 auth check --json
```

Key handling rules:

- An API key is verified against the provider's model catalog before it is
  saved. A key the provider rejects is not kept.
- A network or catalog failure still allows the key to be saved, with a warning,
  so an outage does not block setup.
- Keys are stored in `~/.config/harness/auth.json` with `0600` permissions.
- Environment variables and `harness.json` values are never deleted by logout.
- `d3 --login`, `--logout`, and `--check-auth` remain as aliases for the
  selected provider.
- OpenCode v2 keeps credentials in its own store. If a key is not found in
  harness config, d3 asks `opencode auth export` for the OpenCode Console or
  OpenCode Go credential. This means an existing OpenCode login can be reused
  without copying a token into a file.

ChatGPT sign-in drives the official `codex login --device-auth` flow. The TUI
shows the verification URL and one-time code, opens the browser when possible
(set `D3_NO_BROWSER=1` to disable), keeps the code on screen, and restores the
previous Codex credentials if the login is cancelled. Models are read from
Codex's catalog. ChatGPT credentials remain owned by Codex; `d3 auth logout
chatgpt` explains that `codex logout` removes them.

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

OpenCode Go uses its own endpoint and subscription:

```bash
d3 -p opencode-go -m deepseek-v4.1-flash "implement the endpoint"
d3 models -p opencode-go --json
```

If OpenCode is already signed in, d3 reuses the stored OpenCode credential.
The built-in `opencode-go` provider is the normal way to use a Go subscription.
If a tool or script must run under the literal provider name `opencode` while
the Console balance is empty, point the `opencode` preset at the Go endpoint:

```bash
export OPENCODE_GO_API_KEY="..."
d3 -p opencode --config harness.opencode-go.json "implement the endpoint"
d3 bench -p opencode --config harness.opencode-go.json --suite all
```

The repository includes [`harness.opencode-go.json`](harness.opencode-go.json)
for that setup. It contains only an environment-variable reference, never a key.

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
- Credential management via `/login` (provider picker, masked key, live
  verification) and `/logout`.
- ChatGPT device-code sign-in via `/login`, including the URL and one-time code.
- Live model picker via `/models`, fetched from the provider catalog.
- `/compact` summarizes older history with the current model and replaces it
  with a compact summary, so long sessions can keep going.
- `/export` writes a Markdown transcript; `/copy` copies the last answer;
  `/diff` shows Git status and the working diff; `/status` and `/cost` report
  provider, branch, messages, and token usage.
- `!command` runs a shell command and adds its output to the conversation;
  `!!command` runs it without adding the output. `@path` attaches a file to the
  next prompt, including images.
- `/init` scaffolds `AGENTS.md`; `/permissions` changes shell/edit policy live.
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
| `Esc` twice within one second | stop the current turn without quitting |
| `Ctrl+V` | paste a clipboard image or text |
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

Press Esc twice within one second to stop an active response or tool wait.
The app stays open and keeps your draft and completed history. Unfinished tool
calls receive a stopped result so the conversation can continue. Stopping does
not undo file edits, external requests, or shell side effects that already
happened. Shell subprocess descendants may outlive cancellation.

### Attach images

Paste images into the chatbox. The editor detects image-file paths and adds
them as attachments for your next message. The editor title shows pending image
names. `/detach` removes them. You can send an image without text.

Paste one or more image-file paths on separate lines, including alongside text.
Quoted paths with spaces are accepted. Press `Ctrl+V` to paste a PNG image or
text from the system clipboard. Clipboard image paste
requires `wl-paste` from wl-clipboard or `xclip` on Linux, or `pngpaste` on macOS.
Ordinary terminal text paste still inserts text. A clipboard reader has a
three-second timeout.

Relative attachment paths resolve against the workspace selected by `-C`.

PNG, JPEG, GIF, and WebP attachments have a five-MiB limit per image. The file
signature determines the format. The provider still validates the image and
may impose smaller limits or reject a format. Images appear as named attachments
in chat, not inline terminal previews. Their encoded bytes are stored inside
private conversation snapshots, so resume works after the source file is moved
or deleted. Older d3 versions cannot read snapshots containing images.

Images use the native request format for Chat Completions, Anthropic Messages,
and Responses, including ChatGPT. Select a model that accepts images. When a
fetched model catalog explicitly reports no image support, d3 rejects the
request. For providers that omit capability metadata, d3 sends the attachment
and reports any API rejection. It does not guess support from the model name.
Set `"image_input": false` in a provider configuration to disable image input
explicitly. This also blocks requests that replay images from saved history.

Long chat lines use hanging indentation. Wrapped bullet and numbered-list text
stays under the text after the marker. Other indented lines retain their starting
column. Markdown styles and Unicode graphemes are preserved across wrapping.

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
| `opencode-go` | OpenAI Chat Completions (Go) | `OPENCODE_GO_API_KEY` | `deepseek-v4.1-flash` |
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
`opencode` and `opencode-go`, the harness falls back to OpenCode's credential
store: legacy `auth.json` files first, then `opencode auth export` for OpenCode
v2. ChatGPT account credentials are read from Codex at runtime.

OpenCode's Console model list lives at a different path from its Chat
Completions endpoint, and OpenCode Go has its own base URL. The built-in presets
set both:

```json
"opencode": {
  "base_url": "https://opencode.ai/inference/openai/v1",
  "models_url": "https://opencode.ai/inference/v1/models"
},
"opencode-go": {
  "base_url": "https://opencode.ai/inference/go/openai/v1",
  "models_url": "https://opencode.ai/inference/go/v1/models"
}
```

OpenCode Go expects a stable `x-opencode-session` header. d3 sends one per
provider session and identifies itself with `d3/<version>` as its user agent.

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
next run. It is checked against the provider's model catalog before it is
saved; a key the provider rejects is not kept. A catalog outage still saves the
key with a warning. Run `d3 auth login opencode` to rotate a key, `d3 auth
logout opencode` to remove it, or `d3 auth check opencode --verify` to test it.
Inside the TUI, `/login` and `/logout` do the same without leaving the screen.
When stdin is not a terminal, the harness never prompts; it fails with the
environment variable and config options instead.

Keys can also come from the provider's environment variable, an `api_key` in
`harness.json`, a custom `api_key_env`, or `HARNESS_API_KEY`. For `opencode` and
`opencode-go`, d3 also reuses the credential stored by OpenCode itself.

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
d3 auth <login|logout|check|list> [OPTIONS]
d3 models [SEARCH] [--json]
d3 sessions [--json]
d3 config [--json]
d3 mcp [--json]
d3 init [--force]
d3 update
d3 bench [--suite smoke|tools|all] [--json] [--filter TEXT] [--keep]
d3 version

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
      --login                  sign in, verify, and exit (alias for auth login)
      --logout                 remove the selected stored credential
      --check-auth             check the selected credential locally
  -q, --quiet                  hide the startup banner
      --no-tui                 use the plain line-based REPL
      --no-color               disable color
```

`d3 models` takes an optional case-insensitive search and can print JSON.
`d3 sessions` lists this workspace's saved conversations. `d3 config` shows the
resolved provider, model, permissions, and config/credential paths without
printing secrets. `d3 mcp` connects and lists each server's tools. `d3 init`
writes a starter `AGENTS.md` without overwriting an existing one unless
`--force` is given. `d3 update` checks GitHub for a newer stable release.

A custom system prompt replaces the core instructions, but the harness still
appends the environment, `AGENTS.md`, skills catalog, and harness notes.

One-shot examples:

```bash
d3 "add a --json flag to the CLI"
d3 -C ../other-project "why does this test fail?"
echo "summarize the changes" | d3
git diff | d3 "review this diff for bugs"
d3 models -p opencode-go deepseek
```

Interactive commands:

```text
/help              show commands and shortcuts
/new               start a fresh conversation
/resume [id]       choose a saved conversation
/continue          resume the latest conversation
/compact [focus]   summarize older history to free context
/status            provider, model, branch, and context
/context           message and token counts
/cost              token usage for this session
/diff              show Git status and the working diff
/export [path]     save this conversation as Markdown
/copy              copy the last answer to the clipboard
/model [id]        show or change the model
/models            fetch the provider's models and choose one
/provider          choose and switch provider
/login [provider]  sign in or rotate credentials
/logout [provider] remove a stored credential
/approve [on|off]  toggle auto-approval
/permissions       change shell/edit policy
/tools             list available tools
/skills            select a skill (also type $ in a prompt)
/mcp               list connected MCP servers
/init              create a starter AGENTS.md
/ui                show customization settings
/system            how to change the system prompt
/version           show the harness version
/paste-image       attach a clipboard image (Ctrl+V)
/detach            remove pending images
/exit              leave
```

Prompt shortcuts:

```text
!command           run a shell command and add its output to context
!!command          run it without adding output to context
@path              attach a text file or image to the next prompt
```

### Compaction

`/compact` asks the current model to summarize the conversation, replaces the
older messages with that summary, and keeps the session going. An optional
argument focuses the summary: `/compact keep the failing test and the API
contract`. Compaction is explicit; it never deletes a session file. The TUI
reports how many messages were replaced and the token usage of the summary.

### Export, diff, and clipboard

`/export` writes Markdown to `d3-conversation-<timestamp>.md` in the workspace,
or to the path you pass. `/diff` runs `git status`, `git diff --stat`, and the
working diff. `/copy` uses `pbcopy` on macOS or `wl-copy`/`xclip`/`xsel` on
Linux. `/context` and `/cost` report counts; the harness does not invent a price
when the provider does not return one.

## Benchmark

`d3 bench` runs the real agent loop, real tool registry, and real provider
connection against a temporary workspace. It is a harness check, not a
model-only chat test. See [`docs/benchmarks.md`](docs/benchmarks.md) for a survey
of external coding-agent benchmarks and how they compare with this one.

```bash
d3 bench -p opencode-go -m deepseek-v4.1-flash
d3 bench --suite smoke
d3 bench --suite tools --json
d3 bench --filter write --keep
```

Tasks:

- `reply`: the model must return an exact token.
- `read`: the model must call `read` and report a secret from a file.
- `write`: the model must call `write` and create the expected file.
- `edit`: the model must call `edit` and change the expected line.

The report includes pass/fail, latency, and input/output tokens per task. The
command exits non-zero when any task fails. `--keep` leaves the temporary
workspace in place for inspection.

A live run under the literal `opencode` provider, pointed at the funded OpenCode
Go endpoint with `harness.opencode-go.json`:

```text
harness benchmark · opencode/deepseek-v4.1-flash · suite all
TASK       RESULT       TIME       TOKENS  DETAIL
reply      PASS         719ms   8310/6      HARNESS_OK
read       PASS        2268ms  16701/52     I'll read the file.benchmark-secret-42
write      PASS        4471ms  16734/83     Created `answer.txt` with `benchmark-write-ok` and a trailing newline.
edit       PASS        4977ms  33769/197    I'll read the file first, then make the edit.DONE `edit_me.txt` line 2…

4/4 passed · 12.4s · 75514 input / 338 output tokens
```

### Terminal-Bench

Terminal-Bench latest currently resolves to 66 tasks (task version `2.0.8`).
The repository includes a Harbor adapter for running d3 as a Terminal-Bench
agent:

```bash
cargo build --release
PYTHONPATH=. harbor run \
  -d terminal-bench/terminal-bench@latest \
  --agent integrations.harbor.d3_agent:D3Agent \
  --ak binary_path="$PWD/target/release/d3" \
  -m deepseek-v4.1-flash \
  --ae OPENCODE_GO_API_KEY="$OPENCODE_GO_API_KEY" \
  --allow-agent-host opencode.ai \
  --env docker
```

See [`integrations/harbor/README.md`](integrations/harbor/README.md) for
prerequisites and the `-l 1` smoke-test command. A real run requires Docker or
a configured Harbor cloud environment.

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
- auth check JSON, key-source reporting, logout isolation, and non-interactive
  login failures;
- context compaction through the provider and history replacement;
- the benchmark harness against a mock provider, including tool read/write/edit
  tasks;
- TUI command-palette filtering, turn submission, streaming, and approval modals;
- `/compact`, `/permissions`, `!` shell passthrough, `@file` mentions, and the
  provider sign-in palette;
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

Not included yet: subagents, LSP integration, plugin packages, session
branching, and sandboxing. Those are deliberate omissions, not hidden claims.
