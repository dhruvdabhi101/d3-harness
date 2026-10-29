# harness

A small, honest coding-agent harness for the terminal, written in Rust.

It connects to OpenCode's inference API (or OpenAI, Anthropic, and any
OpenAI-compatible endpoint), runs a tool-calling agent loop, and loads the
context a real project needs: `AGENTS.md`, skills, and MCP servers.

The default system prompt is part of the product. It asks the agent to write
code a careful human would review, to ask when ambiguity matters, to keep the
user oriented, and to verify what it claims.

## Build

```bash
cargo build --release
./target/release/harness --help
```

Install it somewhere on `PATH` if you like:

```bash
cargo install --path .
```

## Quick start with OpenCode

Get an API key from <https://opencode.ai/auth>, then:

```bash
export OPENCODE_API_KEY="..."
harness "explain what this repository does"
```

The default provider is `opencode` and the default model is
`deepseek-v4.1-flash`. Any model available through OpenCode Console can be
selected:

```bash
harness -m gpt-5.3-codex "fix the failing test"
harness -m claude-sonnet-4-5 "review this diff"
```

Run without a prompt for an interactive session:

```bash
harness
```

Options must come before the prompt. `harness "prompt" --yes` treats `--yes` as
part of the prompt.

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
- **Streaming**: SSE streaming for Chat Completions, Anthropic Messages, and
  OpenAI Responses. Non-streaming JSON responses also work.

## Providers

Built-in presets:

| Provider | API | Key | Default model |
| --- | --- | --- | --- |
| `opencode` | OpenAI Chat Completions | `OPENCODE_API_KEY` | `deepseek-v4.1-flash` |
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

List them with `harness --list-providers`.

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

A provider can also set `"api_key"` directly. `HARNESS_API_KEY` overrides every
provider key. For `opencode`, the harness falls back to OpenCode's
`auth.json` if it can find it.

If you already use Codex models through OpenCode, this works too because
OpenCode's gateway translates text models across its APIs:

```bash
harness -p opencode -m gpt-5.3-codex "implement the endpoint"
```

## Configuration

The harness reads configuration in this order:

1. `--config <path>` if given (no other config files are merged).
2. `~/.config/harness/config.json`.
3. The nearest `harness.json` or `.harness/config.json` from the workspace up
   to the filesystem root. Project values override global values.

Environment overrides: `HARNESS_PROVIDER`, `HARNESS_MODEL`,
`HARNESS_MAX_STEPS`, `HARNESS_API_KEY`.

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
- `--yes` ignores `ask` and approves everything.
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
- `<workspace>/.opencode/skills/<name>/SKILL.md`
- `<workspace>/.opencode/skill/<name>/SKILL.md`
- `<workspace>/.claude/skills/<name>/SKILL.md`
- `~/.config/harness/skills/...`
- `~/.config/opencode/skills/...`
- `~/.claude/skills/...`
- Additional paths under `"skills": {"paths": [...]}` in `harness.json`

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
harness [OPTIONS] [PROMPT]...

  -m, --model <MODEL>          model id
  -p, --provider <PROVIDER>    provider name
      --system-prompt <TEXT>   replace the built-in core prompt
      --system-file <PATH>     replace it with a file
  -C, --cwd <DIR>              workspace directory
      --config <PATH>          explicit config file
  -y, --yes                    auto-approve shell commands and edits
      --max-steps <N>          model turns per user message (default 40)
      --dump-context           print prompt, tools, skills, and MCP as JSON
      --list-providers         list providers and exit
  -q, --quiet                  hide the startup banner
      --no-color               disable color
```

A custom system prompt replaces the core instructions, but the harness still
appends the environment, `AGENTS.md`, skills catalog, and harness notes.

One-shot examples:

```bash
harness "add a --json flag to the CLI"
harness -C ../other-project "why does this test fail?"
echo "summarize the changes" | harness
git diff | harness "review this diff for bugs"
```

Interactive commands:

```text
/help              show commands
/new               clear the conversation
/model [id]        show or change the model
/provider          show the current provider
/system            how to change the system prompt
/tools             list available tools
/skills            list available skills
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
- HTTP MCP initialization, session headers, SSE responses, and calls.

To inspect exactly what the model will receive without spending tokens:

```bash
harness --dump-context | jq '.system'
harness --dump-context | jq '.tools[].name'
```

For a live smoke test:

```bash
export OPENCODE_API_KEY="..."
harness -q "Reply with exactly: harness is alive"
```

## Design notes

The harness is deliberately small. It uses one provider abstraction, one
sequential agent loop, and one tool registry. There is no hidden agent
orchestration: every tool call and result is in the conversation, and every file
change is shown as a diff.

Default prompt principles:

- solve the user's actual problem, not a guessed feature;
- ask before guessing when the answer changes the design;
- keep changes small and match the surrounding code;
- write comments only for non-obvious reasons;
- verify with real commands and report failures honestly;
- keep the user oriented so a mistake is visible while it is still cheap to fix.

Not included yet: session persistence/resume, context compaction, image input,
subagents, LSP integration, and sandboxing. Those are deliberate omissions for
a first version, not hidden claims.
