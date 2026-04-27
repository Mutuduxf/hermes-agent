# Hermes Agent — Windows USB-portable distribution

This subdirectory holds everything needed to build a USB-stick that a
**non-technical Windows end user** can plug in, double-click once, and
get a working Hermes chat window in their browser — without ever seeing
a terminal, WSL prompt, or `pip install` command.

---

## 1. Why this exists

Hermes is POSIX-first (`ptyprocess`, SQLite WAL, `bash` launchers,
Playwright Chromium under Linux paths, …). The README explicitly says
"Native Windows is not supported, please install WSL2." Asking a
non-technical user to install WSL2, set up a venv, and edit `.env` is
not realistic.

The distribution here is **the only form factor** we found that
simultaneously satisfies:

* Windows-only end user
* Non-technical end user
* Portable (lives on a USB stick, follows the user across machines)

For the full design rationale and the alternatives we ruled out (pure
embeddable Python, PyInstaller, Docker Desktop, MSIX, Cygwin), see the
plan in the issue / PR description that introduced this directory.

---

## 2. Architecture

```
┌─────────────────────────────────────────────────────────┐
│ Layer 3 — GUI Launcher (Tauri / Rust)                   │
│  · Chinese-localized first-run wizard, tray, safe-eject │
│  · Detects WSL2, runs `wsl --import` on first boot      │
│  · Opens default browser to dashboard chat URL          │
└─────────────────────────────────────────────────────────┘
                       │ wsl.exe --import / --exec
                       ▼
┌─────────────────────────────────────────────────────────┐
│ Layer 2 — Portable WSL2 distro (Debian 13-slim)         │
│  · Python 3.11 venv with hermes + extras pre-installed  │
│  · Node.js + ui-tui + web SPA pre-built                 │
│  · Playwright Chromium (--only-shell)                   │
│  · /etc/wsl.conf bakes in metadata mounts + UID 1000    │
│  · /usr/local/bin/hermes-portable-entry — entrypoint    │
└─────────────────────────────────────────────────────────┘
                       │ HERMES_HOME=/mnt/<drive>/data
                       ▼
┌─────────────────────────────────────────────────────────┐
│ Layer 1 — Hermes Agent (existing, ~near-zero changes)   │
│  · `hermes dashboard --port <dynamic> --no-open --tui`  │
│  · is_portable() → writes token to                      │
│    $HERMES_HOME/cache/dashboard-token.txt               │
│  · graceful WAL TRUNCATE checkpoint on shutdown         │
└─────────────────────────────────────────────────────────┘
```

Sole files modified in the main repo (kept very small on purpose):

* `hermes_constants.py` — new `is_portable()` helper.
* `hermes_cli/web_server.py` — portable-mode token-file write +
  graceful SQLite checkpoint.
* `hermes_cli/banner.py` — show "Portable · data on USB" line.
* `tests/test_hermes_constants.py` — extended.
* `tests/hermes_cli/test_web_server_portable.py` — new.

---

## 3. USB stick layout (end-user view)

```
HermesUSB/                           ← exFAT or NTFS, never FAT32 (SQLite WAL hates FAT32)
├── 🚀 启动 Hermes 助手.exe          ← single user-visible icon
├── 📖 使用说明.pdf                   ← one A4 page with screenshots
├── 🆘 问题排查.txt                   ← FAQ (see usb-template/问题排查.txt)
├── README.txt                       ← see usb-template/README.txt
├── autorun.inf                      ← drive icon + label only
├── runtime\
│   ├── HermesPortable.tar.zst       ← rootfs, ~1.5 GB
│   ├── version.txt                  ← built_at + rootfs_version
│   ├── HermesPortable.tar.zst.sha256
│   └── wsl-vhd\                     ← created on first boot, holds ext4.vhdx
├── launcher\
│   ├── HermesLauncher.exe           ← (the .exe in the root is a copy of this)
│   ├── icons\, ui\, locales\
│   └── ext\
│       └── zstd.exe                 ← bundled to decompress the rootfs
└── data\                            ← user data — survives across machines
    ├── config.yaml, .env (encrypted), state.db
    ├── sessions\, skills\, memory\, plugins\, profiles\, logs\, cache\
```

---

## 4. Build

### 4.1 Rootfs (Linux x86_64, needs Docker)

```sh
bash scripts/build_portable_windows.sh
# → dist/portable-windows/HermesPortable.tar.zst (~1.5 GB)
```

Behind the scenes: spins up a `debian:13-slim` container, installs
`python3.11`, `nodejs`, `uv`, `ripgrep`, `ffmpeg`, copies the repo in,
creates the venv, runs `npm install` + builds for both `web/` and
`ui-tui/`, installs Playwright Chromium-shell, bakes `/etc/wsl.conf`
and `/usr/local/bin/hermes-portable-entry` from `skel/`, then tar+zstd
the whole tree.

### 4.2 Launcher (Windows x86_64, needs Tauri toolchain)

```cmd
cd packaging\portable-windows\launcher
cargo install tauri-cli --version "^1.6"
cargo tauri build --target x86_64-pc-windows-msvc --bundles nsis
```

Output: `target\release\HermesLauncher.exe` (and an NSIS installer,
which we **do not ship** — we want users to drag-and-drop, not run an
installer).

### 4.3 Pack the USB image

The CI workflow assembles the final folder structure (see
`.github/workflows/portable-windows.yml`):

```
HermesPortable-Windows-v<ver>.zip
  ├── 🚀 启动 Hermes 助手.exe       (= HermesLauncher.exe, renamed)
  ├── README.txt, 问题排查.txt, autorun.inf  (from usb-template/)
  ├── runtime\HermesPortable.tar.zst
  ├── runtime\HermesPortable.tar.zst.sha256
  ├── runtime\version.txt
  └── launcher\HermesLauncher.exe + icons + ui + ext\zstd.exe
```

The user downloads the zip, extracts it to the **root** of a USB stick,
and is done.

---

## 5. End-user experience contract

These are non-negotiable. Every change to this directory must preserve
all of them:

* **Single icon** in the USB root. Nothing else looks like an entry-point.
* **No console window** ever flashes (we use `windows_subsystem = "windows"`
  on the launcher and `CREATE_NO_WINDOW` on every `wsl.exe` invocation).
* **Chinese-only** user-facing strings (this distribution targets a
  Chinese non-technical audience). All strings live in
  `launcher/ui/locales/zh-CN.json` — Rust never returns localized text.
* **No API-key prompt at the terminal**. Keys are entered via the
  in-browser dashboard (with portable-mode at-rest encryption).
* **Safe-eject button** is the only correct way to exit. Direct unplug
  is mentioned in 问题排查.txt as a recovery scenario, not the default.

---

## 6. Known unfixable limits (document, do not paper over)

| Limit | What we do |
|---|---|
| WSL distro registration sits in the Windows registry, so first-boot on a new machine takes ~2 min while we re-`wsl --import` | Big friendly progress bar with Chinese copy; plus a tray toggle "退出时彻底卸载" so the user can opt to fully clean up on every exit |
| WSL must be enabled on the host (one-time, may need reboot) | Self-check in the launcher pops a Chinese-language guide |
| Corporate-managed Windows machines often disable WSL via GPO | Self-check detects, shows "请联系 IT" dialog |
| USB stick speed dictates UX — slow sticks feel laggy | README recommends USB 3.2 + ≥150 MB/s |
| Unsigned `.exe` triggers SmartScreen warning | Release pipeline signs with Authenticode (EV cert when available); 问题排查.txt documents the workaround |

---

## 7. Where to look in the code

| Thing | File |
|---|---|
| Rootfs build script | `scripts/build_portable_windows.sh` |
| WSL config baked into rootfs | `packaging/portable-windows/skel/etc/wsl.conf` |
| Entrypoint inside the distro | `packaging/portable-windows/skel/usr/local/bin/hermes-portable-entry` |
| Launcher main loop / tray | `packaging/portable-windows/launcher/src/main.rs` |
| Per-USB distro-name hashing | `packaging/portable-windows/launcher/src/paths.rs` |
| `wsl.exe` wrappers | `packaging/portable-windows/launcher/src/wsl.rs` |
| Safe-eject (`mountvol /p`) | `packaging/portable-windows/launcher/src/usb.rs` |
| First-run UI (HTML/CSS/JS) | `packaging/portable-windows/launcher/ui/` |
| User-facing strings | `packaging/portable-windows/launcher/ui/locales/zh-CN.json` |
| End-user docs that ship on the stick | `packaging/portable-windows/usb-template/` |
| CI build & release | `.github/workflows/portable-windows.yml` |
| Hermes-side hooks | `is_portable()` in `hermes_constants.py`; portable bookkeeping in `hermes_cli/web_server.py::start_server`; banner indicator in `hermes_cli/banner.py` |
