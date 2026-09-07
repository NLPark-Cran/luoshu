//! Integration tests over real loopback HTTP: auth, MCP handshake, tool call.

use std::sync::Arc;

use luoshu_bridge::approval::{ApprovalStore, NullSink};
use luoshu_bridge::sandbox::Sandbox;
use luoshu_bridge::server::router;
use luoshu_bridge::tools::ToolContext;
use serde_json::{json, Value};

const TOKEN: &str = "test-token-0123456789abcdef";

fn test_ctx() -> Arc<ToolContext> {
    Arc::new(ToolContext {
        sandbox: Sandbox::new(vec![std::env::temp_dir()]),
        approvals: ApprovalStore::new(),
        sink: Arc::new(NullSink),
        browser: None,
        screenshots_dir: std::env::temp_dir().join("shots"),
    })
}

async fn spawn_server() -> String {
    let app = router(test_ctx(), TOKEN.to_string());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    format!("http://127.0.0.1:{port}")
}

#[tokio::test]
async fn requires_bearer_token() {
    let base = spawn_server().await;
    let client = reqwest::Client::new();

    // No token → 401
    let res = client
        .post(format!("{base}/mcp"))
        .json(&json!({"jsonrpc":"2.0","id":1,"method":"initialize"}))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 401);

    // Wrong token → 401
    let res = client
        .post(format!("{base}/mcp"))
        .bearer_auth("wrong")
        .json(&json!({"jsonrpc":"2.0","id":1,"method":"initialize"}))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 401);

    // Right token → 200
    let res = client
        .post(format!("{base}/mcp"))
        .bearer_auth(TOKEN)
        .json(&json!({"jsonrpc":"2.0","id":1,"method":"initialize"}))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 200);
    let body: Value = res.json().await.unwrap();
    assert_eq!(body["result"]["serverInfo"]["name"], "luoshu-bridge");
}

#[tokio::test]
async fn notification_gets_202() {
    let base = spawn_server().await;
    let res = reqwest::Client::new()
        .post(format!("{base}/mcp"))
        .bearer_auth(TOKEN)
        .json(&json!({"jsonrpc":"2.0","method":"notifications/initialized"}))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 202);
}

#[tokio::test]
async fn end_to_end_tool_call() {
    let base = spawn_server().await;
    let client = reqwest::Client::new();
    let res = client
        .post(format!("{base}/mcp"))
        .bearer_auth(TOKEN)
        .json(&json!({
            "jsonrpc":"2.0","id":7,"method":"tools/call",
            "params":{"name":"luoshu_shell","arguments":{"command":"echo luoshu-ok"}}
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 200);
    let body: Value = res.json().await.unwrap();
    assert_eq!(body["result"]["isError"], false);
    let text = body["result"]["content"][0]["text"].as_str().unwrap();
    assert!(text.contains("luoshu-ok"), "unexpected tool output: {text}");
}

#[tokio::test]
async fn malformed_json_is_400() {
    let base = spawn_server().await;
    let res = reqwest::Client::new()
        .post(format!("{base}/mcp"))
        .bearer_auth(TOKEN)
        .header("content-type", "application/json")
        .body("{not json")
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 400);
}
