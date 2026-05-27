# Profile V2 Current Sprint State

Last updated: 2026-05-27

## Active Authority

- `MASTER.md` is the roadmap.
- `tracker.md` is the detailed evidence log.
- This file is the short operational view for the next session.

## Post-Ship Meta Sprint

The Profile V2 bedrock release shipped. The active work is now:

1. [S24 - Post-Ship Profile V2 Meta Sprint](S24-post-ship-profile-followup.md).
2. Keep `release-hit-list.md` as historical bug evidence; migrate active work
   into S24.
3. Treat all remaining open Profile V2 sprint files as S24 child sprints.
   Installed proof gaps, small product polish, credential brokerage, workbench,
   metrics/reporting, plugins, local LLM, OpenAPI-to-MCP, quotas, docs/site,
   and S23 product expansion are in scope.
4. Keep old retired boards retired unless their user problem is rewritten into
   a Profile V2 child sprint with current contracts and acceptance criteria.

The installed-app proof gaps and polish items from `release-hit-list.md` are
now the immediate S24 work queue.

## Closed For Bedrock

- S08/S08b/S08c/S08d: engine, corpus, gateway, and benchmark foundations have
  enough evidence for the shipped bedrock release. Remaining benchmark and
  engine polish is S24 child-sprint work.
- S09: CLI integration is closed for the bedrock release. Further command
  naming/output polish is S24 product polish.
- S11: status/debug/provenance is closed for bedrock truth. Full live metrics
  polish remains S12.
- S16: profile UI is closed for the bedrock release. Richer workbench/dashboard
  composition moves to S16a/S17/S19b.
- S18: release gate is historical; the release shipped.
- S19: docs/site contract is closed for bedrock; S24 owns post-ship corrections
  discovered while proving installed behavior.

## Child Sprint Boundaries

- `../credential-pipeline/` owns spec-driven host credential/source detection,
  MCP inventory, and skills inventory.
- S10 owns credential release/brokerage into sessions after the bedrock
  contracts are frozen. It consumes `credential-pipeline`; it is a child sprint
  of S24, not a separate active board.
- S12/S19b own metrics, OTel/export, dashboards, reporting, and operational
  packaging.
- S16a owns the larger workbench/timeline experience.
- S13/S20/S21/S22 own remote plugins, OpenAPI-to-MCP, local LLM, rate limits,
  budgets, and quotas.
- S23 remains the broad product-expansion lane, folded under S24 rather than
  competing with it as another "next" sprint.
