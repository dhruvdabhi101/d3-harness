use axum::{
    Router,
    body::Body,
    extract::State,
    http::{StatusCode, header},
    response::Response,
    routing::post,
};
use serde_json::{Value, json};
use std::process::Stdio;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};
use std::time::Duration;
use tokio::process::Command;

#[derive(Default)]
struct MockState {
    requests: AtomicUsize,
    bodies: Mutex<Vec<Value>>,
}

#[tokio::test]
async fn agent_loop_writes_file_and_keeps_context() {
    let state = Arc::new(MockState::default());
    let app = Router::new()
        .route("/v1/chat/completions", post(mock_chat))
        .with_state(state.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    let project = tempfile::tempdir().unwrap();
    std::fs::write(
        project.path().join("AGENTS.md"),
        "PROJECT RULE: use tabs and run the tests.\n",
    )
    .unwrap();
    let skill = project.path().join(".harness/skills/review");
    std::fs::create_dir_all(&skill).unwrap();
    std::fs::write(
        skill.join("SKILL.md"),
        "---\nname: review\ndescription: Reviews code for platypus bugs\n---\nLook for platypus bugs.\n",
    )
    .unwrap();

    let config_path = project.path().join("harness.json");
    std::fs::write(
        &config_path,
        serde_json::to_string_pretty(&json!({
            "provider": "mock",
            "model": "mock-1",
            "permissions": {"shell": "allow"},
            "providers": {
                "mock": {
                    "type": "openai",
                    "base_url": format!("http://{addr}/v1"),
                    "api_key_env": "MOCK_KEY",
                    "model": "mock-1"
                }
            }
        }))
        .unwrap(),
    )
    .unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_harness"))
        .arg("--yes")
        .arg("--config")
        .arg(&config_path)
        .arg("-C")
        .arg(project.path())
        .arg("create hello.txt")
        .env("MOCK_KEY", "test-key")
        .env_remove("HARNESS_API_KEY")
        .stdin(Stdio::null())
        .output()
        .await
        .unwrap();

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "harness failed\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
    assert!(stdout.contains("Created hello.txt."), "stdout: {stdout}");
    assert_eq!(
        std::fs::read_to_string(project.path().join("hello.txt")).unwrap(),
        "hello from mock\n"
    );

    let bodies = state.bodies.lock().unwrap();
    assert_eq!(bodies.len(), 2, "expected two model turns");
    let system = bodies[0]["messages"][0]["content"].as_str().unwrap();
    assert!(system.contains("PROJECT RULE: use tabs"));
    assert!(system.contains("Reviews code for platypus bugs"));
    assert!(
        bodies[0]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .any(|tool| tool["function"]["name"] == "write")
    );
    assert!(
        bodies[0]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .any(|tool| tool["function"]["name"] == "skill")
    );
    let second_messages = bodies[1]["messages"].as_array().unwrap();
    assert!(
        second_messages
            .iter()
            .any(|message| message["role"] == "tool"
                && message["content"].as_str().unwrap().contains("hello.txt")),
        "tool result was not sent back: {second_messages:?}"
    );
}

async fn mock_chat(State(state): State<Arc<MockState>>, body: String) -> Response {
    let value: Value = serde_json::from_str(&body).unwrap();
    state.bodies.lock().unwrap().push(value);
    let index = state.requests.fetch_add(1, Ordering::SeqCst);

    let sse = if index == 0 {
        let arguments = json!({
            "path": "hello.txt",
            "content": "hello from mock\n"
        })
        .to_string();
        let tool_chunk = json!({
            "choices": [{
                "index": 0,
                "delta": {
                    "tool_calls": [{
                        "index": 0,
                        "id": "call_1",
                        "type": "function",
                        "function": {
                            "name": "write",
                            "arguments": arguments
                        }
                    }]
                },
                "finish_reason": null
            }]
        });
        format!(
            "data: {tool_chunk}\n\ndata: {{\"choices\":[{{\"delta\":{{}},\"finish_reason\":\"tool_calls\"}}]}}\n\ndata: [DONE]\n\n"
        )
    } else {
        let text_chunk = json!({
            "choices": [{
                "index": 0,
                "delta": {"content": "Created hello.txt."},
                "finish_reason": null
            }]
        });
        let usage_chunk = json!({
            "choices": [],
            "usage": {"prompt_tokens": 21, "completion_tokens": 5}
        });
        format!(
            "data: {text_chunk}\n\ndata: {usage_chunk}\n\ndata: {{\"choices\":[{{\"delta\":{{}},\"finish_reason\":\"stop\"}}]}}\n\ndata: [DONE]\n\n"
        )
    };

    tokio::time::sleep(Duration::from_millis(5)).await;
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "text/event-stream")
        .body(Body::from(sse))
        .unwrap()
}
