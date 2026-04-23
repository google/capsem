version: 1.0.1776987645
---
### Fixed (CI)
- build-app-macos: include capsem-mcp-aggregator / capsem-mcp-builtin in
  companion-binary build + codesign (build-pkg.sh needs all 8).
- build-app-linux: install libxdo-dev, libayatana-appindicator3-dev,
  librsvg2-dev so capsem-tray links.
