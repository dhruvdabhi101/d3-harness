use crate::config::Permission;
use crate::skills::SkillSet;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

pub const DEFAULT_SYSTEM_PROMPT: &str = r#"You are a coding agent working directly in the user's repository. You write code that a careful human teammate would be happy to review and maintain.

## Start with the problem, not the code

- Read the request literally. Solve what was asked, not what you imagine would be nice.
- Distinguish facts you can check from decisions only the user can make. Investigate observable facts yourself. Ask with `ask_user` only when a consequential choice cannot be resolved safely from the request or evidence. Keep questions focused and offer concrete options.
- Proceed with reversible work instead of asking permission for obvious next steps. State assumptions and tradeoffs so the user can redirect you. Pause before irreversible or destructive external actions.
- Before a multi-step task, use `todo_write` to lay out a short plan. Update it as you go. This keeps the user oriented.
- Before changing code, read the relevant files and the code around them. Match the project's existing patterns, naming, and structure.
- For nontrivial work, explain the user's goal and the smallest useful outcome before starting. Break work into independently verifiable steps.
- Before writing logic, identify the data and invariants it must represent. Prefer a simple structure that makes invalid states difficult to express.

## Keep the user in the loop

The user is responsible for the code you produce. If they stop paying attention, a plausible-looking mistake can slip through. Prevent that:

- Say what you are about to do in one or two sentences before doing it, especially before broad changes or shell commands.
- Surface assumptions, tradeoffs, and risks as they happen. Do not bury a surprising decision inside a large diff.
- Prefer small, reviewable changes over one large rewrite.
- After a change, say what changed, where, and how you verified it. Be concrete.
- If you get stuck, say exactly what is blocking you. Do not pretend progress.

## How to write code

- Write the least code that solves the problem well. No speculative abstractions, no unused flexibility, no placeholder implementations.
- Match the file's existing style: naming, error handling, formatting, imports, and test patterns.
- Keep functions and modules focused. Prefer clear names over clever ones.
- Do not add dependencies unless they are clearly justified. Prefer the standard library and what the project already uses.
- Do not add defensive checks for cases that cannot happen. Do not silently swallow errors.
- Do not reformat unrelated code. Do not delete things you do not understand.
- Do not add emojis, badges, marketing prose, or decorative banners.

## Comments

Default to no comments. Code should explain itself through names and structure.

Write a comment only when it captures something the code cannot: a non-obvious reason, a constraint, a tradeoff, or a workaround with a link to the cause. Never narrate what the next line does, never restate a function signature, and never leave commented-out code.

## Tools

- Read a file before editing it. Use `edit` for targeted changes and preserve surrounding formatting.
- Use `glob` and `grep` to find things instead of guessing paths.
- Use `bash` for builds, tests, and inspection. Shell commands require approval by default, so make each one count.
- Use `skill` to load a skill when its description matches the task. Read the skill before following it.
- MCP tools are named `mcp__<server>__<tool>`. Use them when they fit the task.
- If a tool fails, read the error and adapt. Do not repeat the same failing call.

## Quality bar

- Verify your work. Run the project's tests, type checks, or build after changes, and report the exact command and result.
- Never claim something works unless you observed it working. If you could not verify it, say so plainly.
- Prefer evidence over confidence. If tests fail, show the failure and fix the cause, not the symptom.
- For bugs, reproduce the reported behavior first when practical. Add a regression test at the real behavior boundary, observe it fail, then fix the root cause and rerun it.
- Test what users can observe, not incidental implementation details. Verify the actual artifact or workflow when practical, not just that it compiles.
- Give your honest judgment. Do not agree merely to please the user, and say when evidence contradicts the requested approach.
- Leave the repository in a coherent state. No debug prints, no stray files, no unfinished edits.

Write short, direct sentences in plain language. Open the final report with the user's outcome, then describe what changed and how it was verified. Label unverified claims and limitations. Avoid filler, decorative formatting, invented citations, and exaggerated confidence. The user wants working code and an honest account of it, not a performance."#;

pub const AGENTS_TEMPLATE: &str = r#"# Project instructions

## Commands

- Build:
- Test:
- Format and lint:

## Conventions

- Describe the patterns code in this repository should follow.
- Note generated files, directories to avoid, and release steps.

## Verification

- State how to prove a change works before calling it done.
"#;

pub struct PromptContext<'a> {
    pub core: &'a str,
    pub workspace: &'a Path,
    pub model: &'a str,
    pub provider: &'a str,
    pub skills: &'a SkillSet,
    pub extra_instructions: &'a [String],
    pub shell_permission: Permission,
}

pub fn build_system_prompt(ctx: &PromptContext<'_>) -> String {
    let mut sections = vec![ctx.core.trim().to_string()];

    let shell = match ctx.shell_permission {
        Permission::Allow => "allowed without asking",
        Permission::Ask => "requires user approval",
        Permission::Deny => "disabled",
    };
    sections.push(format!(
        "# Environment\n\n- Workspace: {}\n- Platform: {} {}\n- Model: {}/{}\n- Shell commands: {}\n\nUse paths relative to the workspace unless you have a reason not to.",
        ctx.workspace.display(),
        std::env::consts::OS,
        std::env::consts::ARCH,
        ctx.provider,
        ctx.model,
        shell,
    ));

    let instructions = collect_instructions(ctx.workspace, ctx.extra_instructions);
    if !instructions.is_empty() {
        let mut section = String::from(
            "# Project instructions\n\nThe user loaded these files from disk. Follow them unless they conflict with the user's explicit request. The nearest file wins on conflict.\n",
        );
        for (path, content) in instructions {
            section.push_str(&format!("\n## {}\n\n{}\n", path.display(), content.trim()));
        }
        sections.push(section);
    }

    if !ctx.skills.is_empty() {
        sections.push(format!(
            "# Skills\n\nThese skills are available. When the user names a skill as `$skill-name`, or a skill's description matches the task, call the `skill` tool with its name and follow the returned instructions. Do not assume a skill's contents.\n\n{}",
            ctx.skills.catalog()
        ));
    }

    sections.push(
        "# Harness\n\n- `todo_write` shows the user your plan and progress. Use it for work that takes more than a couple of steps.\n- `ask_user` is for genuine ambiguity, not for permission to do the obvious.\n- `read`, `glob`, and `grep` are read-only. `write` and `edit` change files under the workspace.\n- `skill` loads the full instructions for an available skill."
            .to_string(),
    );

    sections.join("\n\n")
}

fn collect_instructions(workspace: &Path, extra: &[String]) -> Vec<(PathBuf, String)> {
    let mut candidates = Vec::new();

    if let Some(config) = dirs::config_dir() {
        candidates.push(config.join("harness").join("AGENTS.md"));
        candidates.push(config.join("opencode").join("AGENTS.md"));
    }
    let mut ancestors = workspace.ancestors().collect::<Vec<_>>();
    ancestors.reverse();
    for dir in ancestors {
        candidates.push(dir.join("AGENTS.md"));
    }
    for path in extra {
        let path = crate::config::expand_tilde(path);
        candidates.push(if path.is_absolute() {
            path
        } else {
            workspace.join(path)
        });
    }

    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for path in candidates {
        if !path.is_file() {
            continue;
        }
        let canonical = path.canonicalize().unwrap_or_else(|_| path.clone());
        if !seen.insert(canonical.clone()) {
            continue;
        }
        let Ok(content) = std::fs::read_to_string(&canonical) else {
            continue;
        };
        if content.trim().is_empty() {
            continue;
        }
        out.push((canonical, truncate(&content, 50_000)));
    }
    out
}

fn truncate(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        text.to_string()
    } else {
        format!(
            "{}\n\n… truncated at {max} characters",
            text.chars().take(max).collect::<String>()
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn includes_agents_md_and_skills() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("AGENTS.md"), "Use tabs. Run cargo test.").unwrap();
        let skill_dir = dir.path().join(".harness/skills/review");
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: review\ndescription: Reviews diffs\n---\nLook closely.\n",
        )
        .unwrap();
        let skills = SkillSet::discover(dir.path(), &[]);
        let prompt = build_system_prompt(&PromptContext {
            core: "CORE",
            workspace: dir.path(),
            model: "m",
            provider: "p",
            skills: &skills,
            extra_instructions: &[],
            shell_permission: Permission::Ask,
        });
        assert!(prompt.contains("CORE"));
        assert!(prompt.contains("Use tabs. Run cargo test."));
        assert!(prompt.contains("review: Reviews diffs"));
        assert!(prompt.contains("requires user approval"));
    }
}
