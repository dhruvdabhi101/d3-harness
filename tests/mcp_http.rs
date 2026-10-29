use axum::{
    Router,
    body::Body,
    extract::State,
    http::{HeaderMap, StatusCode, header},
    response::Response,
    routing::post,
};
use oc_harness::config::{McpConfig, McpServerConfig};
use oc_harness::mcp::McpManager;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

#[derive(Default)]
struct MockState {
    requests: AtomicUsize,
}

#[tokio::test]
async fn http_mcp_initializes_sessions_and_handles_sse() {
    let state = Arc::new(MockState::default());
    let app = Router::new()
        .route("/mcp", post(handler))
        .with_state(state.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    let config = McpConfig {
        servers: BTreeMap::from([(
            "remote".to_string(),
            McpServerConfig {
                kind: Some("http".to_string()),
                url: Some(format!("http://{addr}/mcp")),
                headers: Some(BTreeMap::from([(
                    "authorization".to_string(),
                    "Bearer test".to_string(),
                )])),
                ..Default::default()
            },
        )]),
    };
    let dir = tempfile::tempdir().unwrap();
    let mut manager = McpManager::connect(&config, dir.path()).await;
    let tools = manager.tools();
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0].exposed_name, "mcp__remote__echo");
    let (content, is_error) = manager
        .call("remote", "echo", json!({"text": "over http"}))
        .await
        .unwrap();
    assert!(!is_error);
    assert_eq!(content, "http:over http");
    assert_eq!(state.requests.load(Ordering::SeqCst), 4);
}

async fn handler(
    State(state): State<Arc<MockState>>,
    headers: HeaderMap,
    body: String,
) -> Response {
    state.requests.fetch_add(1, Ordering::SeqCst);
    assert_eq!(
        headers
            .get("authorization")
            .and_then(|value| value.to_str().ok()),
        Some("Bearer test")
    );
    let request: Value = serde_json::from_str(&body).unwrap();
    let id = request.get("id").cloned().unwrap_or(Value::Null);
    match request.get("method").and_then(Value::as_str).unwrap_or("") {
        "initialize" => Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, "application/json")
            .header("mcp-session-id", "sess-1")
            .body(Body::from(
                json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": {
                        "protocolVersion": "2025-06-18",
                        "capabilities": {"tools": {}},
                        "serverInfo": {"name": "http-fixture", "version": "1"}
                    }
                })
                .to_string(),
            ))
            .unwrap(),
        "notifications/initialized" => Response::builder()
            .status(StatusCode::ACCEPTED)
            .body(Body::empty())
            .unwrap(),
        "tools/list" => {
            assert_eq!(
                headers
                    .get("mcp-session-id")
                    .and_then(|value| value.to_str().ok()),
                Some("sess-1")
            );
            let result = json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": {
                    "tools": [{
                        "name": "echo",
                        "description": "Echo over HTTP.",
                        "inputSchema": {
                            "type": "object",
                            "properties": {"text": {"type": "string"}}
                        }
                    }]
                }
            });
            Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, "text/event-stream")
                .body(Body::from(format!("event: message\ndata: {result}\n\n")))
                .unwrap()
        }
        "tools/call" => {
            let text = request["params"]["arguments"]["text"]
                .as_str()
                .unwrap_or_default();
            Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": {
                            "content": [{"type": "text", "text": format!("http:{text}")}],
                            "isError": false
                        }
                    })
                    .to_string(),
                ))
                .unwrap()
        }
        _ => Response::builder()
            .status(StatusCode::NOT_FOUND)
            .body(Body::empty())
            .unwrap(),
    }
}
