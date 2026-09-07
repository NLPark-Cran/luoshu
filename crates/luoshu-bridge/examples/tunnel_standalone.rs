//! Standalone reverse-tunnel client for manual end-to-end verification.
//!
//! Usage:
//!   1. Register a device on the cran-code server (POST /api/v2/devices) and
//!      write the credentials to `$LUOSHU_HOME/device.json` (see README).
//!   2. `LUOSHU_HOME=~/.luoshu cargo run -p luoshu-bridge --example tunnel_standalone`
//!
//! The device should then show up as `online: true` in GET /api/v2/devices,
//! and MCP calls relayed by the cloud execute against this machine.

use std::sync::Arc;

use luoshu_bridge::approval::{ApprovalStore, NullSink};
use luoshu_bridge::config::BridgeConfig;
use luoshu_bridge::sandbox::Sandbox;
use luoshu_bridge::tools::ToolContext;
use luoshu_bridge::tunnel::run_tunnel;

#[tokio::main]
async fn main() {
    let config = BridgeConfig::from_env().expect("bridge config");
    let creds = config
        .load_device_credentials()
        .expect("device.json missing — register the device first (see README)");
    let ctx = Arc::new(ToolContext {
        sandbox: Sandbox::new(config.allowed_roots()),
        approvals: ApprovalStore::new(),
        sink: Arc::new(NullSink),
        browser: None,
        screenshots_dir: config.screenshots_dir(),
    });
    eprintln!("tunnel_standalone: dialing {}", creds.cloud_url);
    run_tunnel(ctx, creds, None).await;
}
