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
    if (status.tunnel) setTunnelStatus(status.tunnel);
  } catch {
    setBridgeStatus(false);
  }
}
pollBridgeStatus();
setInterval(pollBridgeStatus, 5000);

// ------------------------------------------------------------ tunnel status

function setTunnelStatus(state) {
  const el = $("tunnel-status");
  const label = $("tunnel-label");
  el.classList.remove("green", "red", "gray");
  if (state === "connected") {
    el.classList.add("green");
    label.textContent = "隧道 已连接";
  } else if (state === "auth_rejected") {
    el.classList.add("red");
    label.textContent = "隧道 凭证失效";
  } else if (state === "disconnected") {
    el.classList.add("gray");
    label.textContent = "隧道 重连中";
  } else {
    el.classList.add("gray");
    label.textContent = "隧道 未登记";
  }
}

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

// ------------------------------------------------------------- settings page

async function refreshDeviceInfo() {
  try {
    const info = await invoke("device_info");
    const cur = $("device-current");
    if (info.enrolled) {
      cur.innerHTML = `当前设备：<b>${escapeHtml(info.name || info.device_id)}</b> <code>${escapeHtml(info.device_id)}</code> @ ${escapeHtml(info.cloud_url)}`;
      $("enroll-clear").style.display = "";
    } else {
      cur.innerHTML = `<span class="hint">尚未登记设备。登记后云端 Agent 才能触达本机。</span>`;
      $("enroll-clear").style.display = "none";
    }
  } catch (e) {
    console.error("device_info failed:", e);
  }
}

function escapeHtml(s) {
  return String(s).replace(/[&<>"']/g, (c) =>
    ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" }[c])
  );
}

$("settings-btn").addEventListener("click", () => {
  refreshDeviceInfo();
  $("settings-overlay").classList.remove("hidden");
});
$("settings-close").addEventListener("click", () =>
  $("settings-overlay").classList.add("hidden")
);

$("enroll-save").addEventListener("click", async () => {
  const err = $("enroll-error");
  err.textContent = "";
  try {
    await invoke("enroll_device", {
      cloudUrl: $("enroll-cloud").value || "https://crys.tt2.li",
      deviceId: $("enroll-id").value,
      deviceToken: $("enroll-token").value,
      name: $("enroll-name").value || null,
    });
    $("enroll-token").value = "";
    await refreshDeviceInfo();
    pollBridgeStatus();
  } catch (e) {
    err.textContent = String(e);
  }
});

$("enroll-clear").addEventListener("click", async () => {
  if (!confirm("确定注销本设备？云端将立即无法连接本机。")) return;
  try {
    await invoke("unenroll_device");
    setTunnelStatus(null);
    await refreshDeviceInfo();
  } catch (e) {
    $("enroll-error").textContent = String(e);
  }
});

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
  listen("tunnel-status", (event) => setTunnelStatus(event.payload.state));
}
