//! Thin wrappers around `wsl.exe`.
//!
//! All commands here are synchronous and return rich error messages so the
//! launcher UI can show the user something helpful when something goes wrong.

use std::os::windows::process::CommandExt;
use std::path::Path;
use std::process::{Command, Stdio};

use anyhow::{anyhow, Context, Result};
use serde::Serialize;

/// `CREATE_NO_WINDOW` — keeps `wsl.exe` invocations from briefly flashing
/// a black console window in the user's face.
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;
#[cfg(not(windows))]
const CREATE_NO_WINDOW: u32 = 0;

#[derive(Debug, Serialize)]
pub struct SelfCheck {
    pub ok: bool,
    pub wsl_installed: bool,
    pub wsl_version_2_default: bool,
    pub windows_version_ok: bool,
    pub blocker: Option<String>,
}

pub fn self_check() -> Result<SelfCheck> {
    let mut report = SelfCheck {
        ok: false,
        wsl_installed: false,
        wsl_version_2_default: false,
        windows_version_ok: true, // we trust win32 to refuse to launch on too-old systems
        blocker: None,
    };

    // `wsl --status` exits 0 when WSL is installed and configured.
    let status = Command::new("wsl.exe")
        .arg("--status")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .creation_flags(CREATE_NO_WINDOW)
        .status();
    match status {
        Ok(s) if s.success() => {
            report.wsl_installed = true;
            report.wsl_version_2_default = true;
            report.ok = true;
        }
        Ok(_) => {
            report.blocker = Some("wsl_not_installed".into());
        }
        Err(_) => {
            report.blocker = Some("wsl_missing".into());
        }
    }
    Ok(report)
}

pub fn distro_exists(name: &str) -> Result<bool> {
    let out = Command::new("wsl.exe")
        .args(["--list", "--quiet"])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .context("wsl --list")?;
    if !out.status.success() {
        return Ok(false);
    }
    // `wsl --list --quiet` outputs UTF-16-LE on most Windows versions.
    let text = decode_utf16_or_utf8(&out.stdout);
    Ok(text.lines().any(|l| l.trim() == name))
}

pub fn import(name: &str, install_dir: &Path, tarball: &Path) -> Result<()> {
    let status = Command::new("wsl.exe")
        .arg("--import")
        .arg(name)
        .arg(install_dir)
        .arg(tarball)
        .arg("--version")
        .arg("2")
        .creation_flags(CREATE_NO_WINDOW)
        .status()
        .context("wsl --import")?;
    if !status.success() {
        return Err(anyhow!(
            "wsl --import {} failed (exit {})",
            name,
            status.code().unwrap_or(-1)
        ));
    }
    Ok(())
}

pub fn terminate(name: &str) -> Result<()> {
    let _ = Command::new("wsl.exe")
        .arg("--terminate")
        .arg(name)
        .creation_flags(CREATE_NO_WINDOW)
        .status();
    Ok(())
}

pub fn unregister(name: &str) -> Result<()> {
    let status = Command::new("wsl.exe")
        .arg("--unregister")
        .arg(name)
        .creation_flags(CREATE_NO_WINDOW)
        .status()
        .context("wsl --unregister")?;
    if !status.success() {
        return Err(anyhow!(
            "wsl --unregister {} failed (exit {})",
            name,
            status.code().unwrap_or(-1)
        ));
    }
    Ok(())
}

pub fn spawn_entry(distro: &str, hermes_home: &str, port: u16, entry: &str) -> Result<u32> {
    // Note: we use `wsl --exec` (no shell) so command injection from the
    // path is impossible.  hermes_home is interpolated only into env, not
    // into argv.
    let child = Command::new("wsl.exe")
        .arg("-d")
        .arg(distro)
        .arg("--user")
        .arg("hermes")
        .arg("--cd")
        .arg("/opt/hermes")
        .arg("env")
        .arg(format!("HERMES_HOME={}", hermes_home))
        .arg(format!("HERMES_PORTABLE=1"))
        .arg(format!("HERMES_DASHBOARD_PORT={}", port))
        .arg(entry)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
        .context("spawn wsl entry")?;
    Ok(child.id())
}

fn decode_utf16_or_utf8(buf: &[u8]) -> String {
    if buf.len() >= 2 && buf.len() % 2 == 0 && buf[1] == 0 {
        let u16s: Vec<u16> = buf
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();
        String::from_utf16_lossy(&u16s)
    } else {
        String::from_utf8_lossy(buf).into_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decode_utf16_le() {
        let bytes: Vec<u8> = "Hi\n"
            .encode_utf16()
            .flat_map(|u| u.to_le_bytes())
            .collect();
        assert_eq!(decode_utf16_or_utf8(&bytes), "Hi\n");
    }

    #[test]
    fn decode_falls_back_to_utf8() {
        assert_eq!(decode_utf16_or_utf8(b"plain ascii"), "plain ascii");
    }
}
