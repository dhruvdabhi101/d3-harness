use crate::agent::Agent;
use crate::cli::BenchArgs;
use crate::config::{Config, ProviderConfig, ProviderKind};
use crate::provider::{Provider, ProviderAuth, Usage};
use crate::tools::ToolRegistry;
use crate::ui_bridge::{AgentUi, ToolUi};
use anyhow::{Context, Result, bail};
use async_trait::async_trait;
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Serialize)]
pub struct TaskResult {
    pub suite: String,
    pub name: String,
    pub passed: bool,
    pub duration_ms: u128,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub detail: String,
}

#[derive(Debug, Serialize)]
pub struct BenchReport {
    pub provider: String,
    pub model: String,
    pub suite: String,
    pub passed: usize,
    pub total: usize,
    pub duration_ms: u128,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub workspace: Option<String>,
    pub tasks: Vec<TaskResult>,
}

#[derive(Default)]
struct BenchUi {
    text: Mutex<String>,
    warnings: Mutex<Vec<String>>,
    tools: Mutex<Vec<String>>,
}

impl BenchUi {
    fn text(&self) -> String {
        self.text.lock().unwrap().clone()
    }

    fn tools(&self) -> Vec<String> {
        self.tools.lock().unwrap().clone()
    }
}

impl AgentUi for BenchUi {
    fn text(&self, delta: &str) {
        self.text.lock().unwrap().push_str(delta);
    }

    fn text_done(&self) {}

    fn tool_call(&self, _id: &str, name: &str, _detail: &str) {
        self.tools.lock().unwrap().push(name.to_string());
    }

    fn tool_result(&self, _id: &str, _summary: &str, _is_error: bool) {}

    fn todos(&self, _lines: &[String]) {}

    fn usage(&self, _input: u64, _output: u64) {}

    fn info(&self, _text: &str) {}

    fn warn(&self, text: &str) {
        self.warnings.lock().unwrap().push(text.to_string());
    }
}

struct BenchToolUi;

#[async_trait]
impl ToolUi for BenchToolUi {
    fn diff(&self, _path: &str, _old: &str, _new: &str) {}

    fn is_interactive(&self) -> bool {
        false
    }

    async fn confirm(&self, _title: &str, _body: &str) -> bool {
        true
    }

    async fn ask(&self, _question: &str, _options: &[String]) -> Option<String> {
        None
    }
}

struct Task {
    suite: &'static str,
    name: &'static str,
    prompt: &'static str,
    setup: fn(&Path) -> Result<()>,
    check: fn(&Path, &str, &[String]) -> Result<()>,
}

pub async fn run(
    cfg: &Config,
    provider_name: &str,
    provider_cfg: &ProviderConfig,
    model: &str,
    auth: ProviderAuth,
    args: &BenchArgs,
) -> Result<BenchReport> {
    let suite = args.suite.to_ascii_lowercase();
    if !matches!(suite.as_str(), "smoke" | "tools" | "all") {
        bail!("unknown suite '{}'; use smoke, tools, or all", args.suite);
    }
    let tasks = tasks_for(&suite, &args.filter);
    if tasks.is_empty() {
        bail!("no benchmark tasks matched the requested suite and filters");
    }

    let workspace = benchmark_workspace()?;
    let started = Instant::now();
    let mut results = Vec::new();
    for task in tasks {
        let task_started = Instant::now();
        let result = run_task(
            cfg,
            provider_name,
            provider_cfg,
            model,
            auth.clone(),
            &workspace,
            &task,
        )
        .await;
        let duration_ms = task_started.elapsed().as_millis();
        match result {
            Ok((detail, usage)) => results.push(TaskResult {
                suite: task.suite.to_string(),
                name: task.name.to_string(),
                passed: true,
                duration_ms,
                input_tokens: usage.input_tokens,
                output_tokens: usage.output_tokens,
                detail,
            }),
            Err(error) => results.push(TaskResult {
                suite: task.suite.to_string(),
                name: task.name.to_string(),
                passed: false,
                duration_ms,
                input_tokens: 0,
                output_tokens: 0,
                detail: format!("{error:#}"),
            }),
        }
    }

    let report = BenchReport {
        provider: provider_name.to_string(),
        model: model.to_string(),
        suite,
        passed: results.iter().filter(|task| task.passed).count(),
        total: results.len(),
        duration_ms: started.elapsed().as_millis(),
        input_tokens: results.iter().map(|task| task.input_tokens).sum(),
        output_tokens: results.iter().map(|task| task.output_tokens).sum(),
        workspace: args.keep.then(|| workspace.display().to_string()),
        tasks: results,
    };

    if args.keep {
        eprintln!("benchmark workspace kept at {}", workspace.display());
    } else {
        let _ = std::fs::remove_dir_all(&workspace);
    }
    Ok(report)
}

async fn run_task(
    cfg: &Config,
    provider_name: &str,
    provider_cfg: &ProviderConfig,
    model: &str,
    auth: ProviderAuth,
    workspace: &Path,
    task: &Task,
) -> Result<(String, Usage)> {
    (task.setup)(workspace).with_context(|| format!("preparing task {}", task.name))?;
    let provider = Provider::new(
        provider_name.to_string(),
        provider_cfg.clone(),
        model.to_string(),
        auth,
    )?;
    let skills = crate::skills::SkillSet::discover(workspace, &[]);
    let mcp = crate::mcp::McpManager::connect(&cfg.mcp, workspace).await;
    let ui = Arc::new(BenchUi::default());
    let tools = ToolRegistry::new(
        workspace.to_path_buf(),
        skills.clone(),
        mcp,
        cfg.permissions.clone(),
        true,
        Arc::new(BenchToolUi),
    );
    let system = crate::prompt::build_system_prompt(&crate::prompt::PromptContext {
        core: crate::prompt::DEFAULT_SYSTEM_PROMPT,
        workspace,
        model,
        provider: provider_name,
        skills: &skills,
        extra_instructions: &[],
        shell_permission: cfg.permissions.shell(),
    });
    let mut agent = Agent::new(
        provider,
        model.to_string(),
        crate::prompt::DEFAULT_SYSTEM_PROMPT.to_string(),
        system,
        tools,
        8,
        ui.clone(),
    );
    agent.run_turn(task.prompt).await?;
    let text = ui.text();
    let tools = ui.tools();
    (task.check)(workspace, &text, &tools)?;
    let detail = summary(&text);
    Ok((detail, agent.usage()))
}

fn summary(text: &str) -> String {
    let words = text.split_whitespace().collect::<Vec<_>>();
    let joined = words.join(" ");
    if joined.chars().count() > 70 {
        format!("{}…", joined.chars().take(70).collect::<String>())
    } else if joined.is_empty() {
        "(no text)".to_string()
    } else {
        joined
    }
}

fn tasks_for(suite: &str, filters: &[String]) -> Vec<Task> {
    let all = vec![
        Task {
            suite: "smoke",
            name: "reply",
            prompt: "Reply with exactly HARNESS_OK and nothing else.",
            setup: |_| Ok(()),
            check: |_, text, _| {
                if contains(text, "HARNESS_OK") {
                    Ok(())
                } else {
                    bail!("expected HARNESS_OK in the reply; got {:?}", summary(text))
                }
            },
        },
        Task {
            suite: "tools",
            name: "read",
            prompt: "Use the read tool to read secret.txt, then reply with the secret text and nothing else.",
            setup: |workspace| {
                std::fs::write(workspace.join("secret.txt"), "benchmark-secret-42\n")?;
                Ok(())
            },
            check: |_, text, tools| {
                if !tools.iter().any(|tool| tool == "read") {
                    bail!("the read tool was not called");
                }
                if contains(text, "benchmark-secret-42") {
                    Ok(())
                } else {
                    bail!("expected the secret in the reply; got {:?}", summary(text))
                }
            },
        },
        Task {
            suite: "tools",
            name: "write",
            prompt: "Create a file named answer.txt containing exactly benchmark-write-ok followed by a newline. Use the write tool. Do not explain.",
            setup: |_| Ok(()),
            check: |workspace, _text, tools| {
                if !tools.iter().any(|tool| tool == "write") {
                    bail!("the write tool was not called");
                }
                let text = std::fs::read_to_string(workspace.join("answer.txt"))
                    .context("answer.txt was not created")?;
                if text.trim() == "benchmark-write-ok" {
                    Ok(())
                } else {
                    bail!("answer.txt has unexpected content: {text:?}")
                }
            },
        },
        Task {
            suite: "tools",
            name: "edit",
            prompt: "Use the edit tool to replace beta with delta in edit_me.txt. Then reply DONE.",
            setup: |workspace| {
                std::fs::write(workspace.join("edit_me.txt"), "alpha\nbeta\ngamma\n")?;
                Ok(())
            },
            check: |workspace, _text, tools| {
                if !tools.iter().any(|tool| tool == "edit") {
                    bail!("the edit tool was not called");
                }
                let text = std::fs::read_to_string(workspace.join("edit_me.txt"))
                    .context("edit_me.txt is missing")?;
                if text.contains("delta") && !text.contains("beta") {
                    Ok(())
                } else {
                    bail!("edit_me.txt was not updated correctly: {text:?}")
                }
            },
        },
    ];
    all.into_iter()
        .filter(|task| suite == "all" || task.suite == suite)
        .filter(|task| {
            filters.is_empty()
                || filters
                    .iter()
                    .any(|filter| task.name.contains(&filter.to_ascii_lowercase()))
        })
        .collect()
}

fn contains(text: &str, needle: &str) -> bool {
    text.to_ascii_lowercase()
        .contains(&needle.to_ascii_lowercase())
}

fn benchmark_workspace() -> Result<PathBuf> {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_nanos();
    let path = std::env::temp_dir().join(format!("d3-bench-{}-{stamp}", std::process::id()));
    std::fs::create_dir_all(&path)
        .with_context(|| format!("creating benchmark workspace {}", path.display()))?;
    Ok(path)
}

pub fn print_report(report: &BenchReport, json: bool) -> Result<()> {
    if json {
        println!("{}", serde_json::to_string_pretty(report)?);
        return Ok(());
    }
    println!(
        "harness benchmark · {}/{} · suite {}",
        report.provider, report.model, report.suite
    );
    println!(
        "{:<10} {:<8} {:>8} {:>12}  DETAIL",
        "TASK", "RESULT", "TIME", "TOKENS"
    );
    for task in &report.tasks {
        println!(
            "{:<10} {:<8} {:>7}ms {:>6}/{:<5}  {}",
            task.name,
            if task.passed { "PASS" } else { "FAIL" },
            task.duration_ms,
            task.input_tokens,
            task.output_tokens,
            task.detail
        );
    }
    println!(
        "\n{}/{} passed · {:.1}s · {} input / {} output tokens",
        report.passed,
        report.total,
        report.duration_ms as f64 / 1000.0,
        report.input_tokens,
        report.output_tokens
    );
    if let Some(workspace) = &report.workspace {
        println!("workspace: {workspace}");
    }
    Ok(())
}

pub fn default_model(provider_cfg: &ProviderConfig) -> String {
    provider_cfg
        .model
        .clone()
        .unwrap_or_else(|| default_model_for_kind(provider_cfg.kind))
}

fn default_model_for_kind(kind: Option<ProviderKind>) -> String {
    match kind.unwrap_or_default() {
        ProviderKind::ChatGpt => "gpt-5.6-sol".to_string(),
        ProviderKind::Anthropic => "claude-sonnet-4-5".to_string(),
        _ => "deepseek-v4.1-flash".to_string(),
    }
}
