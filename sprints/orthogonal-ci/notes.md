# Notes

Running log of implementation details and decisions. Append dated entries.

## 2026-04-23 -- v1.0 release path

v1.0 shipping today with the **combined** release workflow. Rationale:

1. **First release needs both.** There is no prior asset release on GitHub for a binary-only workflow to reference (it would have nothing to download + merge into `binaries.releases`). The first release has to seed the manifest with both sections.
2. **Combined workflow already works** end-to-end -- notary flow, squashfs rootfs gate, manifest v2 merge, minisign signing, SBOM, SLSA attestation. Proven path.
3. **Split is additive, not destructive.** After v1.0 publishes a manifest with both `assets.releases` and `binaries.releases` populated, the split workflows can:
   - Binary-only: download latest manifest, read `assets.current`, reuse those asset URLs, add new `binaries.releases[version]` entry.
   - Asset-only: download latest manifest, keep `binaries.releases` untouched, add new `assets.releases[version]` entry.

## Follow-up triggers

Split workflow design should handle these tag patterns:
- `v1.0.{timestamp}` -- binary release. Skips `build-assets`, skips asset upload.
- `assets-{YYYY.MMDD.N}` -- asset release. Skips `build-app-macos`, `build-app-linux`, `test`.

Current `release.yaml` needs gating, OR split into two files. Two files is cleaner (no `if:` spaghetti on every job).

## Open questions

- Does `capsem update` (runtime self-updater) already poll the manifest for asset diffs, or does it only look at binary version? If the latter, assets-only releases won't reach users until they reinstall. See `crates/capsem/src/update.rs` -- deliverable #6 in plan.md.
- `build-pkg.sh` currently bundles only `manifest.json`, relying on `capsem setup` to download heavy assets on first launch. That already supports asset-independent binary ship -- good.
