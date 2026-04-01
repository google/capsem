version: 0.15.0
---
### Added
- **x86_64 KVM backend** -- full KVM support for x86_64 Linux with bzImage boot, IRQCHIP, 16550 UART, and virtio-mmio
- **Cross-compile Docker image** -- purpose-built `capsem-host-builder` for building Tauri apps targeting Linux
- **Kernel arch-mismatch detection** -- rejects wrong-arch kernels with clear errors

### Changed
- **Colima replaces Podman** -- near-native x86_64 container performance on Apple Silicon via Rosetta

### Fixed
- `just run` blocked on Linux
- x86_64 KVM boot hang (wrong entry point + missing setup header)
- `install.sh` now works on Linux
