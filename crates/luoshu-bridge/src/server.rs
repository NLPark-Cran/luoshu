//! Axum HTTP server: loopback-only, bearer-authed MCP endpoint.

use std::net::{Ipv4Addr, SocketAddr};
use std::sync::Arc;

use axum::body::Body;
use axum::extract::State;
use axum::http::{header, Request, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde_json::{json, Value};
use tokio::net::TcpListener;
use tokio::task::JoinHandle;

use crate::approval::{ApprovalStore, BridgeEventSink};
use crate::auth::bearer_token_valid;
use crate::browser::BrowserControl;
use crate::config::BridgeConfig;
use crate::eval_relay::EvalResultStore;
use crate::mcp;
use crate::sandbox::Sandbox;
use crate::tools::ToolContext;

pub struct RunningBridge {
    pub port: u16,
    pub token: String,
    pub ctx: Arc<ToolContext>,
    handle: JoinHandle<()>,
}

impl RunningBridge {
    pub fn url(&self) -> String {
        format!("http://127.0.0.1:{}/mcp", self.port)
    }

    pub fn shutdown(&self) {
        self.handle.abort();
    }
}

#[derive(Clone)]
struct ServerState {
    ctx: Arc<ToolContext>,
}

/// Build the axum router (also used directly by integration tests).
///
/// `/mcp` and `/healthz` sit behind the bearer middleware; `/eval-result`
/// deliberately does NOT — it is called from webview page JS which must
/// never see the bridge token. It is guarded by a per-eval one-time nonce
/// instead (see `eval_relay`).
pub fn router(ctx: Arc<ToolContext>, token: String) -> Router {
    let authed = Router::new()
        .route("/mcp", post(handle_mcp_post))
        .route("/healthz", get(|| async { Json(json!({"ok": true})) }))
        .layer(middleware::from_fn_with_state(
            token.clone(),
            auth_middleware,
        ));
    Router::new()
        .merge(authed)
        .route("/eval-result", post(handle_eval_result))
        .with_state(ServerState { ctx })
}

async fn auth_middleware(
    State(token): State<String>,
    req: Request<Body>,
    next: Next,
) -> Response {
    let header_value = req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok());
    if bearer_token_valid(&token, header_value) {
        next.run(req).await
    } else {
        (
            StatusCode::UNAUTHORIZED,
            Json(json!({"error": "missing or invalid bearer token"})),
        )
            .into_response()
    }
}

async fn handle_mcp_post(State(state): State<ServerState>, body: Body) -> Response {
    let bytes = match axum::body::to_bytes(body, 4 * 1024 * 1024).await {
        Ok(b) => b,
        Err(_) => return StatusCode::PAYLOAD_TOO_LARGE.into_response(),
    };
    let msg: Value = match serde_json::from_slice(&bytes) {
        Ok(v) => v,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({"jsonrpc":"2.0","id":null,
                            "error":{"code":-32700,"message":"parse error"}})),
            )
                .into_response()
        }
    };
    // Notifications get 202 with no body (Streamable HTTP semantics).
    if mcp::is_notification(&msg) {
        return StatusCode::ACCEPTED.into_response();
    }
    let response = mcp::handle_request(&state.ctx, &msg).await;
    (
        [(header::CONTENT_TYPE, "application/json")],
        Json(response),
    )
        .into_response()
}

/// Receive a browser_eval writeback (nonce-authorized, one-time).
async fn handle_eval_result(State(state): State<ServerState>, body: Body) -> Response {
    let bytes = match axum::body::to_bytes(body, 1024 * 1024).await {
        Ok(b) => b,
        Err(_) => return StatusCode::PAYLOAD_TOO_LARGE.into_response(),
    };
    let msg: Value = match serde_json::from_slice(&bytes) {
        Ok(v) => v,
        Err(_) => return StatusCode::BAD_REQUEST.into_response(),
    };
    let id = msg.get("id").and_then(Value::as_str).unwrap_or("");
    let nonce = msg.get("nonce").and_then(Value::as_str).unwrap_or("");
    let payload = json!({
        "ok": msg.get("ok").cloned().unwrap_or(Value::Bool(false)),
        "value": msg.get("value").cloned().unwrap_or(Value::Null),
    });
    if state.ctx.eval_results.resolve(id, nonce, payload) {
        StatusCode::NO_CONTENT.into_response()
    } else {
        StatusCode::FORBIDDEN.into_response()
    }
}

/// Start the bridge: bind 127.0.0.1 on a random port, write the state file,
/// spawn the server. Returns a handle with the actual port + token.
pub async fn start(
    config: BridgeConfig,
    sink: Arc<dyn BridgeEventSink>,
    browser: Option<Arc<dyn BrowserControl>>,
) -> std::io::Result<RunningBridge> {
    let listener = TcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, 0))).await?;
    let port = listener.local_addr()?.port();
    let ctx = Arc::new(ToolContext {
        sandbox: Sandbox::new(config.allowed_roots()),
        approvals: ApprovalStore::new(),
        sink,
        browser,
        screenshots_dir: config.screenshots_dir(),
        eval_results: EvalResultStore::new(),
        writeback_url: Some(format!("http://127.0.0.1:{port}/eval-result")),
    });
    config.write_state_file(port)?;

    let app = router(ctx.clone(), config.token.clone());
    let handle = tokio::spawn(async move {
        if let Err(e) = axum::serve(listener, app).await {
            eprintln!("luoshu-bridge: server error: {e}");
        }
    });

    Ok(RunningBridge {
        port,
        token: config.token.clone(),
        ctx,
        handle,
    })
}

/// Render the cran-code MCP config snippet for this bridge.
///
/// cran-code reads fastmcp-format MCP config from `~/.cran/mcp.json`
/// (see cran-code `src/cran_code/cli/mcp.py`). Merge the `mcpServers.luoshu`
/// entry into that file, or use the equivalent CLI:
/// `cran-code mcp add --transport http luoshu <url> --header "Authorization: Bearer <token>"`.
pub fn mcp_config_snippet(port: u16, token: &str) -> String {
    let config = json!({
        "mcpServers": {
            "luoshu": {
                "url": format!("http://127.0.0.1:{port}/mcp"),
                "transport": "http",
                "headers": { "Authorization": format!("Bearer {token}") }
            }
        }
    });
    serde_json::to_string_pretty(&config).unwrap_or_default()
}
