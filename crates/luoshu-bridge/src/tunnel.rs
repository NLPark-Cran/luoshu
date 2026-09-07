//! Reverse tunnel client (docs/004-reverse-tunnel.md).
//!
//! The cloud cannot reach the loopback bridge, so Luoshu dials *out*:
//! `wss://<cloud>/api/v2/devices/{id}/tunnel?token=…`. MCP JSON-RPC messages
//! arrive verbatim over the WebSocket and are executed by the exact same
//! `mcp::handle_request` entry point as the loopback HTTP bridge — sandbox
//! and approval gates apply unchanged. Responses go back over the socket.
//!
//! Reconnect policy: exponential backoff 1s → 60s cap with ±25% jitter,
//! reset on each successful connect. A close frame with code 4401 means the
//! device credentials are invalid/revoked: retrying is pointless, so the
//! tunnel stops and surfaces `TunnelEvent::AuthRejected` (the user must
//! re-enroll the device).

use std::sync::Arc;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use serde_json::Value;
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message;

use crate::config::DeviceCredentials;
use crate::mcp;
use crate::tools::ToolContext;

const BACKOFF_INITIAL: Duration = Duration::from_secs(1);
const BACKOFF_MAX: Duration = Duration::from_secs(60);
/// Close code used by the cloud when device credentials are rejected.
const CLOSE_AUTH_REJECTED: u16 = 4401;

/// Lifecycle events, surfaced to the shell UI (status light) and tests.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TunnelEvent {
    Connected,
    Disconnected,
    /// Cloud rejected the device token (close 4401); tunnel stops for good.
    AuthRejected,
}

#[derive(Debug)]
enum TunnelError {
    Connect,
    AuthRejected,
}

/// Run the tunnel forever (reconnecting) until the task is aborted or the
/// cloud rejects the credentials. `events` is optional (headless callers).
pub async fn run_tunnel(
    ctx: Arc<ToolContext>,
    creds: DeviceCredentials,
    events: Option<mpsc::UnboundedSender<TunnelEvent>>,
) {
    let mut attempt: u32 = 0;
    loop {
        match run_once(&ctx, &creds, &events).await {
            Ok(()) => {
                emit(&events, TunnelEvent::Disconnected);
                attempt = attempt.saturating_add(1);
            }
            Err(TunnelError::Connect) => {
                emit(&events, TunnelEvent::Disconnected);
                attempt = attempt.saturating_add(1);
            }
            Err(TunnelError::AuthRejected) => {
                emit(&events, TunnelEvent::AuthRejected);
                return;
            }
        }
        tokio::time::sleep(backoff_delay(attempt)).await;
    }
}

fn emit(events: &Option<mpsc::UnboundedSender<TunnelEvent>>, event: TunnelEvent) {
    if let Some(tx) = events {
        let _ = tx.send(event);
    }
}

/// Exponential backoff: 1s doubling to a 60s cap, ±25% jitter.
fn backoff_delay(attempt: u32) -> Duration {
    let base_ms = BACKOFF_INITIAL.as_millis() as u64;
    let cap_ms = BACKOFF_MAX.as_millis() as u64;
    let scaled = base_ms.saturating_mul(1u64 << attempt.min(6)).min(cap_ms) as f64;
    let jitter = rand::random::<f64>() * 0.5 - 0.25;
    Duration::from_millis((scaled * (1.0 + jitter)) as u64)
}

/// One connection lifecycle: connect, relay until close/error.
async fn run_once(
    ctx: &Arc<ToolContext>,
    creds: &DeviceCredentials,
    events: &Option<mpsc::UnboundedSender<TunnelEvent>>,
) -> Result<(), TunnelError> {
    let url = creds.tunnel_url();
    let (ws, _) = tokio_tungstenite::connect_async(&url)
        .await
        .map_err(|e| {
            eprintln!("luoshu-tunnel: connect failed: {e}");
            TunnelError::Connect
        })?;
    emit(events, TunnelEvent::Connected);
    eprintln!("luoshu-tunnel: connected to {url}");

    let (mut write, mut read) = ws.split();
    // Outbound responses funnel through this channel (requests are handled
    // concurrently, so sends come from many tasks).
    let (tx, mut rx) = mpsc::unbounded_channel::<Value>();
    let writer = tokio::spawn(async move {
        while let Some(msg) = rx.recv().await {
            if write.send(Message::Text(msg.to_string().into())).await.is_err() {
                break;
            }
        }
    });

    let result = loop {
        match read.next().await {
            Some(Ok(Message::Text(text))) => {
                let Ok(msg) = serde_json::from_str::<Value>(&text) else {
                    continue; // drop non-JSON frames defensively
                };
                if mcp::is_notification(&msg) {
                    continue; // notifications carry no reply
                }
                let ctx = ctx.clone();
                let tx = tx.clone();
                tokio::spawn(async move {
                    let response = mcp::handle_request(&ctx, &msg).await;
                    let _ = tx.send(response);
                });
            }
            Some(Ok(Message::Close(frame))) => {
                let rejected = frame
                    .as_ref()
                    .is_some_and(|f| u16::from(f.code) == CLOSE_AUTH_REJECTED);
                break if rejected {
                    Err(TunnelError::AuthRejected)
                } else {
                    Ok(())
                };
            }
            Some(Ok(_)) => {} // pings/pongs/binary: tungstenite answers pings
            Some(Err(_)) | None => break Ok(()),
        }
    };

    writer.abort();
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::approval::{ApprovalStore, NullSink};
    use crate::sandbox::Sandbox;
    use futures_util::stream::{SplitSink, SplitStream};
    use serde_json::json;
    use tokio::net::TcpListener;
    use tokio_tungstenite::WebSocketStream;

    type MockWs = WebSocketStream<tokio::net::TcpStream>;

    fn ctx() -> Arc<ToolContext> {
        Arc::new(ToolContext {
            sandbox: Sandbox::new(vec![std::env::temp_dir()]),
            approvals: ApprovalStore::new(),
            sink: Arc::new(NullSink),
            browser: None,
            screenshots_dir: std::env::temp_dir().join("shots"),
            eval_results: crate::eval_relay::EvalResultStore::new(),
            writeback_url: None,
        })
    }

    fn creds(port: u16) -> DeviceCredentials {
        DeviceCredentials {
            cloud_url: format!("http://127.0.0.1:{port}"),
            device_id: "dev-test".into(),
            device_token: "tok".into(),
            name: None,
        }
    }

    async fn accept(listener: &TcpListener) -> MockWs {
        let (stream, _) = listener.accept().await.unwrap();
        tokio_tungstenite::accept_async(stream).await.unwrap()
    }

    async fn recv_json(read: &mut SplitStream<MockWs>) -> Value {
        let msg = tokio::time::timeout(Duration::from_secs(5), read.next())
            .await
            .expect("timeout waiting for client message")
            .expect("stream closed")
            .expect("ws error");
        serde_json::from_str(msg.into_text().unwrap().as_str()).unwrap()
    }

    async fn send_json(write: &mut SplitSink<MockWs, Message>, value: Value) {
        write.send(Message::Text(value.to_string().into())).await.unwrap();
    }

    #[test]
    fn tunnel_url_rewrites_scheme() {
        let c = DeviceCredentials {
            cloud_url: "https://crys.tt2.li/".into(),
            device_id: "d1".into(),
            device_token: "secret".into(),
            name: None,
        };
        assert_eq!(
            c.tunnel_url(),
            "wss://crys.tt2.li/api/v2/devices/d1/tunnel?token=secret"
        );
    }

    #[test]
    fn backoff_grows_and_caps() {
        assert!(backoff_delay(0) <= Duration::from_millis(1500));
        assert!(backoff_delay(3) >= Duration::from_millis(6000));
        for attempt in 0..20 {
            let d = backoff_delay(attempt);
            assert!(d <= Duration::from_millis(75_000));
        }
    }

    #[tokio::test]
    async fn round_trip_tools_call_over_tunnel() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();

        let (events, mut rx) = mpsc::unbounded_channel();
        let client = tokio::spawn(run_tunnel(ctx(), creds(port), Some(events)));

        // Server side: accept, read a tools/call, answer it.
        let server = tokio::spawn(async move {
            let ws = accept(&listener).await;
            let (mut write, mut read) = ws.split();
            send_json(
                &mut write,
                json!({"jsonrpc":"2.0","id":"t-1","method":"tools/call",
                       "params":{"name":"luoshu_sysinfo","arguments":{}}}),
            )
            .await;
            let response = recv_json(&mut read).await;
            assert_eq!(response["id"], "t-1");
            assert_eq!(response["result"]["isError"], false);
            assert!(response["result"]["content"][0]["text"]
                .as_str()
                .unwrap()
                .contains("cpu_count"));
        });

        assert_eq!(
            tokio::time::timeout(Duration::from_secs(5), rx.recv())
                .await
                .unwrap(),
            Some(TunnelEvent::Connected)
        );
        tokio::time::timeout(Duration::from_secs(10), server)
            .await
            .unwrap()
            .unwrap();
        client.abort();
    }

    #[tokio::test]
    async fn reconnects_after_server_drop() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();

        let (events, mut rx) = mpsc::unbounded_channel();
        let client = tokio::spawn(run_tunnel(ctx(), creds(port), Some(events)));

        // First connection: accept then drop immediately.
        let first = accept(&listener).await;
        drop(first);
        // The client must come back (backoff ~1s) on its own.
        let second = tokio::time::timeout(Duration::from_secs(10), accept(&listener))
            .await
            .expect("client did not reconnect");
        drop(second);

        // Connected twice, with a disconnect in between.
        let mut seen = vec![];
        while let Ok(Some(ev)) =
            tokio::time::timeout(Duration::from_millis(100), rx.recv()).await
        {
            seen.push(ev);
        }
        assert!(seen.contains(&TunnelEvent::Disconnected));
        assert_eq!(
            seen.iter().filter(|e| **e == TunnelEvent::Connected).count(),
            2
        );
        client.abort();
    }

    #[tokio::test]
    async fn auth_rejected_stops_the_tunnel() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();

        let (events, mut rx) = mpsc::unbounded_channel();
        let client = tokio::spawn(run_tunnel(ctx(), creds(port), Some(events)));

        let ws = accept(&listener).await;
        let (mut write, _) = ws.split();
        write
            .send(Message::Close(Some(tokio_tungstenite::tungstenite::protocol::CloseFrame {
                code: CLOSE_AUTH_REJECTED.into(),
                reason: "Invalid device credentials".into(),
            })))
            .await
            .unwrap();

        // The tunnel task ends and reports AuthRejected instead of retrying.
        let mut saw_auth_rejected = false;
        for _ in 0..4 {
            match tokio::time::timeout(Duration::from_secs(5), rx.recv()).await {
                Ok(Some(TunnelEvent::AuthRejected)) => saw_auth_rejected = true,
                Ok(Some(_)) => {}
                _ => break,
            }
        }
        assert!(saw_auth_rejected);
        tokio::time::timeout(Duration::from_secs(5), client)
            .await
            .expect("tunnel task did not stop after 4401")
            .unwrap();
    }
}
