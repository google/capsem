# Profile V2 Current Sprint State

Last updated: 2026-05-27

## Active Authority

- `../profile-foundation/MASTER.md` is now the active post-ship Profile V2
  Foundation roadmap.
- This S-numbered board remains historical evidence and source material.
- Use this file only when tracing old Profile V2 bedrock decisions.

## Superseded By Foundation Sprint

The Profile V2 bedrock release shipped. The active work moved to:

1. [Profile Foundation Sprint](../profile-foundation/MASTER.md).
2. Foundation F00-F12 are the trusted order.
3. Old S-numbered sprint files remain detailed source material and crosswalk
   entries, not execution order.
4. Installed proof gaps, product polish, credential brokerage, workbench,
   metrics/reporting, plugins, local LLM, OpenAPI-to-MCP, quotas, docs/site,
   and S23 product expansion are all Foundation scope.

The installed-app proof gaps and polish items from `release-hit-list.md` are
now Foundation F01 input.

## Closed For Bedrock

- S08/S08b/S08c/S08d: engine, corpus, gateway, and benchmark foundations have
  enough evidence for the shipped bedrock release. Remaining benchmark and
  engine polish is Foundation F02-F04 work.
- S09: CLI integration is closed for the bedrock release. Further command
  naming/output polish is Foundation F01/F05 product polish.
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
  of Foundation F06, not a separate active board.
- S12/S19b own metrics, OTel/export, dashboards, reporting, and operational
  packaging.
- S16a owns the larger workbench/timeline experience.
- S13/S20/S21/S22 own remote plugins, OpenAPI-to-MCP, local LLM, rate limits,
  budgets, and quotas.
- S23 remains the broad product-expansion lane, folded under S24 rather than
  competing with it as another "next" sprint.
