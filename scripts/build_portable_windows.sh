#!/usr/bin/env bash
# Build the portable WSL2 rootfs image for the Windows-USB Hermes distribution.
#
# Output: HermesPortable.tar.zst — a zstd-compressed tar of a Debian 13-slim
# rootfs that contains:
#
#   /opt/hermes               full repo + .venv (uv-managed) with optional
#                             extras (messaging, cron, cli, mcp, honcho, acp, pty)
#   /opt/hermes/ui-tui        prebuilt Ink TUI bundle (npm install + build done)
#   /opt/hermes/web           prebuilt dashboard SPA (npm install + build done)
#   /home/hermes              UID 1000, default user
#   /etc/wsl.conf             baked-in WSL config (automount metadata, no systemd)
#   /opt/hermes/.portable-marker   sentinel for the launcher to confirm distro
#   /usr/local/bin/hermes-portable-entry   entrypoint the launcher invokes
#
# This script is meant to run on Linux x86_64 (GitHub Actions ubuntu-latest).
# It uses Docker so the host distro doesn't matter.
#
# Usage:
#   bash scripts/build_portable_windows.sh [--output DIR]
#
# Defaults:
#   --output  ./dist/portable-windows
#
# The companion launcher under packaging/portable-windows/launcher consumes
# the resulting tar.zst via `wsl --import` on the user's Windows machine.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
OUTPUT_DIR="${REPO_ROOT}/dist/portable-windows"
ROOTFS_NAME="HermesPortable.tar.zst"
DEBIAN_TAG="debian:13-slim"
PYTHON_VERSION="3.11"
NODE_MAJOR="20"

while [[ $# -gt 0 ]]; do
    case "$1" in
        --output) OUTPUT_DIR="$2"; shift 2 ;;
        --debian-tag) DEBIAN_TAG="$2"; shift 2 ;;
        -h|--help)
            sed -n '2,30p' "$0"
            exit 0 ;;
        *) echo "Unknown arg: $1" >&2; exit 2 ;;
    esac
done

mkdir -p "$OUTPUT_DIR"

if ! command -v docker >/dev/null 2>&1; then
    echo "ERROR: docker is required to build the portable rootfs." >&2
    exit 1
fi

if ! command -v zstd >/dev/null 2>&1; then
    echo "Note: host zstd not found — using zstd from inside the build container." >&2
fi

SKEL_DIR="${REPO_ROOT}/packaging/portable-windows/skel"
if [[ ! -d "$SKEL_DIR" ]]; then
    echo "ERROR: missing skel directory: $SKEL_DIR" >&2
    exit 1
fi

echo "==> Building portable rootfs in a $DEBIAN_TAG container"
echo "    Repo:    $REPO_ROOT"
echo "    Output:  $OUTPUT_DIR/$ROOTFS_NAME"

# Run the build inside a Debian container so dependencies match the
# target rootfs exactly. Mount the repo read-only and the output dir
# read-write.  /work is the in-container staging area; we then tar
# everything below /work/rootfs and stream it through zstd to the
# bind-mounted output dir.
docker run --rm \
    --platform linux/amd64 \
    -v "${REPO_ROOT}:/src:ro" \
    -v "${OUTPUT_DIR}:/out" \
    -e PYTHON_VERSION="${PYTHON_VERSION}" \
    -e NODE_MAJOR="${NODE_MAJOR}" \
    -e ROOTFS_NAME="${ROOTFS_NAME}" \
    "${DEBIAN_TAG}" \
    bash -euo pipefail -c '
        export DEBIAN_FRONTEND=noninteractive

        echo "==> Installing build deps in builder image"
        apt-get update -qq
        apt-get install -y --no-install-recommends \
            ca-certificates curl gnupg xz-utils zstd tar git \
            python3 python3-venv python3-pip \
            build-essential pkg-config \
            libssl-dev libffi-dev \
            ripgrep ffmpeg \
            >/dev/null

        echo "==> Installing Node.js ${NODE_MAJOR}.x"
        curl -fsSL "https://deb.nodesource.com/setup_${NODE_MAJOR}.x" | bash - >/dev/null
        apt-get install -y --no-install-recommends nodejs >/dev/null

        echo "==> Installing uv (Python dep manager)"
        curl -fsSL https://astral.sh/uv/install.sh | env UV_INSTALL_DIR=/usr/local/bin sh >/dev/null

        ROOTFS=/work/rootfs
        rm -rf "$ROOTFS"
        mkdir -p "$ROOTFS/opt/hermes" "$ROOTFS/etc" "$ROOTFS/home" "$ROOTFS/opt/data"

        echo "==> Copying repo into rootfs (excluding .git, dist, node_modules, __pycache__)"
        rsync -a --delete \
            --exclude=.git \
            --exclude=node_modules \
            --exclude=__pycache__ \
            --exclude=dist \
            --exclude=build \
            --exclude=.venv \
            --exclude=venv \
            --exclude=.pytest_cache \
            --exclude=.mypy_cache \
            /src/ "$ROOTFS/opt/hermes/"

        echo "==> Creating Python venv"
        cd "$ROOTFS/opt/hermes"
        uv venv .venv --python "${PYTHON_VERSION}"
        # shellcheck disable=SC1091
        source .venv/bin/activate

        echo "==> Installing Python deps (extras: messaging,cron,cli,mcp,acp)"
        # Heavy/optional extras (voice, matrix-olm) intentionally omitted to
        # keep the rootfs slim — users can install them post-hoc into the
        # USB-resident venv if desired.
        uv pip install -e ".[messaging,cron,cli,mcp,acp]"

        echo "==> Building dashboard SPA (web/)"
        if [[ -f web/package.json ]]; then
            (cd web && npm install --omit=dev --no-audit --no-fund && npm run build)
        fi

        echo "==> Building Ink TUI (ui-tui/)"
        if [[ -f ui-tui/package.json ]]; then
            (cd ui-tui && npm install --no-audit --no-fund && npm run build)
        fi

        echo "==> Installing Playwright Chromium (--only-shell to save space)"
        # Chromium-shell is ~150 MB vs ~500 MB for full chromium.
        # Playwright is a Python dependency here (NOT npm) — the previous
        # check looked at node_modules, which never exists in this venv.
        if .venv/bin/python -c "import playwright" 2>/dev/null; then
            PLAYWRIGHT_BROWSERS_PATH=/opt/hermes/.playwright \
                .venv/bin/python -m playwright install --only-shell chromium || true
        fi

        echo "==> Baking skel files (/etc/wsl.conf, entrypoint, marker)"
        cp -a /src/packaging/portable-windows/skel/. "$ROOTFS/"
        chmod 0755 "$ROOTFS/usr/local/bin/hermes-portable-entry" || true

        echo "==> Creating hermes user (UID 1000) inside the rootfs metadata"
        # We cannot run useradd against the target rootfs reliably (no chroot
        # set up).  Instead, write minimal passwd/group/shadow entries — WSL
        # accepts these on first boot.
        mkdir -p "$ROOTFS/home/hermes"
        cat >> "$ROOTFS/etc/passwd" <<EOF2
hermes:x:1000:1000:Hermes Portable:/home/hermes:/bin/bash
EOF2
        cat >> "$ROOTFS/etc/group" <<EOF2
hermes:x:1000:
EOF2
        cat >> "$ROOTFS/etc/shadow" <<EOF2
hermes:!:19000:0:99999:7:::
EOF2
        chown -R 1000:1000 "$ROOTFS/home/hermes" "$ROOTFS/opt/hermes" "$ROOTFS/opt/data"

        echo "==> Cleaning caches to shrink rootfs"
        find "$ROOTFS/opt/hermes" -name __pycache__ -type d -prune -exec rm -rf {} + || true
        rm -rf "$ROOTFS/opt/hermes/.pytest_cache" "$ROOTFS/opt/hermes/.mypy_cache"
        rm -rf "$ROOTFS/root/.cache"

        echo "==> Marking portable distro"
        printf "portable-windows\n%s\n" "$(date -u +%Y-%m-%dT%H:%M:%SZ)" \
            > "$ROOTFS/opt/hermes/.portable-marker"

        echo "==> Tar + zstd (level 19) the rootfs"
        cd "$ROOTFS"
        tar --numeric-owner -cf - . | zstd -19 -T0 -o "/out/${ROOTFS_NAME}"

        echo "==> Recording metadata"
        sha256sum "/out/${ROOTFS_NAME}" > "/out/${ROOTFS_NAME}.sha256"
        ls -lh "/out/${ROOTFS_NAME}"
    '

# Stamp a version file alongside the rootfs so the launcher can detect upgrades.
GIT_DESCRIBE="$(git -C "$REPO_ROOT" describe --tags --always --dirty 2>/dev/null || echo unknown)"
printf "rootfs_version=%s\nbuilt_at=%s\n" \
    "$GIT_DESCRIBE" \
    "$(date -u +%Y-%m-%dT%H:%M:%SZ)" \
    > "$OUTPUT_DIR/version.txt"

echo
echo "==> Done. Artifacts:"
ls -lh "$OUTPUT_DIR/"
