# T4: Docs and Release Notes

## Objective

Make release and public docs match what actually ships. Docs must not claim
configured hook dispatch, stale artifact formats, old DNS architecture, or
outdated telemetry fields while the fix sprint is still closing those gaps.

## Owned Files

- `README.md`
- `docs/src/content/docs/releases/1-0.md`
- `docs/src/content/docs/security/policy.md`
- `docs/src/content/docs/architecture/session-telemetry.md`
- `docs/src/content/docs/architecture/asset-pipeline.md`
- `docs/src/content/docs/architecture/service-architecture.md`
- `docs/src/content/docs/getting-started.md`
- `docs/src/content/docs/development/ci.md`
- `docs/src/content/docs/development/stack.md`
- `docs/src/content/docs/development/just-recipes.md`
- `docs/src/content/docs/security/build-verification.md`
- `docs/src/content/docs/benchmarks/results.md`
- `site/src/components/CTA.svelte`
- `site/src/lib/data.ts`

## Findings

- [P1] Release docs, policy docs, and changelog imply configured external hook
  callouts are product-shipped. Code has Spec0/client/audit/fail-closed
  infrastructure, but no production path that wires user/corp hook config into
  MCP/HTTP/DNS/model runtime dispatch.
- [P1] Active install/release docs still advertise dead artifacts:
  `.dmg`/DMG, AppImage, and required `latest.json`.
- [P2] Session telemetry docs lack `policy_hook_events` in the ER diagram,
  data-flow diagram, and write-op table.
- [P2] Session telemetry docs still say `tool_calls.origin =
  native/local/mcp_proxy` and `mcp_call_id` reserved, while current code/tests
  use `origin = mcp` and `mcp_call_id`.
- [P2] Model/tool policy telemetry wording under-describes current enforcement
  and redaction.
- [P2] Public site FAQ still references fake DNS via `dnsmasq`; current path is
  `capsem-dns-proxy`.
- [P3] Benchmark docs still document a 12MB fork image gate; current test gate
  is 16MB.

## Swarm Transfer Tracker

| Source | Priority | Owner task | Required transfer point | Required proof |
|---|---:|---|---|---|
| FD02 docs-release-metadata | P0 | T4.1 | Hook dispatch is overclaimed in release/security docs while T8 has not proved production hook dispatch. | Stale-term search finds no release-facing external/remote hook dispatch claim unless T8 ships it. |
| FD02 docs-release-metadata | P0 | T4.2 | Active docs/site still advertise stale artifacts and updater state: DMG, AppImage, and required `latest.json`. | Stale-term search plus docs/site builds pass after wording matches T0. |
| FD02 docs-release-metadata | P1 | T4.3 | Session telemetry docs are stale around tool-call origin, `mcp_call_id`, `policy_hook_events`, and `WriteOp::PolicyHookEvent`. | Docs updated after T6 verifies schema/tooling behavior. |
| FD02 docs-release-metadata | P1 | T4.4 | Public site and benchmark docs contain stale implementation claims such as `dnsmasq`, fake DNS, and 12MB. | Stale-term search returns no active stale claims. |
| FD02 docs-release-metadata | P1 | T4.2, T9.4 | Release notes/page must pin artifact truth: `.pkg`, `.deb`, manifest/signature, SBOM, arch VM assets, and no `latest.json` unless shipped. | Release page and changelog match the T0 artifact decision and T12 asset expectations. |

## Task List

### T4.1 Release Claims

- [x] Reword v1.0 release page to distinguish shipped Hook Spec0/client/audit
  infrastructure from configured hook dispatch follow-up.
- [x] Reword security policy docs the same way.
- [x] Record changelog/latest-release wording requirements for T9 so release
  metadata does not overclaim runtime hook dispatch.
- [x] If T8 hides hook UI/runtime for this release, document that limit.

### T4.2 Artifact and Install Docs

- [x] Replace active `.dmg`/DMG references with the current `.pkg` behavior.
- [x] Replace AppImage references with the current `.deb`-only Linux behavior.
- [x] Remove required `latest.json` updater feed references unless T0 ships a
  real updater feed.
- [x] Update README, getting started, CI docs, stack docs, just recipes, build
  verification docs, service architecture docs, and site CTA.
- [x] Update release verification docs to require package payload manifest
  signature checks and clean install checks.

### T4.3 Session Telemetry Docs

- [x] Add `policy_hook_events` to the ER diagram.
- [x] Add policy hook source/write path to the data-flow diagram.
- [x] Add `WriteOp::PolicyHookEvent` to the write-op table.
- [x] Add Policy V2 columns for `net_events`, `mcp_calls`, and `dns_events`
  where docs enumerate tables.
- [x] Update `tool_calls.origin` values and `mcp_call_id` wording.
- [x] Refresh model/tool policy telemetry wording for current enforcement and
  redaction.

### T4.4 Site and Benchmark Stale References

- [x] Replace stale `dnsmasq` public-site references with `capsem-dns-proxy`.
- [x] Update benchmark docs from 12MB to 16MB fork image gate.
- [x] Search for `vsock:5003`, old DNS proxy descriptions, and old artifact
  names after edits.

## Proof Matrix

| Category | Required proof |
|---|---|
| Search | stale terms for old artifacts, updater feed, DNS architecture, and old benchmark gate are gone or intentionally explained. |
| Build | docs and marketing site build. |
| Release | changelog/latest release text matches final T0/T8 implementation decisions. |
| Telemetry | session telemetry docs include hook and Policy V2 audit fields. |

## Verification

- [x] `rg -n "dnsmasq|vsock:?5003|DMG|\\.dmg|AppImage|image < 12MB|12MB" README.md docs/src/content/docs site/src`
  (no matches).
- [x] `rg -n "latest\\.json" README.md docs/src/content/docs site/src`
  returns only wording that explains updater support is disabled/deferred, or
  no matches if T0 ships no updater feed (no matches).
- [x] `rg -n "external Policy Hook|hook endpoint|hook attempts|remote hook|configured hook|Policy Hook Spec0 callouts|policy_action IN \\('ask', 'deny'|policy_action = 'deny'" LATEST_RELEASE.md docs/src/content/docs/releases/1-0.md docs/src/content/docs/security/policy.md docs/src/content/docs/architecture/session-telemetry.md`
  (no matches).
- [x] `pnpm -C docs run build` (45 pages built).
- [x] `pnpm -C site run build` (1 page built; required `pnpm -C site install`
  first because `site/node_modules` was missing and the local pnpm store was
  missing `svelte`).

## Exit Criteria

- [x] Docs do not overclaim hook dispatch.
- [x] Docs describe exactly the artifacts CI publishes for the current package
  contract.
- [x] Session telemetry docs include hook and Policy V2 audit fields.
- [x] Public site no longer references old DNS implementation.
- [ ] Release notes/changelog match final implementation state; T9 owns the
  final curated `CHANGELOG.md`/`LATEST_RELEASE.md` pass after T0-T8 decisions.
