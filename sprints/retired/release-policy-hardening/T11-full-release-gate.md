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

## Swarm Transfer Tracker

| Source | Priority | Owner task | Required transfer point | Required proof |
|---|---:|---|---|---|
| FD02 docs-release-metadata | P1 | T11.1, T11.4 | Manifest-signing readiness and artifact truth must be part of the final local gate. | `scripts/check-release-workflow.sh` and package evidence prove manifest signing and no stale artifact copy. |
| FD03 sprint-consistency | P1 | T11.4 | Tracker verification command rollup must be labeled or complete, and T7 active/completed sync must be proved before final gate. | T11.5 release readiness review includes T7/T10.7 evidence and no stale active-swarm status. |
| FD06 cli-updater-install | P0 | T11.3, T11.4, T11.5 | Do not tag yet; T0/T5/T9/T10/T11 are blockers until local package/install/product proof passes. | T11.6 records no tag/CI start until Elie signs Gate D. |
| FD10 ci-packaging | P0/P1 | T11.1, T11.3 | Local gate must validate `.pkg`/`.deb` payloads, helper binaries, signed manifests, rootfs/package expectations, and updater strategy. | `just install`, package expansion, installed CLI, and desktop launch evidence recorded. |
| FD11 verification-architecture | P0/P1 | T11.2, T11.3, T11.6 | Invalid `just run` command must be replaced; T11/T12 split must preserve post-publish owner. | Valid `just exec`/installed `capsem run` commands are recorded, and T12 starts only after T11 sign-off. |
| FD12 manual-ui-cli-gates | P0 | T11.3 | T11 must run final local package install gate before CI. | `just install`, installed `~/.capsem/bin/capsem version`, `doctor`, `run`, payload inspection, and installed desktop evidence. |
| FD12 manual-ui-cli-gates | P0 | T11.3 | `just run "capsem-doctor"` is invalid; use `just exec "capsem-doctor"` or installed `capsem run`. | Final gate command list uses valid commands only. |
| FD12 manual-ui-cli-gates | P1 | T11.4 | Gate A-D stop points must be blocking checklist rows with evidence and Elie sign-off. | Tracker evidence ledger has sign-off/evidence path for each gate. |
| FD12 manual-ui-cli-gates | P1 | T11.3, T11.4 | Evidence capture must be durable for screenshots, console logs, CLI transcripts, package logs, and installed app notes. | Evidence ledger paths point to files, not chat history. |
| FD12 manual-ui-cli-gates | P2 | T11.3, T11.4 | Dev desktop and installed desktop full-launch paths are distinct and both required. | Gate C and Gate D both have screenshots/logs and sign-off. |
| FD13 ci-release-landing-1-1 | P1 | T11.1, T11.6 | Local release-check scripts must catch Linux best-effort publishing, stale helper validation, updater strategy, and exact version discipline before tag. | Preflight/check-release workflow pass after enforcing these checks. |

## Task List

### T11.1 Preflight

- [x] Run `just doctor`.
- [x] Run `scripts/preflight.sh`.
- [x] Run or update `scripts/check-release-workflow.sh`.
- [x] Confirm manifest signing key/public key match from a restored local
  private dir and CI-compatible passwordless minisign flow.
- [x] Confirm release workflow artifact expectations match T0/T5/T10.

T11.1 note: `scripts/preflight.sh` uses `uv run python` for the guest binary
import, so the preflight no longer fails because host Python lacks project
dependencies. `just doctor` is green at 42 passed / 0 skipped / 0 warnings,
including `minisign` and local asset manifest signature verification. After
`private/` was restored, the Apple certificate/notarization and manifest
signing checks pass locally as well.

Restored-private rerun: local `private/` was restored with the manifest key at
`private/minisign/manifest.key`. `scripts/preflight.sh` now passes 40 / 0,
including Apple certificate import, notarization credential history, manifest
signing, and `config/manifest-sign.pub` verification. `scripts/check-release-workflow.sh`
passes 13 / 0 and validates the passwordless minisign flow used by CI.

### T11.2 Full Suite

- [x] Run `just test`.
- [x] Do not bypass integration, frontend, cross-compile, Docker install, VM,
  benchmark, or docs gates.
- [x] If interrupted, rerun from clean process state and record what was
  restarted.

T11.2 note: final `just test` passed on 2026-05-11 after the installed-tray
healing and Linux dead-code cfg fixes. The run included frontend
check/build/tests (19 Vitest files, 388 tests; Astro built 2 pages), Rust
coverage (68.07%), Python xdist (1344 passed / 69 skipped, coverage 91.24%),
build-chain tests (22 passed), injection (5 passed), local manifest signature
verification, integration (in-VM diagnostics 94 passed / 2 skipped; host
integration 47 passed / 0 failed / 0 warnings), ephemeral model check,
benchmark (1 passed), Linux `.deb` cross-compile validation for
`Capsem_1.1.1778456247_arm64.deb`, and Docker/systemd install e2e (33 passed /
31 skipped).

### T11.3 Local Package Generation and Install

- [x] Run `just test-install` equivalent through the final `just test`
  Docker/systemd install e2e gate.
- [x] Run `just install` to build the release package and install it locally.
- [x] Record generated package paths and installed binary paths in
  `tracker.md`.
- [x] Expand the generated `.pkg` or `.deb` and verify manifest, minisig,
  helper binaries, service files, and app bundle match T0/T5 expectations.
- [x] From the installed layout, run `~/.capsem/bin/capsem version`.
- [x] From the installed layout, run `~/.capsem/bin/capsem doctor`.
- [x] From the installed layout, run
  `~/.capsem/bin/capsem run "echo installed-cli-ok"`.
- [x] Confirm any package-specific manual proof from T0/T10 is recorded.

T11.3 note: the Docker/systemd `.deb` install gate is green, and the Linux
release cross-compile validates exactly one fresh `.deb`. Host `just install`
is still an explicit Gate D action because it mutates the local machine and
requires installed-app sign-off.

Host install rerun: `just install` built
`packages/Capsem-1.1.1778456247.pkg`, opened Installer.app, installed
`~/.capsem/bin/capsem` and companion binaries, and the service health check
responded. The recipe exposed a local postinstall bug where a pre-existing
`~/.capsem/assets -> repo/assets` symlink let root-owned package asset
manifests land in the repo; `scripts/pkg-scripts/postinstall` now removes that
symlink and creates a real per-user asset directory before seeding assets.
After repair, `~/.capsem/assets` is user-owned, the manifest verifies with the
dev key, service asset health is ready for `2026.0510.20`, and
`~/.capsem/bin/capsem run "echo installed-demo-ok"` printed
`installed-demo-ok`. The installed UI is now materialized at
`/Applications/Capsem.app` for the demo. Follow-up hardening added a
service-owned `/companions/tray/ensure` path plus app launch/focus/periodic
heal calls so a killed tray is relaunched by `capsem-service`, not by a
standalone tray process. Live proof: after killing the tray, opening
`/Applications/Capsem.app` spawned `capsem-tray` as a child of the installed
service; with the app already running, killing tray PID `79960` was healed to
tray PID `79981` under service PID `78909`. Installed doctor proof is now
captured: `~/.capsem/bin/capsem doctor` passed with 308 passed / 4 skipped and
`PASS -- all diagnostics passed`.

### T11.4 Elie + Codex Product Sign-Off

- [ ] Gate A sign-off recorded: JS UI policy flow verified with
  `just dev-frontend`, browser screenshots/evidence, and zero console
  errors/warnings.
- [x] Gate B command proof recorded: CLI and VM behavior verified with
  `just exec "echo cli-ok"`, `just exec "capsem-doctor"`, and the T8 policy
  E2E proof.
- [ ] Gate C sign-off recorded: desktop dev/full launch verified with
  `just build-ui` and `just run-ui --`. Command/process proof is captured;
  Elie visual sign-off remains open.
- [ ] Gate D sign-off recorded: installed package app launches and matches the
  CLI/package proof from T11.3. Installed app/tray relaunch proof is captured;
  Elie visual sign-off remains open.

### T11.5 Release Readiness Review

- [x] Re-run `git diff --check`.
- [x] Review `git status --short` and ensure unrelated user changes are not
  staged.
- [x] Confirm T0-T10 exit criteria are complete or explicitly release-blocking.
- [x] Confirm `CHANGELOG.md` and release metadata are included.
- [x] Confirm no active swarm agents are still running.

T11.5 note: no staging, commit, tag, push, or PR was performed. `git diff
--check` passed; the dirty tree remains intentionally unstaged.

Gate C note: `just build-ui` passed and rebuilt `target/debug/capsem-app`.
`just run-ui --` launched the dev desktop app, process proof showed the app
running with the installed service/gateway/tray, and `/version` responded.
Screenshot capture was blocked by macOS display permission
(`could not create image from display`), so Elie visual sign-off remains the
only Gate C hold. After Elie fixed screen capture permission, the release-polish
pass removed the visible `build {__BUILD_TS__}` timestamp from the toolbar/tab
area. `pnpm -C frontend run check`, `pnpm -C frontend run build`, and
`just build-ui` passed, and the screenshot evidence at
`sprints/release-policy-hardening/evidence/T11-gate-c-no-build-stamp.png`
shows the shell without the build stamp.

### T11.6 Handoff to T12

- [x] Do not tag until all P0/P1 blockers are closed.
- [x] Do not push a release tag from a dirty or partially verified tree.
- [x] If the release is deferred, record the blocking track and owner in
  `tracker.md`.
- [x] Record that T12 may begin only after this local gate is signed off by
  Elie.

## Proof Matrix

| Category | Required proof |
|---|---|
| Preflight | doctor, signing, notarization, manifest signing, workflow checks. |
| Full suite | `just test` passes after targeted verification. |
| Install/VM | `just install`, installed CLI proof, package proof, and capsem-doctor proof recorded. |
| Human proof | Elie signs off JS UI, CLI, desktop dev launch, and installed app launch. |
| Release hygiene | clean diff check, explicit staging, changelog/version metadata. |

Current evidence ledger:
`sprints/release-policy-hardening/evidence/T11-2026-05-10-full-release-gate.md`.

## Verification

- [x] `just doctor`
- [x] `env UV_CACHE_DIR=target/uv-cache scripts/preflight.sh` after restoring
  `private/` (40 passed / 0 failed)
- [x] `scripts/check-release-workflow.sh` after restoring `private/`
  (13 passed / 0 failed)
- [x] `just test`
- [x] Docker/systemd install e2e inside `just test`
- [x] `just install`
- [x] `~/.capsem/bin/capsem version`
- [x] `~/.capsem/bin/capsem doctor`
- [x] `~/.capsem/bin/capsem run "echo installed-demo-ok"`
- [ ] `just dev-frontend`
- [x] `just exec "capsem-doctor"`
- [x] `just build-ui`
- [x] `just run-ui --`
- [x] `open /Applications/Capsem.app`
- [x] `cd assets && b3sum --check B3SUMS`
- [x] `bash scripts/verify-local-manifest-signature.sh assets config/manifest-sign.pub`
- [x] `git diff --check`
- [x] `git status --short`

## Exit Criteria

- [x] Full suite is green.
- [x] `just install` generated and installed the package locally.
- [ ] Installed CLI, JS UI, desktop dev launch, and installed app launch are
  signed off by Elie.
- [x] Install/VM smoke is green or blocked with a recorded release owner.
- [x] Changelog/version/release metadata are synchronized.
- [ ] No release tag is created until T12 starts from a signed-off local gate.
