//! USB-stick path resolution.
//!
//! The launcher binary lives at `<USB>\launcher\HermesLauncher.exe` (or
//! `<USB>\HermesLauncher.exe` if shipped flat).  Either way, we walk up
//! from `current_exe()` until we find the marker file `runtime\HermesPortable.tar.zst`
//! and treat that ancestor as the USB root.

use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context, Result};

#[derive(Debug, Clone)]
pub struct Paths {
    /// e.g. `D:\` — the root of the USB stick.
    pub usb_root: PathBuf,
}

impl Paths {
    pub fn detect() -> Result<Self> {
        let exe = std::env::current_exe().context("current_exe")?;
        let mut cur: &Path = &exe;
        while let Some(parent) = cur.parent() {
            if parent.join("runtime").join("HermesPortable.tar.zst").exists()
                || parent.join("runtime").exists() && parent.join("data").exists()
            {
                return Ok(Paths {
                    usb_root: parent.to_path_buf(),
                });
            }
            cur = parent;
        }
        Err(anyhow!(
            "could not locate the Hermes USB root from {} — \
             please run HermesLauncher.exe directly from the USB stick",
            exe.display()
        ))
    }

    /// Per-USB unique distro name.  We embed a 4-byte hash of the USB root
    /// path so that two sticks plugged into the same Windows machine don't
    /// collide on the same distro registration.
    pub fn distro_name(&self) -> String {
        let key = self.usb_root.to_string_lossy();
        format!("HermesPortable_{}", crate::short_hash(&key))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn distro_name_is_deterministic_per_path() {
        let a = Paths {
            usb_root: PathBuf::from("D:\\"),
        };
        let b = Paths {
            usb_root: PathBuf::from("D:\\"),
        };
        let c = Paths {
            usb_root: PathBuf::from("E:\\"),
        };
        assert_eq!(a.distro_name(), b.distro_name());
        assert_ne!(a.distro_name(), c.distro_name());
        assert!(a.distro_name().starts_with("HermesPortable_"));
        assert_eq!(a.distro_name().len(), "HermesPortable_".len() + 8);
    }
}
