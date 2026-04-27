# Icon placeholders

The Tauri build expects:

* `icons/icon.ico` — main app icon (multi-resolution: 16, 32, 48, 64, 128, 256)
* `icons/tray.ico` — system-tray icon (16/32 px)

These are intentionally **not** committed as binary blobs in this PR. The
release pipeline (`.github/workflows/portable-windows.yml`) generates them
from `assets/logo.png` (the Hermes caduceus mark already shipped in the
repo) using ImageMagick:

```sh
magick convert assets/logo.png -define icon:auto-resize=256,128,64,48,32,16 \
       packaging/portable-windows/launcher/icons/icon.ico
magick convert assets/logo.png -resize 32x32 \
       packaging/portable-windows/launcher/icons/tray.ico
```

For local dev (`cargo tauri dev`) you can drop any 256×256 .ico in here
and Tauri will accept it.
