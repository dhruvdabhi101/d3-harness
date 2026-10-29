use similar::TextDiff;
use std::io::{self, IsTerminal, Write};
use std::sync::atomic::{AtomicBool, Ordering};

static COLOR: AtomicBool = AtomicBool::new(false);

pub fn init(no_color: bool) {
    let enabled = !no_color
        && std::env::var_os("NO_COLOR").is_none()
        && io::stderr().is_terminal()
        && io::stdout().is_terminal();
    COLOR.store(enabled, Ordering::Relaxed);
}

fn enabled() -> bool {
    COLOR.load(Ordering::Relaxed)
}

fn paint(code: &str, text: &str) -> String {
    if enabled() {
        format!("\x1b[{code}m{text}\x1b[0m")
    } else {
        text.to_string()
    }
}

pub fn bold(text: &str) -> String {
    paint("1", text)
}

pub fn dim(text: &str) -> String {
    paint("2", text)
}

pub fn red(text: &str) -> String {
    paint("31", text)
}

pub fn green(text: &str) -> String {
    paint("32", text)
}

pub fn yellow(text: &str) -> String {
    paint("33", text)
}

pub fn cyan(text: &str) -> String {
    paint("36", text)
}

pub fn magenta(text: &str) -> String {
    paint("35", text)
}

pub fn banner(provider: &str, model: &str, workspace: &std::path::Path) {
    eprintln!(
        "{} {}  {}",
        bold("harness"),
        dim(env!("CARGO_PKG_VERSION")),
        dim("small, honest coding agent")
    );
    eprintln!("  {} {}  ·  {}", dim("provider"), provider, model);
    eprintln!("  {} {}", dim("workspace"), workspace.display());
    eprintln!("  {}", dim("type /help for commands, /exit to leave"));
    eprintln!();
}

pub fn tool_call(name: &str, detail: &str) {
    if detail.is_empty() {
        eprintln!("{} {}", cyan("→"), bold(name));
    } else {
        eprintln!("{} {} {}", cyan("→"), bold(name), dim(detail));
    }
}

pub fn tool_done(summary: &str) {
    if summary.is_empty() {
        return;
    }
    for line in summary.lines() {
        eprintln!("  {} {}", dim("↳"), line);
    }
}

pub fn warn(text: &str) {
    eprintln!("{} {}", yellow("!"), text);
}

pub fn error(text: &str) {
    eprintln!("{} {}", red("error:"), text);
}

pub fn info(text: &str) {
    eprintln!("{}", dim(text));
}

pub fn print_usage(input: u64, output: u64) {
    if input == 0 && output == 0 {
        return;
    }
    eprintln!("{}", dim(&format!("  {input} in / {output} out tokens")));
}

pub fn print_diff(path: &str, old: &str, new: &str) {
    let diff = TextDiff::from_lines(old, new);
    let rendered = diff
        .unified_diff()
        .context_radius(3)
        .header(&format!("a/{path}"), &format!("b/{path}"))
        .to_string();
    let mut lines = rendered.lines().peekable();
    let mut shown = 0;
    while let Some(line) = lines.next() {
        if shown >= 160 {
            let remaining = lines.count();
            eprintln!("{}", dim(&format!("  … {remaining} more diff lines")));
            break;
        }
        let painted = if line.starts_with('+') && !line.starts_with("+++") {
            green(line)
        } else if line.starts_with('-') && !line.starts_with("---") {
            red(line)
        } else if line.starts_with("@@") {
            cyan(line)
        } else {
            dim(line)
        };
        eprintln!("{painted}");
        shown += 1;
    }
}

pub fn confirm(question: &str) -> bool {
    eprint!("{} {} ", yellow("?"), question);
    eprint!("{} ", dim("[y/N]"));
    let _ = io::stderr().flush();
    let mut answer = String::new();
    if io::stdin().read_line(&mut answer).is_err() {
        return false;
    }
    matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes")
}

pub fn read_line(prompt: &str) -> io::Result<Option<String>> {
    eprint!("{prompt}");
    io::stderr().flush()?;
    let mut line = String::new();
    let read = io::stdin().read_line(&mut line)?;
    if read == 0 {
        return Ok(None);
    }
    Ok(Some(line))
}
