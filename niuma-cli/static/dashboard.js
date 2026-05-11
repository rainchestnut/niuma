const serverInput = document.getElementById("server-url");
const configMessage = document.getElementById("config-message");
const fileAccessRootsInput = document.getElementById("file-access-roots");
const fileAccessPrecheckTimeoutInput = document.getElementById("file-access-precheck-timeout");
const fileAccessReadTimeoutInput = document.getElementById("file-access-read-timeout");
const fileAccessMessage = document.getElementById("file-access-message");
const fileAccessRootsList = document.getElementById("file-access-roots-list");
const pairingMessage = document.getElementById("pairing-message");
const pairingsElement = document.getElementById("pairings");
const confirmDialog = document.getElementById("confirm-dialog");
const confirmDevice = document.getElementById("confirm-device");
const confirmBinding = document.getElementById("confirm-binding");

async function fetchText(path, options = {}) {
  const response = await fetch(path, { cache: "no-store", ...options });
  const text = await response.text();
  if (!response.ok) {
    try {
      const value = JSON.parse(text);
      throw new Error(value.detail || text);
    } catch (error) {
      if (error instanceof SyntaxError) throw new Error(text || response.statusText);
      throw error;
    }
  }
  return text;
}

async function fetchJSON(path, options = {}) {
  const text = await fetchText(path, options);
  return JSON.parse(text);
}

function pretty(value) {
  return JSON.stringify(value, null, 2);
}

function formatTime(timestamp) {
  if (!timestamp) return "-";
  return new Date(timestamp * 1000).toLocaleString();
}

function compactDeviceName(deviceID) {
  const text = String(deviceID || "");
  if (text.startsWith("ios-")) return "iOS 设备";
  return text || "未知设备";
}

function setText(id, value) {
  document.getElementById(id).textContent = value ?? "-";
}

function setPill(id, ok, text) {
  const element = document.getElementById(id);
  element.className = `pill ${ok ? "ok" : "bad"}`;
  element.textContent = text;
}

function setNotice(text, tone = "") {
  configMessage.className = `notice ${tone}`;
  configMessage.textContent = text;
}

function setPairingNotice(text, tone = "") {
  pairingMessage.className = text ? `notice ${tone}` : "notice quiet";
  pairingMessage.textContent = text || "-";
}

function setFileAccessNotice(text, tone = "") {
  fileAccessMessage.className = `notice ${tone}`;
  fileAccessMessage.textContent = text;
}

function fileAccessStatusTone(status) {
  if (status === "granted") return "ok";
  if (status === "checking" || status === "not_checked") return "warn";
  return "bad";
}

function renderFileAccess(fileAccess) {
  const roots = fileAccess?.roots || [];
  const allGranted = roots.length > 0 && roots.every((root) => root.status === "granted");
  const running = Boolean(fileAccess?.running);
  const stateText = running ? "checking" : (allGranted ? "granted" : "needs attention");
  setPill("file-access-state", !running && allGranted, stateText);
  if (running) {
    document.getElementById("file-access-state").className = "pill warn";
  }
  if (document.activeElement !== fileAccessRootsInput && !fileAccessRootsInput.dataset.dirty) {
    fileAccessRootsInput.value = fileAccess?.roots_text || "";
  }
  if (
    document.activeElement !== fileAccessPrecheckTimeoutInput
    && !fileAccessPrecheckTimeoutInput.dataset.dirty
  ) {
    fileAccessPrecheckTimeoutInput.value = fileAccess?.precheck_timeout_seconds ?? 60;
  }
  if (
    document.activeElement !== fileAccessReadTimeoutInput
    && !fileAccessReadTimeoutInput.dataset.dirty
  ) {
    fileAccessReadTimeoutInput.value = fileAccess?.read_timeout_seconds ?? 10;
  }
  if (!fileAccessMessage.dataset.locked) {
    const updated = fileAccess?.updated_at ? `最近检查：${formatTime(fileAccess.updated_at)}` : "尚未完成检查";
    setFileAccessNotice(running ? "正在预检查文件访问权限..." : updated);
  }
  if (!roots.length) {
    fileAccessRootsList.innerHTML = `<div class="empty-state">未配置预检查目录</div>`;
    return;
  }
  fileAccessRootsList.innerHTML = roots.map((root) => `
    <div class="file-access-row">
      <div class="file-access-path">
        <strong class="mono">${escapeHTML(root.root)}</strong>
        <span class="mono">${escapeHTML(root.normalized_root || "-")}</span>
        ${root.message ? `<span>${escapeHTML(root.message)}</span>` : ""}
      </div>
      <span class="pill ${fileAccessStatusTone(root.status)}">${escapeHTML(root.status)}</span>
    </div>
  `).join("");
}

function renderStatus(status) {
  setText("gateway-mode", status.mode);
  document.getElementById("gateway-mode").className = "pill ok";
  setPill("server-state", status.server_connected, status.server_connected ? "connected" : "offline");
  setPill("pairing-state", status.pairing_payload_ready, status.pairing_payload_ready ? "ready" : "not ready");
  setText("pair-expiry", formatTime(status.pair_token_expires_at));
  setText("device-name", status.device_name);
  setText("agent-id", status.agent_id);
  setText("active-server-url", status.server_url);
  setText("config-path", status.config_path);
  setText("saved-server-url", status.saved_server_url || "-");
  setText("server-source", status.server_url_source);
  document.getElementById("server-source").className =
    status.server_url_source === "cli" || status.server_url_source === "env" ? "pill warn" : "pill ok";
  if (document.activeElement !== serverInput && !serverInput.dataset.dirty) {
    serverInput.value = status.saved_server_url || status.server_url || "";
  }
  setText("metric-server", status.server_connected ? "已连接" : "未连接");
  setText("metric-auth", status.authenticated ? "已认证" : "未认证");
  setText("metric-ws", status.agent_ws_connected ? "已连接" : "未连接");
  setText("metric-codex", status.codex_app_server_running ? "运行中" : "未启动");
  setText("started-at", formatTime(status.started_at));
  setText("state-root", status.state_root);
  setText("ws-error", status.agent_ws_last_error || "-");
  if (!configMessage.dataset.locked) {
    const restartHint = status.server_url === status.saved_server_url
      ? "当前运行配置与配置文件一致。"
      : "配置变更需要重启 Gateway 后生效。";
    setNotice(restartHint);
  }
  renderFileAccess(status.file_access);
}

function renderPairings(devices) {
  setText("pairing-count", String(devices.length));
  if (!devices.length) {
    pairingsElement.innerHTML = `<div class="empty-state">暂无本地配对记录</div>`;
    return;
  }
  pairingsElement.innerHTML = devices.map((device) => `
    <div class="pairing-row">
      <div class="pairing-device">
        <strong>${escapeHTML(compactDeviceName(device.device_id))}</strong>
        <span class="mono">${escapeHTML(device.device_id)}</span>
      </div>
      <div class="pairing-binding mono">${escapeHTML(device.binding_id)}</div>
      <div class="pairing-time">${formatTime(device.paired_at)}</div>
      <button
        class="icon-button danger-icon"
        data-delete-pairing="${escapeHTML(device.binding_id)}"
        data-device-id="${escapeHTML(device.device_id)}"
        aria-label="删除 ${escapeHTML(device.binding_id)}"
        title="删除配对"
      >
        <svg viewBox="0 0 24 24" aria-hidden="true" focusable="false">
          <path d="M3 6h18"></path>
          <path d="M8 6V4h8v2"></path>
          <path d="M19 6l-1 14H6L5 6"></path>
          <path d="M10 11v5"></path>
          <path d="M14 11v5"></path>
        </svg>
        <span class="sr-only">删除</span>
      </button>
    </div>
  `).join("");
}

function escapeHTML(value) {
  return String(value).replace(/[&<>"']/g, (char) => ({
    "&": "&amp;",
    "<": "&lt;",
    ">": "&gt;",
    "\"": "&quot;",
    "'": "&#39;",
  }[char]));
}

async function refresh(force = false) {
  if (force) await fetchText("/api/pairing/refresh", { method: "POST" });
  const [status, pairings] = await Promise.all([
    fetchJSON("/api/status"),
    fetchJSON("/api/pairings"),
  ]);
  renderStatus(status);
  renderPairings(pairings);
  document.getElementById("status").textContent = pretty(status);
  try {
    const payload = await fetchJSON("/api/pairing/payload");
    document.getElementById("payload").textContent = pretty(payload);
  } catch (error) {
    document.getElementById("payload").textContent = error.message;
  }
  document.getElementById("qr").src = "/api/pairing/qr.svg?ts=" + Date.now();
}

async function withButton(button, task) {
  button.disabled = true;
  try {
    await task();
  } finally {
    button.disabled = false;
  }
}

function confirmDeletePairing(deviceID, bindingID) {
  if (!confirmDialog?.showModal) {
    return Promise.resolve(window.confirm(`删除 ${deviceID} 的配对绑定？`));
  }
  confirmDevice.textContent = deviceID || "-";
  confirmBinding.textContent = bindingID || "-";
  return new Promise((resolve) => {
    const settle = () => {
      confirmDialog.removeEventListener("close", settle);
      resolve(confirmDialog.returnValue === "confirm");
    };
    confirmDialog.returnValue = "";
    confirmDialog.addEventListener("close", settle);
    confirmDialog.showModal();
  });
}

async function deletePairing(button) {
  const bindingID = button.dataset.deletePairing;
  const deviceID = button.dataset.deviceId;
  if (!bindingID) return;
  const confirmed = await confirmDeletePairing(deviceID, bindingID);
  if (!confirmed) return;
  await withButton(button, async () => {
    setPairingNotice("正在撤销服务端绑定...");
    const result = await fetchJSON(`/api/pairings/${encodeURIComponent(bindingID)}`, {
      method: "DELETE",
    });
    const serverText = result.server_revoked ? "服务端已撤销" : "服务端已无 active 绑定";
    const localText = result.local_removed ? "本地记录已删除" : "本地记录不存在";
    setPairingNotice(`${serverText}，${localText}。`, "ok");
    await refresh(false);
  });
}

serverInput.addEventListener("input", () => {
  serverInput.dataset.dirty = "true";
  configMessage.dataset.locked = "";
});

for (const input of [fileAccessRootsInput, fileAccessPrecheckTimeoutInput, fileAccessReadTimeoutInput]) {
  input.addEventListener("input", () => {
    input.dataset.dirty = "true";
    fileAccessMessage.dataset.locked = "";
  });
}

document.getElementById("refresh").addEventListener("click", (event) => {
  withButton(event.currentTarget, () => refresh(true).catch((error) => setNotice(error.message, "bad")));
});

document.getElementById("test-server").addEventListener("click", (event) => {
  withButton(event.currentTarget, async () => {
    configMessage.dataset.locked = "true";
    setNotice("正在测试连接...");
    const result = await fetchJSON("/api/config/server-url/test", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ server_url: serverInput.value.trim() }),
    });
    setNotice(`${result.server_url} 可连接。`, "ok");
  }).catch((error) => setNotice(error.message, "bad"));
});

document.getElementById("save-server").addEventListener("click", (event) => {
  withButton(event.currentTarget, async () => {
    configMessage.dataset.locked = "true";
    const result = await fetchJSON("/api/config/server-url", {
      method: "PUT",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ server_url: serverInput.value.trim() }),
    });
    serverInput.dataset.dirty = "";
    const override = result.saved_config_overridden_on_restart
      ? " 当前启动参数会覆盖配置文件。"
      : "";
    setNotice(`已保存，重启 Gateway 后生效。${override}`, "ok");
    await refresh(false);
  }).catch((error) => setNotice(error.message, "bad"));
});

document.getElementById("rerun-file-access").addEventListener("click", (event) => {
  withButton(event.currentTarget, async () => {
    fileAccessMessage.dataset.locked = "true";
    setFileAccessNotice("正在重新预检查...");
    await fetchJSON("/api/file-access/precheck", { method: "POST" });
    await refresh(false);
  }).catch((error) => setFileAccessNotice(error.message, "bad"));
});

document.getElementById("save-file-access").addEventListener("click", (event) => {
  withButton(event.currentTarget, async () => {
    fileAccessMessage.dataset.locked = "true";
    setFileAccessNotice("正在保存并预检查...");
    const result = await fetchJSON("/api/config/file-access", {
      method: "PUT",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({
        precheck_roots: fileAccessRootsInput.value.trim(),
        precheck_timeout_seconds: Number(fileAccessPrecheckTimeoutInput.value || 60),
        read_timeout_seconds: Number(fileAccessReadTimeoutInput.value || 10),
      }),
    });
    fileAccessRootsInput.dataset.dirty = "";
    fileAccessPrecheckTimeoutInput.dataset.dirty = "";
    fileAccessReadTimeoutInput.dataset.dirty = "";
    setFileAccessNotice(`已保存到 ${result.config_path}，正在后台预检查。`, "ok");
    await refresh(false);
  }).catch((error) => setFileAccessNotice(error.message, "bad"));
});

pairingsElement.addEventListener("click", (event) => {
  const button = event.target.closest("[data-delete-pairing]");
  if (!button) return;
  deletePairing(button).catch((error) => setPairingNotice(error.message, "bad"));
});

refresh(false);
setInterval(() => refresh(false).catch((error) => setNotice(error.message, "bad")), 5000);
