# Sprint: 1.3 Route Surface Wiring

## Tasks

- [x] Capture backend route inventory from service/gateway.
- [x] Capture frontend route helper/caller inventory.
- [x] Capture TUI/CLI/MCP route caller inventory.
- [x] Remove stale settings-owned MCP UI mutation path.
- [x] Add tests proving frontend MCP server/tool surfaces use profile routes.
- [x] Add grep/adversarial guard for retired caller routes and settings-owned MCP mutations.
- [x] Audit UI debug link/support bundle and `capsem debug`.
- [x] Patch `capsem status`/debug payloads to include version, service/gateway
  health, profile inventory/status, asset readiness, and corp config presence.
- [x] Add diagnostic tests proving pasted status/debug output has enough
  profile/corp/version context.
- [x] Run frontend/TUI/CLI/MCP focused verification.
- [ ] Update changelog if user-visible behavior changed.
- [ ] Commit and push.

## Initial Findings

- Frontend `api.ts` had `setMcpServerEnabled`, `addMcpServer`, and
  `removeMcpServer` writing `mcp.servers.*` through `/settings/edit`; those
  helpers were removed and replaced with profile MCP server edit/delete route
  helpers.
- `McpSection.svelte` derived servers from `settingsStore.model?.mcpServers`;
  it now renders the profile-owned MCP runtime store.
- Runtime MCP list/tool/refresh/call helpers already use
  `/profiles/{profile_id}/mcp/servers/...`.
- `capsem-mcp` host tools already call profile-scoped MCP routes.
- TUI currently uses `/profiles/list`, `/status`, and `/vms/...` routes; no
  stale MCP/settings mutation route callers found.
- User-requested diagnostic gate: debug link, `capsem debug`, and
  `capsem status` need enough profile/corp/version/readiness context for bug
  reports.
- `capsem debug` is now a CLI alias for the redacted support bundle command.
- `capsem debug` support bundles now include
  `system/config-diagnostics.json` with profile inventory and corp install
  source/hash metadata.
- `window.__capsemDebug.snapshot()` now returns versions, frontend log path,
  websocket tail, and gateway route snapshots for `/status`,
  `/profiles/status`, and `/corp/info`.
- Settings/About no longer hard-codes runtime/kernel version claims; it shows
  live diagnostics from the debug snapshot.
- Session rows now display the backend-provided `profile_id`.

## Coverage Ledger

- Unit/contract:
  - `pnpm -C frontend test src/lib/__tests__/api.test.ts src/lib/__tests__/mcp-store.test.ts`
  - `cargo test -p capsem parse_ -- --nocapture`
  - `cargo test -p capsem-gateway gateway_security_routes_are_explicitly_forwarded -- --nocapture`
  - `cargo test -p capsem-service handle_profiles_status_reports_builtin_catalog_readiness -- --nocapture`
  - `cargo test -p capsem-service mounted_read_routes_reflect_profile_settings_corp_mcp_and_assets_contracts -- --nocapture`
  - `cargo run -q -p capsem -- debug --sessions 0 --max-session-bytes 0`
- Functional:
  - `pnpm -C frontend check`
  - `target/debug/capsem status`
  - MCP UI/store route helpers now call `/profiles/{profile_id}/mcp/...`
  - `capsem status` prints profile catalog readiness and corp install/source
    state when the daemon is running.
- Adversarial:
  - `rg` guard found no active frontend/TUI/CLI/MCP callers for old
    settings-owned MCP mutations; remaining retired strings are negative
    gateway/service tests.
- E2E/VM: not required unless runtime VM execution is touched.
- Telemetry/observability: not applicable.
- Performance: not applicable.
- Missing/deferred:
  - No VM boot was run in this route-surface slice; VM proof remains covered by
    the broader 1.3 smoke gate.
  - Manual `target/debug/capsem status` was run with the local service stopped,
    so route-backed profile/corp lines were not available in that manual output;
    focused service/API tests cover the route payloads.
  - Profile MCP server edit/delete routes are explicit and profile-scoped but
    still fail closed until profile file persistence lands.
