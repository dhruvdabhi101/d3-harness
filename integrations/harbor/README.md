# Running Terminal-Bench with d3

This directory contains a Harbor agent adapter for `d3`. Harbor is the evaluation
framework behind Terminal-Bench.

## Prerequisites

- Docker (or another Harbor environment backend with credentials)
- `uv`
- A statically linked `d3` binary for the task container
- A model credential, for example `OPENCODE_GO_API_KEY`

```bash
uv tool install harbor
rustup target add x86_64-unknown-linux-musl
sudo apt-get install -y musl-tools
cargo build --release --target x86_64-unknown-linux-musl
```

A glibc build can fail inside older task images (`GLIBC_2.39 not found`).
The GitHub Actions workflow in `.github/workflows/terminal-bench.yml` builds the
musl binary automatically.

## Run

```bash
export OPENCODE_GO_API_KEY="..."
PYTHONPATH=. harbor run \
  -d terminal-bench/terminal-bench@latest \
  --agent integrations.harbor.d3_agent:D3Agent \
  --ak binary_path="$PWD/target/x86_64-unknown-linux-musl/release/d3" \
  -m deepseek-v4.1-flash \
  --ae OPENCODE_GO_API_KEY="$OPENCODE_GO_API_KEY" \
  --allow-agent-host opencode.ai \
  --env docker \
  --n-concurrent 1
```

Use `-l 1` first to smoke-test the adapter. To select one task, pass its fully
qualified name, for example `-i terminal-bench/html-js-filter`.

The adapter:

1. uploads the statically linked `d3` binary into the task container;
2. installs it at `/usr/local/bin/d3`;
3. runs one non-interactive, auto-approved turn in the task workdir;
4. writes the transcript to the trial's `agent/d3.txt`;
5. records input and output token counts from d3's stderr summary.

Harbor's own agent preflight can be checked without Docker:

```bash
PYTHONPATH=. harbor run \
  -d terminal-bench/terminal-bench@latest \
  --agent integrations.harbor.d3_agent:D3Agent \
  --ak binary_path="$PWD/target/x86_64-unknown-linux-musl/release/d3" \
  -m deepseek-v4.1-flash \
  --dry-run --yes
```

Docker or a configured cloud environment is still required for a real run.
