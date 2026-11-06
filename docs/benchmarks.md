# Coding-agent benchmarks

This note surveys the benchmark suites and evaluation harnesses used to compare
coding agents. It is a map, not a leaderboard. Scores move quickly and should be
read from the benchmark's own leaderboard.

## Task suites

| Benchmark | What it measures | Typical shape | Why it matters |
| --- | --- | --- | --- |
| **SWE-bench** | Fixing real GitHub issues | Repo checkout, issue text, patch, hidden tests in Docker | The original and most cited coding-agent benchmark |
| **SWE-bench Verified** | Same, human-filtered | 500 instances reviewed for clear text, correct tests, and solvability | The headline benchmark most vendors quote |
| **SWE-bench Pro** | Long-horizon, harder software work | Larger multi-file tasks, contamination-resistant design | Better signal once Verified saturates |
| **SWE-bench Multilingual** | Cross-language issue fixing | Multiple language ecosystems | Tests language-general tool use |
| **Multi-SWE-bench** | Multi-language, multi-repo fixes | Per-language task sets | Complements SWE-bench's Python-heavy history |
| **Terminal-Bench 2.x** | End-to-end terminal work | 89+ containerized tasks: build, debug, configure, train, serve | Closest match to a terminal coding harness |
| **SWE-Lancer** | Real freelance-style engineering work | Tasks with dollar-valued outcomes | Measures practical, scoped delivery |
| **Senior SWE-Bench** | Senior-level engineering assignments | 100 Harbor-compatible tasks | Tests judgment and larger changes |
| **Aider Polyglot** | Precise code editing | 225 Exercism exercises across C++, Go, Java, JavaScript, Python, Rust | Strong edit-format and instruction-following signal |
| **LiveCodeBench** | Code generation without contamination | Time-stamped contest-style problems | Tracks new problems as models train |
| **RepoBench / RepoQA** | Repository-level retrieval and context use | Find and use the right code across a repo | Measures context assembly, not just editing |
| **SWE-Rebench** | Fresh, less-contaminated issue fixing | Continuously refreshed repository tasks | Addresses benchmark leakage |
| **HumanEval / MBPP** | Small standalone code generation | Function-level prompts and tests | Historical baseline; no longer useful for frontier agents |

General-agent suites such as **OSWorld**, **WebArena**, **AgentBench**, **GAIA**,
and **Tau-Bench** are useful for browsing, desktop, or tool-orchestration work,
but they are not primarily coding-harness benchmarks.

## Evaluation harnesses

| Harness | Role |
| --- | --- |
| **Harbor** | The framework behind Terminal-Bench. Runs tasks in isolated sandboxes, supports many agents and models, and is designed for large parallel runs. |
| **SWE-bench harness** | The official Docker-based runner and grading path for SWE-bench instances. |
| **Inspect AI** | General agent and model evaluation framework with tool use, sandboxes, and reproducible logs. |
| **EleutherAI lm-evaluation-harness** | Standard harness for language-model tasks; not agentic coding by default. |
| **HELM** | Stanford's broad, multi-scenario model evaluation suite. |
| **OpenAI Evals** | Open-source framework for defining and running model evals. |
| **Aider benchmark scripts** | The runner behind the Aider leaderboards, focused on edit formats and test pass rates. |

## What fits d3

- `d3 bench` is the fast, dependency-free regression check. It runs the real
  agent loop and tools and takes seconds.
- **Terminal-Bench via Harbor** is the best external fit: it exercises a terminal
  coding agent in containers and already supports pluggable agents.
- **SWE-bench Verified** is the best-known external number, but it needs Docker,
  repository setup, patch extraction, and hidden-test grading. It is a larger
  integration than Harbor.
- **Aider Polyglot** is the best lightweight external check for edit accuracy,
  but it mostly bypasses shell and repository-navigation behavior.
- **LiveCodeBench** is useful for model comparison, not for testing a harness.

## Suggested integration order

1. Keep `d3 bench` for smoke/read/write/edit regressions.
2. Add a Harbor adapter so `d3` can be run as a Terminal-Bench agent.
3. Add a small SWE-bench Verified runner or adapter for the standard number.
4. Add Aider Polyglot only if edit-format quality becomes a focus.

## Sources

- <https://www.swebench.com/>
- <https://www.swebench.com/verified.html>
- <https://github.com/harbor-framework/terminal-bench>
- <https://harborframework.com/docs>
- <https://aider.chat/docs/leaderboards/>
- <https://www.tbench.ai/>
- <https://inspect.aisi.org.uk/>
- <https://github.com/EleutherAI/lm-evaluation-harness>
- <https://crfm.stanford.edu/helm/>
- <https://github.com/openai/evals>
