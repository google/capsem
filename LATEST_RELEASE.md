version: 1.5.1784071469
---
### Fixed
- Made the public macOS curl installer synchronously apply the downloaded
  package with `/usr/sbin/installer`, so the command completes a real install
  instead of handing the package to a generic `open` action.
- Made both macOS and Linux public installers reject package downloads whose
  byte size or SHA-256 does not match the release manifest before elevation.
- Strengthened release CI to verify exact installed package versions, complete
  binary cohorts, service readiness, and PTY-driven `capsem shell` execution
  inside a KVM-backed guest on Linux.
- Made Debian-package SBOM inspection parse the package archive directly, so
  release acceptance tests work with both BSD and GNU host toolchains.
- Pinned the clean-build SSE stream dependency to its supported patch API so
  untracked local resolution state cannot mask release CI compilation drift.
- Made macOS CI install the release-site lockfile before integration tests so
  cached local Node modules cannot mask clean-runner release-contract failures.
