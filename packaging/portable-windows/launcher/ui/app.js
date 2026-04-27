// Hermes portable launcher — front-end glue.
//
// Communicates with the Rust backend via Tauri's invoke() bridge. The Rust
// side never returns localized strings — only message keys + params — so
// translation lives entirely here.

const { invoke } = window.__TAURI__.tauri;
const { listen } = window.__TAURI__.event;

let LOCALE = {};
let dashboardInfo = null;

async function loadLocale() {
  try {
    const res = await fetch("locales/zh-CN.json");
    LOCALE = await res.json();
  } catch (e) {
    console.error("locale load failed", e);
    LOCALE = {};
  }
  // Apply data-i18n attributes so static markup is also localized.
  document.querySelectorAll("[data-i18n]").forEach((el) => {
    const key = el.getAttribute("data-i18n");
    if (LOCALE[key]) el.textContent = LOCALE[key];
  });
}

function t(key, params = {}) {
  const tmpl = LOCALE[key] || key;
  return tmpl.replace(/\{(\w+)\}/g, (_, k) =>
    params[k] !== undefined ? String(params[k]) : `{${k}}`
  );
}

const stageIcons = {
  self_check: "🔍",
  importing: "📦",
  booting: "🚀",
  ready: "✅",
  stopping: "🛑",
  error: "⚠️",
};

function setProgress(stage, percent, messageKey, params) {
  document.getElementById("stage-icon").textContent = stageIcons[stage] || "⏳";
  document.getElementById("stage-title").textContent = t(`stage_${stage}`);
  document.getElementById("stage-detail").textContent = t(messageKey, params);
  const bar = document.getElementById("bar");
  if (percent < 0) {
    bar.classList.add("indeterminate");
  } else {
    bar.classList.remove("indeterminate");
    bar.style.width = Math.max(0, Math.min(100, percent)) + "%";
  }
}

function showReady() {
  document.getElementById("progress-card").classList.add("hidden");
  document.getElementById("error-card").classList.add("hidden");
  document.getElementById("ready-card").classList.remove("hidden");
}

function showError(detail) {
  document.getElementById("progress-card").classList.add("hidden");
  document.getElementById("ready-card").classList.add("hidden");
  document.getElementById("error-card").classList.remove("hidden");
  document.getElementById("error-detail").textContent = detail;
}

async function startup() {
  await loadLocale();
  setProgress("self_check", 5, "self_check_running");

  try {
    dashboardInfo = await invoke("ensure_and_boot");
  } catch (e) {
    showError(String(e));
    return;
  }
  showReady();
  // Open the chat in the user's default browser immediately.
  invoke("open_chat", { info: dashboardInfo }).catch((e) =>
    console.error("open_chat failed", e)
  );
  // Auto-hide to tray after 3s so the launcher window doesn't stay in the way.
  setTimeout(() => window.__TAURI__.window.appWindow.hide(), 3000);
}

document.getElementById("open-chat-btn").addEventListener("click", () => {
  if (dashboardInfo) invoke("open_chat", { info: dashboardInfo });
});
document.getElementById("hide-btn").addEventListener("click", () =>
  window.__TAURI__.window.appWindow.hide()
);
document.getElementById("retry-btn").addEventListener("click", startup);
document.getElementById("logs-btn").addEventListener("click", () => {
  invoke("open_logs").catch((e) => console.error("open_logs failed", e));
});

listen("hermes://progress", (e) => {
  const p = e.payload;
  setProgress(p.stage, p.percent, p.message_key, p.params || {});
});

listen("hermes://tray", (e) => {
  if (e.payload === "open_chat" && dashboardInfo) {
    invoke("open_chat", { info: dashboardInfo });
  } else if (e.payload === "safe_exit") {
    invoke("safe_exit", { unregister: false });
  }
});

window.addEventListener("DOMContentLoaded", startup);
