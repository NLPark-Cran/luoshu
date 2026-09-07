//! MCP protocol: JSON-RPC 2.0 subset over Streamable HTTP.
//!
//! Implemented methods: `initialize`, `ping`, `tools/list`, `tools/call`
//! (plus `notifications/initialized`, handled transport-side as 202).
//! We deliberately implement the subset by hand instead of pulling an MCP
//! SDK crate: the surface is ~100 lines, keeps the dependency tree small,
//! and avoids SDK version drift. See docs/003-mvp-architecture.md.

use serde_json::{json, Value};
use std::sync::Arc;

use crate::tools::{call_tool, tool_descriptors, ToolContext};

pub const PROTOCOL_VERSION: &str = "2025-06-18";
pub const SERVER_NAME: &str = "luoshu-bridge";
pub const SERVER_VERSION: &str = env!("CARGO_PKG_VERSION");

/// True if this JSON-RPC message is a notification or response (no reply body).
pub fn is_notification(msg: &Value) -> bool {
    msg.get("id").is_none()
}

/// Handle one JSON-RPC request. Returns the response object.
pub async fn handle_request(ctx: &Arc<ToolContext>, msg: &Value) -> Value {
    let id = msg.get("id").cloned().unwrap_or(Value::Null);
    let method = msg.get("method").and_then(Value::as_str).unwrap_or("");

    match method {
        "initialize" => ok(
            id,
            json!({
                "protocolVersion": PROTOCOL_VERSION,
                "capabilities": { "tools": {} },
                "serverInfo": { "name": SERVER_NAME, "version": SERVER_VERSION },
                "instructions": "Luoshu local device bridge. Tools act on the USER'S LOCAL MACHINE. \
                                 Dangerous shell commands need a one-time approval token from the Luoshu UI."
            }),
        ),
        "ping" => ok(id, json!({})),
        "tools/list" => ok(id, json!({ "tools": tool_descriptors() })),
        "tools/call" => {
            let params = msg.get("params").cloned().unwrap_or(Value::Null);
            let name = params.get("name").and_then(Value::as_str).unwrap_or("");
            let args = params.get("arguments").cloned().unwrap_or(json!({}));
            match call_tool(ctx, name, &args).await {
                Ok(result) => ok(
                    id,
                    json!({
                        "content": [{
                            "type": "text",
                            "text": serde_json::to_string_pretty(&result).unwrap_or_default()
                        }],
                        "structuredContent": result,
                        "isError": false
                    }),
                ),
                Err(e) => ok(
                    id,
                    json!({
                        "content": [{ "type": "text", "text": e.0 }],
                        "isError": true
                    }),
                ),
            }
        }
        "" => err(id, -32600, "invalid request: missing method"),
        other => err(id, -32601, &format!("method not found: {other}")),
    }
}

fn ok(id: Value, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

fn err(id: Value, code: i64, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::approval::{ApprovalStore, NullSink};
    use crate::sandbox::Sandbox;

    fn ctx() -> Arc<ToolContext> {
        Arc::new(ToolContext {
            sandbox: Sandbox::new(vec![std::env::temp_dir()]),
            approvals: ApprovalStore::new(),
            sink: Arc::new(NullSink),
            browser: None,
            screenshots_dir: std::env::temp_dir().join("shots"),
        })
    }

    #[tokio::test]
    async fn initialize_and_list() {
        let ctx = ctx();
        let res = handle_request(
            &ctx,
            &json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}),
        )
        .await;
        assert_eq!(res["result"]["serverInfo"]["name"], "luoshu-bridge");
        let res = handle_request(&ctx, &json!({"jsonrpc":"2.0","id":2,"method":"tools/list"})).await;
        let tools = res["result"]["tools"].as_array().unwrap();
        assert!(tools.len() >= 7);
        assert!(tools.iter().any(|t| t["name"] == "luoshu_shell"));
    }

    #[tokio::test]
    async fn unknown_method_error() {
        let ctx = ctx();
        let res = handle_request(&ctx, &json!({"jsonrpc":"2.0","id":9,"method":"nope"})).await;
        assert_eq!(res["error"]["code"], -32601);
    }

    #[tokio::test]
    async fn tools_call_sysinfo() {
        let ctx = ctx();
        let res = handle_request(
            &ctx,
            &json!({"jsonrpc":"2.0","id":3,"method":"tools/call",
                    "params":{"name":"luoshu_sysinfo","arguments":{}}}),
        )
        .await;
        assert_eq!(res["result"]["isError"], false);
        assert!(res["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("cpu_count"));
    }

    #[tokio::test]
    async fn tools_call_unknown_tool_is_error_result() {
        let ctx = ctx();
        let res = handle_request(
            &ctx,
            &json!({"jsonrpc":"2.0","id":4,"method":"tools/call",
                    "params":{"name":"nope","arguments":{}}}),
        )
        .await;
        assert_eq!(res["result"]["isError"], true);
    }
}
