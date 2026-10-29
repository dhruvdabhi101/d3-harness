use anyhow::{Context, Result, bail};
use globset::{Glob, GlobBuilder, GlobMatcher};
use ignore::WalkBuilder;
use regex::Regex;
use serde_json::Value;
use std::path::Path;

pub fn read(root: &Path, args: &Value) -> Result<(String, String)> {
    let raw = args
        .get("path")
        .and_then(Value::as_str)
        .context("read needs a `path` string")?;
    let path = super::resolve_path_for(root, raw)?;
    if path.is_dir() {
        bail!(
            "{} is a directory; use glob to list files",
            display(root, &path)
        );
    }
    let text = std::fs::read_to_string(&path)
        .with_context(|| format!("reading {}", display(root, &path)))?;
    let lines = text.lines().collect::<Vec<_>>();
    let offset = args
        .get("offset")
        .and_then(Value::as_u64)
        .unwrap_or(1)
        .max(1) as usize;
    let limit = args
        .get("limit")
        .and_then(Value::as_u64)
        .unwrap_or(2000)
        .clamp(1, 10_000) as usize;
    let start = (offset - 1).min(lines.len());
    let end = (start + limit).min(lines.len());
    let mut content = String::new();
    if lines.is_empty() {
        content.push_str("(file is empty)");
    } else if start >= lines.len() {
        content.push_str(&format!(
            "(offset {offset} is past the end; the file has {} lines)",
            lines.len()
        ));
    } else {
        let width = end.to_string().len().max(4);
        for (index, line) in lines[start..end].iter().enumerate() {
            content.push_str(&format!(
                "{:>width$}\t{}\n",
                start + index + 1,
                line,
                width = width
            ));
        }
        if end < lines.len() {
            content.push_str(&format!(
                "… {} more lines (use offset={} to continue)\n",
                lines.len() - end,
                end + 1
            ));
        }
    }
    let content = truncate(&content, 60_000);
    let summary = if lines.is_empty() {
        format!("{} (empty)", display(root, &path))
    } else {
        format!(
            "{} (lines {}-{} of {})",
            display(root, &path),
            start + 1,
            end.max(start),
            lines.len()
        )
    };
    Ok((content, summary))
}

pub fn glob(root: &Path, args: &Value) -> Result<(String, String)> {
    let raw_pattern = args
        .get("pattern")
        .and_then(Value::as_str)
        .context("glob needs a `pattern` string")?;
    let pattern = if raw_pattern.contains('/') {
        raw_pattern.to_string()
    } else {
        format!("**/{raw_pattern}")
    };
    let matcher = GlobBuilder::new(&pattern)
        .literal_separator(true)
        .build()
        .with_context(|| format!("invalid glob pattern '{raw_pattern}'"))?
        .compile_matcher();

    let search_dir = match args.get("path").and_then(Value::as_str) {
        Some(path) => super::resolve_path_for(root, path)?,
        None => root.to_path_buf(),
    };
    if !search_dir.is_dir() {
        bail!("{} is not a directory", display(root, &search_dir));
    }
    let limit = args
        .get("limit")
        .and_then(Value::as_u64)
        .unwrap_or(200)
        .clamp(1, 5000) as usize;

    let mut matches = Vec::new();
    let walker = WalkBuilder::new(&search_dir)
        .hidden(false)
        .git_ignore(true)
        .git_global(true)
        .git_exclude(true)
        .require_git(false)
        .follow_links(false)
        .filter_entry(|entry| entry.file_name() != ".git")
        .build();
    for entry in walker.filter_map(Result::ok) {
        if !entry.file_type().is_some_and(|kind| kind.is_file()) {
            continue;
        }
        let path = entry.path();
        let relative = path.strip_prefix(&search_dir).unwrap_or(path);
        if matcher.is_match(relative) {
            matches.push(display(root, path));
        }
        if matches.len() >= limit {
            break;
        }
    }
    matches.sort();
    matches.dedup();
    if matches.is_empty() {
        return Ok(("No files matched.".to_string(), "no matches".to_string()));
    }
    let content = matches.join("\n");
    let summary = summarise_list(&matches, 20, "file");
    Ok((content, summary))
}

pub fn grep(root: &Path, args: &Value) -> Result<(String, String)> {
    let pattern = args
        .get("pattern")
        .and_then(Value::as_str)
        .context("grep needs a `pattern` string")?;
    let literal = args
        .get("literal")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let case_sensitive = args
        .get("case_sensitive")
        .and_then(Value::as_bool)
        .unwrap_or(true);
    let mut expression = if literal {
        regex::escape(pattern)
    } else {
        pattern.to_string()
    };
    if !case_sensitive {
        expression = format!("(?i){expression}");
    }
    let regex = Regex::new(&expression)
        .with_context(|| format!("invalid regular expression '{pattern}'"))?;

    let search_path = match args.get("path").and_then(Value::as_str) {
        Some(path) => super::resolve_path_for(root, path)?,
        None => root.to_path_buf(),
    };
    let include = args
        .get("include")
        .and_then(Value::as_str)
        .map(compile_include)
        .transpose()?;
    let limit = args
        .get("limit")
        .and_then(Value::as_u64)
        .unwrap_or(100)
        .clamp(1, 2000) as usize;

    let mut matches = Vec::new();
    if search_path.is_file() {
        search_file(
            root,
            &search_path,
            &regex,
            include.as_ref(),
            &mut matches,
            limit,
        );
    } else {
        let walker = WalkBuilder::new(&search_path)
            .hidden(false)
            .git_ignore(true)
            .git_global(true)
            .git_exclude(true)
            .require_git(false)
            .follow_links(false)
            .filter_entry(|entry| entry.file_name() != ".git")
            .build();
        for entry in walker.filter_map(Result::ok) {
            if !entry.file_type().is_some_and(|kind| kind.is_file()) {
                continue;
            }
            search_file(
                root,
                entry.path(),
                &regex,
                include.as_ref(),
                &mut matches,
                limit,
            );
            if matches.len() >= limit {
                break;
            }
        }
    }

    if matches.is_empty() {
        return Ok(("No matches.".to_string(), "no matches".to_string()));
    }
    let mut content = matches.join("\n");
    if matches.len() >= limit {
        content.push_str(&format!("\n… stopped at the {limit}-match limit"));
    }
    let summary = summarise_list(&matches, 20, "match");
    Ok((content, summary))
}

fn search_file(
    root: &Path,
    path: &Path,
    regex: &Regex,
    include: Option<&GlobMatcher>,
    matches: &mut Vec<String>,
    limit: usize,
) {
    if let Some(include) = include {
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("");
        let relative = path.strip_prefix(root).unwrap_or(path);
        if !include.is_match(name) && !include.is_match(relative) {
            return;
        }
    }
    let Ok(metadata) = path.metadata() else {
        return;
    };
    if metadata.len() > 2_000_000 {
        return;
    }
    let Ok(bytes) = std::fs::read(path) else {
        return;
    };
    if bytes.iter().take(1024).any(|byte| *byte == 0) {
        return;
    }
    let text = String::from_utf8_lossy(&bytes);
    for (index, line) in text.lines().enumerate() {
        if regex.is_match(line) {
            matches.push(format!(
                "{}:{}: {}",
                display(root, path),
                index + 1,
                truncate(line.trim_end(), 300)
            ));
            if matches.len() >= limit {
                return;
            }
        }
    }
}

fn compile_include(pattern: &str) -> Result<GlobMatcher> {
    Ok(Glob::new(pattern)
        .with_context(|| format!("invalid include glob '{pattern}'"))?
        .compile_matcher())
}

fn display(root: &Path, path: &Path) -> String {
    let relative = path.strip_prefix(root).unwrap_or(path);
    let text = relative.display().to_string();
    if text.is_empty() {
        ".".to_string()
    } else {
        text
    }
}

fn summarise_list(items: &[String], max: usize, noun: &str) -> String {
    let shown = items
        .iter()
        .take(max)
        .cloned()
        .collect::<Vec<_>>()
        .join("\n");
    let plural = if items.len() == 1 {
        noun.to_string()
    } else if noun.ends_with('h') {
        format!("{noun}es")
    } else {
        format!("{noun}s")
    };
    if items.len() > max {
        format!("{shown}\n… {} more {plural}", items.len() - max)
    } else {
        format!("{} {plural}\n{shown}", items.len())
    }
}

fn truncate(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        text.to_string()
    } else {
        format!("{}…", text.chars().take(max).collect::<String>())
    }
}
