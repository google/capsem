# CI Release Landing 1.1 Findings
Status: completed
Agent: Codex

## Scope

Focused static gap review for the release-versioning and CI landing shift to
`1.1.xxx`. Reviewed the sprint control docs, release workflow, release scripts,
just recipes, and current version metadata. No production code changes.

## Findings

- [P0] Version stamping still emits the old `1.0.{timestamp}` line, so the next
  release can be accidentally tagged outside the sprint target. Impact:
  `just install` and `just cut-release` both depend on `_stamp-version`, which
  currently sets `NEW="1.0.$(date +%s)"`; the checked-in binary metadata is also
  still `1.0.1778378133`. Exact paths: `justfile`, `Cargo.toml`,
  `pyproject.toml`, `crates/capsem-app/tauri.conf.json`,
  `sprints/release-policy-hardening/MASTER.md`,
  `sprints/release-policy-hardening/T9-release-metadata-changelog.md`. Owning
  sprint target: T9. Required proof: `rg -n "1\\.0\\.|1\\.1\\." justfile
  Cargo.toml pyproject.toml crates/capsem-app/tauri.conf.json
  sprints/release-policy-hardening` shows only intentional historical references
  and all release-facing metadata/stamping paths select the exact `1.1.xxx`
  version before tag.

- [P0] The final CI landing track is only partially represented. Impact:
  `MASTER.md` defines T12 as the tag/CI-green/published-asset landing gate, but
  `tracker.md`, `plan.md`, `swarm.md`, and `T11-full-release-gate.md` still end
  release control at T11 or only mention tag hold, leaving no executable owner
  for waiting on CI green and verifying live published assets before declaring
  the release landed. Exact paths:
  `sprints/release-policy-hardening/MASTER.md`,
  `sprints/release-policy-hardening/tracker.md`,
  `sprints/release-policy-hardening/plan.md`,
  `sprints/release-policy-hardening/swarm.md`,
  `sprints/release-policy-hardening/T11-full-release-gate.md`. Owning sprint
  target: T12, with T11 handoff. Required proof: add/expand T12 in tracker and
  plan with explicit commands such as `just release v1.1.xxx`,
  `gh run view/watch`, `gh release view`, package payload inspection, and clean
  install/update proof from the live release.

- [P0] Linux release publication remains best-effort despite the new release
  checklist requiring package/rootfs validation to block publication. Impact:
  `build-app-linux` has `continue-on-error: true`, `create-release` downloads
  both Linux artifacts with `continue-on-error: true`, and the release summary
  permits no `.deb`; this contradicts `MASTER.md`'s "release-blocking" CI
  checklist and can land `v1.1.xxx` without required Linux package proof. Exact
  paths: `.github/workflows/release.yaml`,
  `sprints/release-policy-hardening/MASTER.md`,
  `sprints/release-policy-hardening/tracker.md`. Owning sprint target: T12
  with T0/T5. Required proof: release workflow fails before `create-release`
  when Linux `.deb` artifacts, package validation, or rootfs checks fail; no
  `continue-on-error` path can publish missing expected artifacts.

- [P0] Linux CI still builds and validates the stale companion-binary package
  contract. Impact: the macOS job builds `capsem-mcp-aggregator` and
  `capsem-mcp-builtin`, but the Linux job only builds
  `capsem`, `capsem-service`, `capsem-process`, `capsem-mcp`,
  `capsem-gateway`, and `capsem-tray`; its `.deb` validation only greps for
  service/gateway/tray, so the release can publish a Linux package missing MCP
  helpers required by `capsem-process`. Exact paths:
  `.github/workflows/release.yaml`, `justfile`,
  `sprints/release-policy-hardening/MASTER.md`,
  `sprints/release-policy-hardening/tracker.md`. Owning sprint target: T5 with
  T12 gate. Required proof: CI builds, repacks, and asserts
  `capsem-mcp-aggregator` and `capsem-mcp-builtin` in every `.deb` payload,
  e.g. `dpkg-deb -c ... | rg
  'capsem-mcp-aggregator|capsem-mcp-builtin'`.

- [P1] Rootfs validation in the release workflow is narrower than the sprint
  checklist. Impact: `MASTER.md` requires validating every guest binary needed
  by `capsem-init`, including `capsem-dns-proxy` and `capsem-sysutil`, but the
  Linux workflow check only looks for `capsem-pty-agent`, `capsem-net-proxy`,
  `capsem-mcp-server`, `capsem-doctor`, `capsem-bench`, and `snapshots`. Exact
  paths: `.github/workflows/release.yaml`, `scripts/preflight.sh`, `justfile`,
  `sprints/release-policy-hardening/MASTER.md`. Owning sprint target: T1/T5
  with T12 gate. Required proof: CI rootfs validation and local preflight use
  the same source-of-truth guest binary list and fail on missing
  `capsem-dns-proxy` or `capsem-sysutil`.

- [P1] Updater configuration remains incompatible with the publish workflow.
  Impact: `tauri.conf.json` enables updater artifacts and points at
  `latest.json`, while the release workflow creates/releases `.pkg`, `.deb`,
  `manifest.json`, and VM assets but does not publish or verify `latest.json`
  or compatible updater archives. Exact paths:
  `crates/capsem-app/tauri.conf.json`, `.github/workflows/release.yaml`,
  `scripts/check-release-workflow.sh`,
  `sprints/release-policy-hardening/MASTER.md`,
  `sprints/release-policy-hardening/tracker.md`. Owning sprint target: T0/T9
  with T12 gate. Required proof: either updater is disabled/hidden for
  `1.1.xxx`, or CI publishes and verifies the exact updater metadata and
  archives the configured Tauri updater will request.

- [P1] Local release-check scripts do not yet cover all new CI landing
  invariants. Impact: `scripts/preflight.sh` checks Apple certs, notarization,
  ephemeral init, and guest binary references; `scripts/check-release-workflow.sh`
  checks tool availability, Tauri key format, manifest signing dry-run, rootfs
  not bundled, and version sync. They do not fail on the workflow's Linux
  best-effort publishing, stale `.deb` helper validation, missing `latest.json`
  strategy, or the exact `1.1.xxx` stamp discipline. Exact paths:
  `scripts/preflight.sh`, `scripts/check-release-workflow.sh`,
  `.github/workflows/release.yaml`, `justfile`. Owning sprint target: T11/T12
  with T9/T0/T5. Required proof: `scripts/check-release-workflow.sh` catches
  those workflow-policy violations locally before a tag is pushed.

## Tests Run

Static review only. Cheap commands used: `sed`, `rg`, and `test -e` over the
approved sprint/workflow/version files.
