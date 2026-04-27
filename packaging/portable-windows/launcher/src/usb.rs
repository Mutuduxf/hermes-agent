//! Safe-eject helper for the USB stick the launcher lives on.
//!
//! On Windows we issue `mountvol <letter>: /p` which flushes caches and
//! marks the volume as ready to remove.  This is **best-effort** — if the
//! user has files open on the stick from elsewhere it'll fail, and we
//! report that back rather than silently wedging the launcher.

use std::os::windows::process::CommandExt;
use std::path::Path;
use std::process::Command;

use anyhow::{anyhow, Result};

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;
#[cfg(not(windows))]
const CREATE_NO_WINDOW: u32 = 0;

pub fn eject(usb_root: &Path) -> Result<()> {
    let drive = usb_root
        .to_string_lossy()
        .chars()
        .next()
        .ok_or_else(|| anyhow!("cannot read drive letter from {}", usb_root.display()))?;
    let arg = format!("{}:", drive.to_ascii_uppercase());
    let status = Command::new("mountvol.exe")
        .arg(&arg)
        .arg("/p")
        .creation_flags(CREATE_NO_WINDOW)
        .status()?;
    if !status.success() {
        return Err(anyhow!(
            "mountvol {} /p failed (exit {})",
            arg,
            status.code().unwrap_or(-1)
        ));
    }
    Ok(())
}
