# Release Policy Hardening Swarm

## Purpose

Coordinate the no-edit investigation swarm for `sprint-policy-vw` /
`release-policy-hardening`. This is the operating board for agent slots,
domain ownership, finding-doc targets, and intake status.

## Rules

- Agents investigate only unless explicitly reassigned to implement.
- Each agent owns one domain and returns severity-ranked findings.
- Every finding must name exact code paths, tests/proofs, and a sprint task ID.
- Findings are captured in `sprints/release-policy-hardening/swarm-findings/`.
- Once a finding is transferred into T0-T12, mark the finding doc item as
  captured and close the agent slot.
- Do not tag or prepare release artifacts while any P0/P1 swarm finding remains
  uncaptured.

## Status Legend

- `[x] Done`: findings returned and captured in a finding doc.
- `[ ] In progress`: agent launched and still running.
- `[ ] Not launched`: waiting for an agent slot or more context.

## Finding Docs Index

| Status | Domain | Agent | Finding doc | Sprint targets |
|---|---|---|---|---|
| Done | UI policy/settings support | Jason `019e1263-534b-7702-864a-ca1f7b3a4f74` | [ui-policy-settings.md](swarm-findings/ui-policy-settings.md) | T2, T8, T10 |
| Done | T2 frontend execution audit | Boole `019e12e6-3d40-70e1-b10e-3c9c4d09e6e1` | [ui-policy-settings.md](swarm-findings/ui-policy-settings.md) | T2 |
| Done | Docs and release metadata | Copernicus `019e1263-54c4-7292-8d50-9f818cf7779f` | [docs-release-metadata.md](swarm-findings/docs-release-metadata.md) | T4, T9, T11 |
| Done | Sprint consistency | Meitner `019e1263-5600-72d0-9cdc-f19479b74540` | [sprint-consistency.md](swarm-findings/sprint-consistency.md) | T7, T10, T11 |
| Done | Core policy/assets | Kant `019e1264-dba6-7ae3-b34e-20edf051132d` | [core-policy-assets.md](swarm-findings/core-policy-assets.md) | T1, T3, T8, T10 |
| Done | Service/process integration | Kierkegaard `019e1264-dcba-79b0-9159-bebebceea23a` | [service-process.md](swarm-findings/service-process.md) | T3, T5, T8, T10 |
| Done | CLI/install/updater | Hypatia `019e1264-dd92-7c23-8767-a72c4f9ffc58` | [cli-updater-install.md](swarm-findings/cli-updater-install.md) | T0, T5, T9, T10, T11 |
| Done | MCP policy boundary | Chandrasekhar `019e1268-9d79-7cf1-bae8-7581987836b8` | [mcp-policy-boundary.md](swarm-findings/mcp-policy-boundary.md) | T3, T5, T6, T8, T10 |
| Done | Telemetry and session tooling | Hubble `019e1268-9f19-72e2-a586-3b2512af7d6e` | [telemetry-session.md](swarm-findings/telemetry-session.md) | T3, T6, T8, T10 |
| Done | Guest/image builder/rootfs | Erdos `019e1268-9e40-78f2-9751-b0550b4584d5` | [guest-image-builder.md](swarm-findings/guest-image-builder.md) | T1, T5, T10 |
| Done | CI packaging and release artifacts | Bernoulli `019e1269-a192-72f0-a6ee-67e338b017aa` | [ci-packaging.md](swarm-findings/ci-packaging.md) | T0, T1, T5, T10, T11 |
| Done | Verification architecture | Cicero `019e126a-d865-7ce3-9ec6-cc9b15637250` | [verification-architecture.md](swarm-findings/verification-architecture.md) | T7, T10, T11 |
| Done | Manual UI/CLI gates | Nietzsche `019e127d-2a58-77b2-9670-85ae8bc5d3a5` | [manual-ui-cli-gates.md](swarm-findings/manual-ui-cli-gates.md) | T10, T11 |
| Done | CI release landing 1.1 | Euler `019e127d-299b-7b12-af43-97a6d06e38aa` | [ci-release-landing-1-1.md](swarm-findings/ci-release-landing-1-1.md) | T9, T11, T12 |
| Done | Swarm transfer closeout | Lovelace `019e127d-2bf1-7a93-836a-92b03b40b854` | [swarm-transfer-closeout-2026-05-10.md](swarm-findings/swarm-transfer-closeout-2026-05-10.md) | T7, T10, T12 |
| Done | T3 hook client/spec execution audit | Lagrange `019e12fd-d72b-7ad1-ac5e-f1907235feac` | [core-policy-assets.md](swarm-findings/core-policy-assets.md) | T3 |
| Done | T3 MCP notification/telemetry execution audit | Socrates `019e12fd-d81b-72d3-a023-e618a6c2edb6` | [mcp-policy-boundary.md](swarm-findings/mcp-policy-boundary.md) | T3 |
| Done | T5 package/helper binary execution audit | Descartes `019e1312-4d46-7153-b010-aadc111f3797` | [ci-packaging.md](swarm-findings/ci-packaging.md) | T5 |
| Done | T5 process/env/reload execution audit | Volta `019e1312-61f4-7622-b6e6-ebc4fc63b508` | [service-process.md](swarm-findings/service-process.md) | T5 |
| Done | T5 route/rootfs validation execution audit | Hubble `019e1312-7f9e-7fe0-9608-67af861606f3` | [guest-image-builder.md](swarm-findings/guest-image-builder.md) | T5 |
| Done | T7 transfer mapping audit | Galileo `019e133b-8160-78b1-82e8-1b59a3f86a26` | [swarm-transfer-closeout-2026-05-10.md](swarm-findings/swarm-transfer-closeout-2026-05-10.md) | T7, T8, tracker |
| Done | T8 hook ship/defer scope audit | Gibbs `019e1342-9f35-7261-a62f-953938ceb395` | [core-policy-assets.md](swarm-findings/core-policy-assets.md), [service-process.md](swarm-findings/service-process.md), [ui-policy-settings.md](swarm-findings/ui-policy-settings.md) | T8, T9 |
| Done | T8 reload/telemetry proof audit | Mendel `019e1342-9fe8-7b81-b5cb-39d3712ef196` | [service-process.md](swarm-findings/service-process.md), [telemetry-session.md](swarm-findings/telemetry-session.md), [ui-policy-settings.md](swarm-findings/ui-policy-settings.md) | T8, T10 |

Compaction note: after any context reset, reopen this table first, then read the
finding docs for every row whose status is Done or In progress.

## Resume Protocol

After a crash, compaction, or handoff:

1. Read this file first.
2. Read every finding doc marked `Done` or `In progress` in the Finding Docs
   Index.
3. Poll all `In progress` agents by id if they still exist.
4. For any missing agent id, keep the finding doc status as the source of truth
   and relaunch only that domain if its doc has no completed findings.
5. Do not update T0-T12 implementation sprint docs until every P0/P1 finding
   from the current active wave is captured.
6. When all finding docs are populated, create or expand implementation
   sub-sprints so each P0/P1 has an owning task, exact files, exact tests, and
   release-gate proof.

## Required Finding Shape

Every populated finding doc must give enough information to build a detailed
sub-sprint without reading chat history:

- Severity: P0/P1/P2/P3.
- Release impact: what would ship broken if ignored.
- Exact paths and line anchors when known.
- Owning sprint task IDs or proposed new task/sub-sprint IDs.
- Required code/test proof.
- Required CI/package/UI/docs/VM proof where applicable.
- Whether tests were run or this was static investigation only.
- Transfer status: pending, captured in T0-T12, deferred with reason, or
  superseded by another finding doc.

## Completed Agents

- [x] Done: Jason, UI policy/settings support.
  - Agent id: `019e1263-534b-7702-864a-ca1f7b3a4f74`
  - Finding doc: [ui-policy-settings.md](swarm-findings/ui-policy-settings.md)
  - Sprint targets: T2, T8, T10.
  - Status: findings captured; slot ready to recycle.

- [x] Done: Boole, T2 frontend execution audit.
  - Agent id: `019e12e6-3d40-70e1-b10e-3c9c4d09e6e1`
  - Finding doc: [ui-policy-settings.md](swarm-findings/ui-policy-settings.md)
  - Sprint targets: T2.
  - Status: findings captured during T2 implementation; agent closed.

- [x] Done: Copernicus, docs and release metadata.
  - Agent id: `019e1263-54c4-7292-8d50-9f818cf7779f`
  - Finding doc:
    [docs-release-metadata.md](swarm-findings/docs-release-metadata.md)
  - Sprint targets: T4, T9, T11.
  - Status: findings captured; slot ready to recycle.

- [x] Done: Meitner, sprint consistency.
  - Agent id: `019e1263-5600-72d0-9cdc-f19479b74540`
  - Finding doc: [sprint-consistency.md](swarm-findings/sprint-consistency.md)
  - Sprint targets: T7, T10, T11.
  - Status: findings captured; slot ready to recycle.

- [x] Done: Hypatia, CLI/install/updater.
  - Agent id: `019e1264-dd92-7c23-8767-a72c4f9ffc58`
  - Finding doc: [cli-updater-install.md](swarm-findings/cli-updater-install.md)
  - Sprint targets: T0, T5, T9, T10, T11.
  - Status: findings captured; slot ready to recycle.

- [x] Done: Kant, capsem-core policy/assets.
  - Agent id: `019e1264-dba6-7ae3-b34e-20edf051132d`
  - Finding doc: [core-policy-assets.md](swarm-findings/core-policy-assets.md)
  - Sprint targets: T1, T3, T8, T10.
  - Status: findings captured; slot ready to recycle.

- [x] Done: Kierkegaard, service/process integration.
  - Agent id: `019e1264-dcba-79b0-9159-bebebceea23a`
  - Finding doc: [service-process.md](swarm-findings/service-process.md)
  - Sprint targets: T3, T5, T8, T10.
  - Status: findings captured; slot ready to recycle.

- [x] Done: Chandrasekhar, MCP policy boundary.
  - Agent id: `019e1268-9d79-7cf1-bae8-7581987836b8`
  - Finding doc: [mcp-policy-boundary.md](swarm-findings/mcp-policy-boundary.md)
  - Sprint targets: T3, T5, T6, T8, T10.
  - Status: findings captured; slot ready to recycle.

- [x] Done: Hubble, telemetry and session tooling.
  - Agent id: `019e1268-9f19-72e2-a586-3b2512af7d6e`
  - Finding doc: [telemetry-session.md](swarm-findings/telemetry-session.md)
  - Sprint targets: T3, T6, T8, T10.
  - Status: findings captured; slot ready to recycle.

- [x] Done: Erdos, guest/image builder/rootfs.
  - Agent id: `019e1268-9e40-78f2-9751-b0550b4584d5`
  - Finding doc: [guest-image-builder.md](swarm-findings/guest-image-builder.md)
  - Sprint targets: T1, T5, T10.
  - Status: findings captured; slot ready to recycle.

- [x] Done: Bernoulli, CI packaging and release artifacts.
  - Agent id: `019e1269-a192-72f0-a6ee-67e338b017aa`
  - Finding doc: [ci-packaging.md](swarm-findings/ci-packaging.md)
  - Sprint targets: T0, T1, T5, T10, T11.
  - Status: findings captured; slot ready to recycle.

- [x] Done: Cicero, verification architecture and sprint slicing.
  - Agent id: `019e126a-d865-7ce3-9ec6-cc9b15637250`
  - Finding doc:
    [verification-architecture.md](swarm-findings/verification-architecture.md)
  - Sprint targets: T7, T10, T11.
  - Status: findings captured; slot ready to recycle.

- [x] Done: Nietzsche, manual UI/CLI gates.
  - Agent id: `019e127d-2a58-77b2-9670-85ae8bc5d3a5`
  - Finding doc: [manual-ui-cli-gates.md](swarm-findings/manual-ui-cli-gates.md)
  - Sprint targets: T10, T11.
  - Status: findings captured; slot ready to recycle.

- [x] Done: Euler, CI release landing 1.1.
  - Agent id: `019e127d-299b-7b12-af43-97a6d06e38aa`
  - Finding doc:
    [ci-release-landing-1-1.md](swarm-findings/ci-release-landing-1-1.md)
  - Sprint targets: T9, T11, T12.
  - Status: findings captured; slot ready to recycle.

- [x] Done: Lovelace, swarm transfer closeout.
  - Agent id: `019e127d-2bf1-7a93-836a-92b03b40b854`
  - Finding doc:
    [swarm-transfer-closeout-2026-05-10.md](swarm-findings/swarm-transfer-closeout-2026-05-10.md)
  - Sprint targets: T7, T10, T12.
  - Status: findings captured; slot ready to recycle.

- [x] Done: Lagrange, T3 hook client/spec execution audit.
  - Agent id: `019e12fd-d72b-7ad1-ac5e-f1907235feac`
  - Finding doc: [core-policy-assets.md](swarm-findings/core-policy-assets.md)
  - Sprint targets: T3.
  - Status: findings captured during T3 implementation; agent closed.

- [x] Done: Socrates, T3 MCP notification/telemetry execution audit.
  - Agent id: `019e12fd-d81b-72d3-a023-e618a6c2edb6`
  - Finding doc:
    [mcp-policy-boundary.md](swarm-findings/mcp-policy-boundary.md)
  - Sprint targets: T3.
  - Status: findings captured during T3 implementation; agent closed.

- [x] Done: Galileo, T7 transfer mapping audit.
  - Agent id: `019e133b-8160-78b1-82e8-1b59a3f86a26`
  - Finding doc:
    [swarm-transfer-closeout-2026-05-10.md](swarm-findings/swarm-transfer-closeout-2026-05-10.md)
  - Sprint targets: T7, T8, tracker.
  - Status: no orphaned P0/P1 findings found; stale status wording and
    conditional missing-test ownership captured during T7 closeout.

- [x] Done: Gibbs, T8 hook ship/defer scope audit.
  - Agent id: `019e1342-9f35-7261-a62f-953938ceb395`
  - Finding docs:
    [core-policy-assets.md](swarm-findings/core-policy-assets.md),
    [service-process.md](swarm-findings/service-process.md),
    [ui-policy-settings.md](swarm-findings/ui-policy-settings.md)
  - Sprint targets: T8, T9.
  - Status: defer decision captured; direct `policy.hook.*` write/import
    rejection and release-wording tightening transferred into T8.

- [x] Done: Mendel, T8 reload/telemetry proof audit.
  - Agent id: `019e1342-9fe8-7b81-b5cb-39d3712ef196`
  - Finding docs:
    [service-process.md](swarm-findings/service-process.md),
    [telemetry-session.md](swarm-findings/telemetry-session.md),
    [ui-policy-settings.md](swarm-findings/ui-policy-settings.md)
  - Sprint targets: T8, T10.
  - Status: SettingsPage banner/dismissal gap and live `/settings` +
    `/reload-config` E2E/timeline proof gaps transferred into T8.

## Active Agents

- [x] None.

## Launch Queue

- [x] Empty: all planned domains have launched.

## Intake Checklist

- [x] Poll active T3 agents.
- [x] Move completed T3 agent output into its finding doc.
- [x] Mark each T3 agent `[x] Done` after the finding doc is populated.
- [x] Launch the next queued agent into the freed slot.
- [x] Deduplicate P0/P1 findings across finding docs.
- [x] Transfer captured findings into the owning T0-T12 sprint docs.
- [x] Update `tracker.md`, `MASTER.md`, and `T7-active-review-followups.md`
  after all current agent outputs are captured.
- [x] Run stale-status searches after transfer.
- [x] Poll active T5 agents.
- [x] Move completed T5 agent output into finding docs.
- [x] Mark each T5 agent `[x] Done` after the finding doc is populated.

## Completeness Gate

The swarm investigation is not complete until all of these are true:

- [x] No finding doc contains `Awaiting agent output`.
- [x] The Finding Docs Index has no `In progress` rows.
- [x] Every completed finding doc has at least one severity-ranked finding or
  explicitly says the domain had no release-blocking findings.
- [x] Every P0/P1 finding has an owning T0-T12 task ID or proposed new
  sub-sprint ID.
- [x] Every P0/P1 finding names required proof commands/tests.
- [x] `verification-architecture.md` has reviewed whether the finding docs are
  sufficient to generate detailed implementation sprints.
- [x] `MASTER.md`, `tracker.md`, and `T7-active-review-followups.md` have been
  synchronized after the final intake.

## Current Slot Limit

The app currently allows six active agent threads in this workspace. All planned
investigation domains and the final T7 mapping audit have launched and
reported. Remaining work is resolving the FD01-FD14 downstream blocker
checkboxes in `T7-active-review-followups.md` while executing T8-T12.
