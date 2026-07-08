version: 1.5.1783547869
---
### Fixed
- Made the binary release lane update graph-shaped stable and nightly channel
  manifests directly, preserving profile image metadata while replacing package
  and host SBOM evidence.
- Made `capsem-admin assets channel build` accept graph manifests as input so
  tag-triggered binary releases can rebuild channel catalog, health, and site
  output without raw VM asset manifests or image rebuilds.
- Added Linux-side macOS `.pkg` executable inventory extraction for
  productbuild XAR/CPIO payloads, keeping Ubuntu release-channel assembly able
  to publish package-owned binary hashes.
- Regenerated release SBOM evidence from packaged host artifacts before
  attestation so `capsem-sbom.spdx.json` carries SHA-256 checksums for the
  shipped `.pkg` and `.deb` executables.
