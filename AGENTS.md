# oc-harness

A Rust CLI coding agent. The binary is `harness`, the crate is `oc-harness`.

## Build and verify

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```

Run the binary from source with `cargo run -- <args>` or
`./target/debug/harness <args>`.

## Layout

- `src/lib.rs` — startup, provider selection, context assembly, CLI dispatch.
- `src/agent.rs` — the sequential model/tool loop.
- `src/ui_bridge.rs` — `AgentUi` and `ToolUi` traits shared by the CLI and TUI.
- `src/tui/` — ratatui interface. Uses `tui-textarea-2` for the message
  editor, `tui-markdown` for assistant answers, and ratatui `List`/`Scrollbar`
  widgets for the palette and scrollback.
- `src/provider/` — OpenAI Chat Completions, Anthropic Messages, OpenAI Responses.
- `src/tools/` — built-in tools and the tool registry.
- `src/mcp/` — stdio and streamable-HTTP MCP clients.
- `src/prompt.rs` — default system prompt and AGENTS.md loading.
- `src/skills.rs` — SKILL.md discovery and parsing.
- `src/config.rs` — harness.json, provider presets, permissions.
- `tests/` — mock API, provider, tool, and MCP integration tests.

## Rules

- Keep the harness small. Prefer one clear implementation over a framework.
- Do not add dependencies without a concrete reason.
- The default system prompt is a product decision, not filler. Changes to it
  should preserve the principles: solve the actual problem, ask when ambiguity
  matters, keep the user oriented, write sparse useful comments, verify claims.
- Tool output is model-facing. Keep it precise and avoid large dumps.
- User-facing status goes to stderr; model text goes to stdout. This keeps
  `harness "..." > answer.txt` clean.
- File tools must stay inside the workspace and reject path escapes.
- Shell commands default to asking. Never silently approve a command just
  because stdin is not a terminal.
- API keys belong in `~/.config/harness/auth.json`, never in committed files.
  Key prompts must hide input and must not run when stdin is not a terminal.
- Model lists come from the provider catalog. OpenCode keeps its model list at
  `/inference/v1/models`, not under the Chat Completions base path.
- Update README.md and harness.example.json when configuration or CLI behavior
  changes.
- Add or update an integration test for behavior that crosses the model, tool,
  or provider boundary.
- TUI code must restore the terminal on every exit path and must not read from
  stdin directly; approvals and questions go through `ToolUi`.
- Permission approvals are docked at the bottom of the TUI. Do not turn them
  back into centered modals. User prompts and assistant answers use distinct
  background colors.
- Provider selection and authentication belong inside the TUI. `/provider`
  switches providers and `/login` starts the ChatGPT device-code flow.
- ChatGPT auth uses the official `codex login --device-auth` device flow and reads
  Codex credentials at runtime. Never copy those tokens into harness config or
  logs.
- Before starting a device login, preserve any existing Codex auth file so a
  cancelled or failed login does not log the user out.
