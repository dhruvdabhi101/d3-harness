use async_trait::async_trait;
use axum::{
    Router,
    body::Body,
    extract::State,
    http::{StatusCode, header},
    response::Response,
    routing::post,
};
use oc_harness::agent::Agent;
use oc_harness::config::{McpConfig, Permissions, ProviderConfig, ProviderKind};
use oc_harness::provider::{Provider, ProviderAuth};
use oc_harness::skills::SkillSet;
use oc_harness::tools::ToolRegistry;
use oc_harness::ui_bridge::{AgentUi, ToolUi};
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};

#[derive(Default)]
struct MockState {
    bodies: Mutex<Vec<Value>>,
}

struct SilentUi;

impl AgentUi for SilentUi {
    fn text(&self, _delta: &str) {}
    fn text_done(&self) {}
    fn tool_call(&self, _id: &str, _name: &str, _detail: &str) {}
    fn tool_result(&self, _id: &str, _summary: &str, _is_error: bool) {}
    fn todos(&self, _lines: &[String]) {}
    fn usage(&self, _input: u64, _output: u64) {}
    fn info(&self, _text: &str) {}
    fn warn(&self, _text: &str) {}
}

struct NoopToolUi;

#[async_trait]
impl ToolUi for NoopToolUi {
    fn diff(&self, _path: &str, _old: &str, _new: &str) {}
    fn is_interactive(&self) -> bool {
        false
    }
    async fn confirm(&self, _title: &str, _body: &str) -> bool {
        true
    }
    async fn ask(&self, _question: &str, _options: &[String]) -> Option<String> {
        None
    }
}

#[tokio::test]
async fn compact_replaces_history_with_a_model_summary() {
    let state = Arc::new(MockState::default());
    let app = Router::new()
        .route("/v1/chat/completions", post(mock_chat))
        .with_state(state.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    let workspace = tempfile::tempdir().unwrap();
    let provider = Provider::new(
        "mock".into(),
        ProviderConfig {
            kind: Some(ProviderKind::OpenAi),
            base_url: Some(format!("http://{address}/v1")),
            ..Default::default()
        },
        "mock-model".into(),
        ProviderAuth {
            key: "test".into(),
            account_id: None,
        },
    )
    .unwrap();
    let skills = SkillSet::discover(workspace.path(), &[]);
    let mcp = oc_harness::mcp::McpManager::connect(&McpConfig::default(), workspace.path()).await;
    let tools = ToolRegistry::new(
        workspace.path().to_path_buf(),
        skills,
        mcp,
        Permissions::default(),
        true,
        Arc::new(NoopToolUi),
    );
    let mut agent = Agent::new(
        provider,
        "mock-model".into(),
        "core".into(),
        "system".into(),
        tools,
        4,
        Arc::new(SilentUi),
    );
    agent.run_turn("hello").await.unwrap();
    assert_eq!(agent.messages().len(), 2);

    let info = agent.compact(Some("keep the test detail")).await.unwrap();
    assert_eq!(info.before_messages, 2);
    assert_eq!(info.after_messages, 2);
    assert!(
        agent.messages()[0]
            .text()
            .contains("Compacted conversation summary")
    );
    assert!(agent.messages()[0].text().contains("compacted detail"));
    assert_eq!(
        agent.messages()[1].role,
        oc_harness::conversation::Role::Assistant
    );

    let bodies = state.bodies.lock().unwrap();
    assert_eq!(bodies.len(), 2);
    let request = &bodies[1];
    let system = request["messages"][0]["content"].as_str().unwrap();
    assert!(
        system.contains("compact coding-agent conversations"),
        "{system}"
    );
    assert!(system.contains("keep the test detail"));
    assert!(
        request
            .get("tools")
            .and_then(|tools| tools.as_array())
            .is_none_or(Vec::is_empty)
    );
}

async fn mock_chat(State(state): State<Arc<MockState>>, body: String) -> Response {
    let value: Value = serde_json::from_str(&body).unwrap();
    let compact = value["messages"][0]["content"]
        .as_str()
        .is_some_and(|system| system.contains("compact coding-agent conversations"));
    state.bodies.lock().unwrap().push(value);
    let text = if compact {
        "# Summary\n\n- compacted detail: the user said hello\n- the assistant answered"
    } else {
        "first answer"
    };
    let chunk = json!({
        "choices": [{"index": 0, "delta": {"content": text}, "finish_reason": null}]
    });
    let usage = json!({
        "choices": [],
        "usage": {"prompt_tokens": 30, "completion_tokens": 10}
    });
    let sse = format!(
        "data: {chunk}\n\ndata: {usage}\n\ndata: {{\"choices\":[{{\"delta\":{{}},\"finish_reason\":\"stop\"}}]}}\n\ndata: [DONE]\n\n"
    );
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "text/event-stream")
        .body(Body::from(sse))
        .unwrap()
}
