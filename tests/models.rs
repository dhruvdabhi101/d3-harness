use axum::{
    Router,
    extract::State,
    http::{HeaderMap, StatusCode, header},
    response::Response,
    routing::get,
};
use std::process::Stdio;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use tokio::process::Command;

#[derive(Default)]
struct MockState {
    calls: AtomicUsize,
}

#[tokio::test]
async fn list_models_fetches_and_prints_the_provider_catalog() {
    let state = Arc::new(MockState::default());
    let app = Router::new()
        .route("/v1/models", get(models))
        .with_state(state.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    let project = tempfile::tempdir().unwrap();
    let config = project.path().join("harness.json");
    std::fs::write(
        &config,
        serde_json::to_string_pretty(&serde_json::json!({
            "provider": "mock",
            "model": "a-model",
            "providers": {
                "mock": {
                    "type": "openai",
                    "base_url": format!("http://{addr}/v1"),
                    "api_key_env": "MOCK_KEY",
                    "model": "a-model"
                }
            }
        }))
        .unwrap(),
    )
    .unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_harness"))
        .arg("--list-models")
        .arg("--config")
        .arg(&config)
        .arg("-C")
        .arg(project.path())
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
    assert!(stdout.contains("z-model"), "stdout: {stdout}");
    assert!(stdout.contains("A Model"), "stdout: {stdout}");
    assert!(
        stdout.find("a-model").unwrap() < stdout.find("z-model").unwrap(),
        "models should be sorted: {stdout}"
    );
    assert_eq!(state.calls.load(Ordering::SeqCst), 1);
}

async fn models(State(state): State<Arc<MockState>>, headers: HeaderMap) -> Response {
    state.calls.fetch_add(1, Ordering::SeqCst);
    assert_eq!(
        headers
            .get("authorization")
            .and_then(|value| value.to_str().ok()),
        Some("Bearer test-key")
    );
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/json")
        .body(axum::body::Body::from(
            serde_json::json!({
                "object": "list",
                "data": [
                    {"id": "z-model"},
                    {"id": "a-model", "name": "A Model"}
                ]
            })
            .to_string(),
        ))
        .unwrap()
}
