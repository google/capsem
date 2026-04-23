version: 1.0.1776981476
---
### Fixed (CI)
- test-install runner now installs libgtk-3-dev + libwebkit2gtk-4.1-dev
  + libayatana-appindicator3-dev + librsvg2-dev + libxdo-dev + libssl-dev
  so `_build-host` can `cargo build` the tray / tauri-adjacent crates.
