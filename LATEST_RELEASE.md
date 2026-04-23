version: 0.16.2
---
### Fixed
- **`snapshots` tool missing from release VM images** -- the `snapshots` CLI was only injected via `just _pack-initrd` (local dev), which is not run during CI release builds. Added `snapshots` to the rootfs Dockerfile so it ships with every release. Also fixed `snapshots` permissions from 755 to 555 (matching guest binary security invariant).

### Added
- **Multi-layer safeguards against missing guest artifacts** -- single source of truth (`ROOTFS_SCRIPTS`, `ROOTFS_SCRIPT_DIRS`, `ROOTFS_SUPPORT_FILES` constants in `docker.py`) imported by builder doctor, config validator, and used by `prepare_build_context()`. CI release workflow now validates source artifacts pre-build (macOS) and rootfs binary content via squashfs mount (Linux). In-VM `capsem-doctor` now fails (not skips) when required guest binaries are missing.

### Fixed
- **`just doctor-fix` fails on fresh machines** -- `build-assets` triggered `_ensure-setup` which ran `doctor` which failed on missing assets, creating a circular dependency. Fix commands now set `CAPSEM_SKIP_ASSET_CHECK=1` and `touch .dev-setup` to break the cycle. Guest binary checks are also skipped when asset check is skipped (no assets = no binaries). Fixes bail on first failure instead of continuing to run dependent steps.
- **Docker cross-arch builds fail (legacy builder cache poisoning)** -- Docker's legacy builder shared intermediate layer cache across `--platform` values, reusing arm64 layers for x86_64 builds. Fixed by requiring Docker BuildKit (buildx). Added buildx and Colima Rosetta checks to `just doctor` and `scripts/bootstrap.sh`.
