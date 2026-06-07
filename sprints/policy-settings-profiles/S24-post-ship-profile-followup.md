# S24 - Post-Ship Profile V2 Meta Sprint

Status: superseded by
[Profile Foundation Sprint](../profile-foundation/MASTER.md).

S24 established that all remaining Profile V2 work is in scope. The Foundation
sprint replaces S24 as the active execution board with F-numbered ordering,
code reality checks, and explicit foundation exit criteria.

## Goal

Run one clean Profile V2 meta sprint after the bedrock release.

The release shipped. S24 established that all remaining Profile V2 work belongs
in one parent queue, including installed proof gaps, small product polish, and
the larger follow-on product sprints that were previously parked as "next" or
"post-bedrock." Active execution has moved to Foundation F00-F12.

## Operating Rule

- The Foundation sprint is the active Profile V2 parent sprint.
- Child sprint files remain the source of detailed design and acceptance
  criteria.
- `release-hit-list.md` remains historical installed-app evidence; active
  fixes and proof tasks are worked through S24.
- `../credential-pipeline/` remains a standalone precursor board for
  credential/source discovery inventory, but S10 credential brokerage is a
  Profile V2 child sprint under S24.
- Bigger workbench, metrics, reporting, quotas, local LLM, OpenAPI-to-MCP, and
  plugin work are in scope as S24 child sprints. They are sequenced, not
  rejected.

## Child Sprint Map

| Lane | Child Sprint | Status | Scope |
| --- | --- | --- | --- |
| Immediate installed proof | `release-hit-list.md` plus this S24 file | Foundation F01 Input | Prove the shipped package, service/gateway readiness, dashboard/profile cards, Settings Profiles, CLI/VM startup, repeated install coherence, and profile provisioning truth. |
| Engine/performance follow-up | [S08b](S08b-bedrock-engine.md), [S08d](S08d-engine-performance-benchmarks.md) | Child Sprint | Preserve shipped bedrock contracts while closing remaining runtime dispatch, journal, benchmark, concurrency, and release artifact proof gaps. |
| CLI/product polish | [S09](S09-cli-integration.md) | Child Sprint | Command naming/output polish that was not needed for the bedrock release but affects day-to-day Profile V2 usability. |
| Credential brokerage | [S10](S10-credential-brokerage.md) | Child Sprint | Release credentials from service/profile settings into sessions using the frozen Profile V2 contracts and the `credential-pipeline` discovery output. |
| Status, metrics, dashboards | [S11](S11-status-debug-provenance.md), [S12](S12-observability-plugin.md), [S19b](S19b-reporting-setup.md) | Child Sprints | Finish live status truth, token/cost counters, OTel/export polish, dashboard/reporting packaging, privacy guidance, and operational proof. |
| Rules and confirm UX | [S14](S14-rules-ui-components.md), [S15](S15-confirm-ux.md), [S17](S17-security-capabilities-ui.md) | Child Sprints | Rule editors, ask/confirm resolution, capability controls, finding/backtest views, and any user-facing ask behavior. |
| Product UI and workbench | [S16](S16-profile-ui.md), [S16a](S16a-unified-timeline-and-agent-workbench.md) | Child Sprints | Profile UI polish plus the everyday-work timeline/workbench and structured `/timeline/{id}` API. |
| Product integrations | [S13](S13-remote-policy-plugin.md), [S20](S20-openapi-to-mcp.md), [S21](S21-local-llm.md), [S22](S22-rate-limits-budgets-and-quotas.md) | Child Sprints | Remote plugins, OpenAPI-to-MCP, local model providers, rate limits, budgets, and quotas. |
| Site and market story | [S19](S19-documentation-and-site.md), [S19a](S19a-marketing-site-refresh.md) | Child Sprints | Post-ship docs corrections, marketing refresh, performance-backed claims, and install/setup wording. |
| Product expansion umbrella | [S23](S23-post-bedrock-improvements.md) | Foundation Crosswalk Input | Broad post-bedrock product ideas stay in S23 as source material and are now ordered by the Foundation sprint rather than a competing "next" sprint. |

## Immediate Work Queue

### A. Installed Product Proof

Close the shipped-but-needs-proof items from `release-hit-list.md`:

- Package/UI waits for setup, service, and gateway readiness before opening.
- Onboarding, Settings Profiles, and dashboard profile cards use installed
  profiles and do not surface catalog emptiness as a user error.
- Dashboard/session creation starts from visible profile cards.
- UI does not show offline while service, gateway, tray, and CLI are healthy.
- `capsem run "echo test"` and `capsem shell` work immediately after install.
- Interrupted or repeated installs leave profile metadata and asset hashes
  coherent.
- Settings opens against the Profile V2 `/settings` envelope.
- Profile cards do not advertise unprovisionable profiles.

### B. Small Product Polish

- RHB-006: profile auth 401 during setup/onboarding.
- RHB-011: confusing developer `just install` output after setup completes.
- RHB-012: verify 4 CPU / 8 GB RAM / 8 active VM defaults and active-only
  counting.
- RHB-013: old credential scan clarity and source-by-source diagnostics,
  coordinated with `../credential-pipeline/` and then carried into S10.
- RHB-017: installed VM proof for Gemini credential projection and wrapper
  defaults.
- RHB-018: installed VM proof that live `/status` and toolbar counters reflect
  model tokens/cost.

### C. Board Reconciliation

- Mark `release-hit-list.md` items as closed, migrated, or active under
  Foundation F01.
- Reconcile stale S08b/S08d/S09/S11/S16/S18/S23 wording so the active board
  reflects the shipped release plus remaining child sprint work.
- Keep child sprint acceptance criteria visible instead of burying work in a
  generic "later" bucket.

## Tasks

- [ ] T0: Installed-state inventory. Record current installed version, service
      status, gateway status, profile list, profile catalog state, and app
      startup state.
- [ ] T1: Installed UI proof. Onboarding, Settings Profiles, dashboard profile
      cards, Settings page, and offline-state recovery.
- [ ] T2: Installed CLI/VM proof. `capsem status`, `capsem run "echo test"`,
      `capsem shell`, profile asset coherence, and package hook readiness.
- [ ] T3: Product polish fixes. RHB-006, RHB-011, RHB-012, RHB-013.
- [ ] T4: VM/provider proof. Gemini env/wrapper and live token/cost counters.
- [ ] T5: Child sprint reconciliation. Update S08b/S08d/S09/S10/S11/S12/S13/
      S14/S15/S16/S16a/S17/S19/S19a/S19b/S20/S21/S22/S23 statuses as each
      lane starts or closes.
- [ ] T6: Board closeout. Update `release-hit-list.md`, `NOW.md`, `MASTER.md`,
      `tracker.md`, and this file with proof and remaining child sprint state.

## Coverage Ledger

- Unit/contract: focused tests for any code changes in each child sprint.
- Functional: installed app/profile/settings/dashboard flows; child sprint API
  and CLI paths.
- Adversarial: stale auth token, service restart, missing signed catalog
  revision, interrupted install, profile asset mismatch, quota/credential
  denial paths, and ask timeout/denial behavior where implemented.
- E2E/VM: installed `capsem run`, `capsem shell`, Gemini env/wrapper proof,
  live metrics proof, and child sprint VM paths when they cross runtime
  boundaries.
- Telemetry: `/status`, toolbar counters, logs/debug breadcrumbs, OTel/export
  paths, reports, and child sprint auditability.
- Performance: S08d and any workbench/metrics/reporting regressions.
- Missing/deferred: none at the meta-sprint level. Unstarted child sprints are
  visible Foundation scope, not exclusions.
