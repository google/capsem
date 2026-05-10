# Manual UI CLI Gate Findings
Status: completed
Agent: Codex

## Scope

Focused static review for explicit Elie + Codex manual verification gates before
CI: CLI behavior, JS UI policy flow, desktop full launch, and local package
install. Reviewed only the sprint docs, toolchain skills, CLI command surface,
and `justfile` command definitions needed to validate gate commands and proof
capture expectations.

## Findings

- [P0] T11 does not run the final local package install gate before CI. Impact:
  the master execution spine says T11 must be the local ship gate and Gate D
  says no T12 CI release work starts until `just install` plus installed
  CLI/UI/full-launch proof is signed off, but T11 only lists `just test`,
  `just test-install`, and an install smoke. Exact paths:
  `sprints/release-policy-hardening/MASTER.md`,
  `sprints/release-policy-hardening/T11-full-release-gate.md`,
  `sprints/release-policy-hardening/tracker.md`, `justfile`. Owning sprint
  target: T11 local release candidate gate, with T0/T5 package contents as
  dependencies. Required proof: add `just install` as the final local gate in
  T11/tracker, then record package-installed `~/.capsem/bin/capsem version`,
  `~/.capsem/bin/capsem doctor`, `~/.capsem/bin/capsem run "echo installed-cli-ok"`,
  package payload inspection, installed desktop launch evidence, and Elie's
  sign-off in `tracker.md`.

- [P0] T11 contains an invalid/manual CLI smoke command. Impact: the listed
  `just run "capsem-doctor"` command cannot execute because `justfile` defines
  `exec +CMD` but no `run` recipe; a release executor can block at the final
  gate or silently substitute an unrecorded command. Exact paths:
  `sprints/release-policy-hardening/T11-full-release-gate.md`,
  `sprints/release-policy-hardening/plan.md`, `crates/capsem/src/main.rs`,
  `justfile`. Owning sprint target: T11.3 VM and Install Smoke. Required proof:
  replace the recipe command with a valid one, likely `just exec "capsem-doctor"`
  for the dev recipe path or `~/.capsem/bin/capsem run "capsem-doctor"` for the
  installed path, and capture the exact stdout/stderr path or transcript in
  `tracker.md`.

- [P1] Manual stop points exist in `MASTER.md` but are not mirrored as blocking
  checklist rows in T10/T11/tracker. Impact: Elie + Codex verification can be
  treated as descriptive guidance instead of a release-blocking workflow,
  especially for JS UI, CLI/session proof, desktop full launch, and installed
  app proof. Exact paths: `sprints/release-policy-hardening/MASTER.md`,
  `sprints/release-policy-hardening/T10-focused-verification.md`,
  `sprints/release-policy-hardening/T11-full-release-gate.md`,
  `sprints/release-policy-hardening/tracker.md`. Owning sprint targets: T10.3,
  T10.5, T10.7, T11.3, T11.5. Required proof: add explicit Gate A-D rows to
  tracker/T10/T11 with status, command, evidence path, Elie sign-off, and a
  rule that T10 cannot close before Gates A/B and T11/T12 cannot start before
  Gates C/D are signed off.

- [P1] Evidence capture is underspecified for durable docs versus chat. Impact:
  Gate A asks for screenshot or Chrome DevTools evidence in `tracker.md`, and
  T10.7 asks for command/pass/fail/follow-up owner, but there is no consistent
  place or schema for screenshot paths, console output, installed app evidence,
  package payload logs, or CLI transcripts; proof could disappear in chat.
  Exact paths: `sprints/release-policy-hardening/MASTER.md`,
  `sprints/release-policy-hardening/T2-frontend-policy-settings.md`,
  `sprints/release-policy-hardening/T10-focused-verification.md`,
  `sprints/release-policy-hardening/T11-full-release-gate.md`,
  `sprints/release-policy-hardening/tracker.md`. Owning sprint targets: T2.7,
  T10.7, T11.3. Required proof: define an evidence ledger in `tracker.md` with
  durable artifact paths for screenshots, console checks, command transcripts,
  package inspection output, and installed desktop launch notes.

- [P2] Desktop full-launch verification is split between dev binary and
  installed app, but T11 does not require the installed app full-launch path.
  Impact: Gate C validates `just build-ui` / `just run-ui --`, while Gate D
  requires the package-installed app; T11 currently covers neither as a named
  blocking checklist item, so embedded frontend and package launcher regressions
  can escape local gating. Exact paths:
  `sprints/release-policy-hardening/MASTER.md`,
  `sprints/release-policy-hardening/T11-full-release-gate.md`,
  `skills/dev-just/SKILL.md`, `justfile`. Owning sprint target: T11.3 VM and
  Install Smoke. Required proof: record both dev desktop evidence
  (`just build-ui`, `just run-ui --`) and installed desktop evidence after
  `just install`, including stamped build/version, gateway/service reachability,
  Settings -> Policy render, reload-failure truth, screenshots, and console/log
  paths.

## Tests Run

Static review only. Read sprint docs, `skills/dev-just/SKILL.md`,
`skills/dev-testing-frontend/SKILL.md`, `crates/capsem/src/main.rs`, and
`justfile`; no runtime verification commands were executed.
