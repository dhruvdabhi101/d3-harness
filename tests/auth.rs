use serde_json::{Value, json};
use std::process::{Command, Stdio};

fn write_config(dir: &std::path::Path, with_key: bool) -> std::path::PathBuf {
    let path = dir.join("harness.json");
    let mut provider = json!({
        "type": "openai",
        "base_url": "https://example.invalid/v1",
        "model": "mock-model"
    });
    if with_key {
        provider["api_key"] = json!("config-key");
    }
    std::fs::write(
        &path,
        serde_json::to_string_pretty(&json!({
            "provider": "mock",
            "model": "mock-model",
            "providers": {"mock": provider}
        }))
        .unwrap(),
    )
    .unwrap();
    path
}

fn run(args: &[&str], config: &std::path::Path, dir: &std::path::Path) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_d3"))
        .args(args)
        .arg("--config")
        .arg(config)
        .arg("-C")
        .arg(dir)
        .env("XDG_CONFIG_HOME", dir.join("config-home"))
        .env("XDG_DATA_HOME", dir.join("data-home"))
        .env_remove("HARNESS_API_KEY")
        .stdin(Stdio::null())
        .output()
        .unwrap()
}

#[test]
fn auth_check_reports_config_key_and_json() {
    let dir = tempfile::tempdir().unwrap();
    let config = write_config(dir.path(), true);
    let output = run(&["auth", "check", "mock", "--json"], &config, dir.path());
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value[0]["provider"], "mock");
    assert_eq!(value[0]["ready"], true);
    assert_eq!(value[0]["source"], "harness.json api_key");

    let missing = run(
        &["auth", "check", "mock"],
        &write_config(dir.path(), false),
        dir.path(),
    );
    assert!(!missing.status.success());
    let stderr = String::from_utf8_lossy(&missing.stderr);
    assert!(stderr.contains("not signed in"), "{stderr}");
}

#[test]
fn auth_logout_removes_only_the_stored_credential() {
    let dir = tempfile::tempdir().unwrap();
    let config = write_config(dir.path(), false);
    let auth_dir = dir.path().join("config-home").join("harness");
    std::fs::create_dir_all(&auth_dir).unwrap();
    let auth_path = auth_dir.join("auth.json");
    std::fs::write(
        &auth_path,
        serde_json::to_string_pretty(&json!({
            "mock": {"type": "api", "key": "stored-key"},
            "other": {"type": "api", "key": "other-key"}
        }))
        .unwrap(),
    )
    .unwrap();

    let check = run(&["auth", "check", "mock"], &config, dir.path());
    assert!(check.status.success());
    assert!(
        String::from_utf8_lossy(&check.stdout).contains("harness auth.json"),
        "{}",
        String::from_utf8_lossy(&check.stdout)
    );

    let logout = run(&["auth", "logout", "mock"], &config, dir.path());
    assert!(
        logout.status.success(),
        "{}",
        String::from_utf8_lossy(&logout.stderr)
    );
    let remaining: Value =
        serde_json::from_str(&std::fs::read_to_string(&auth_path).unwrap()).unwrap();
    assert!(remaining.get("mock").is_none());
    assert_eq!(remaining["other"]["key"], "other-key");
}

#[test]
fn auth_login_noninteractive_without_a_key_fails_clearly() {
    let dir = tempfile::tempdir().unwrap();
    let config = write_config(dir.path(), false);
    let output = run(&["auth", "login", "mock"], &config, dir.path());
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("missing API key"), "{stderr}");
    assert!(!stderr.contains("config-key"), "{stderr}");
}
