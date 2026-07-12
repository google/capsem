version: 1.5.1783872793
---
### Fixed
- Loaded `vhost_vsock` and made `/dev/vhost-vsock` accessible before the Linux
  release gate, so the complete VM and IronBank suites can boot guests on a
  KVM-capable hosted runner instead of cascading from a root-only vsock device
  into hundreds of fixture failures.
