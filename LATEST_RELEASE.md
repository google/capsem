version: 1.5.1783543478
---
### Fixed
- Made public install and download entrypoints resolve stable release-channel
  packages instead of GitHub's asset tag page, keeping `.pkg` and `.deb`
  downloads on the binary release rail.
- Split binary package publication from VM image/profile asset publication so
  stable and nightly binaries can move independently while continuing to use
  the current EROFS/lz4hc image assets.
- Made runtime asset updates and package-build profile materialization accept
  release-channel profile manifests, preserve their exact image artifact URLs,
  and write validated raw v2 asset manifests for downstream compatibility.
- Cleared denied-warning release build failures in settings export, MCP SSE
  setup, Linux-only overlay helpers, and platform-specific `statvfs` counters.
