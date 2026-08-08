# 1.3 Route Surface Wiring Sprint

## Purpose

Verify the new profile-first service/gateway route contract is actually wired into user-facing surfaces: frontend UI, TUI, CLI, and MCP host tools. The risk is that routes were restored/ported in the backend after UI/TUI restore, leaving stale settings/global-route callers alive.

## Scope

- Audit backend service/gateway routes against frontend API helpers and TUI/CLI/MCP callers.
- Remove stale UI callers that mutate profile-owned behavior through settings.
- Split frontend surfaces by contract: Settings shows UI/app preferences only;
  Profile shows profile-owned rules, plugins, MCP, assets, and availability;
  runtime/session views show VM/session state. No AI-provider UI object exists
  in 1.3.
- Audit and patch diagnostic surfaces: UI debug link/support bundle,
  `capsem debug`, and `capsem status` must include version, service/gateway
  health, current profile inventory/status, profile asset readiness, active
  corp config presence/status, and enough route/runtime state for a pasted bug
  report to be actionable.
- Add or adjust frontend/TUI tests so route drift is caught.
- Keep backend contracts intact: no fallback routes, no compatibility aliases, no settings-owned MCP/security/profile behavior.

## Done Means

- Frontend settings page does not render profile-owned MCP/plugin/rule/asset
  controls.
- Frontend profile/session surfaces call profile-owned APIs or explicitly
  display read-only/backend-unavailable state.
- Frontend does not invent AI-provider configuration; credential state is shown
  only through credential broker/plugin runtime routes when available.
- TUI uses current `/profiles` and `/vms` routes only.
- CLI/MCP host tools use current `/profiles` and `/vms` routes only.
- Tests prove stale `/settings` MCP mutation and retired global routes are not used by callers.
- Debug/status output includes profile/corp/version/readiness information and
  is covered by focused tests.
- Sprint tracker records any backend-only routes that intentionally have no UI/TUI caller.

## Verification Matrix

- Unit/contract: frontend API/store tests for profile-scoped route helpers.
- Functional: `pnpm -C frontend test ...`, `pnpm -C frontend check`, `cargo test -p capsem-tui`, focused CLI/MCP route tests.
- Adversarial: grep/test guard against `mcp.servers.*` settings mutations and retired route strings in frontend/TUI/CLI/MCP callers.
- E2E/VM: not required unless a caller change touches runtime VM execution; current full smoke from S6 remains the VM proof.
- Telemetry/performance: not applicable to route wiring.
- Diagnostics: focused CLI/service tests for `status`/debug/support bundle
  profile/corp/version fields.
