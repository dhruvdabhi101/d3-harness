"""Harbor agent adapter for the d3 harness.

Use it with Terminal-Bench via Harbor:

    PYTHONPATH=. harbor run \
        -d terminal-bench/terminal-bench@latest \
        --agent integrations.harbor.d3_agent:D3Agent \
        --ak binary_path="$PWD/target/release/d3" \
        -m deepseek-v4.1-flash \
        --ae OPENCODE_GO_API_KEY="$OPENCODE_GO_API_KEY" \
        --allow-agent-host opencode.ai

The adapter uploads a locally built d3 binary into the task container, runs one
non-interactive turn with auto-approval, and saves the transcript to the trial's
agent logs.
"""

from __future__ import annotations

import os
import re
import shlex
from pathlib import Path
from typing import Any, override

from harbor.agents.base import BaseAgent
from harbor.agents.capabilities import AgentCapabilities
from harbor.environments.base import BaseEnvironment
from harbor.models.agent.context import AgentContext


class D3Agent(BaseAgent):
    capabilities = AgentCapabilities()

    def __init__(
        self,
        *args: Any,
        binary_path: str | None = None,
        provider: str = "opencode-go",
        max_steps: int = 300,
        **kwargs: Any,
    ) -> None:
        super().__init__(*args, **kwargs)
        self.binary_path = Path(
            binary_path or os.environ.get("D3_BINARY", "target/release/d3")
        ).expanduser()
        self.provider = provider
        self.max_steps = max_steps
        self._version: str | None = None

    @staticmethod
    @override
    def name() -> str:
        return "d3"

    @override
    def version(self) -> str | None:
        return self._version

    @override
    async def setup(self, environment: BaseEnvironment) -> None:
        if not self.binary_path.is_file():
            raise FileNotFoundError(
                f"d3 binary not found at {self.binary_path}. "
                "Build it with `cargo build --release` or pass "
                "`--ak binary_path=/absolute/path/to/d3`."
            )
        remote_tmp = "/tmp/d3-harbor-agent"
        await environment.upload_file(self.binary_path, remote_tmp)
        await environment.exec(
            f"install -m 0755 {shlex.quote(remote_tmp)} /usr/local/bin/d3",
            user="root",
            timeout_sec=60,
        )
        result = await environment.exec("d3 --version", timeout_sec=30)
        if result.return_code != 0:
            raise RuntimeError(f"d3 is not runnable in the task: {result.stderr}")
        self._version = (result.stdout or "").strip() or None

    @override
    async def run(
        self,
        instruction: str,
        environment: BaseEnvironment,
        context: AgentContext,
    ) -> None:
        model = self.model_name or "deepseek-v4.1-flash"
        workdir = environment.task_env_config.workdir or "/app"
        env = self._agent_env()
        command = " ".join(
            [
                "d3",
                "-q",
                "-y",
                "--no-tui",
                "-p",
                shlex.quote(self.provider),
                "-m",
                shlex.quote(model),
                "--max-steps",
                str(self.max_steps),
                shlex.quote(instruction),
            ]
        )
        result = await environment.exec(
            command=command,
            cwd=workdir,
            env=env,
            timeout_sec=int(os.environ.get("D3_HARBOR_TIMEOUT_SEC", "28800")),
        )
        transcript = "\n".join(
            part for part in (result.stdout, result.stderr) if part
        )
        self.logs_dir.mkdir(parents=True, exist_ok=True)
        (self.logs_dir / "d3.txt").write_text(transcript, encoding="utf-8")
        input_tokens, output_tokens = parse_usage(transcript)
        context.n_input_tokens = input_tokens
        context.n_output_tokens = output_tokens
        context.metadata = {
            "provider": self.provider,
            "model": model,
            "exit_code": result.return_code,
        }
        if result.return_code != 0:
            raise RuntimeError(
                f"d3 exited with {result.return_code}\n{transcript[-4000:]}"
            )

    def _agent_env(self) -> dict[str, str]:
        env = {
            "D3_NO_BROWSER": "1",
            "NO_COLOR": "1",
        }
        for key in (
            "OPENCODE_GO_API_KEY",
            "OPENCODE_API_KEY",
            "HARNESS_API_KEY",
            "OPENAI_API_KEY",
            "ANTHROPIC_API_KEY",
        ):
            value = self._extra_env.get(key) or os.environ.get(key)
            if value:
                env[key] = value
        return env


def parse_usage(transcript: str) -> tuple[int | None, int | None]:
    matches = re.findall(r"(\d+) in / (\d+) out tokens", transcript)
    if not matches:
        return None, None
    input_tokens = sum(int(match[0]) for match in matches)
    output_tokens = sum(int(match[1]) for match in matches)
    return input_tokens, output_tokens
