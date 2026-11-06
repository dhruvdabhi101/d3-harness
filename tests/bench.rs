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
use std::sync::{Arc, Mutex};
use tokio::process::Command;

#[derive(Default)]
struct MockState {
    bodies: Mutex<Vec<Value>>,
}

#[tokio::test]
async fn bench_runs_smoke_and_tool_tasks_through_the_agent_loop() {
    let state = Arc::new(MockState::default());
    let app = Router::new()
        .route("/v1/chat/completions", post(mock_chat))
        .with_state(state.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    let project = tempfile::tempdir().unwrap();
    let config = project.path().join("harness.json");
    std::fs::write(
        &config,
        serde_json::to_string_pretty(&json!({
            "provider": "mock",
            "model": "mock-model",
            "permissions": {"shell": "allow"},
            "providers": {
                "mock": {
                    "type": "openai",
                    "base_url": format!("http://{address}/v1"),
                    "api_key": "test-key",
                    "model": "mock-model"
                }
            }
        }))
        .unwrap(),
    )
    .unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_d3"))
        .args(["bench", "--suite", "all", "--json", "--config"])
        .arg(&config)
        .arg("-C")
        .arg(project.path())
        .env("XDG_CONFIG_HOME", project.path().join("config-home"))
        .env("XDG_DATA_HOME", project.path().join("data-home"))
        .env_remove("HARNESS_API_KEY")
        .stdin(Stdio::null())
        .output()
        .await
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "stdout:\n{stdout}\nstderr:\n{stderr}"
    );
    let report: Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(report["passed"], 4, "{stdout}");
    assert_eq!(report["total"], 4);
    assert_eq!(report["provider"], "mock");
    for task in report["tasks"].as_array().unwrap() {
        assert_eq!(task["passed"], true, "{task}");
    }
    let names = report["tasks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|task| task["name"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(names, ["reply", "read", "write", "edit"]);
    assert!(state.bodies.lock().unwrap().len() >= 6);
}

async fn mock_chat(State(state): State<Arc<MockState>>, body: String) -> Response {
    let value: Value = serde_json::from_str(&body).unwrap();
    let messages = value["messages"].as_array().cloned().unwrap_or_default();
    let last = messages.last().cloned().unwrap_or_default();
    let role = last["role"].as_str().unwrap_or("");
    let content = last["content"].as_str().unwrap_or("");
    state.bodies.lock().unwrap().push(value);

    if role == "tool" {
        let text = if content.contains("benchmark-secret-42") {
            "benchmark-secret-42"
        } else if content.contains("Wrote") {
            "done"
        } else if content.contains("Edited") {
            "DONE"
        } else {
            "ok"
        };
        return text_response(text);
    }

    if content.contains("HARNESS_OK") {
        return text_response("HARNESS_OK");
    }
    let call = if content.contains("secret.txt") {
        Some(("read", json!({"path": "secret.txt"})))
    } else if content.contains("answer.txt") {
        Some((
            "write",
            json!({"path": "answer.txt", "content": "benchmark-write-ok\n"}),
        ))
    } else if content.contains("edit_me.txt") {
        Some((
            "edit",
            json!({"path": "edit_me.txt", "old_string": "beta", "new_string": "delta"}),
        ))
    } else {
        None
    };
    match call {
        Some((name, arguments)) => tool_response(name, arguments),
        None => text_response("done"),
    }
}

fn text_response(text: &str) -> Response {
    let chunk = json!({
        "choices": [{"index": 0, "delta": {"content": text}, "finish_reason": null}]
    });
    let usage = json!({"choices": [], "usage": {"prompt_tokens": 20, "completion_tokens": 5}});
    sse(format!(
        "data: {chunk}\n\ndata: {usage}\n\ndata: {{\"choices\":[{{\"delta\":{{}},\"finish_reason\":\"stop\"}}]}}\n\ndata: [DONE]\n\n"
    ))
}

fn tool_response(name: &str, arguments: Value) -> Response {
    let chunk = json!({
        "choices": [{
            "index": 0,
            "delta": {
                "tool_calls": [{
                    "index": 0,
                    "id": "call_1",
                    "type": "function",
                    "function": {"name": name, "arguments": arguments.to_string()}
                }]
            },
            "finish_reason": null
        }]
    });
    sse(format!(
        "data: {chunk}\n\ndata: {{\"choices\":[{{\"delta\":{{}},\"finish_reason\":\"tool_calls\"}}]}}\n\ndata: [DONE]\n\n"
    ))
}

fn sse(body: String) -> Response {
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "text/event-stream")
        .body(Body::from(body))
        .unwrap()
}
