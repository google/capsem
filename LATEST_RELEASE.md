version: 1.2.1779662531
---
### Fixed
- Fixed package setup for manifest-only installs so packaged Profile V2
  sidecars install before local heavy VM asset fallback, allowing `.deb`
  postinstall to complete from signed packaged profiles without bundled
  kernel/initrd/rootfs files.
