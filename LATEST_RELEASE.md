version: 1.1.1778539599
---
### Changed
- Disabled the unsupported desktop self-updater surface for the next release:
  Tauri updater config, updater permissions, launch-time checks, and frontend
  update controls are removed until release artifacts support full-install
  updates.
- Package installers now fail loudly when release-critical `capsem install` or
  `capsem setup` fails, instead of reporting success for a non-bootable install.
- Policy Hook Spec0 remains infrastructure-only for the next release:
  configured external hook dispatch is not exposed as a shipped settings/UI
  surface until a production integration gate wires and verifies it.

### Fixed
- macOS `.pkg` and Linux `.deb` package flows now carry signed
  `manifest.json` snapshots plus all host helper binaries, and release CI
  verifies package payload signatures before publishing.
- Release install E2E now consumes clean-checkout VM assets, locally signs the
  package manifest, and repacks the Linux `.deb` in place so CI installs the
  tested package instead of the unrepacked Tauri artifact.
- Setup, `capsem update --assets`, service startup, status, and doctor
  diagnostics now use verified manifest loading so unsigned or invalid
  manifests cannot silently downgrade asset verification.
- Release preflight now validates the manifest signing key against
  `config/manifest-sign.pub`, keeps Linux package publication
  release-blocking, and includes the signed manifest plus boot assets in
  provenance attestation.
- VM asset manifests now use consistent same-day patch selection across
  full image builds and local initrd repacks, preserve numeric asset-version
  ordering, clean stale per-arch hash aliases, and validate rootfs contents
  from the canonical guest artifact lists before release publication.
- Settings save and frontend import now reject new `policy.hook.*` rules, so
  users cannot save inert hook-decision policy that appears enforced.
- Settings reload failures now return structured saved-but-not-applied state,
  including affected session IDs, so the UI can keep a persistent retry banner.

### Security
- Manifest loading now verifies release signatures in setup, update, service,
  status, and doctor paths so unsigned or invalid asset manifests cannot
  silently downgrade boot asset verification.
- Policy hook controls and `policy.hook.*` writes are hidden or rejected until
  configured external hook dispatch has a production integration path and
  black-box E2E proof.
