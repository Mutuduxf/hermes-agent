//! Hermes Agent — Windows USB-portable GUI launcher.
//!
//! This binary lives on the user's USB stick (`<USB>\launcher\HermesLauncher.exe`)
//! and is the single entry-point the non-technical end user double-clicks.
//!
//! Flow (see packaging/portable-windows/README.md for the long version):
//!
//!  1. `self_check`  — verify Win10 21H2+, WSL enabled, WSL2 default.
//!  2. `ensure_distro` — first run only: extract the bundled rootfs and
//!     `wsl --import` it onto the USB stick (vhdx kept on the stick).
//!  3. `boot_hermes`   — `wsl -d <distro> -- /usr/local/bin/hermes-portable-entry`,
//!     wait for `data\cache\dashboard-token.txt`, open default browser
//!     to the chat URL.
//!  4. `tray`          — minimize to system tray; menu items dispatch to the
//!     other commands (open chat, settings, restart, safe-eject, quit).
//!  5. `safe_exit`     — `wsl --terminate <distro>` (and optionally
//!     `--unregister`), eject the USB stick, exit.
//!
//! All user-visible strings come from `ui/locales/zh-CN.json`. The Rust side
//! never prints English to the user; it only emits structured JSON events
//! that the HTML UI renders with the localized strings.

#![cfg_attr(all(not(debug_assertions), target_os = "windows"), windows_subsystem = "windows")]

use std::process::Command;
use std::sync::Mutex;
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tauri::{
    AppHandle, CustomMenuItem, Manager, RunEvent, SystemTray, SystemTrayEvent, SystemTrayMenu,
    SystemTrayMenuItem, WindowEvent,
};

mod paths;
mod usb;
mod wsl;

use paths::Paths;

/// Held while a `wsl` child is alive so the tray can ask "is it running?"
static HERMES_PID: Lazy<Mutex<Option<u32>>> = Lazy::new(|| Mutex::new(None));

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "snake_case")]
#[allow(dead_code)] // variants used by the JS UI via serde, not by Rust
enum Stage {
    SelfCheck,
    Importing,
    Booting,
    Ready,
    Stopping,
    Error,
}

#[derive(Debug, Serialize, Clone)]
struct Progress {
    stage: Stage,
    /// 0–100, or -1 for indeterminate.
    percent: i32,
    /// Non-localized identifier; the UI maps it to a string.
    message_key: String,
    /// Free-form parameters passed to message-format placeholders.
    params: serde_json::Value,
}

impl Progress {
    fn new(stage: Stage, percent: i32, key: &str) -> Self {
        Progress {
            stage,
            percent,
            message_key: key.to_string(),
            params: serde_json::json!({}),
        }
    }
    /// Future hook: attach format params for the JS-side message renderer.
    /// Currently unused — silenced to keep the builder API on hand.
    #[allow(dead_code)]
    fn with_params(mut self, params: serde_json::Value) -> Self {
        self.params = params;
        self
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct DashboardInfo {
    url: String,
    token: String,
}

// ────────────────────────── Tauri commands (called from JS) ──────────────────

#[tauri::command]
async fn run_self_check() -> Result<serde_json::Value, String> {
    let report = wsl::self_check().map_err(|e| e.to_string())?;
    Ok(serde_json::to_value(report).unwrap())
}

#[tauri::command]
async fn ensure_and_boot(app: AppHandle) -> Result<DashboardInfo, String> {
    inner_ensure_and_boot(app).await.map_err(|e| {
        eprintln!("ensure_and_boot failed: {e:?}");
        e.to_string()
    })
}

async fn inner_ensure_and_boot(app: AppHandle) -> Result<DashboardInfo> {
    let paths = Paths::detect().context("detect paths")?;
    let distro_name = paths.distro_name();

    let emit = |p: Progress| {
        let _ = app.emit_all("hermes://progress", &p);
    };

    emit(Progress::new(Stage::SelfCheck, 5, "self_check_running"));
    let report = wsl::self_check()?;
    if !report.ok {
        return Err(anyhow!(
            "WSL self-check failed: {}",
            report.blocker.unwrap_or_else(|| "unknown".into())
        ));
    }

    if !wsl::distro_exists(&distro_name)? {
        emit(Progress::new(Stage::Importing, 10, "importing_first_run"));
        ensure_distro(&paths, &distro_name, &emit).context("import portable rootfs")?;
    }

    emit(Progress::new(Stage::Booting, 70, "booting_hermes"));
    let port = pick_free_port().unwrap_or(9119);
    boot_hermes(&paths, &distro_name, port).context("start hermes inside WSL")?;

    emit(Progress::new(Stage::Booting, 85, "waiting_for_dashboard"));
    let info = wait_for_dashboard(&paths, Duration::from_secs(45))
        .context("dashboard did not respond within 45s")?;

    emit(Progress::new(Stage::Ready, 100, "ready"));
    Ok(info)
}

#[tauri::command]
async fn open_chat(info: DashboardInfo) -> Result<(), String> {
    open_in_browser(&format!("{}/chat?token={}", info.url, info.token))
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn safe_exit(unregister: bool) -> Result<(), String> {
    let paths = Paths::detect().map_err(|e| e.to_string())?;
    let distro = paths.distro_name();
    if let Err(e) = wsl::terminate(&distro) {
        eprintln!("terminate failed (ignored): {e:?}");
    }
    if unregister {
        if let Err(e) = wsl::unregister(&distro) {
            eprintln!("unregister failed (ignored): {e:?}");
        }
    }
    if let Err(e) = usb::eject(&paths.usb_root) {
        eprintln!("usb eject failed (ignored): {e:?}");
    }
    Ok(())
}

#[tauri::command]
async fn open_logs() -> Result<(), String> {
    let paths = Paths::detect().map_err(|e| e.to_string())?;
    let log_dir = paths.usb_root.join("data").join("logs");
    if let Err(e) = std::fs::create_dir_all(&log_dir) {
        return Err(format!("create log dir: {e}"));
    }
    Command::new("explorer.exe")
        .arg(&log_dir)
        .spawn()
        .map_err(|e| format!("open explorer: {e}"))?;
    Ok(())
}

// ─────────────────────────── Helpers ─────────────────────────────────────────

fn ensure_distro<F>(paths: &Paths, distro: &str, emit: &F) -> Result<()>
where
    F: Fn(Progress),
{
    // 1. Decompress the bundled .tar.zst into %LOCALAPPDATA%\Temp.
    let temp_tar = std::env::temp_dir().join(format!("{}-rootfs.tar", distro));
    emit(Progress::new(Stage::Importing, 25, "decompressing_rootfs"));
    let zstd = paths.usb_root.join("launcher\\ext\\zstd.exe");
    let rootfs = paths.usb_root.join("runtime\\HermesPortable.tar.zst");
    if !rootfs.exists() {
        return Err(anyhow!(
            "rootfs missing: {} — the USB stick may be corrupted",
            rootfs.display()
        ));
    }
    let status = Command::new(&zstd)
        .args(["-d", "-f"])
        .arg(&rootfs)
        .arg("-o")
        .arg(&temp_tar)
        .status()
        .with_context(|| format!("invoke zstd at {}", zstd.display()))?;
    if !status.success() {
        return Err(anyhow!("zstd failed (exit {})", status.code().unwrap_or(-1)));
    }

    // 2. Create the per-USB vhdx folder ON the stick (not on the host disk).
    let vhd_dir = paths.usb_root.join("runtime\\wsl-vhd");
    std::fs::create_dir_all(&vhd_dir).context("create wsl-vhd dir on USB stick")?;

    // 3. wsl --import.
    emit(Progress::new(Stage::Importing, 55, "importing_to_wsl"));
    wsl::import(distro, &vhd_dir, &temp_tar).context("wsl --import")?;

    // 4. Cleanup tmp tar — the vhdx now contains everything.
    let _ = std::fs::remove_file(&temp_tar);
    Ok(())
}

fn boot_hermes(paths: &Paths, distro: &str, port: u16) -> Result<()> {
    // Translate the Windows USB drive letter (e.g. "D:\") to the WSL
    // automount path "/mnt/d".  This is what HERMES_HOME points at.
    let drive_letter = paths
        .usb_root
        .to_string_lossy()
        .chars()
        .next()
        .ok_or_else(|| anyhow!("cannot determine USB drive letter"))?
        .to_ascii_lowercase();
    let hermes_home = format!("/mnt/{}/data", drive_letter);

    let pid = wsl::spawn_entry(
        distro,
        &hermes_home,
        port,
        "/usr/local/bin/hermes-portable-entry",
    )?;
    *HERMES_PID.lock().unwrap() = Some(pid);
    Ok(())
}

fn wait_for_dashboard(paths: &Paths, timeout: Duration) -> Result<DashboardInfo> {
    let token_path = paths.usb_root.join("data\\cache\\dashboard-token.txt");
    let deadline = std::time::Instant::now() + timeout;
    loop {
        if token_path.exists() {
            if let Ok(content) = std::fs::read_to_string(&token_path) {
                if let Some(info) = parse_token_file(&content) {
                    return Ok(info);
                }
            }
        }
        if std::time::Instant::now() > deadline {
            return Err(anyhow!(
                "timed out waiting for {}",
                token_path.display()
            ));
        }
        std::thread::sleep(Duration::from_millis(400));
    }
}

fn parse_token_file(s: &str) -> Option<DashboardInfo> {
    let mut url = None;
    let mut token = None;
    for line in s.lines() {
        if let Some(v) = line.strip_prefix("url=") {
            url = Some(v.trim().to_string());
        } else if let Some(v) = line.strip_prefix("token=") {
            token = Some(v.trim().to_string());
        }
    }
    Some(DashboardInfo {
        url: url?,
        token: token?,
    })
}

fn pick_free_port() -> Option<u16> {
    use std::net::TcpListener;
    TcpListener::bind("127.0.0.1:0")
        .ok()
        .and_then(|l| l.local_addr().ok())
        .map(|a| a.port())
}

fn open_in_browser(url: &str) -> Result<()> {
    let status = Command::new("cmd")
        .args(["/C", "start", "", url])
        .status()
        .context("invoke cmd /C start")?;
    if !status.success() {
        return Err(anyhow!(
            "browser launch failed (exit {})",
            status.code().unwrap_or(-1)
        ));
    }
    Ok(())
}

// ─────────────────────────── Tray + App ──────────────────────────────────────

fn build_tray() -> SystemTray {
    let menu = SystemTrayMenu::new()
        .add_item(CustomMenuItem::new("open_chat", "🟢 打开聊天界面"))
        .add_item(CustomMenuItem::new("status", "📊 查看状态"))
        .add_native_item(SystemTrayMenuItem::Separator)
        .add_item(CustomMenuItem::new("settings", "🔑 设置 API 密钥…"))
        .add_item(CustomMenuItem::new("logs", "📝 查看日志"))
        .add_native_item(SystemTrayMenuItem::Separator)
        .add_item(CustomMenuItem::new("safe_exit", "❌ 安全退出并弹出 U 盘"));
    SystemTray::new().with_menu(menu)
}

fn main() {
    env_logger::init();

    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            run_self_check,
            ensure_and_boot,
            open_chat,
            open_logs,
            safe_exit
        ])
        .system_tray(build_tray())
        .on_system_tray_event(|app, event| match event {
            SystemTrayEvent::MenuItemClick { id, .. } => match id.as_str() {
                "open_chat" => {
                    app.emit_all("hermes://tray", "open_chat").ok();
                }
                "status" => {
                    if let Some(w) = app.get_window("main") {
                        let _ = w.show();
                        let _ = w.set_focus();
                    }
                }
                "settings" => {
                    app.emit_all("hermes://tray", "settings").ok();
                    if let Some(w) = app.get_window("main") {
                        let _ = w.show();
                    }
                }
                "logs" => {
                    if let Ok(p) = Paths::detect() {
                        let log = p.usb_root.join("data\\logs");
                        let _ = Command::new("explorer").arg(&log).spawn();
                    }
                }
                "safe_exit" => {
                    app.emit_all("hermes://tray", "safe_exit").ok();
                }
                _ => {}
            },
            _ => {}
        })
        .on_window_event(|event| {
            if let WindowEvent::CloseRequested { api, .. } = event.event() {
                // Don't kill the launcher when the user closes the window —
                // hermes is still running in WSL.  Hide to tray instead.
                api.prevent_close();
                let _ = event.window().hide();
            }
        })
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app, event| {
            if let RunEvent::ExitRequested { api, .. } = event {
                // Keep the process alive until the tray "Safe exit" path runs.
                let _ = app;
                api.prevent_exit();
            }
        });
}

// Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_token_happy_path() {
        let s = "url=http://127.0.0.1:42777\ntoken=abc.def\npid=1234\n";
        let info = parse_token_file(s).unwrap();
        assert_eq!(info.url, "http://127.0.0.1:42777");
        assert_eq!(info.token, "abc.def");
    }

    #[test]
    fn parse_token_missing_token_returns_none() {
        let s = "url=http://127.0.0.1:42777\npid=1234\n";
        assert!(parse_token_file(s).is_none());
    }

    #[test]
    fn parse_token_extra_whitespace() {
        let s = "url=  http://127.0.0.1:1\ntoken=  xyz   \n";
        let info = parse_token_file(s).unwrap();
        assert_eq!(info.url, "http://127.0.0.1:1");
        assert_eq!(info.token, "xyz");
    }
}

// Hash helper kept here so paths.rs and usb.rs can both import it without a
// separate "util" module.
pub(crate) fn short_hash(s: &str) -> String {
    let mut h = Sha256::new();
    h.update(s.as_bytes());
    let digest = h.finalize();
    digest.iter().take(4).map(|b| format!("{:02x}", b)).collect()
}
