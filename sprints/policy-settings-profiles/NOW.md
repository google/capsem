# Profile V2 Current Sprint State

Last updated: 2026-05-27

## Active Authority

- `MASTER.md` is the roadmap.
- `tracker.md` is the detailed evidence log.
- This file is the short operational view for the next session.

## Bedrock Release Path

The Profile V2 sprint is no longer a broad exploration. Most implementation
lanes are closed for the bedrock cut. The active release decision is S18:

1. Run the broader `just smoke` / release packaging gate, or explicitly accept
   the narrower S18 replay matrix already recorded in `tracker.md`.
2. Keep `ask` non-user-facing unless S15 confirm resolution is implemented and
   verified. If ask stays disabled/pass-through, S15 is not a release blocker.
3. Do not reopen dashboard, forensics, Linux, old audit bugs, service split, or
   old frontend boards unless they are rewritten against the Profile V2
   contracts.

The installed-app bug closeout remains in `release-hit-list.md`. Most entries
are fixed in repo but still need installed package/UI/VM proof during S18.

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
- S19: docs/site contract is closed for bedrock; S18 owns the final docs replay.

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
