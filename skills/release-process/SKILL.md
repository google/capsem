---
name: release-process
description: Capsem's release process: orthogonal binary/runtime CI, signing, notarization, channel deployment. Use for release commands, failures, manifests, or anything affecting what ships.
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
  workflow/shell parity, test composition, artifact staging, `RuntimeContent`, or
  `--force` (dirty outer checkout only; never the `just test` journal).
- Read `references/lane-workflows.md` before changing channel locking, preview
  deployment, runtime/binary ownership, nightly sequencing, staged activation,
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
  notes, binary/runtime versions, compatibility bounds, runtime revision
  identity, release-set identities, or release commit practice.

## Operational entrypoints

Use the public command forms defined by `RELEASE.md`:

```bash
just release-binaries <channel> <source-commit>
just release-assets <channel> <source-commit>
```

Do not dispatch downstream workflows or author source manifests by hand.
**Run `just test <source-commit>` to success first**: both commands refuse a
source without a complete, passing journal on this machine, because a hosted
attempt takes hours to find what the local glow-up and functional lanes find in
minutes. The hosted lane still qualifies what it publishes. Only the
unattended nightly scheduler (`[release].unattended_channels`) is exempt, and
release `--force` never waives the journal. Failed or interrupted runs never
count; an approved `just test <commit> force "<reason>"` that passes does.

The proof is matched by source identity, the Git tree (every tracked byte,
lockfile, and submodule pin), so the intended flow is: fix on a branch, run
`just test <branch-head>` there, then get it onto `main` one of two ways:

1. fast-forward `main` to the tested commit, and release that commit; or
2. merge the PR, and release the merge commit only when its tree equals the
   tested tree (`git rev-parse <merge>^{tree} <tested>^{tree}` prints one
   hash twice). A merge that brought in anything else has a different tree
   and is refused; test the merge commit itself.

Use focused tests during ordinary development. The release commands and
complete local test own their sandbox, egress, machine lock, journal, and
teardown, so do not nest or wrap them.

For failures, select the reference matching the affected boundary above.
Diagnostic continuation, CI-only `--force`, graph retirement, signing,
Cloudflare recovery, and installed transition checks all have narrower rules
in those references. When a reference appears to change product behavior,
reconcile it against `RELEASE.md` and the executable contract tests first.

## Tested operational handoffs

Assets and materialized configuration travel as one `RuntimeContent` root.
Package construction, Debian proof, macOS Tart/physical-VZ proof, and final
install/glow-up must derive both paths from that one value and validate it
before Docker or Colima. Release CI stages raw manifest inputs into the paired
root on the host; the sealed proof never rematerializes them or falls back to
checkout `assets`/`cache/target/config` selectors.

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
