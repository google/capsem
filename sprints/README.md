# Sprint Inventory

This directory keeps active sprint control boards at the top level and
historical boards under `sprints/retired/`.

## Active Release Authority

- `policy-settings-profiles/` - active Profile V2 and bedrock release board.
  Enter through `policy-settings-profiles/MASTER.md`, then use
  `policy-settings-profiles/tracker.md` for current status.
- `policy-settings-profiles/release-hit-list.md` - active installed-app and
  release usability closeout board.

## Next Profile V2 Work

The release-blocking Profile V2 path is tracked inside
`policy-settings-profiles/MASTER.md`:

- `S08b - Bedrock Engine` - finish engine boundaries, canonical event journal,
  emitter ownership, and runtime dispatch for shipped event families.
- `S09 - CLI Integration` - keep the usable CLI surface aligned with the
  bedrock contract.
- `S11 - Status, Debug, Provenance` - make status/debug/logs explain shipped
  truth.
- `S15 - Confirm UX (Ask)` - replace placeholder ask behavior with real UI/CLI
  resolution before advertising user-facing ask.
- `S16 - Profile UI` - first-class profile catalog, profile-backed session
  creation, and runtime visibility.
- `S18 - Full Verification And Release Gate` - final install/VM/E2E release
  proof.
- `S19 - Documentation And Site` - document shipped behavior and explicit
  deferrals.

## Other Live Queues

- `audit-bugs/` - security/policy bug queue from audit findings.
- `credential-pipeline/` - future credential detection/brokerage pipeline.
- `debug-report-settings/` - debug report verification follow-up.
- `forensics/` - future forensic API/CLI/search work.
- `linux/` - Linux package/release notes.

These queues are not allowed to override the Profile V2 release board. Promote
requirements into `policy-settings-profiles/` before treating them as release
blocking for the bedrock release.

## Retired

`sprints/retired/` contains historical planning boards that are useful for
archaeology but are no longer planning authority. Do not infer active scope,
endpoint names, command names, or release requirements from retired boards.

Important retired groups:

- `retired/profile-v2-*` - early Profile V2 rescue and side-sprint boards.
- `retired/mcp-policy-v2`, `retired/mitm-mcp-unification`,
  `retired/mitm-redesign`, `retired/mcp-endpoint-coverage` - superseded by the
  Profile V2 bedrock board.
- `retired/release-policy-hardening` and `retired/release-debug-loop` -
  historical release hardening notes; current release process lives in the
  release skill and the active Profile V2 release hit list.
- `retired/next-gen` - historical platform roadmap, superseded for current
  release sequencing.

When reviving a retired idea, copy the user problem, acceptance criteria, and
current architecture fit into a live sprint file instead of editing the retired
board in place.
