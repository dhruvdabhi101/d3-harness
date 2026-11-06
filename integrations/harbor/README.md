# Running Terminal-Bench with d3

This directory contains a Harbor agent adapter for `d3`. Harbor is the evaluation
framework behind Terminal-Bench.

## Prerequisites

- Docker (or another Harbor environment backend with credentials)
- `uv`
- A locally built `target/release/d3`
- A model credential, for example `OPENCODE_GO_API_KEY`

```bash
uv tool install harbor
cargo build --release
```

## Run

```bash
export OPENCODE_GO_API_KEY="..."
PYTHONPATH=. harbor run \
  -d terminal-bench/terminal-bench@latest \
  --agent integrations.harbor.d3_agent:D3Agent \
  --ak binary_path="$PWD/target/release/d3" \
  -m deepseek-v4.1-flash \
  --ae OPENCODE_GO_API_KEY="$OPENCODE_GO_API_KEY" \
  --allow-agent-host opencode.ai \
  --env docker \
  --n-concurrent 1
```

Use `-l 1` first to smoke-test the adapter. The adapter:

1. uploads `target/release/d3` into the task container;
2. installs it at `/usr/local/bin/d3`;
3. runs one non-interactive, auto-approved turn in the task workdir;
4. writes the transcript to the trial's `agent/d3.txt`;
5. records input and output token counts from d3's stderr summary.

Harbor's own agent preflight can be checked without Docker:

```bash
PYTHONPATH=. harbor run \
  -d terminal-bench/terminal-bench@latest \
  --agent integrations.harbor.d3_agent:D3Agent \
  --ak binary_path="$PWD/target/release/d3" \
  -m deepseek-v4.1-flash \
  --dry-run --yes
```

Docker or a configured cloud environment is still required for a real run.
