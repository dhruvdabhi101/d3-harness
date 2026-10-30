use std::collections::HashSet;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct Skill {
    pub name: String,
    pub description: String,
    pub path: PathBuf,
    pub body: String,
}

#[derive(Debug, Clone, Default)]
pub struct SkillSet {
    skills: Vec<Skill>,
}

impl SkillSet {
    pub fn discover(workspace: &Path, extra_paths: &[String]) -> Self {
        Self::discover_with_home(workspace, extra_paths, dirs::home_dir().as_deref())
    }

    fn discover_with_home(workspace: &Path, extra_paths: &[String], home: Option<&Path>) -> Self {
        let mut roots = vec![
            workspace.join(".harness").join("skills"),
            workspace.join(".agents").join("skills"),
            workspace.join(".opencode").join("skills"),
            workspace.join(".opencode").join("skill"),
            workspace.join(".claude").join("skills"),
        ];
        if let Some(home) = home {
            roots.push(home.join(".agents").join("skills"));
            roots.push(home.join(".agent").join("skills"));
            roots.push(home.join(".config").join("harness").join("skills"));
            roots.push(home.join(".config").join("opencode").join("skills"));
            roots.push(home.join(".claude").join("skills"));
        }
        for extra in extra_paths {
            let path = crate::config::expand_tilde(extra);
            roots.push(if path.is_absolute() {
                path
            } else {
                workspace.join(path)
            });
        }

        let mut skills = Vec::new();
        let mut seen = HashSet::new();
        for root in roots {
            for path in skill_files(&root) {
                if let Some(skill) = Skill::from_file(&path)
                    && seen.insert(skill.name.clone())
                {
                    skills.push(skill);
                }
            }
        }
        SkillSet { skills }
    }

    pub fn is_empty(&self) -> bool {
        self.skills.is_empty()
    }

    pub fn get(&self, name: &str) -> Option<&Skill> {
        self.skills.iter().find(|skill| skill.name == name)
    }

    pub fn names(&self) -> Vec<String> {
        self.skills.iter().map(|skill| skill.name.clone()).collect()
    }

    pub fn catalog(&self) -> String {
        self.skills
            .iter()
            .map(|skill| format!("- {}: {}", skill.name, skill.description))
            .collect::<Vec<_>>()
            .join("\n")
    }

    pub fn summaries(&self) -> Vec<serde_json::Value> {
        self.skills
            .iter()
            .map(|skill| {
                serde_json::json!({
                    "name": skill.name,
                    "description": skill.description,
                    "path": skill.path,
                })
            })
            .collect()
    }

    pub fn iter(&self) -> impl Iterator<Item = &Skill> {
        self.skills.iter()
    }
}

fn skill_files(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if root.join("SKILL.md").is_file() {
        out.push(root.join("SKILL.md"));
    }
    let Ok(entries) = std::fs::read_dir(root) else {
        return out;
    };
    let mut dirs = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .collect::<Vec<_>>();
    dirs.sort();
    for dir in dirs {
        if dir.is_dir() && dir.join("SKILL.md").is_file() {
            out.push(dir.join("SKILL.md"));
        }
    }
    out
}

impl Skill {
    fn from_file(path: &Path) -> Option<Self> {
        let text = std::fs::read_to_string(path).ok()?;
        let (front, body) = split_frontmatter(&text);
        let fallback_name = path
            .parent()
            .and_then(|dir| dir.file_name())
            .and_then(|name| name.to_str())
            .unwrap_or("skill")
            .to_string();
        let meta = parse_frontmatter(&front);
        let name = meta
            .name
            .filter(|name| !name.trim().is_empty())
            .unwrap_or(fallback_name);
        let description = meta
            .description
            .filter(|desc| !desc.trim().is_empty())
            .unwrap_or_else(|| first_line(&body));
        Some(Self {
            name,
            description: truncate(&description, 300),
            path: path.to_path_buf(),
            body,
        })
    }
}

struct Meta {
    name: Option<String>,
    description: Option<String>,
}

fn split_frontmatter(text: &str) -> (String, String) {
    let text = text.replace("\r\n", "\n");
    if !text.starts_with("---") {
        return (String::new(), text);
    }
    let rest = &text[3..];
    let rest = rest.strip_prefix('\n').unwrap_or(rest);
    if let Some(end) = rest.find("\n---") {
        let front = rest[..end].to_string();
        let mut body = &rest[end + 4..];
        body = body.strip_prefix('\n').unwrap_or(body);
        (front, body.to_string())
    } else {
        (String::new(), text)
    }
}

fn parse_frontmatter(front: &str) -> Meta {
    let mut meta = Meta {
        name: None,
        description: None,
    };
    let lines: Vec<&str> = front.lines().collect();
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        let Some((key, value)) = line.split_once(':') else {
            i += 1;
            continue;
        };
        let key = key.trim();
        let value = value.trim();
        if key != "name" && key != "description" {
            i += 1;
            continue;
        }
        let parsed = if value == ">" || value == "|" || value == ">-" || value == "|-" {
            let fold = value.starts_with('>');
            let mut parts = Vec::new();
            i += 1;
            while i < lines.len() {
                let next = lines[i];
                if !next.is_empty() && !next.starts_with(' ') && !next.starts_with('\t') {
                    break;
                }
                parts.push(next.trim().to_string());
                i += 1;
            }
            let joined = if fold {
                parts.join(" ")
            } else {
                parts.join("\n")
            };
            joined.trim().to_string()
        } else {
            i += 1;
            strip_quotes(value).to_string()
        };
        if key == "name" {
            meta.name = Some(parsed);
        } else {
            meta.description = Some(parsed);
        }
    }
    meta
}

fn strip_quotes(value: &str) -> &str {
    if value.len() >= 2 {
        let bytes = value.as_bytes();
        if (bytes[0] == b'"' && bytes[value.len() - 1] == b'"')
            || (bytes[0] == b'\'' && bytes[value.len() - 1] == b'\'')
        {
            return &value[1..value.len() - 1];
        }
    }
    value
}

fn first_line(body: &str) -> String {
    body.lines()
        .map(str::trim)
        .find(|line| !line.is_empty() && !line.starts_with('#'))
        .unwrap_or("No description")
        .to_string()
}

fn truncate(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        text.to_string()
    } else {
        format!("{}…", text.chars().take(max).collect::<String>())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_frontmatter() {
        let meta = parse_frontmatter(
            "name: review\ncolor: blue\ndescription: >\n  Checks code\n  for bugs.\n",
        );
        assert_eq!(meta.name.as_deref(), Some("review"));
        assert_eq!(meta.description.as_deref(), Some("Checks code for bugs."));
    }

    #[test]
    fn splits_body() {
        let (front, body) = split_frontmatter("---\nname: x\n---\n# Body\n");
        assert!(front.contains("name: x"));
        assert_eq!(body, "# Body\n");
    }

    #[test]
    fn discovers_agent_home_skills() {
        let workspace = tempfile::tempdir().unwrap();
        let home = tempfile::tempdir().unwrap();
        let skill = home.path().join(".agent/skills/new-skill");
        std::fs::create_dir_all(&skill).unwrap();
        std::fs::write(
            skill.join("SKILL.md"),
            "---\nname: new-skill\n---\nInstructions",
        )
        .unwrap();
        let set = SkillSet::discover_with_home(workspace.path(), &[], Some(home.path()));
        assert!(set.get("new-skill").is_some());
    }

    #[test]
    fn discovers_agents_skills_in_workspace_and_home() {
        let workspace = tempfile::tempdir().unwrap();
        let home = tempfile::tempdir().unwrap();
        for (root, name) in [(workspace.path(), "project"), (home.path(), "global")] {
            let dir = root.join(".agents/skills").join(name);
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(
                dir.join("SKILL.md"),
                format!("---\nname: {name}\n---\nInstructions"),
            )
            .unwrap();
        }
        let set = SkillSet::discover_with_home(workspace.path(), &[], Some(home.path()));
        assert!(set.get("project").is_some());
        assert!(set.get("global").is_some());
    }

    #[test]
    fn discovers_project_skills() {
        let dir = tempfile::tempdir().unwrap();
        let skill = dir.path().join(".harness/skills/review");
        std::fs::create_dir_all(&skill).unwrap();
        std::fs::write(
            skill.join("SKILL.md"),
            "---\nname: review\ndescription: Review code\n---\nDo the review.\n",
        )
        .unwrap();
        let set = SkillSet::discover(dir.path(), &[]);
        assert_eq!(set.names().first().map(String::as_str), Some("review"));
        assert!(set.get("review").unwrap().body.contains("Do the review."));
    }
}
