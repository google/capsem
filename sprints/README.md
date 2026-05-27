# Sprint Inventory

This directory keeps only active sprint control boards at the top level.
Completed boards live under `sprints/done/`; historical or superseded boards
live under `sprints/retired/`.

## Active Release Authority

- `profile-foundation/` - active post-ship Profile V2 Foundation meta sprint.
  Enter through `profile-foundation/MASTER.md`; `tracker.md` records code
  checks and execution status. This board owns the current F-numbered order.
- `policy-settings-profiles/` - active Profile V2 and bedrock release board.
  This is now historical evidence and source material for the Foundation
  sprint. Enter through `policy-settings-profiles/NOW.md` only when tracing old
  S-numbered decisions.
- `policy-settings-profiles/release-hit-list.md` - historical installed-app and
  release usability evidence feeding the S24 immediate work queue.
- `credential-pipeline/` - standalone precursor for spec-driven credential,
  MCP, and skills detection. It feeds Profile V2 S10 credential brokerage,
  which is owned by the S24 meta sprint inside `policy-settings-profiles/`.

## Next Profile V2 Work

The active Profile V2 path is tracked inside
`profile-foundation/MASTER.md`:

- `Profile Foundation Sprint` - active parent sprint for all remaining
  foundation work. It exits only when installed Profile V2 behavior, the full
  Security Event system, security plugins, remote decisions, remote alert
  logging, Google/Gemini integration, credentials, OpenTelemetry,
  metrics/reporting, dashboard improvements, workbench, integrations, quotas,
  docs/site, and final release proof are trusted.
- F00-F12 are the execution order. Old S-numbered files remain source material,
  not ordering authority.

## Folded Product Threads

- Better dashboard and stats work is folded into Profile V2: launch/profile UX
  in S16, structured timeline/workbench in S16a, live metrics in S12, and
  reporting/dashboard packaging in S19b. All are Foundation child lanes.
- Credential release belongs to
  `policy-settings-profiles/S10-credential-brokerage.md`; source discovery and
  inventory stay in `credential-pipeline/`. S10 is carried by Foundation F06.
- Linux, old audit bugs, old forensics, and older service/frontend refactors
  are retired until they are rewritten against the Profile V2 contracts.

## Retired

`sprints/done/` contains completed one-off boards. `sprints/retired/` contains
historical planning boards that are useful for archaeology but are no longer
planning authority. Do not infer active scope, endpoint names, command names, or
release requirements from retired boards.

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
- `retired/analytics-dashboard` and `retired/better_stats` - useful dashboard
  ideas folded into S16/S16a/S12/S19b.
- `retired/linux*`, `retired/audit-bugs`, and `retired/forensics` - old queues
  that need a Profile V2-native reboot before becoming active work again.

When reviving a retired idea, copy the user problem, acceptance criteria, and
current architecture fit into a live sprint file instead of editing the retired
board in place.
