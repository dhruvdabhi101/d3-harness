use anyhow::{Context, Result};
use std::path::Path;
use std::process::Stdio;
use std::time::Duration;
use tokio::process::Command;

pub struct ShellOutput {
    pub content: String,
}

pub async fn run(command: &str, cwd: &Path, timeout_secs: u64) -> Result<ShellOutput> {
    let mut cmd = Command::new("sh");
    cmd.arg("-c")
        .arg(command)
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);

    let child = cmd
        .spawn()
        .with_context(|| format!("spawning shell in {}", cwd.display()))?;

    let output =
        match tokio::time::timeout(Duration::from_secs(timeout_secs), child.wait_with_output())
            .await
        {
            Ok(result) => result.context("waiting for shell command")?,
            Err(_) => {
                return Ok(ShellOutput {
                    content: format!("Command timed out after {timeout_secs}s and was killed."),
                });
            }
        };

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let mut content = String::new();
    if !stdout.trim().is_empty() {
        content.push_str(stdout.trim_end());
        content.push('\n');
    }
    if !stderr.trim().is_empty() {
        if !content.is_empty() {
            content.push('\n');
        }
        content.push_str("stderr:\n");
        content.push_str(stderr.trim_end());
        content.push('\n');
    }
    if !output.status.success() {
        content.push_str(&format!(
            "\nexit code: {}",
            output.status.code().unwrap_or(-1)
        ));
    }
    if content.trim().is_empty() {
        content.push_str("(no output)");
    }
    if output.status.success() && !content.contains("exit code:") {
        content.push_str("\nexit code: 0");
    }
    Ok(ShellOutput {
        content: truncate(&content, 30_000),
    })
}

fn truncate(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        text.to_string()
    } else {
        format!(
            "{}\n… output truncated at {max} characters",
            text.chars().take(max).collect::<String>()
        )
    }
}
