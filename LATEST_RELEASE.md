version: 1.5.1783550716
---
### Fixed
- Removed the stale macOS `cargo sbom` handoff from release artifacts and made
  channel assembly regenerate packaged host SBOM evidence from the downloaded
  `.pkg` and `.deb` files before validating stable/nightly manifests.
