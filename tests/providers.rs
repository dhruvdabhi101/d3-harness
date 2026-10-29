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
use tokio::process::Command;

#[derive(Default)]
struct MockState {
    requests: AtomicUsize,
    bodies: Mutex<Vec<Value>>,
}

async fn spawn_server(app: Router) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    format!("http://{addr}")
}

fn sse(body: String) -> Response {
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "text/event-stream")
        .body(Body::from(body))
        .unwrap()
}

fn write_config(
    project: &std::path::Path,
    base: &str,
    kind: &str,
    model: &str,
) -> std::path::PathBuf {
    let path = project.join("harness.json");
    std::fs::write(
        &path,
        serde_json::to_string_pretty(&json!({
            "provider": "mock",
            "model": model,
            "permissions": {"shell": "allow"},
            "providers": {
                "mock": {
                    "type": kind,
                    "base_url": format!("{base}/v1"),
                    "api_key_env": "MOCK_KEY",
                    "model": model
                }
            }
        }))
        .unwrap(),
    )
    .unwrap();
    path
}

#[tokio::test]
async fn anthropic_messages_stream_and_tool_results() {
    let state = Arc::new(MockState::default());
    let app = Router::new()
        .route("/v1/messages", post(anthropic_handler))
        .with_state(state.clone());
    let base = spawn_server(app).await;

    let project = tempfile::tempdir().unwrap();
    std::fs::write(project.path().join("note.txt"), "anthropic read this\n").unwrap();
    let config = write_config(project.path(), &base, "anthropic", "claude-test");

    let output = Command::new(env!("CARGO_BIN_EXE_harness"))
        .arg("--yes")
        .arg("--config")
        .arg(&config)
        .arg("-C")
        .arg(project.path())
        .arg("--system-prompt")
        .arg("You are a test.")
        .arg("read the note")
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
    assert!(stdout.contains("The note says hello."), "stdout: {stdout}");

    let bodies = state.bodies.lock().unwrap();
    assert_eq!(bodies.len(), 2);
    let system = bodies[0]["system"].as_str().unwrap();
    assert!(system.starts_with("You are a test."), "system: {system}");
    assert!(system.contains("# Environment"));
    assert!(
        bodies[0]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .any(|tool| tool["input_schema"].is_object())
    );
    let second = bodies[1]["messages"].as_array().unwrap();
    let has_result = second.iter().any(|message| {
        message["content"]
            .as_array()
            .is_some_and(|blocks| blocks.iter().any(|block| block["type"] == "tool_result"))
    });
    assert!(has_result, "tool result missing: {second:?}");
}

async fn anthropic_handler(State(state): State<Arc<MockState>>, body: String) -> Response {
    let value: Value = serde_json::from_str(&body).unwrap();
    state.bodies.lock().unwrap().push(value);
    let index = state.requests.fetch_add(1, Ordering::SeqCst);
    if index == 0 {
        sse([
            r#"event: message_start
data: {"type":"message_start","message":{"usage":{"input_tokens":11}}}

"#,
            r#"event: content_block_start
data: {"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"toolu_1","name":"read","input":{}}}

"#,
            r#"event: content_block_delta
data: {"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{\"path\":\"note.txt\"}"}}

"#,
            r#"event: content_block_stop
data: {"type":"content_block_stop","index":0}

"#,
            r#"event: message_delta
data: {"type":"message_delta","usage":{"output_tokens":9}}

event: message_stop
data: {"type":"message_stop"}

"#,
        ]
        .concat())
    } else {
        sse([
            r#"event: message_start
data: {"type":"message_start","message":{"usage":{"input_tokens":20}}}

"#,
            r#"event: content_block_start
data: {"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}

"#,
            r#"event: content_block_delta
data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"The note says hello."}}

"#,
            r#"event: content_block_stop
data: {"type":"content_block_stop","index":0}

"#,
            r#"event: message_delta
data: {"type":"message_delta","usage":{"output_tokens":5}}

event: message_stop
data: {"type":"message_stop"}

"#,
        ]
        .concat())
    }
}

#[tokio::test]
async fn responses_stream_and_send_function_call_output() {
    let state = Arc::new(MockState::default());
    let app = Router::new()
        .route("/v1/responses", post(responses_handler))
        .with_state(state.clone());
    let base = spawn_server(app).await;

    let project = tempfile::tempdir().unwrap();
    let config = write_config(project.path(), &base, "responses", "codex-test");

    let output = Command::new(env!("CARGO_BIN_EXE_harness"))
        .arg("--yes")
        .arg("--config")
        .arg(&config)
        .arg("-C")
        .arg(project.path())
        .arg("write the file")
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
    assert!(
        stdout.contains("Wrote it with Responses."),
        "stdout: {stdout}"
    );
    assert_eq!(
        std::fs::read_to_string(project.path().join("responses.txt")).unwrap(),
        "from responses\n"
    );

    let bodies = state.bodies.lock().unwrap();
    assert_eq!(bodies.len(), 2);
    let tools = bodies[0]["tools"].as_array().unwrap();
    assert!(tools.iter().any(|tool| tool["name"] == "write"));
    assert!(
        bodies[1]["input"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["type"] == "function_call_output" && item["call_id"] == "call_1")
    );
}

async fn responses_handler(State(state): State<Arc<MockState>>, body: String) -> Response {
    let value: Value = serde_json::from_str(&body).unwrap();
    state.bodies.lock().unwrap().push(value);
    let index = state.requests.fetch_add(1, Ordering::SeqCst);
    if index == 0 {
        let arguments = json!({"path": "responses.txt", "content": "from responses\n"}).to_string();
        let added = json!({
            "type": "response.output_item.added",
            "item": {
                "type": "function_call",
                "id": "item_1",
                "call_id": "call_1",
                "name": "write",
                "arguments": ""
            }
        });
        let delta = json!({
            "type": "response.function_call_arguments.delta",
            "item_id": "item_1",
            "delta": arguments
        });
        let completed = json!({
            "type": "response.completed",
            "response": {
                "usage": {"input_tokens": 15, "output_tokens": 8},
                "output": [
                    {
                        "type": "reasoning",
                        "id": "rs_1",
                        "encrypted_content": "opaque-reasoning",
                        "summary": []
                    },
                    {
                        "type": "function_call",
                        "id": "item_1",
                        "call_id": "call_1",
                        "name": "write",
                        "arguments": arguments
                    }
                ]
            }
        });
        sse(format!(
            "data: {added}\n\ndata: {delta}\n\ndata: {completed}\n\n"
        ))
    } else {
        sse([
            r#"data: {"type":"response.output_text.delta","delta":"Wrote it with "}

"#,
            r#"data: {"type":"response.output_text.delta","delta":"Responses."}

"#,
            r#"data: {"type":"response.completed","response":{"usage":{"input_tokens":25,"output_tokens":5}}}

"#,
        ]
        .concat())
    }
}

#[tokio::test]
async fn chatgpt_auth_uses_account_headers_and_responses_wire() {
    let app = Router::new().route("/v1/responses", post(chatgpt_handler));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    let project = tempfile::tempdir().unwrap();
    let config = project.path().join("harness.json");
    std::fs::write(
        &config,
        serde_json::to_string_pretty(&json!({
            "provider": "chatgpt",
            "model": "gpt-test",
            "providers": {
                "chatgpt": {
                    "type": "chatgpt",
                    "base_url": format!("http://{addr}/v1"),
                    "api_key": "test-access-token",
                    "account_id": "acct-test",
                    "model": "gpt-test"
                }
            }
        }))
        .unwrap(),
    )
    .unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_harness"))
        .arg("--config")
        .arg(&config)
        .arg("-C")
        .arg(project.path())
        .arg("-p")
        .arg("chatgpt")
        .arg("say hello")
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
    assert!(stdout.contains("chatgpt mock ok"), "stdout: {stdout}");
}

async fn chatgpt_handler(headers: axum::http::HeaderMap, body: String) -> Response {
    let auth = headers
        .get("authorization")
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();
    let account = headers
        .get("chatgpt-account-id")
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();
    let originator = headers
        .get("originator")
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();
    if auth != "Bearer test-access-token" || account != "acct-test" || originator != "codex_cli_rs"
    {
        return Response::builder()
            .status(StatusCode::UNAUTHORIZED)
            .body(Body::from("bad chatgpt headers"))
            .unwrap();
    }
    let request: Value = serde_json::from_str(&body).unwrap();
    if request["instructions"].as_str().is_none() || request["input"].as_array().is_none() {
        return Response::builder()
            .status(StatusCode::BAD_REQUEST)
            .body(Body::from("not a responses request"))
            .unwrap();
    }
    sse([
        r#"data: {"type":"response.output_text.delta","delta":"chatgpt mock ok"}

"#,
        r#"data: {"type":"response.completed","response":{"usage":{"input_tokens":9,"output_tokens":3}}}

"#,
    ]
    .concat())
}
