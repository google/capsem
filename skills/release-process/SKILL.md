---
name: release-process
description: Capsem's release process: orthogonal binary/profile CI, signing, notarization, channel deployment. Use for release commands, failures, manifests, or anything affecting what ships.
---

# Release Process

## Manifest handling

Read the manifest model in `RELEASE.md`, then use
`references/release-graph.md` for concrete paths, authoring tools, retirement,
and verification mechanics. Do not infer a selection from ambient cache or
workflow state while diagnosing a release.

## Release authority

Read root [`RELEASE.md`](../../RELEASE.md) before changing release commands,
manifests, workflows, test composition, artifact publication, or update
behavior. This skill routes implementation work and preserves operational
lessons; it does not restate product policy.

## Reference routing

- Read `references/qualification-and-test-composition.md` before changing public
  release commands, source guards, sandbox/egress, execution-envelope or
  workflow/shell parity, test composition, artifact staging, `ProfileContent`, or
  `--force` (CI-only changes; never shipped bytes).
- Read `references/lane-workflows.md` before changing channel locking, preview
  deployment, profile/binary ownership, nightly sequencing, staged activation,
  base-image materialization, or corporate authoring.
- Read `references/release-graph.md` before changing graph generation, channel
  membership, manifest authoring, immutable identity, or public activation.
- Read `references/installation-verification-and-retry.md` before changing
  evidence/integrity rules, native package acceptance, installed status,
  artifact retention, live validation, retry, diagnostic continuation, or
  Cloudflare deployment checks.
- Read `references/ci-invariants.md` before editing release workflows,
  platform/toolchain/scanner setup, Docker/storage behavior, package rails, or
  hosted-runner capacity. It contains the hard-won parity lessons.
- Read `references/apple-signing.md` before touching signing, notarization,
  certificates, Tauri keys, Apple agreements, or release CI secrets.
- Read `references/post-release-verification.md` after any public deployment
  and before changing the public installer, transition, or glow-up proof.
- Read `references/versions-and-commit-discipline.md` before changing release
  notes, binary/profile versions, compatibility bounds, profile revision
  advancement, release-set identities, or release commit practice.

## Operational entrypoints

Use the public command forms defined by `RELEASE.md`:

```bash
just release-binaries <channel> <source-commit>
just release-profile <channel> <profile> <source-commit>
```

Do not dispatch downstream workflows or author source manifests by hand. Each
release command is sufficient on its own because its hosted lane performs
release qualification; `just test <source-commit>` is optional reusable local
whole-system verification, not a release prerequisite. Use focused tests
during ordinary development. The release commands and complete local test own
their sandbox, egress, machine lock, journal, and teardown, so do not nest or
wrap them.

For failures, select the reference matching the affected boundary above.
Diagnostic continuation, CI-only `--force`, graph retirement, signing,
Cloudflare recovery, and installed transition checks all have narrower rules
in those references. When a reference appears to change product behavior,
reconcile it against `RELEASE.md` and the executable contract tests first.

## Tested operational handoffs

Assets and materialized configuration travel as one `ProfileContent` root.
Package construction, Debian proof, macOS Tart/physical-VZ proof, and final
install/glow-up must derive both paths from that one value and validate it
before Docker or Colima. Release CI stages raw manifest inputs into the paired
root on the host; the sealed proof never rematerializes them or falls back to
checkout `assets`/`target/config` selectors.

Linux package replacement embeds `deb-preinst.sh` as `DEBIAN/preinst`.
Ordinary replacement uses `systemctl --user stop capsem.service` and retires
the stale helper cohort before package replacement. When `/proc/self/cgroup`
proves that the old service owns the update, preinstall preserves the old
cohort and postinstall defers manifest hydration, status refresh, service
registration, and readiness so that service can activate the verified
candidate and request its managed restart.

## Version and commit essentials

Read `references/versions-and-commit-discipline.md` for release-note,
versioning, compatibility, and commit mechanics. Stage explicitly with a
conventional subject and never stage release secrets.
