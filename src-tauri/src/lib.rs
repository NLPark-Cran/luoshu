//! 洛书 Luoshu — Tauri shell.
//!
//! One window, two webviews:
//! - `shell` (top, 48px): native toolbar — nav buttons, bridge status,
//!   MCP config copy, approval dialogs, about page.
//! - `main`  (rest): the cloud app (default https://crys.tt2.li, override via
//!   `LUOSHU_TARGET_URL`).
//!
//! On startup we spawn the loopback device bridge (luoshu-bridge crate) and
//! expose its state to the shell webview via IPC.

use std::sync::{Arc, Mutex};

use luoshu_bridge::approval::BridgeEventSink;
use luoshu_bridge::browser::{BrowserControl, BrowserError, HistoryEntry};
use luoshu_bridge::config::BridgeConfig;
use luoshu_bridge::server::{self, RunningBridge};
use luoshu_bridge::tools::ToolContext;
use serde::Serialize;
use serde_json::json;
use tauri::{AppHandle, Emitter, LogicalPosition, LogicalSize, Manager, State, WebviewUrl};

const DEFAULT_TARGET_URL: &str = "https://crys.tt2.li";
const TARGET_URL_ENV: &str = "LUOSHU_TARGET_URL";
const TOOLBAR_HEIGHT: f64 = 48.0;

fn target_url() -> String {
    std::env::var(TARGET_URL_ENV).unwrap_or_else(|_| DEFAULT_TARGET_URL.to_string())
}

/// Shared app state managed by Tauri.
struct AppState {
    bridge: Mutex<Option<BridgeInfo>>,
    /// Last reverse-tunnel event as a string: connected/disconnected/auth_rejected.
    tunnel: Mutex<Option<String>>,
    /// Navigation history of the main webview (oldest first, capped).
    history: Mutex<Vec<HistoryEntry>>,
    /// Enrolled device metadata (token itself never leaves `device.json`).
    device: Mutex<Option<DeviceMeta>>,
    /// Running tunnel task (aborted on re-enroll / unenroll).
    tunnel_task: Mutex<Option<tauri::async_runtime::JoinHandle<()>>>,
}

#[derive(Clone, serde::Serialize)]
struct DeviceMeta {
    device_id: String,
    name: Option<String>,
    cloud_url: String,
}

const HISTORY_CAP: usize = 500;

/// Record a navigation, deduping consecutive repeats of the same URL.
fn record_history(state: &AppState, url: &str) {
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let mut history = state.history.lock().unwrap();
    if history.last().map(|e| e.url.as_str()) == Some(url) {
        return;
    }
    history.push(HistoryEntry {
        url: url.to_string(),
        ts,
    });
    if history.len() > HISTORY_CAP {
        let excess = history.len() - HISTORY_CAP;
        history.drain(..excess);
    }
}

struct BridgeInfo {
    port: u16,
    token: String,
    ctx: Arc<ToolContext>,
    home_dir: std::path::PathBuf,
    _handle: Arc<RunningBridge>,
}

#[derive(Serialize)]
struct BridgeStatus {
    running: bool,
    port: Option<u16>,
    url: Option<String>,
    /// Reverse-tunnel state (None when the device is not enrolled).
    tunnel: Option<String>,
}

// ------------------------------------------------------------------ events

struct TauriSink {
    app: AppHandle,
}

impl BridgeEventSink for TauriSink {
    fn on_approval_requested(&self, command: &str, token: &str) {
        let _ = self.app.emit_to(
            "shell",
            "bridge-approval",
            json!({ "command": command, "token": token }),
        );
    }
}

// ------------------------------------------------------- browser control

struct TauriBrowser {
    app: AppHandle,
}

#[async_trait::async_trait]
impl BrowserControl for TauriBrowser {
    async fn eval(&self, script: &str) -> Result<String, BrowserError> {
        let wv = self
            .app
            .get_webview("main")
            .ok_or(BrowserError::Unavailable)?;
        // Tauri's eval is fire-and-forget; the JS runs in the page context.
        // Return values reach the caller via the bridge's writeback harness
        // (eval_relay) — this layer just needs to deliver the script.
        wv.eval(script).map_err(|e| BrowserError::Failed(e.to_string()))?;
        Ok("null".into())
    }

    async fn navigate(&self, url: &str) -> Result<(), BrowserError> {
        let wv = self
            .app
            .get_webview("main")
            .ok_or(BrowserError::Unavailable)?;
        let url_parsed = url
            .parse()
            .map_err(|_| BrowserError::Failed(format!("invalid url: {url}")))?;
        wv.navigate(url_parsed)
            .map_err(|e| BrowserError::Failed(e.to_string()))?;
        let state: State<'_, AppState> = self.app.state();
        record_history(&state, url);
        Ok(())
    }

    async fn history(&self) -> Result<Vec<HistoryEntry>, BrowserError> {
        let state: State<'_, AppState> = self.app.state();
        let entries = state.history.lock().unwrap().clone();
        Ok(entries)
    }
}

// ------------------------------------------------------------------- IPC

#[tauri::command]
fn bridge_status(state: State<'_, AppState>) -> BridgeStatus {
    let guard = state.bridge.lock().unwrap();
    let tunnel = state.tunnel.lock().unwrap().clone();
    match guard.as_ref() {
        Some(info) => BridgeStatus {
            running: true,
            port: Some(info.port),
            url: Some(format!("http://127.0.0.1:{}/mcp", info.port)),
            tunnel,
        },
        None => BridgeStatus {
            running: false,
            port: None,
            url: None,
            tunnel,
        },
    }
}

#[tauri::command]
fn mcp_config(state: State<'_, AppState>) -> Result<String, String> {
    let guard = state.bridge.lock().unwrap();
    let info = guard.as_ref().ok_or("bridge not running yet")?;
    Ok(server::mcp_config_snippet(info.port, &info.token))
}

#[tauri::command]
fn get_target_url() -> String {
    target_url()
}

#[tauri::command]
fn nav(app: AppHandle, action: String) -> Result<(), String> {
    let wv = app.get_webview("main").ok_or("main webview not found")?;
    match action.as_str() {
        "back" => wv.eval("history.back()"),
        "forward" => wv.eval("history.forward()"),
        "reload" => wv.eval("location.reload()"),
        "home" => {
            let url: tauri::Url = target_url().parse().map_err(|_| "bad target url")?;
            return wv.navigate(url).map_err(|e| e.to_string());
        }
        other => return Err(format!("unknown nav action: {other}")),
    }
    .map_err(|e| e.to_string())
}

#[tauri::command]
fn deny_approval(state: State<'_, AppState>, token: String) {
    let guard = state.bridge.lock().unwrap();
    if let Some(info) = guard.as_ref() {
        info.ctx.approvals.revoke(&token);
    }
}

// ------------------------------------------------------- device enrollment

#[derive(Serialize)]
struct DeviceInfo {
    enrolled: bool,
    device_id: Option<String>,
    name: Option<String>,
    cloud_url: Option<String>,
    tunnel: Option<String>,
}

#[tauri::command]
fn device_info(state: State<'_, AppState>) -> DeviceInfo {
    let device = state.device.lock().unwrap().clone();
    let tunnel = state.tunnel.lock().unwrap().clone();
    match device {
        Some(meta) => DeviceInfo {
            enrolled: true,
            device_id: Some(meta.device_id),
            name: meta.name,
            cloud_url: Some(meta.cloud_url),
            tunnel,
        },
        None => DeviceInfo {
            enrolled: false,
            device_id: None,
            name: None,
            cloud_url: None,
            tunnel,
        },
    }
}

#[tauri::command]
fn enroll_device(
    app: AppHandle,
    state: State<'_, AppState>,
    cloud_url: String,
    device_id: String,
    device_token: String,
    name: Option<String>,
) -> Result<(), String> {
    let cloud_url = cloud_url.trim().trim_end_matches('/').to_string();
    let device_id = device_id.trim().to_string();
    let device_token = device_token.trim().to_string();
    if cloud_url.is_empty() || device_id.is_empty() || device_token.is_empty() {
        return Err("cloud_url / device_id / device_token 均不能为空".into());
    }
    if !cloud_url.starts_with("https://") && !cloud_url.starts_with("http://") {
        return Err("cloud_url 必须以 https:// 或 http:// 开头".into());
    }

    let (ctx, home_dir) = {
        let guard = state.bridge.lock().unwrap();
        let info = guard.as_ref().ok_or("设备桥尚未启动，稍后再试")?;
        (info.ctx.clone(), info.home_dir.clone())
    };

    let creds = luoshu_bridge::config::DeviceCredentials {
        cloud_url: cloud_url.clone(),
        device_id: device_id.clone(),
        device_token,
        name: name.filter(|n| !n.trim().is_empty()),
    };
    // Persist first (0600), then (re)start the tunnel.
    let cfg = BridgeConfig {
        home_dir,
        token: String::new(), // unused for device.json writes
    };
    cfg.write_device_credentials(&creds)
        .map_err(|e| format!("写入 device.json 失败: {e}"))?;

    stop_tunnel(&app);
    let handle = start_tunnel(&app, ctx, creds.clone());
    *state.tunnel_task.lock().unwrap() = Some(handle);
    *state.device.lock().unwrap() = Some(DeviceMeta {
        device_id,
        name: creds.name,
        cloud_url,
    });
    Ok(())
}

#[tauri::command]
fn unenroll_device(app: AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    let home_dir = {
        let guard = state.bridge.lock().unwrap();
        guard.as_ref().map(|info| info.home_dir.clone())
    };
    if let Some(home) = home_dir {
        let path = home.join("device.json");
        if path.exists() {
            std::fs::remove_file(&path).map_err(|e| format!("删除 device.json 失败: {e}"))?;
        }
    }
    stop_tunnel(&app);
    *state.device.lock().unwrap() = None;
    Ok(())
}

// ------------------------------------------------------------------- run

/// Spawn the reverse tunnel (ADR 004) and forward its lifecycle events to
/// both the shell webview and `AppState` (for the status IPC). Returns the
/// tunnel task handle so enrollment changes can abort it.
fn start_tunnel(
    app: &AppHandle,
    ctx: Arc<ToolContext>,
    creds: luoshu_bridge::config::DeviceCredentials,
) -> tauri::async_runtime::JoinHandle<()> {
    use luoshu_bridge::tunnel::{run_tunnel, TunnelEvent};

    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let handle = tauri::async_runtime::spawn(run_tunnel(ctx, creds, Some(tx)));
    let app_handle = app.clone();
    tauri::async_runtime::spawn(async move {
        while let Some(event) = rx.recv().await {
            let state_str = match event {
                TunnelEvent::Connected => "connected",
                TunnelEvent::Disconnected => "disconnected",
                TunnelEvent::AuthRejected => "auth_rejected",
            };
            let state: State<'_, AppState> = app_handle.state();
            *state.tunnel.lock().unwrap() = Some(state_str.to_string());
            let _ = app_handle.emit_to("shell", "tunnel-status", json!({"state": state_str}));
        }
    });
    handle
}

/// Stop the current tunnel task (if any) and clear its status.
fn stop_tunnel(app: &AppHandle) {
    let state: State<'_, AppState> = app.state();
    if let Some(handle) = state.tunnel_task.lock().unwrap().take() {
        handle.abort();
    }
    *state.tunnel.lock().unwrap() = None;
    let _ = app.emit_to("shell", "tunnel-status", json!({"state": null}));
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(AppState {
            bridge: Mutex::new(None),
            tunnel: Mutex::new(None),
            history: Mutex::new(Vec::new()),
            device: Mutex::new(None),
            tunnel_task: Mutex::new(None),
        })
        .invoke_handler(tauri::generate_handler![
            bridge_status,
            mcp_config,
            get_target_url,
            nav,
            deny_approval,
            device_info,
            enroll_device,
            unenroll_device
        ])
        .setup(|app| {
            let url: tauri::Url = target_url()
                .parse()
                .map_err(|_| format!("invalid {TARGET_URL_ENV}"))?;

            // Shell window: the primary webview is the toolbar itself.
            let webview_window = tauri::WebviewWindowBuilder::new(
                app,
                "shell",
                WebviewUrl::App("index.html".into()),
            )
            .title("洛书 Luoshu")
            .inner_size(1280.0, 840.0)
            .min_inner_size(720.0, 480.0)
            .build()?;

            // Child webview for the cloud app (multi-webview = `unstable` feature).
            let window = app
                .get_window("shell")
                .ok_or("shell window missing after build")?;
            let size = webview_window.inner_size()?;
            let app_for_nav = app.handle().clone();
            let main_webview =
                tauri::webview::WebviewBuilder::new("main", WebviewUrl::External(url))
                    .on_navigation(move |nav_url| {
                        let state: State<'_, AppState> = app_for_nav.state();
                        record_history(&state, nav_url.as_str());
                        true
                    });
            window.add_child(
                main_webview,
                LogicalPosition::new(0.0, TOOLBAR_HEIGHT),
                LogicalSize::new(
                    f64::from(size.width),
                    f64::from(size.height) - TOOLBAR_HEIGHT,
                ),
            )?;

            // Keep the main webview fitted below the toolbar on resize.
            let app_for_resize = app.handle().clone();
            window.on_window_event(move |event| {
                if let tauri::WindowEvent::Resized(size) = event {
                    if let Some(wv) = app_for_resize.get_webview("main") {
                        let _ = wv.set_position(LogicalPosition::new(0.0, TOOLBAR_HEIGHT));
                        let _ = wv.set_size(LogicalSize::new(
                            f64::from(size.width),
                            (f64::from(size.height) - TOOLBAR_HEIGHT).max(0.0),
                        ));
                    }
                }
            });

            // Start the device bridge on the Tauri async runtime.
            let app_handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                let sink = Arc::new(TauriSink {
                    app: app_handle.clone(),
                });
                let browser: Arc<dyn BrowserControl> = Arc::new(TauriBrowser {
                    app: app_handle.clone(),
                });
                match BridgeConfig::from_env() {
                    Ok(config) => {
                        // Cloud enrollment (ADR 004): dial out so cloud agents
                        // can reach this device through the tunnel relay.
                        let device_creds = config.load_device_credentials();
                        let home_dir = config.home_dir.clone();
                        match server::start(config, sink, Some(browser)).await {
                        Ok(running) => {
                            if let Some(creds) = device_creds {
                                let meta = DeviceMeta {
                                    device_id: creds.device_id.clone(),
                                    name: creds.name.clone(),
                                    cloud_url: creds.cloud_url.clone(),
                                };
                                let handle =
                                    start_tunnel(&app_handle, running.ctx.clone(), creds);
                                let state: State<'_, AppState> = app_handle.state();
                                *state.tunnel_task.lock().unwrap() = Some(handle);
                                *state.device.lock().unwrap() = Some(meta);
                            }
                            let info = BridgeInfo {
                                port: running.port,
                                token: running.token.clone(),
                                ctx: running.ctx.clone(),
                                home_dir,
                                _handle: Arc::new(running),
                            };
                            let state: State<'_, AppState> = app_handle.state();
                            *state.bridge.lock().unwrap() = Some(info);
                            let _ = app_handle.emit_to("shell", "bridge-status", json!({"running": true}));
                        }
                        Err(e) => eprintln!("luoshu: bridge failed to start: {e}"),
                        }
                    }
                    Err(e) => eprintln!("luoshu: bridge config error: {e}"),
                }
            });

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running luoshu");
}
