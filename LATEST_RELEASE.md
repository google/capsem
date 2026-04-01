version: 0.15.1
---
### Fixed
- x86_64 Linux build: aarch64 boot module not cfg-gated (14 compile errors)
- Cross-compile linker error on arm64 hosts (missing gnu cross-linker config)
- Multiarch dpkg conflict in cross-compile Docker image (pango .gir overwrite)

### Changed
- `build-assets` now builds both arm64 and x86_64
- `full-test` includes `cross-compile` to catch platform-gating errors locally
