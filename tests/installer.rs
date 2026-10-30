use std::fs;
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::process::{Command, Stdio};

#[test]
fn piped_installer_reports_download_and_bounds_network_waits() {
    let dir = tempfile::tempdir().unwrap();
    let bin = dir.path().join("bin");
    fs::create_dir(&bin).unwrap();
    let curl = bin.join("curl");
    fs::write(
        &curl,
        "#!/bin/sh\nprintf '%s\\n' \"$*\" > \"$CURL_ARGS\"\nexit 28\n",
    )
    .unwrap();
    fs::set_permissions(&curl, fs::Permissions::from_mode(0o755)).unwrap();
    let args = dir.path().join("curl-args");
    let mut child = Command::new("sh")
        .env(
            "PATH",
            format!("{}:{}", bin.display(), std::env::var("PATH").unwrap()),
        )
        .env("CURL_ARGS", &args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(include_bytes!("../install.sh"))
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("Downloading"));
    let arguments = fs::read_to_string(args).unwrap();
    assert!(arguments.contains("--connect-timeout"));
    assert!(arguments.contains("--max-time"));
}
