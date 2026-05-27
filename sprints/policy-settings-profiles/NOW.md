# Profile V2 Current Sprint State

Last updated: 2026-05-27

## Active Authority

- `MASTER.md` is the roadmap.
- `tracker.md` is the detailed evidence log.
- This file is the short operational view for the next session.

## Post-Ship Path

The Profile V2 bedrock release shipped. The active work is now:

1. [S24 - Post-Ship Profile V2 Follow-Up](S24-post-ship-profile-followup.md).
2. Keep `release-hit-list.md` as historical bug evidence; migrate active work
   into S24.
3. Keep `ask` non-user-facing unless S15 confirm resolution is implemented and
   verified. If ask stays disabled/pass-through, S15 is post-ship work.
4. Do not reopen dashboard, forensics, Linux, old audit bugs, service split, or
   old frontend boards unless they are rewritten against the Profile V2
   contracts.

The installed-app proof gaps and polish items from `release-hit-list.md` are
now S24 tasks.

## Closed For Bedrock

- S08/S08b/S08c/S08d: engine, corpus, gateway, and benchmark foundations have
  enough evidence for S18 to replay. Remaining benchmark/reporting polish is
  post-bedrock unless S18 finds a release-blocking gap.
- S09: CLI integration is closed for the bedrock release. Further command
  naming/output polish is post-bedrock.
- S11: status/debug/provenance is closed for bedrock truth. Full live metrics
  polish remains S12.
- S16: profile UI is closed for the bedrock release. Richer workbench/dashboard
  composition moves to S16a/S17/S19b.
- S18: release gate is historical; the release shipped.
- S19: docs/site contract is closed for bedrock; S24 owns post-ship corrections
  discovered while proving installed behavior.

## Still Separate

- `../credential-pipeline/` owns spec-driven host credential/source detection,
  MCP inventory, and skills inventory.
- S10 owns credential release/brokerage into sessions after the bedrock
  contracts are frozen. It consumes `credential-pipeline`; it is not the same
  work.

## Later / Not Release Blocking

- S12 full OTel/dashboard polish.
- S13 remote enforcement plugin.
- S14/S15/S17 richer rules/confirm/capabilities UX unless ask is shipped.
- S16a unified timeline/workbench.
- S19a marketing refresh.
- S19b reporting setup and dashboards.
- S20/S21/S22/S23 product expansions.
