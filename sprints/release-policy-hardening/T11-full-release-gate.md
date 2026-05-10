# T11: Local Release Candidate Gate

## Objective

Run the final local release-candidate gate only after focused verification is
green. This track is the no-shortcuts checkpoint before any CI tag or publish
work. It must build and install the package locally, then verify CLI, JS UI,
and installed desktop launch with Elie.

## Owned Files

- `justfile`
- `scripts/preflight.sh`
- `scripts/check-release-workflow.sh`
- `.github/workflows/release.yaml`
- `sprints/release-policy-hardening/tracker.md`
- `CHANGELOG.md`
- `LATEST_RELEASE.md`
- release tag and commit metadata

## Findings

- [P0] Before this pass, the final gate existed only as a tracker row.
- [P0] `just test` remains the single source of truth and must run after
  targeted checks pass.
- [P0] Local package generation and installation must happen before CI/tagging;
  package payload checks alone are not enough.
- [P1] Release preflight must include Apple signing/notarization readiness,
  manifest signing readiness, and updater/package truth.
- [P1] Elie needs explicit stop points to personally verify CLI, JS UI, and
  full installed app launch before CI.

## Task List

### T11.1 Preflight

- [ ] Run `just doctor`.
- [ ] Run `scripts/preflight.sh`.
- [ ] Run or update `scripts/check-release-workflow.sh`.
- [ ] Confirm manifest signing key/public key match.
- [ ] Confirm release workflow artifact expectations match T0/T5/T10.

### T11.2 Full Suite

- [ ] Run `env UV_CACHE_DIR=/private/tmp/capsem-uv-cache just test`.
- [ ] Do not bypass integration, frontend, cross-compile, Docker install, VM,
  benchmark, or docs gates.
- [ ] If interrupted, rerun from clean process state and record what was
  restarted.

### T11.3 Local Package Generation and Install

- [ ] Run `just test-install`.
- [ ] Run `just install` to build the release package and install it locally.
- [ ] Record generated package paths and installed binary paths in
  `tracker.md`.
- [ ] Expand the generated `.pkg` or `.deb` and verify manifest, minisig,
  helper binaries, service files, and app bundle match T0/T5 expectations.
- [ ] From the installed layout, run `~/.capsem/bin/capsem version`.
- [ ] From the installed layout, run `~/.capsem/bin/capsem doctor`.
- [ ] From the installed layout, run
  `~/.capsem/bin/capsem run "echo installed-cli-ok"`.
- [ ] Confirm any package-specific manual proof from T0/T10 is recorded.

### T11.4 Elie + Codex Product Sign-Off

- [ ] Gate A sign-off recorded: JS UI policy flow verified with
  `just dev-frontend`, browser screenshots/evidence, and zero console
  errors/warnings.
- [ ] Gate B sign-off recorded: CLI and VM behavior verified with
  `just exec "echo cli-ok"`, `just exec "capsem-doctor"`, and the T8 policy
  E2E proof.
- [ ] Gate C sign-off recorded: desktop dev/full launch verified with
  `just build-ui` and `just run-ui --`.
- [ ] Gate D sign-off recorded: installed package app launches and matches the
  CLI/package proof from T11.3.

### T11.5 Release Readiness Review

- [ ] Re-run `git diff --check`.
- [ ] Review `git status --short` and ensure unrelated user changes are not
  staged.
- [ ] Confirm T0-T10 exit criteria are complete or explicitly release-blocking.
- [ ] Confirm `CHANGELOG.md` and release metadata are included.
- [ ] Confirm no active swarm agents are still running.

### T11.6 Handoff to T12

- [ ] Do not tag until all P0/P1 blockers are closed.
- [ ] Do not push a release tag from a dirty or partially verified tree.
- [ ] If the release is deferred, record the blocking track and owner in
  `tracker.md`.
- [ ] Record that T12 may begin only after this local gate is signed off by
  Elie.

## Proof Matrix

| Category | Required proof |
|---|---|
| Preflight | doctor, signing, notarization, manifest signing, workflow checks. |
| Full suite | `just test` passes after targeted verification. |
| Install/VM | `just install`, installed CLI proof, package proof, and capsem-doctor proof recorded. |
| Human proof | Elie signs off JS UI, CLI, desktop dev launch, and installed app launch. |
| Release hygiene | clean diff check, explicit staging, changelog/version metadata. |

## Verification

- [ ] `just doctor`
- [ ] `scripts/preflight.sh`
- [ ] `scripts/check-release-workflow.sh`
- [ ] `env UV_CACHE_DIR=/private/tmp/capsem-uv-cache just test`
- [ ] `just test-install`
- [ ] `just install`
- [ ] `~/.capsem/bin/capsem version`
- [ ] `~/.capsem/bin/capsem doctor`
- [ ] `~/.capsem/bin/capsem run "echo installed-cli-ok"`
- [ ] `just dev-frontend`
- [ ] `just exec "capsem-doctor"`
- [ ] `just build-ui`
- [ ] `just run-ui --`
- [ ] `git diff --check`
- [ ] `git status --short`

## Exit Criteria

- [ ] Full suite is green.
- [ ] `just install` generated and installed the package locally.
- [ ] Installed CLI, JS UI, desktop dev launch, and installed app launch are
  signed off by Elie.
- [ ] Install/VM smoke is green or blocked with a recorded release owner.
- [ ] Changelog/version/release metadata are synchronized.
- [ ] No release tag is created until T12 starts from a signed-off local gate.
