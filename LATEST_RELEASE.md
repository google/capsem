version: 0.16.1
---
### Added
- **KVM boot diagnostics** -- when vCPU creation fails on Linux, Capsem now runs automatic diagnostic probes: kernel version, nested KVM status, KVM capabilities, and a fresh-VM-without-IRQCHIP test to isolate the root cause. All results logged at ERROR level so they appear without `RUST_LOG=debug`.
- **`scripts/kvm-diagnostic.py`** -- standalone diagnostic script for manual KVM environment debugging. Tests 7 phases: /dev/kvm basics, capabilities, Capsem boot sequence, no-irqchip mode, reversed ordering, split IRQCHIP, and environment info.

### Fixed
- **KVM boot errors are now actionable** -- `/dev/kvm` missing explains how to enable KVM (modprobe, BIOS). Permission denied suggests `usermod -aG kvm`. EEXIST on vCPU creation explains restricted/nested KVM and points to the diagnostic script.
- **Linux boot failure shows macOS error message** -- `gui.rs` said "unsigned binary or missing entitlement" on all platforms. Now shows platform-specific guidance: KVM troubleshooting on Linux, entitlement info on macOS.
- **LATEST_RELEASE.md stale at v0.15.1** -- boot screen showed wrong version. Regenerated from CHANGELOG.md.

### Changed
- **`just doctor` rewritten as standalone scripts** -- moved from 265-line inline justfile recipe to `scripts/doctor-common.sh` + platform-specific `doctor-macos.sh` and `doctor-linux.sh`. Colored output (green/red/yellow), structured recap table, and auto-fix: detects fixable issues (missing rustup targets, cargo tools, broken symlinks) and prompts to fix them automatically. `--fix` flag for non-interactive auto-fix.
