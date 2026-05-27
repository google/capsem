# Sprint Inventory

This directory keeps only active sprint control boards at the top level.
Completed boards live under `sprints/done/`; historical or superseded boards
live under `sprints/retired/`.

## Active Release Authority

- `policy-settings-profiles/` - active Profile V2 and bedrock release board.
  Enter through `policy-settings-profiles/NOW.md` for the current operational
  view, `policy-settings-profiles/MASTER.md` for the roadmap, and
  `policy-settings-profiles/tracker.md` for evidence.
- `policy-settings-profiles/release-hit-list.md` - historical installed-app and
  release usability evidence feeding the S24 immediate work queue.
- `credential-pipeline/` - standalone precursor for spec-driven credential,
  MCP, and skills detection. It feeds Profile V2 S10 credential brokerage,
  which is owned by the S24 meta sprint inside `policy-settings-profiles/`.

## Next Profile V2 Work

The release-blocking Profile V2 path is tracked inside
`policy-settings-profiles/NOW.md` and `policy-settings-profiles/MASTER.md`:

- `S24 - Post-Ship Profile V2 Meta Sprint` - active parent sprint for all
  remaining Profile V2 work: installed proof, release-hit-list migration,
  profile UI/settings/dashboard polish, Gemini/metrics installed VM proof,
  credential brokerage, workbench, metrics/reporting, plugins, local LLM,
  OpenAPI-to-MCP, quotas, docs/site, product expansion, and board
  reconciliation.
- S10/S12/S13/S14/S15/S16a/S17/S19a/S19b/S20/S21/S22/S23 remain separate
  child sprint files for design and acceptance criteria. They are in S24 scope,
  not parked outside the active Profile V2 plan.

## Folded Product Threads

- Better dashboard and stats work is folded into Profile V2: launch/profile UX
  in S16, structured timeline/workbench in S16a, live metrics in S12, and
  reporting/dashboard packaging in S19b. All are S24 child lanes.
- Credential release belongs to
  `policy-settings-profiles/S10-credential-brokerage.md`; source discovery and
  inventory stay in `credential-pipeline/`. S10 is a S24 child lane.
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
