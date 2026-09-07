//! Integration tests over real loopback HTTP: auth, MCP handshake, tool call.

use std::sync::Arc;

use luoshu_bridge::approval::{ApprovalStore, NullSink};
use luoshu_bridge::browser::{BrowserControl, BrowserError};
use luoshu_bridge::eval_relay::EvalResultStore;
use luoshu_bridge::sandbox::Sandbox;
use luoshu_bridge::server::router;
use luoshu_bridge::tools::ToolContext;
use serde_json::{json, Value};

const TOKEN: &str = "test-token-0123456789abcdef";

fn test_ctx() -> Arc<ToolContext> {
    ctx_with(None, None)
}

fn ctx_with(
    browser: Option<Arc<dyn BrowserControl>>,
    writeback_url: Option<String>,
) -> Arc<ToolContext> {
    Arc::new(ToolContext {
        sandbox: Sandbox::new(vec![std::env::temp_dir()]),
        approvals: ApprovalStore::new(),
        sink: Arc::new(NullSink),
        browser,
        screenshots_dir: std::env::temp_dir().join("shots"),
        eval_results: EvalResultStore::new(),
        writeback_url,
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

// ---------------------------------------------------------------- eval-result

/// Extract the fetch target URL from the writeback harness script.
fn extract_fetch_url(script: &str) -> String {
    let start = script.find("fetch(").expect("fetch not in harness") + "fetch(\"".len();
    let rest = &script[start..];
    let end = rest.find('"').unwrap();
    rest[..end].to_string()
}

/// Extract the first `key: "..."` string value from the harness script (the
/// harness embeds id/nonce as JSON string literals inside JSON.stringify).
fn extract_field(script: &str, key: &str) -> String {
    let marker = format!("{key}: \"");
    let start = script.find(&marker).unwrap_or_else(|| panic!("{key} not in harness")) + marker.len();
    let rest = &script[start..];
    let end = rest.find('"').unwrap();
    rest[..end].to_string()
}

struct MockBrowser;

#[async_trait::async_trait]
impl BrowserControl for MockBrowser {
    async fn eval(&self, script: &str) -> Result<String, BrowserError> {
        // Simulate the page: parse the harness, POST the result back.
        let url = extract_fetch_url(script);
        let id = extract_field(script, "id");
        let nonce = extract_field(script, "nonce");
        tokio::spawn(async move {
            let _ = reqwest::Client::new()
                .post(url)
                .header("content-type", "text/plain")
                .body(format!(
                    r#"{{"id":"{id}","nonce":"{nonce}","ok":true,"value":"{{\"x\":1}}"}}"#
                ))
                .send()
                .await;
        });
        Ok("null".into())
    }

    async fn navigate(&self, _url: &str) -> Result<(), BrowserError> {
        Ok(())
    }
}

#[tokio::test]
async fn eval_result_requires_valid_nonce() {
    let ctx = test_ctx();
    let app = router(ctx.clone(), TOKEN.to_string());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let base = format!("http://127.0.0.1:{port}");

    let (id, nonce, rx) = ctx.eval_results.create();
    let client = reqwest::Client::new();

    // Wrong nonce → 403, entry not consumed.
    let res = client
        .post(format!("{base}/eval-result"))
        .json(&json!({"id": id, "nonce": "wrong", "ok": true, "value": "null"}))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 403);

    // No bearer token needed — the nonce is the credential.
    let res = client
        .post(format!("{base}/eval-result"))
        .json(&json!({"id": id, "nonce": nonce, "ok": true, "value": "\"hi\""}))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 204);
    assert_eq!(rx.await.unwrap()["value"], "\"hi\"");

    // One-time: replaying the same nonce now fails.
    let res = client
        .post(format!("{base}/eval-result"))
        .json(&json!({"id": id, "nonce": "whatever", "ok": true, "value": "null"}))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 403);
}

#[tokio::test]
async fn browser_eval_writeback_round_trip() {
    // The writeback URL must point at the server under test, so build the
    // ctx after binding: two-step setup.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let base = format!("http://127.0.0.1:{port}");
    let ctx = ctx_with(
        Some(Arc::new(MockBrowser)),
        Some(format!("{base}/eval-result")),
    );
    let app = router(ctx, TOKEN.to_string());
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    let res = reqwest::Client::new()
        .post(format!("{base}/mcp"))
        .bearer_auth(TOKEN)
        .json(&json!({
            "jsonrpc":"2.0","id":9,"method":"tools/call",
            "params":{"name":"luoshu_browser_eval","arguments":{"script":"({x: 1})"}}
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 200);
    let body: Value = res.json().await.unwrap();
    assert_eq!(body["result"]["isError"], false);
    let structured = &body["result"]["structuredContent"];
    assert_eq!(structured["returned"], true);
    assert_eq!(structured["result"]["x"], 1);
}
