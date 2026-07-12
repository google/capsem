version: 1.5.1783869563
---
### Fixed
- Installed the complete Linux GTK, GLib, WebKit, SSL, musl, pkg-config, X11,
  and virtual-display prerequisites before the canonical release `just test`
  gate, so its full-workspace Clippy and application compilation execute on a
  clean GitHub runner instead of failing after 33 minutes on missing
  `glib-2.0.pc`.
