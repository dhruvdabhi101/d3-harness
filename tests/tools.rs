use oc_harness::config::{McpCommand, McpConfig, McpServerConfig, Permissions};
use oc_harness::conversation::ToolCall;
use oc_harness::mcp::McpManager;
use oc_harness::skills::SkillSet;
use oc_harness::tools::ToolRegistry;
use std::collections::BTreeMap;
use std::path::Path;

fn call(name: &str, arguments: serde_json::Value) -> ToolCall {
    ToolCall {
        id: "call_1".to_string(),
        name: name.to_string(),
        arguments: arguments.to_string(),
    }
}

#[tokio::test]
async fn write_then_edit_changes_only_the_target() {
    let dir = tempfile::tempdir().unwrap();
    let mut tools = ToolRegistry::new(
        dir.path().to_path_buf(),
        SkillSet::default(),
        McpManager::connect(&McpConfig::default(), dir.path()).await,
        Permissions::default(),
        true,
        false,
    );

    let outcome = tools
        .execute(&call(
            "write",
            serde_json::json!({"path": "src/lib.rs", "content": "fn a() {}\nfn b() {}\n"}),
        ))
        .await;
    assert!(!outcome.is_error, "{}", outcome.content);
    assert_eq!(
        std::fs::read_to_string(dir.path().join("src/lib.rs")).unwrap(),
        "fn a() {}\nfn b() {}\n"
    );

    let outcome = tools
        .execute(&call(
            "edit",
            serde_json::json!({
                "path": "src/lib.rs",
                "old_string": "fn b() {}",
                "new_string": "fn b() { println!(\"b\"); }"
            }),
        ))
        .await;
    assert!(!outcome.is_error, "{}", outcome.content);
    assert_eq!(
        std::fs::read_to_string(dir.path().join("src/lib.rs")).unwrap(),
        "fn a() {}\nfn b() { println!(\"b\"); }\n"
    );

    let outcome = tools
        .execute(&call(
            "edit",
            serde_json::json!({
                "path": "src/lib.rs",
                "old_string": "fn",
                "new_string": "fn"
            }),
        ))
        .await;
    assert!(outcome.is_error);
    assert!(outcome.content.contains("appears 2 times"));
}

#[tokio::test]
async fn paths_cannot_escape_the_workspace() {
    let dir = tempfile::tempdir().unwrap();
    let mut tools = ToolRegistry::new(
        dir.path().to_path_buf(),
        SkillSet::default(),
        McpManager::connect(&McpConfig::default(), dir.path()).await,
        Permissions::default(),
        true,
        false,
    );

    let outcome = tools
        .execute(&call("read", serde_json::json!({"path": "/etc/passwd"})))
        .await;
    assert!(outcome.is_error);
    assert!(outcome.content.contains("outside the workspace"));

    let outcome = tools
        .execute(&call(
            "write",
            serde_json::json!({"path": "../escape.txt", "content": "nope"}),
        ))
        .await;
    assert!(outcome.is_error);
    assert!(!dir.path().parent().unwrap().join("escape.txt").exists());
}

#[tokio::test]
async fn glob_and_grep_respect_the_workspace() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    std::fs::write(
        dir.path().join("src/main.rs"),
        "fn main() {\n    println!(\"needle\");\n}\n",
    )
    .unwrap();
    std::fs::write(dir.path().join("README.md"), "needle in docs\n").unwrap();
    std::fs::write(dir.path().join("root.rs"), "fn root() {}\n").unwrap();
    let mut tools = ToolRegistry::new(
        dir.path().to_path_buf(),
        SkillSet::default(),
        McpManager::connect(&McpConfig::default(), dir.path()).await,
        Permissions::default(),
        true,
        false,
    );

    let outcome = tools
        .execute(&call("glob", serde_json::json!({"pattern": "*.rs"})))
        .await;
    assert!(!outcome.is_error, "{}", outcome.content);
    assert!(outcome.content.contains("src/main.rs"));
    assert!(outcome.content.contains("root.rs"));

    let outcome = tools
        .execute(&call(
            "grep",
            serde_json::json!({"pattern": "needle", "include": "*.rs"}),
        ))
        .await;
    assert!(!outcome.is_error, "{}", outcome.content);
    assert!(outcome.content.contains("src/main.rs:2"));
    assert!(!outcome.content.contains("README.md"));
}

#[tokio::test]
async fn mcp_stdio_tools_are_listed_and_called() {
    let dir = tempfile::tempdir().unwrap();
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/mcp_server.py");
    let config = McpConfig {
        servers: BTreeMap::from([(
            "fixture".to_string(),
            McpServerConfig {
                command: Some(McpCommand::One("python3".to_string())),
                args: Some(vec![script.display().to_string()]),
                ..Default::default()
            },
        )]),
    };
    let mut manager = McpManager::connect(&config, dir.path()).await;
    let tools = manager.tools();
    assert_eq!(tools.len(), 1, "tools: {tools:?}");
    assert_eq!(tools[0].exposed_name, "mcp__fixture__echo");
    assert!(tools[0].description.contains("Echo"));

    let (content, is_error) = manager
        .call("fixture", "echo", serde_json::json!({"text": "hello"}))
        .await
        .unwrap();
    assert!(!is_error);
    assert_eq!(content, "echo:hello");
}

#[tokio::test]
async fn skills_load_and_todos_are_visible() {
    let dir = tempfile::tempdir().unwrap();
    let skill_dir = dir.path().join(".harness/skills/review");
    std::fs::create_dir_all(&skill_dir).unwrap();
    std::fs::write(
        skill_dir.join("SKILL.md"),
        "---\nname: review\ndescription: Reviews diffs\n---\nCheck every hunk.\n",
    )
    .unwrap();
    let skills = SkillSet::discover(dir.path(), &[]);
    let mut tools = ToolRegistry::new(
        dir.path().to_path_buf(),
        skills,
        McpManager::connect(&McpConfig::default(), dir.path()).await,
        Permissions::default(),
        true,
        false,
    );

    let outcome = tools
        .execute(&call("skill", serde_json::json!({"name": "review"})))
        .await;
    assert!(!outcome.is_error, "{}", outcome.content);
    assert!(outcome.content.contains("Check every hunk."));

    let outcome = tools
        .execute(&call(
            "todo_write",
            serde_json::json!({"todos": [
                {"content": "read code", "status": "completed"},
                {"content": "write fix", "status": "in_progress"}
            ]}),
        ))
        .await;
    assert!(!outcome.is_error, "{}", outcome.content);
    assert!(outcome.summary.contains("[x] read code"));
    assert!(outcome.summary.contains("[~] write fix"));
}

#[tokio::test]
async fn shell_runs_in_the_workspace() {
    let dir = tempfile::tempdir().unwrap();
    let mut tools = ToolRegistry::new(
        dir.path().to_path_buf(),
        SkillSet::default(),
        McpManager::connect(&McpConfig::default(), dir.path()).await,
        Permissions {
            shell: Some(oc_harness::config::Permission::Allow),
            ..Default::default()
        },
        true,
        false,
    );
    let outcome = tools
        .execute(&call(
            "bash",
            serde_json::json!({"command": "pwd && printf 'ok\\n'"}),
        ))
        .await;
    assert!(!outcome.is_error, "{}", outcome.content);
    assert!(outcome.content.contains("ok"));
    assert!(outcome.content.contains(&dir.path().display().to_string()));
}
