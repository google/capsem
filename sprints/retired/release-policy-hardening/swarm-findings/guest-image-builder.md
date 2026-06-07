# Guest, Image Builder, and Rootfs Findings

Status: completed; transferred to T7 FD09 and owner rows in T1/T5/T10.
Downstream implementation remains open.

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

- [x] [P1] Release rootfs validation is stale and not a hard release gate.
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
  - Transfer status: resolved in T1/T5; live rootfs artifact validation remains
    T10/T11.

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

## T5.2/T5.4 Execution Audit, 2026-05-10

Agent: Hubble (`019e1312-7f9e-7fe0-9608-67af861606f3`)

Status: completed; findings captured for T5.2 and T5.4.

### Findings

- [x] [P1] Clean-checkout policy hook artifact proof is missing.
  - Release impact: `config/policy-hook-openapi.json` can drift or be left
    unstaged while local Rust tests still pass against generated code.
  - Paths: `config/policy-hook-openapi.json`,
    `crates/capsem-core/src/net/policy_hook_spec/tests.rs:52`,
    `tests/test_release_workflow_policy.py`.
  - Required proof: static release workflow test proves the artifact is tracked
    with `git ls-files --error-unmatch` and parses as JSON.
  - Sprint IDs: T5.2, T10.5.
  - Transfer status: resolved in T5.

- [x] [P2] `/policy-hook/spec` has handler coverage but not gateway route/auth
  matrix coverage.
  - Release impact: the public service route can regress outside the direct
    handler test, especially through gateway fallback/auth behavior.
  - Paths: `crates/capsem-service/src/main.rs:4653`,
    `crates/capsem-service/src/tests.rs:1437`,
    `crates/capsem-gateway/src/auth/tests.rs:287`,
    `tests/capsem-gateway/conftest.py:94`.
  - Required proof: gateway auth matrix includes `/policy-hook/spec`; gateway
    integration tests cover unauthenticated/wrong-token `401` and valid-token
    JSON proxying.
  - Sprint IDs: T5.2, T10.5.
  - Transfer status: resolved in T5.

- [x] [P2] Rootfs validation is wired, but local proof is indirect.
  - Release impact: string-based workflow checks can pass while the canonical
    required binary list drifts away from validation.
  - Paths: `scripts/validate-rootfs.sh:27`,
    `src/capsem/builder/docker.py:30`,
    `tests/capsem-rootfs-artifacts/test_rootfs_artifacts.py:16`.
  - Required proof: tests assert `GUEST_BINARIES` includes
    `capsem-dns-proxy` and `capsem-sysutil`, and that the validator derives
    `/usr/local/bin/<name>` requirements from `GUEST_BINARIES`.
  - Sprint IDs: T5.4, T10.2.
  - Transfer status: resolved in T5.

- [x] [P2] Canonical guest binary list still has multiple practical sources.
  - Release impact: preflight can pass by grepping names in the justfile even
    when `GUEST_BINARIES` or `validate-rootfs.sh` drifts.
  - Paths: `scripts/preflight.sh:281`,
    `scripts/preflight.sh:299`,
    `src/capsem/builder/docker.py:30`.
  - Required proof: preflight or tests compare Cargo guest bin names with
    `capsem.builder.docker.GUEST_BINARIES` and assert the release validator is
    the hard gate.
  - Sprint IDs: T5.4, T10.2.
  - Transfer status: resolved in T5.

### Tests Not Run

- Static code-reading investigation only; no builds/tests were run.
