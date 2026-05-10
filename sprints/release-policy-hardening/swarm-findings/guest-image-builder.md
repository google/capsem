# Guest, Image Builder, and Rootfs Findings

Status: completed, pending transfer into T1/T5/T10.

Agent: Erdos (`019e1268-9e40-78f2-9751-b0550b4584d5`)

## Scope

- Guest binary presence.
- `capsem-dns-proxy` and `capsem-sysutil`.
- Initrd repack manifests.
- Asset cleanup.
- Guest binary chmod 555.
- Arch-specific asset manifests and builder tests.

## Findings

- [ ] [P1] Release-built initrds can miss `capsem-sysutil` runtime deployment.
  - Release impact: release images can boot without the runtime diagnostic
    helper that lifecycle diagnostics require.
  - Paths: `.github/workflows/release.yaml:104`,
    `src/capsem/builder/templates/Dockerfile.kernel.j2:42`,
    `guest/artifacts/capsem-init:352`,
    `guest/artifacts/diagnostics/test_lifecycle.py:13`.
  - Detail: release `build-assets` runs `just build-kernel` and
    `just build-rootfs`, but not `_pack-initrd`; kernel initrd contains only
    busybox and `capsem-init`. `capsem-init` deploys sysutil only from
    `/capsem-sysutil`, with no `/usr/local/bin/capsem-sysutil` fallback.
  - Proof: release-built initrd/rootfs validation must prove
    `/run/capsem-sysutil` and required symlinks exist.
  - Sprint IDs: T1.3, T1.5, T5.4, T10.2, T10.5.

- [ ] [P1] Release rootfs validation is stale and not a hard release gate.
  - Release impact: CI can publish rootfs assets missing binaries required by
    `capsem-init`.
  - Paths: `.github/workflows/release.yaml:398`,
    `.github/workflows/release.yaml:485`,
    `src/capsem/builder/docker.py:28`.
  - Detail: Linux app job is `continue-on-error`; mounted-rootfs check omits
    `capsem-dns-proxy` and `capsem-sysutil`, while canonical
    `GUEST_BINARIES` includes both. macOS only runs source-file validation, not
    mounted squashfs validation.
  - Proof: rootfs validation derives binary list from `GUEST_BINARIES` and is a
    hard pre-publish gate.
  - Sprint IDs: T1.5, T5.4, T10.2.

- [ ] [P1] `_pack-initrd` can rewrite the manifest as host-arch-only.
  - Release impact: local repacks can drop one arch from `assets/manifest.json`,
    causing wrong/missing asset resolution.
  - Paths: `justfile:1376`, `scripts/gen_manifest.py:69`,
    `tests/capsem-build-chain/test_manifest_regen.py:38`.
  - Detail: `_pack-initrd` regenerates `B3SUMS` for only `$arch/*`, then
    `gen_manifest.py` writes a fresh manifest from that partial file. Agent
    observed `assets/x86_64/` exists while `assets/manifest.json` listed only
    `["arm64"]`.
  - Proof: test manifest regeneration with both arch dirs present after
    `_pack-initrd`.
  - Sprint IDs: T1.3, T10.2.

- [ ] [P1] Full-build and repack manifest versioning disagree.
  - Release impact: repeated same-day builds can produce conflicting asset
    versions across build paths.
  - Paths: `src/capsem/builder/docker.py:695`,
    `scripts/gen_manifest.py:43`, `tests/test_gen_manifest.py:122`,
    `tests/test_docker.py:993`.
  - Detail: `generate_checksums()` always emits `YYYY.MMDD.1`;
    `gen_manifest.py` increments same-day patches. Tests cover only
    `gen_manifest.py` incrementing, not `generate_checksums()` collision
    behavior.
  - Proof: add generate-checksums same-day collision test and unify versioning.
  - Sprint IDs: T1.2, T10.2.

- [ ] [P2] Asset cleanup does not match installed per-arch layout.
  - Release impact: stale hash files and legacy directories survive cleanup,
    increasing chance of stale asset selection/debug confusion.
  - Paths: `crates/capsem-core/src/asset_manager.rs:535`,
    `crates/capsem-core/src/asset_manager.rs:563`,
    `crates/capsem-service/src/main.rs:4485`.
  - Detail: service comment says cleanup removes legacy `v*/` dirs and stale
    assets, but implementation skips directories entirely. Tests only cover
    flat files.
  - Proof: per-arch stale hash fixture and legacy `v*/` directory fixture.
  - Sprint IDs: T1.4, T10.2.

- [ ] [P2] Initrd/security tests still use stale three-binary contracts and
  skip missing files.
  - Release impact: missing `capsem-dns-proxy` or `capsem-sysutil` can pass
    permission/initrd tests.
  - Paths: `tests/capsem-build-chain/test_pack_initrd.py:18`,
    `tests/capsem-security/test_binary_perms.py:14`,
    `scripts/doctor-common.sh:232`.
  - Detail: binary lists omit `capsem-dns-proxy` and `capsem-sysutil`;
    candidate loops `continue` when a binary is absent.
  - Proof: tests require all canonical guest binaries and fail missing files.
  - Sprint IDs: T1.3, T5.4, T10.2.

- [ ] [P2] Guest artifact permissions are inconsistent for `capsem-doctor`.
  - Release impact: packaged rootfs and overlay/initrd paths do not enforce the
    same read-only binary invariant.
  - Paths: `src/capsem/builder/templates/Dockerfile.rootfs.j2:75`,
    `guest/artifacts/capsem-init:367`,
    `guest/artifacts/diagnostics/test_sandbox.py:72`.
  - Detail: rootfs installs `capsem-doctor` as `755`, while initrd overlay
    deploys it as `555`; release-built minimal initrds do not overlay it. The
    in-VM binary permission list omits `capsem-doctor`.
  - Proof: binary permission diagnostics cover `capsem-doctor` and enforce
    expected mode.
  - Sprint IDs: T1.5, T5.4, T10.2.

## Tests Not Run

- Static no-edit investigation only; no tests were run.
