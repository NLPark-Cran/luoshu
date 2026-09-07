// 洛书 shell toolbar logic. Runs inside the `shell` webview (tauri://localhost).
// Uses the Tauri IPC bridge injected at window.__TAURI__.

const invoke = window.__TAURI__?.core?.invoke;
const listen = window.__TAURI__?.event?.listen;

const $ = (id) => document.getElementById(id);

function nav(action) {
  invoke?.("nav", { action }).catch((e) => console.error("nav failed:", e));
}

$("nav-back").addEventListener("click", () => nav("back"));
$("nav-forward").addEventListener("click", () => nav("forward"));
$("nav-reload").addEventListener("click", () => nav("reload"));
$("nav-home").addEventListener("click", () => nav("home"));

// ------------------------------------------------------------ bridge status

function setBridgeStatus(running, port) {
  const el = $("bridge-status");
  el.classList.toggle("green", running);
  el.classList.toggle("red", !running);
  $("bridge-label").textContent = running ? `设备桥 :${port}` : "设备桥 未启动";
  el.title = running
    ? `设备桥运行中：http://127.0.0.1:${port}/mcp`
    : "设备桥未运行";
}

async function pollBridgeStatus() {
  try {
    const status = await invoke("bridge_status");
    setBridgeStatus(status.running, status.port);
  } catch {
    setBridgeStatus(false);
  }
}
pollBridgeStatus();
setInterval(pollBridgeStatus, 5000);

// --------------------------------------------------------- copy MCP config

$("copy-mcp").addEventListener("click", async () => {
  try {
    const snippet = await invoke("mcp_config");
    await copyText(snippet);
    $("copy-mcp").textContent = "已复制 ✓";
    setTimeout(() => ($("copy-mcp").textContent = "复制 MCP 配置"), 1500);
  } catch (e) {
    alert(`无法获取 MCP 配置：${e}`);
  }
});

async function copyText(text) {
  try {
    await navigator.clipboard.writeText(text);
  } catch {
    const ta = document.createElement("textarea");
    ta.value = text;
    document.body.appendChild(ta);
    ta.select();
    document.execCommand("copy");
    ta.remove();
  }
}

// ------------------------------------------------------------- about page

$("about-btn").addEventListener("click", () =>
  $("about-overlay").classList.remove("hidden")
);
$("about-close").addEventListener("click", () =>
  $("about-overlay").classList.add("hidden")
);

// -------------------------------------------------------- approval dialog

let currentToken = null;

function showApproval(command, token) {
  currentToken = token;
  $("approval-command").textContent = command;
  $("approval-token").textContent = token;
  $("approval-overlay").classList.remove("hidden");
}

$("approval-deny").addEventListener("click", () => {
  if (currentToken) invoke?.("deny_approval", { token: currentToken });
  currentToken = null;
  $("approval-overlay").classList.add("hidden");
});

if (listen) {
  listen("bridge-approval", (event) => {
    showApproval(event.payload.command, event.payload.token);
  });
  listen("bridge-status", () => pollBridgeStatus());
}
