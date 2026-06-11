# Sprint: 1.3 Debug Loop

## Tasks

- [x] Capture live-debug ground rule: do not kill, purge, reinstall, or restart
  the current VM without explicit user approval.
- [x] Capture bug 1: VM lifecycle/status actions must not offer resume/start
  for non-resumable VMs, and purge must delete defunct VM state.
- [x] Capture bug 2: AGY needs a safe profile-owned alias/wrapper for its
  dangerous-permission flag.
- [x] Capture bug 3: AGY activity currently does not appear in stats/security
  evidence: no model activity, tool calls, or related ledger events are visible.
- [x] Capture bug 4: credential broker may not be working or observable.
  Statistics show nothing, and broker evidence is buried under process instead
  of being exposed as its own first-class plugin/broker view.
- [x] Capture bug 5: process audit is unclear and low-signal. It appears as a
  list of processes with identical timestamps/dates, so the user cannot tell
  whether it is a snapshot, process lifecycle log, poll artifact, or security
  evidence.
- [x] Capture bug 6: MCP stats show around 200 calls, but most look like junk
  or internal noise such as `snapshot`. The MCP view is likely counting
  infrastructure calls as meaningful MCP activity.
- [x] Capture bug 7: snapshot view does not seem useful or possibly does not
  work as intended. It shows thousands of files, which overwhelms the user and
  does not make clear what changed, what matters, or whether snapshot capture
  succeeded.
- [x] Capture bug 8: many files appeared in the working directory. It is unclear
  whether snapshot, AGY, or another process created them. Do not delete them
  before tracing provenance.
- [x] Capture bug 9: AGY reported high-risk DNS tunneling exfiltration. The DNS
  proxy resolves arbitrary domains, so an agent may encode data into DNS queries
  such as `[data].attacker.com` and bypass HTTP/HTTPS allowlists.
- [x] Capture bug 10: AGY reported raw VSOCK access risk. The guest exposes
  `/dev/vsock` to root/default guest execution, allowing direct communication
  with host-side listener services outside the intended audited rails.
- [x] Capture bug 11: AGY reported MCP tool response pagination crash. The MCP
  server prepends a text header to responses over 5000 characters; Python JSON
  parsers in `snapshots` and `capsem-doctor` crash when listing large workspace
  changes such as `.cache` or `.venv`.
- [x] Capture bug 12: UI settings/profile surface does not support multiple
  profiles correctly. It appears to show a static `Profile` field/label instead
  of a select box or route-backed profile picker. A `co-work` profile may be
  added as a real second profile fixture for UI and profile-contract testing.
- [x] Capture bug 13: UI `Policy` surface is useless/misnamed. It does not show
  concrete enforcement rules, detection rules, plugin state, or how to modify
  them. The agreed contract is explicit `enforcement`, `detection`, and
  `plugins`, not a vague general policy page.
- [x] Capture bug 14: plugin UI/state needs clearer mode semantics. Dummy
  plugins should be disabled by default and greyed out. Plugin modes/actions
  such as ask, block, pass/allow, rewrite, and disable need recognizable icons
  so users can understand behavior at a glance.
- [x] Capture bug 15: MCP and rule UI need the same mode/status clarity as
  plugins. MCP servers/tools/resources and enforcement/detection rules should
  show disabled/default/test states clearly, and actions such as ask, block,
  allow/pass, rewrite, detect, and disable should use consistent icons and
  enum-backed controls.
- [x] Capture bug 16: MCP UI shows `local` and marks it as `stopped`, which is
  confusing. If this is Capsem-owned MCP, the label should likely be `builtin`;
  `stopped` should only appear for a real server lifecycle state, not for static
  builtin capability/config.
- [x] Capture bug 17: MCP edit path returns `API error 501:
  profile MCP server edit requires profile file persistence, which is not
  enabled yet`. The UI is exposing a mutation route/affordance that the backend
  does not support.
- [x] Capture bug 18: disabled MCP/rule/plugin rows need proper greyed-out
  styling and still need the correct policy/mode icon. Disabled should make
  inactive state obvious without hiding whether the configured behavior is ask,
  block, allow/pass, rewrite, detect, or disabled.
- [x] Capture bug 19: MCP UI has no way to select/change the default policy
  rule for MCP. Defaults are supposed to be visible real rules, but the MCP
  surface does not expose the default MCP rule/policy selector.
- [x] Capture bug 20: MCP UI has no way to configure per-tool overrides of the
  default MCP policy. Users need to set specific tool/server/resource behavior
  that overrides the default through the same rule contract.
- [x] Capture bug 21: asset status UI is unclear. It should show the profile
  assets as a checklist/list with checkmarks or errors, rather than vague
  aggregate text.
- [x] Capture bug 22: overview should prioritize available surfaces and
  available credentials. It should make clear which UI/terminal/mobile/shell/API
  surfaces are enabled for the selected profile and list broker-visible
  credential references/status.
- [x] Capture bug 23: plugins need first-class info/introspection. The UI cannot
  tell whether AGY OAuth was intercepted, plugin activity is absent from VM
  stats, and supported credential types are not listed. Each plugin should
  expose structured info/status/capabilities/counters that the UI can render.
  Plugins may also expose typed route-backed detail surfaces for custom UI
  panels when generic counters are not enough; credential broker needs such a
  panel for inventory, per-profile/per-VM grants, capture/replay evidence, and
  profile/fork exposure.
- [x] Capture bug 24: AI provider detection is host/registry-biased and misses
  unknown-domain OpenAI/Gemini/Claude-compatible traffic. Bounded
  request/response sniffing should detect protocol shape, emit
  `model.provider` plus `http.host`, and a default/high-signal detection rule
  should flag `model.provider == "<provider>"` when the host is not a known or
  profile/corp-declared endpoint.
- [x] Capture bug 25: brokered credentials are not yet a complete next-VM reuse
  loop. The broker should accumulate credentials over time as an opaque
  credential vault, then expose only allowed credential capabilities/refs to
  each profile or forked VM. Corp config can constrain the broker plugin, such
  as disallowing selected OAuth providers or flows. AGY/Gemini/Claude/Codex
  auth replay/refresh is one consumer of that vault, not a guest config-writing
  shortcut.
- [x] Implement bug 1 slice: TDD over CLI purge messaging, service purge of
  defunct persistent VMs, and TUI resume gating from `can_resume`.
- [x] Implement bug 2 slice: TDD over the checked-in code profile installer so
  `agy` is profile-owned, preserves the real binary as `agy-real`, and launches
  with `--dangerously-skip-permissions` without hand edits inside the VM.
- [ ] Implement bug 3 after user resumes coding: TDD over AGY traffic/tool-call
  observability so stats reflect model/tool activity through the unified
  security-event/session DB path.
  - [x] AGY model telemetry slice: live DB proved AGY sends model traffic to
    `daily-cloudcode-pa.googleapis.com` on `/v1internal:streamGenerateContent`
    and `/v1internal:generateContent`. Added that host as a Google protocol
    alias and covered the telemetry path so AGY generation emits `ModelCall`
    rows once the new service build runs.
  - [x] AGY Google tool-call telemetry slice: non-streaming Google
    `functionCall` response parts now produce first-party `tool_calls` with
    deterministic synthetic `gemini_<name>_<index>` IDs matching the Google
    `functionResponse` request-parser shape.
  - [x] Stats trace visibility slice: frontend trace SQL no longer hides
    model traces whose token totals are zero or unavailable, so AGY/tool-only
    activity remains inspectable once model rows exist.
  - [x] Ledger proof slice: AGY Google `functionCall` responses now have a
    regression that builds the telemetry `ModelCall`, writes it through the
    real session DB writer, and proves `session_stats`, `tool_usage_frequency`,
    and `tool_calls_for` expose the tool row the UI consumes. Proof:
    `cargo test -p capsem-core agy_google_tool_call_survives_into_session_stats
    -- --nocapture`.
  - [ ] Remaining: verify against a rebuilt service/VM without destroying the
    current evidence VM until approved.
- [ ] Implement bug 4 after user resumes coding: prove broker capture/rewrite
  with a local hermetic flow, expose broker/plugin counters and recent evidence
  as first-class stats, and ensure UI/TUI do not bury it under generic process
  activity.
  - [x] Credential broker OAuth/runtime slice: live DB proved AGY OAuth traffic
    hit `oauth2.googleapis.com/token` but body previews were empty and
    `substitution_events=0`. Added Google OAuth JSON/form credential detection,
    broker-owned credential-candidate preview caps for MITM request/response
    bodies, and profile plugin runtime status derived from session DB
    `substitution_events` via `capsem-logger::DbReader`.
  - [x] VM Stats broker visibility slice: credential-broker evidence now has a
    first-class `Credentials` tab backed by `substitution_events`; the Process
    tab only shows command executions and audit-port process observations.
    Proof: `pnpm --dir frontend test -- --run
    frontend/src/lib/__tests__/stats-view-contract.test.ts`; `pnpm --dir
    frontend check`.
  - [ ] Remaining: verify against a rebuilt service/VM without destroying the
    current evidence VM, expose richer credential-broker capability/status in
    the TUI/status surfaces, and add a hermetic OAuth/broker flow once the local
    HTTP test server is in the next-gen testing harness.
- [ ] Implement bug 5 after user resumes coding: define what process audit is
  supposed to represent, fix timestamp semantics if it is a snapshot, and rename
  or reshape the UI so it reflects the actual data contract rather than a vague
  audit label.
  - [x] Process Stats wording slice: the Stats process tab now labels
    `audit_events` as audit-port `Process Observations`, keeps command
    executions separate as `Process Exec Events`, and uses `process
    observation` as the detail type.
    Proof: `pnpm --dir frontend test -- --run
    frontend/src/lib/__tests__/stats-view-contract.test.ts`; `pnpm --dir
    frontend check`.
  - [ ] Remaining: inspect live timestamps/provenance for repeated same-time
    rows and decide whether producer semantics need changes beyond UI wording.
- [x] Implement bug 6 slice: classify headline MCP stats so user-facing totals
  count only user tool calls (`tools/call`) and exclude protocol handshakes,
  `tools/list`, and builtin snapshot maintenance while raw rows remain in
  session DB for forensics.
- [x] Implement bug 7 slice: keep hypervisor snapshot internals out of generic
  Stats surfaces while preserving explicit MCP access for AI/tool callers.
  - [x] Snapshot visibility boundary: Stats no longer exposes a standalone
    Snapshot tab or reads `snapshot_events`; explicit snapshot MCP invocations
    still show up as MCP calls, but host snapshot state is no longer a
    session.db/security-event table.
  - [x] Snapshot ledger burn: `snapshot_events`, `snapshot.event`,
    `SnapshotEvent`, and the `Snapshot` runtime security-event family were
    removed from the logger/security-engine contract. New and migrated
    databases reject/destroy the old table so hypervisor recovery state cannot
    masquerade as user/security activity.
  - [x] Route-backed snapshot state: `/vms/{vm_id}/snapshots/status` and
    `/vms/{vm_id}/snapshots/list` read the running VM's in-memory
    `capsem-process` scheduler over IPC. Stopped VM inspection reconstructs
    from that VM's snapshot metadata only on demand; no session DB fallback.
  - [x] Compact snapshot MCP table: `snapshots_list` defaults to
    created/edited/deleted summary counts and only returns full per-file
    changes when the MCP caller passes `include_changes=true`.
  - Proof: `cargo test -p capsem-core
    mcp::file_tools::tests::list_ -- --nocapture`; `cargo test -p
    capsem-logger --lib -- --nocapture`; `cargo test -p capsem-proto
    snapshot_status -- --nocapture`; `cargo test -p capsem-process
    classify_snapshot_status_is_job_query -- --nocapture`; `cargo test -p
    capsem-service snapshot_status_from_session_dir_reads_snapshot_metadata_without_db
    -- --nocapture`; `cargo test -p capsem-core runtime_security_event_ --
    --nocapture`; `cargo test -p capsem-mcp inspect_schema_has_all_tables --
    --nocapture`; `pnpm --dir frontend test -- --run
    frontend/src/lib/__tests__/api.test.ts
    frontend/src/lib/__tests__/stats-view-contract.test.ts`; `pnpm --dir
    frontend check`; `cargo check -p capsem-logger -p capsem-proto -p
    capsem-process -p capsem-service -p capsem-core -p capsem-mcp`;
    `cargo build -p capsem-service -p capsem-process -p capsem-gateway -p
    capsem-tray -p capsem-mcp-builtin`; `uv run python -m pytest
    tests/capsem-session-lifecycle/test_db_schema.py
    tests/capsem-session-lifecycle/test_db_exists.py
    tests/capsem-session-lifecycle/test_multiple_events.py
    tests/capsem-session/test_cross_table.py -q`.
- [ ] Implement bug 8 after user resumes coding: non-destructively trace file
  provenance from paths, mtimes, process/security logs, and session DB evidence;
  prove whether snapshot is read-only or mutating the workspace; then add a
  regression test that snapshot cannot create workspace files unless explicitly
  requested.
  - [x] Snapshot read-only rail slice: `AutoSnapshotScheduler` now refuses to
    run if snapshot storage or a snapshot slot resolves inside the live
    workspace, including symlinked `auto_snapshots` paths. Capture and compact
    tests prove live workspace entries/hash do not change.
    Proof: `cargo test -p capsem-core auto_snapshot:: -- --nocapture`.
  - [x] Host-only snapshot state slice: automatic snapshots now emit structured
    process logs only and keep scheduler state in memory for live VMs; they no
    longer write `SnapshotEvent` rows to the session ledger. This keeps
    capsem-doctor/agent snapshot activity from bleeding into generic user-facing
    activity unless an MCP/tool caller explicitly invokes a snapshot tool.
  - [x] Restore symlink escape rail slice: `snapshots_revert` now rejects
    snapshot parent symlinks that would make restore read outside checkpoint
    storage, skips no-op comparisons for live symlinks, and reads regular
    restore sources with no-follow semantics. Tests prove the old
    symlink-outside pull-in shape is rejected, live final symlinks are replaced
    without touching targets, and snapshot symlinks are restored as symlinks
    rather than copied target bytes.
    Proof: `cargo test -p capsem-core
    mcp::file_tools::tests::revert_file_ -- --nocapture`; `cargo test -p
    capsem-core mcp::file_tools::tests:: -- --nocapture`.
  - [ ] Remaining: inspect live VM/session DB evidence for the files the user
    observed and attribute them to AGY/process/file events without deleting the
    current VM evidence.
- [ ] Implement bug 9 after user resumes coding: design and test DNS policy as
  first-class enforcement, including deny/ask/default DNS rules, DNS query
  length/entropy/rate guards, and ledger evidence for suspicious query payloads.
- [ ] Implement bug 10 after user resumes coding: inventory host VSOCK listener
  exposure, define the allowed guest/host VSOCK contract, and test that raw
  guest access cannot bypass audited service entry points.
- [x] Implement bug 11 slice: make snapshot MCP JSON responses protocol-valid
  for large payloads by bypassing prose pagination for `format=json`, with a
  large-response parser regression test. Root cause: `handle_list_snapshots`
  prepended human pagination text before a JSON object, so consumers calling
  `json.loads()` saw `Content length:` instead of `{`.
- [ ] Implement bug 12 after user resumes coding: make profile selection
  route-backed and multi-profile aware in the UI, using select controls for the
  profile enum/list; add a real `co-work` profile fixture if needed to prevent
  single-profile assumptions from creeping back in.
  - [x] Admin rail guard: add/use `/dev-capsem-admin`, and create the second
    profile only through `capsem-admin`; no hand-copied profile directory as
    proof.
    Proof: `cargo test -p capsem-admin profile_init -- --nocapture`; actual
    profile created with `cargo run -p capsem-admin -- profile init --output
    config/profiles/co-work/profile.toml --id co-work --name 'Co-work'
    --description 'Shared profile for collaborative agent sessions.' --from
    config/profiles/code/profile.toml`.
  - [x] Service/status proof: checked-in config catalog exposes both `code` and
    `co-work`, with validated payload pins and status/readiness data.
    Proof: `cargo run -p capsem-admin -- profile validate
    config/profiles/code/profile.toml --config-root config --json`; same for
    `co-work`; `cargo test -p capsem-service profile -- --nocapture`.
  - [x] UI proof: profile/settings surfaces pass selected profile ids into
    plugins, MCP, enforcement, detection, assets, and credential broker detail
    routes; no `code` fallback in those surfaces.
    Proof: `pnpm --dir frontend test -- --run
    frontend/src/lib/__tests__/mcp-store.test.ts
    frontend/src/lib/__tests__/api.test.ts
    frontend/src/lib/__tests__/settings-store.test.ts`; `pnpm --dir frontend
    check`; hardcode scan for profile-less calls returned empty.
  - [x] TUI/CLI proof: status and shell/profile selection paths list both
    profile-backed options and do not synthesize defaults.
    Proof: `cargo test -p capsem-tui gateway_provider_ -- --nocapture`.
- [x] Implement bug 13 slice: burn the Profile UI's generic `Policy` tab and
  split it into first-class `Enforcement` and `Detection` tabs backed by the
  existing profile rule routes. Plugins remain a separate plugin route surface.
  Proof: `pnpm --dir frontend test -- --run
  frontend/src/lib/__tests__/profile-page-contract.test.ts`; `pnpm --dir
  frontend check`; frontend source scan only finds old policy names in negative
  tests.
- [x] Implement bug 14 slice: default dummy plugins to
  disabled, render disabled plugins as inactive/greyed out, and add consistent
  iconography for ask/block/pass-or-allow/rewrite/disable modes using the
  plugin contract values rather than UI-invented labels.
  Proof: `cargo test -p capsem-service plugin -- --nocapture`; `pnpm --dir
  frontend test -- --run frontend/src/lib/__tests__/plugin-section-contract.test.ts
  frontend/src/lib/__tests__/api.test.ts`; `pnpm --dir frontend check`.
- [x] Implement bug 15 slice: apply the same contract-backed
  visual language to MCP and rules: grey out disabled MCP servers/tools/resources
  and disabled rules, group default rules visibly without making them a separate
  engine, and use consistent icons/select boxes/toggles for enum/boolean
  controls.
  - [x] Route-backed UI iconography slice: Profile enforcement/detection rows
    now render typed action/detection metadata instead of raw grey pills, and
    MCP tools show allow/ask/block permission badges while preserving the
    selector as the only mutation control. Disabled MCP servers are greyed from
    `server.enabled`.
    Proof: `pnpm --dir frontend test -- --run
    frontend/src/lib/__tests__/profile-page-contract.test.ts
    frontend/src/lib/__tests__/mcp-section-contract.test.ts
    frontend/src/lib/__tests__/plugin-section-contract.test.ts
    frontend/src/lib/__tests__/mcp-store.test.ts`; `pnpm --dir frontend check`.
  - [x] Disabled-rule contract slice: `SecurityRule.enabled` defaults to true,
    compiled rule inventory preserves disabled rules, `SecurityRuleSet`
    evaluation skips them, profile enforcement/detection list DTOs expose
    `enabled`, and the Profile UI greys disabled rule rows from that field.
    Proof: `cargo test -p capsem-core
    disabled_rules_remain_inventory_but_do_not_match -- --nocapture`;
    `cargo test -p capsem-service rules -- --nocapture`; frontend proof below.
  - [x] Default-rule grouping slice: Profile enforcement/detection rule lists
    are grouped from `rule.default_rule` into default rules and profile/corp
    rules without adding another endpoint or rule engine.
    Proof: `pnpm --dir frontend test -- --run
    frontend/src/lib/__tests__/profile-page-contract.test.ts`; `pnpm --dir
    frontend check`.
- [x] Implement bug 16 slice: make MCP source/lifecycle display respect the
  existing route contract. The profile route exposes `local` as
  `source = builtin` with `running = false` because it is static Capsem-owned
  capability, not an external stopped server. The MCP UI now renders builtin
  entries as `Built-in`/`Disabled`, and frontend runtime counts exclude builtin
  entries.
  Proof: `cargo test -p capsem-service
  mounted_mcp_routes_are_profile_scoped_mechanics_only -- --nocapture`;
  `pnpm --dir frontend test -- --run
  frontend/src/lib/__tests__/mcp-store.test.ts`; `pnpm --dir frontend check`.
- [x] Implement bug 17 slice: remove unsupported MCP server add/toggle/delete
  affordances and frontend helpers that hit the deliberate 501 server edit
  routes. The MCP UI now only exposes route-backed operations that exist:
  server/tool list, refresh, and per-tool permission mutation.
  Proof: `pnpm --dir frontend test -- --run
  frontend/src/lib/__tests__/api.test.ts
  frontend/src/lib/__tests__/mcp-store.test.ts`; `pnpm --dir frontend check`;
  frontend hardcode scan only finds the burned server helpers in negative
  tests.
- [x] Implement bug 18 slice: create shared row/icon
  semantics for disabled entries across plugins, MCP, enforcement rules, and
  detection rules: grey/inactive styling for disabled state, plus policy/mode
  icon from the underlying enum.
  - [x] Plugin and MCP parts covered by bug 14 and bug 15 UI iconography
    slices.
  - [x] Enforcement/detection disabled-rule rendering is backed by the
    first-party `enabled` rule DTO field.
- [x] Implement bug 19 slice: expose the default MCP rule
  as a visible, editable rule/policy selector where allowed by profile/corp
  constraints; test that changing the selector mutates the same rule contract
  used by enforcement, not a separate MCP policy field.
  Proof: `cargo test -p capsem-core profile_mcp_default_permission --
  --nocapture`; `cargo test -p capsem-core
  profile_mcp_tool_permission_override_wins_after_default_mutation --
  --nocapture`; `cargo test -p capsem-service
  profile_mcp_default_edit_writes_default_rule_and_mutation_ledger --
  --nocapture`; `pnpm --dir frontend test -- --run
  frontend/src/lib/__tests__/api.test.ts
  frontend/src/lib/__tests__/mcp-store.test.ts
  frontend/src/lib/__tests__/mcp-section-contract.test.ts`; `pnpm --dir
  frontend check`.
- [x] Implement bug 20 slice: per-tool MCP overrides are now backed by
  profile-managed enforcement rules. `Profile::mcp_tool_permission` reads the
  default MCP rule or the managed override from pinned enforcement TOML,
  `/profiles/{profile_id}/mcp/servers/{server_id}/tools/list` returns
  `permission_action` and `permission_source`, and the UI renders a select box
  for `allow`/`ask`/`block`.
  Proof: `cargo test -p capsem-core
  profile_mcp_tool_permission_mutation_updates_rule_and_pin -- --nocapture`;
  `cargo test -p capsem-service
  profile_mcp_tool_edit_writes_profile_rule_and_mutation_ledger --
  --nocapture`; frontend test/check commands above.
- [x] Implement bug 21 slice: render per-profile asset readiness as a
  checklist instead of raw JSON. Profile UI now uses
  `/profiles/{profile_id}/assets/status`, displays manifest source/hash, VM
  assets, profile files, verified/missing/invalid/downloading state, paths, and
  size details.
  Proof: `pnpm --dir frontend test -- --run
  frontend/src/lib/__tests__/profile-page-contract.test.ts`; `pnpm --dir
  frontend check`.
- [ ] Implement bug 22 after user resumes coding: reshape overview to show
  profile capability/readiness: available surfaces, enabled plugins, credential
  broker status and credential reference list, plus blockers that prevent using
  a surface.
  - [x] Profile overview surfaces/credentials slice: Profile UI now renders
    web/shell/mobile availability from `profile.profile.availability` and
    broker-visible credential inventory/grant state from the credential broker
    detail route.
    Proof: `pnpm --dir frontend test -- --run
    frontend/src/lib/__tests__/profile-page-contract.test.ts`; `pnpm --dir
    frontend check`.
  - [ ] Remaining: add explicit surface blockers/readiness reasons and enabled
    plugin summary into the overview without duplicating the plugin or asset
    tabs.
- [ ] Implement bug 23 after user resumes coding: define and wire a plugin info
  contract for each plugin: name, description, version, mode, pre/post phase,
  supported event families, supported credential kinds/providers where relevant,
  status, counters, last activity, recent evidence links, and optional typed
  detail routes for plugin-specific UI. Render the generic contract in plugin
  UI/VM stats, and add a credential-broker-specific route/panel for inventory,
  grant editing/visibility per profile and VM, corp-denied provider/flow
  constraints, capture/replay evidence, and profile/fork exposure.
  - [x] Plugin detail-route contract slice: `PluginInfo` now advertises typed
    custom detail routes, and credential broker exposes
    `/profiles/{profile_id}/plugins/credential_broker/credentials/info` for
    broker inventory plus the initial grant/corp-constraint surface.
  - [x] Plugin capability UI slice: `PluginInfo` now carries plugin-owned
    capability metadata, and the credential broker reports watched event
    families, supported providers (`anthropic`, `google`, `openai`, `github`,
    `mcp`), and concrete credential source shapes (`http.authorization`,
    `http.body.oauth_token`, `file.env`, `mcp.auth_reference`). The Plugin UI
    renders those fields next to broker inventory/counters.
    Proof: `cargo test -p capsem-service plugin -- --nocapture`; `pnpm --dir
    frontend test -- --run frontend/src/lib/__tests__/api.test.ts
    frontend/src/lib/__tests__/plugin-section-contract.test.ts`; `pnpm --dir
    frontend check`.
  - [ ] Remaining: add route-backed grant mutation/corp constraints, connect
    those grants to broker replay/substitution decisions, and surface broker
    activity in VM stats with recent evidence links.
- [ ] Implement bug 24 after user resumes coding: add TDD for unknown-domain AI
  protocol sniffing and rogue/custom endpoint detection. The fix must use
  bounded request/response previews, set first-party `model.provider` on the
  same security event as `http.host`, preserve declared custom endpoint support,
  and add adversarial tests proving unknown-domain OpenAI/Gemini/Claude shapes
  are detected without allowing unbounded body capture or host-only bypasses.
  - [x] Canonical path promotion slice: unknown hosts using first-party model
    paths such as `/v1/chat/completions`, `/v1/messages`, or Google
    `generateContent` paths now promote to the matching model protocol so the
    same event carries `model.provider` and `http.host`.
  - [ ] Remaining: bounded request/response body-shape sniffing for
    non-canonical/private gateway paths, plus default detection rules that flag
    undeclared model endpoints without treating declared custom endpoints as
    rogue.
- [ ] Implement bug 25 after user resumes coding: complete broker reuse across
  VM lifecycles. Add broker inventory and grant semantics so accumulated
  credential refs can be exposed per profile and inherited/limited by forked
  VMs, with explicit controls to turn credential use on/off for a profile or
  VM. Add corp plugin constraints that can disallow selected OAuth providers or
  flows. Add broker/provider adapters that recognize repeated
  auth/token-refresh dances from observed request shape, satisfy or replay the
  exchange host-side only when the active profile/fork has the credential
  capability and corp constraints permit it, and add an AGY/Google OAuth e2e
  showing a second VM can reuse a valid brokered exchange without exposing raw
  secrets or requiring a user-facing OAuth dance.
- [ ] Broker/provider hardening lane dependency: bugs 4, 23, 24, and 25 must be
  validated together. Provider on/off is only trustworthy when provider
  detection, profile enforcement, broker capture/replay, and plugin/broker
  runtime evidence all agree on the same security-event ledger.

## Notes

- Current AGY VM is important evidence. Do not destroy it while diagnosing why
  stats are empty.
- Copied live VM report from `code-mq8nrnzr` to
  `sprints/1.3-debug-loop/evidence/capsem_security_assessment-code-mq8nrnzr.md`.
- Live VM evidence before fixes: session DB had `model_calls=0`, `tool_calls=0`,
  `mcp_calls=855`, `snapshot_events=8`, `substitution_events=0`,
  `dns_events=76`, and `net_events=452`. MCP rows were mostly
  `initialize`, `notifications/initialized`, and snapshot maintenance calls,
  with only a handful of real tool invocations.
- Root-cause hypotheses to verify later, not conclusions:
  - AGY may be reaching model/tool endpoints without passing through the
    current monitored proxy/MITM path.
  - AGY may use a provider/request shape our model parser does not classify
    yet.
  - Unknown-domain AI-compatible traffic currently needs a declared
    profile/corp model endpoint before the MITM treats it as model traffic.
    That means a private or rogue OpenAI/Gemini/Claude-compatible endpoint can
    remain ordinary HTTP unless future sniffing promotes the event to
    first-party model telemetry and detection.
  - AGY tool activity may be local-process or MCP-shaped activity that is not
    being converted into first-party model/tool-call events.
  - Stats UI may be reading stale counters/routes even if session DB events
    exist.
  - Credential broker may be executing but not incrementing plugin counters, or
    not executing at all because plugin enablement/config was not attached to
    the running profile.
  - Credential broker events may be emitted as generic process/file evidence
    instead of first-class broker/plugin security evidence.
  - Verified root cause for AGY OAuth broker silence: non-AI OAuth request and
    response body preview caps were zero when `log_bodies=false`, so the broker
    never saw the `oauth2.googleapis.com/token` body. Runtime plugin status was
    also a placeholder that always returned zero counters even if broker rows
    existed in session DB.
  - Broker capture and replay are currently separate primitives: Keychain/test
    storage plus `credential:blake3:<hash>` refs exist, and some HTTP/MCP
    paths can rehydrate refs when explicitly configured. What is missing is the
    broker ledger between them: accumulated credential inventory, per-profile,
    per-VM, and per-fork exposure/grants, corp plugin constraints, and
    structured evidence for why a credential was or was not available to a VM.
  - For AGY/Google OAuth specifically, the missing adapter is not a guest
    config file; it is a broker-owned request-shape adapter that recognizes the
    captured dance and satisfies the token/refresh exchange at the host boundary
    with structured logging after profile/fork/VM grants, corp broker
    constraints, and provider enforcement allow it.
  - Spike shape for bug 25: launch AGY, capture the exact OAuth/token requests
    and responses, add the minimal host-side replay/refresh adapter, then retry
    AGY in a fresh VM and prove the guest no longer requires a user-facing auth
    dance while raw secrets never enter guest config.
  - Broker exposure/replay is also the enforcement point for profile provider
    toggles: if a profile blocks or asks for a provider, the broker must not
    silently expose or replay credentials for that provider outside the same
    rule decision path.
  - Process audit may be rendering snapshot collection time for every row
    rather than per-process start time or per-event emission time.
  - Process audit may be mixing inventory/snapshot data with security-event
    language, causing the UI to imply an event log where it only has a point-in-
    time process list.
  - MCP counters may be aggregating all MCP-framed traffic without distinguishing
    first-party user/tool calls from Capsem/internal maintenance operations.
  - Snapshot-related MCP traffic may need its own category or exclusion from the
    user-facing MCP call count, while still remaining visible in forensic
    details.
  - Snapshot may be dumping full filesystem inventory without computing or
    surfacing deltas, so the user sees volume instead of signal.
  - Snapshot rows may need to distinguish baseline, current inventory, changed
    files, deleted files, and high-risk paths.
  - Snapshot capture may have an extraction/materialization path that writes into
    the workspace instead of only reading/recording metadata.
  - AGY may have created files as part of its run, but the current stats/snapshot
    UI does not attribute those writes clearly enough to tell.
  - DNS exfiltration is not merely a DNS feature gap if DNS remains unaudited or
    less enforceable than HTTP; it is a policy spine gap.
  - Raw VSOCK access may be acceptable only for tightly scoped device/service
    paths with explicit host-side authentication and structured logging; any
    generic raw path is suspect.
  - MCP pagination must be protocol-valid. A human-readable header before JSON
    is a format violation and explains the snapshot/doctor crash class.
  - The parse failure itself must be diagnosed from evidence: exact bytes in,
    exact parser invoked, exact error, and exact code path that produced the
    malformed response.
  - UI settings may still be treating profile as a singleton or display label
    rather than a profile-backed selection contract.
  - UI may still be carrying old `policy` vocabulary after the architecture
    split into enforcement rules, detection rules/Sigma, and plugins.
  - Dummy plugins may currently look active or product-real even though they
    should be disabled test fixtures.
  - MCP/rule views may have the same problem as plugins: the UI may be showing
    raw rows without communicating whether something is active, default,
    disabled, blocking, asking, allowing, rewriting, or only detecting.
  - MCP `local` may be a legacy label for builtin tools, or the UI may be
    collapsing builtin and external MCP server lifecycle into one status field.
  - MCP edit UI may have been wired ahead of profile persistence, violating the
    route-backed UI contract.
  - Disabled rows may currently lose their configured policy meaning or look
    indistinguishable from active rows.
  - MCP UI may be treating per-server/tool state separately from the default MCP
    enforcement rule, leaving users unable to control non-matching MCP calls.
  - MCP per-tool override may require a structured rule annotation/key so the UI
    can find or create the one rule for a server/tool without inventing a second
    storage path.
  - Asset readiness may currently be compressed into one text line, which hides
    which exact asset blocks VM creation.
  - Overview may be spending space on generic labels instead of the profile
    contract users need before launching or debugging: surfaces, credentials,
    plugins, assets, and blockers.
  - Credential broker may not expose enough metadata for the UI to answer which
    credential classes/providers are supported, whether AGY OAuth was captured,
    or whether any rewrite/capture activity happened.
  - Plugin activity may exist in logs/session DB but not be rolled up into VM
    stats, or plugins may not emit stats at all.
- External report note: user said AGY wrote `capsem_security_assessment.md`, but
  it was not present in this source worktree when checked with `rg --files`.
  Treat the live VM/workspace copy as evidence to collect later without
  destructive cleanup.

## Coverage Ledger

- Unit/contract:
  - `cargo test -p capsem-core mcp::file_tools::tests:: -- --nocapture`
    passed; includes large snapshot JSON parser regression.
  - `cargo test -p capsem-core mcp::file_tools::tests::list_ -- --nocapture`
    passed; proves `snapshots_list` defaults to compact
    created/edited/deleted counts and requires `include_changes=true` for full
    per-file diffs.
  - `pnpm --dir frontend test -- --run frontend/src/lib/__tests__/stats-view-contract.test.ts`
    passed; package script ran the frontend suite and proves Stats does not
    expose a generic Snapshot tab/query.
  - `pnpm --dir frontend check` passed; Astro and Svelte checks have 0 errors
    and 0 warnings after removing the Snapshot tab.
  - `cargo test -p capsem-core auto_snapshot:: -- --nocapture` passed; proves
    snapshot capture/compaction do not mutate live workspace entries and
    rejects snapshot storage symlinked into the workspace.
  - `cargo test -p capsem-core mcp::file_tools::tests::revert_file_ -- --nocapture`
    passed; proves restore rejects snapshot parent symlink escapes and does
    not pull outside target bytes through symlink paths.
  - `cargo test -p capsem-core mcp::file_tools::tests:: -- --nocapture`
    passed; full snapshot MCP file-tools suite remains green after restore
    symlink hardening.
  - `cargo test -p capsem-logger mcp_call_stats_counts_user_tool_calls_not_protocol_or_snapshot_noise -- --nocapture`
    passed; proves backend MCP headline stats filter protocol/snapshot noise.
  - `pnpm --dir frontend test -- --run frontend/src/lib/__tests__/mcp-sql.test.ts`
    passed; package script ran the frontend suite and proves UI SQL uses the
    same MCP user-call predicate for headline/tool-list queries.
  - `cargo test -p capsem-service purge_default_removes_defunct_persistent_and_keeps_healthy_stopped -- --nocapture`
    passed; proves default purge removes defunct persistent VMs and keeps
    healthy stopped persistent VMs.
  - `cargo test -p capsem-tui gateway_status_can_resume_false_blocks_tui_resume_even_when_profile_ready -- --nocapture`
    passed; proves TUI does not offer resume when service says `can_resume=false`.
  - `cargo test -p capsem purge_summary_ -- --nocapture` passed; proves CLI
    purge output names broken persistent removals.
  - `cargo test -p capsem-admin -- --nocapture` passed; includes the AGY
    profile-wrapper contract and profile/image validation tests.
  - `cargo run -p capsem-admin -- profile check config/profiles/code/profile.toml --config-root config`
    passed after refreshing the `install.sh` profile hash pin.
  - `cargo test -p capsem-core provider_defaults_build_settings_defined_endpoint_registry -- --nocapture`
    passed; proves AGY Cloud Code host maps to Google protocol.
  - `cargo test -p capsem-core agy_cloudcode_stream_generate_content_is_a_model_call -- --nocapture`
    passed; proves AGY Cloud Code generation paths emit model telemetry when
    provider metadata is present.
  - `cargo test -p capsem-core --lib non_streaming_google_tool_calls -- --nocapture`
    passed; proves non-streaming Google response `functionCall` parts parse
    into deterministic first-party model tool calls.
  - `cargo test -p capsem-core --lib net::ai_traffic::events::tests:: -- --nocapture`
    passed; proves the event parser suite including non-streaming usage,
    gzip, and Google tool-call parsing.
  - `cargo test -p capsem-core --lib google_non_streaming_function_call_is_logged_as_model_tool_call -- --nocapture`
    passed; proves the MITM telemetry hook logs AGY/Google non-streaming
    function calls as model tool-call rows.
  - `cargo test -p capsem-core --lib net::ai_traffic::request_parser::tests::google -- --nocapture`
    passed; proves Google function responses still parse under the same
    synthetic ID family.
  - `cargo test -p capsem-core --lib net::interpreters::google_interpreter::tests:: -- --nocapture`
    passed after one transient local code-sign wrapper retry; proves streaming
    Google tool calls use the same deterministic synthetic ID shape.
  - `pnpm --dir frontend test -- --run frontend/src/lib/__tests__/mcp-sql.test.ts`
    passed after a red failure on the old token-only trace filter; proves model
    trace SQL does not hide zero-token/tool-only traces.
  - `pnpm --dir frontend test -- --run frontend/src/lib/__tests__/api.test.ts frontend/src/lib/__tests__/mcp-store.test.ts`
    passed as a focused frontend regression around API/MCP consumers after the
    trace visibility change.
  - `cargo test -p capsem-core --lib http_body_detector_finds_google_oauth -- --nocapture`
    passed; proves Google OAuth JSON and form token exchanges are recognized
    and redacted by the credential broker.
  - `cargo test -p capsem-core --lib http_body_credential_candidate_is_limited_to_known_exchange_paths -- --nocapture`
    passed; proves broker-owned body preview enablement stays scoped to known
    credential exchange paths.
  - `cargo test -p capsem-core --lib net::mitm_proxy::tests:: -- --nocapture`
    passed; proves OAuth broker candidates get bounded body previews while
    unrelated non-AI HTTP stays at zero preview when body logging is off.
  - `cargo test -p capsem-core --lib net::mitm_proxy::telemetry_hook::tests:: -- --nocapture`
    passed; proves telemetry still emits/redacts broker substitution events and
    AGY Cloud Code model telemetry.
  - `cargo test -p capsem-service credential_broker_plugin_runtime_reports_session_db_substitutions -- --nocapture`
    passed; proves `/profiles/{profile_id}/plugins/list` reports credential
    broker counters and refs from session DB substitution ledger rows.
  - `cargo test -p capsem-service profile_plugin_endpoint_matrix_dynamically_controls_enforcement_evaluation -- --nocapture`
    passed after one transient local code-sign wrapper retry; proves the plugin
    endpoint matrix still controls enforcement evaluation.
  - `cargo test -p capsem-service credential_broker_detail_route_exposes_inventory_and_grant_surface -- --nocapture`
    passed after a transient local code-sign wrapper retry; proves the
    credential broker exposes a plugin-owned detail route for inventory and the
    initial grant surface.
  - `cargo test -p capsem-service plugin -- --nocapture` passed after plugin
    capabilities were added; proves plugin list/info expose broker-owned
    capability metadata including event families, supported providers, and
    credential source shapes.
  - `cargo test -p capsem-service plugin -- --nocapture` passed; proves debug
    dummy plugins are disabled by default, only affect evaluation when
    explicitly enabled, and plugin route updates still control the same
    SecurityEvent evaluation path.
  - `pnpm --dir frontend test -- --run frontend/src/lib/__tests__/plugin-section-contract.test.ts frontend/src/lib/__tests__/api.test.ts`
    passed; proves plugin UI mode labels/icons are derived from the typed enum
    and disabled plugins stay visible but inactive.
  - `cargo test -p capsem-core disabled_rules_remain_inventory_but_do_not_match -- --nocapture`
    passed; proves disabled rules remain in compiled inventory but cannot
    match/evaluate.
  - `cargo test -p capsem-service rules -- --nocapture` passed; proves profile
    enforcement/detection rule routes expose `enabled` and list disabled rules
    without letting them affect evaluation.
  - `pnpm --dir frontend test -- --run frontend/src/lib/__tests__/profile-page-contract.test.ts frontend/src/lib/__tests__/mcp-section-contract.test.ts`
    passed; proves Profile/MCP UI rows render typed policy metadata and disabled
    state from backend fields.
  - `pnpm --dir frontend test -- --run frontend/src/lib/__tests__/profile-page-contract.test.ts`
    passed after default grouping; proves Profile rule lists group from
    `rule.default_rule` rather than a second policy path.
  - `pnpm --dir frontend test -- --run frontend/src/lib/__tests__/api.test.ts`
    passed; proves frontend API helpers understand plugin detail routes and
    the credential broker detail endpoint.
  - `pnpm --dir frontend check` passed with zero Svelte/TypeScript warnings.
  - `cargo test -p capsem-core profile_mcp_tool_permission_mutation_updates_rule_and_pin -- --nocapture`
    passed; proves MCP tool permission readback resolves the real default MCP
    rule first, then the profile-managed rule after mutation, while preserving
    profile file pins.
  - `cargo test -p capsem-service profile_mcp_tool_edit_writes_profile_rule_and_mutation_ledger -- --nocapture`
    passed; proves the route mutation writes the profile mutation ledger and
    `tools/list` returns the effective `permission_action`/`permission_source`.
  - `cargo test -p capsem-core profile_mcp_default_permission -- --nocapture`
    passed; proves `default.mcp` readback/mutation updates the pinned
    enforcement rule file and changes fallback behavior for non-overridden MCP
    tools.
  - `cargo test -p capsem-core
    profile_mcp_tool_permission_override_wins_after_default_mutation --
    --nocapture` passed; proves profile-managed per-tool MCP overrides still
    win after the default MCP rule changes.
  - `cargo test -p capsem-service
    profile_mcp_default_edit_writes_default_rule_and_mutation_ledger --
    --nocapture` passed; proves `/profiles/{profile_id}/mcp/default/edit`
    mutates `[default.mcp]`, updates the profile file pin, writes the DB
    mutation ledger, and makes tool list readback inherit the new default.
  - `cargo test -p capsem-service mounted_mcp_routes_are_profile_scoped_mechanics_only -- --nocapture`
    passed; proves profile MCP routes expose the Capsem-owned local MCP entry
    as `source = builtin`, not as a settings-owned or live external runtime.
  - `pnpm --dir frontend test -- --run frontend/src/lib/__tests__/api.test.ts frontend/src/lib/__tests__/mcp-store.test.ts`
    passed; proves frontend MCP clients send `{ action }`, require explicit
    profile ids, and no longer expose unsupported server edit/delete helpers.
  - `pnpm --dir frontend test -- --run
    frontend/src/lib/__tests__/api.test.ts
    frontend/src/lib/__tests__/mcp-store.test.ts
    frontend/src/lib/__tests__/mcp-section-contract.test.ts` passed; proves the
    default MCP selector is route-backed and tied to `default.mcp` instead of
    local UI policy state.
  - `pnpm --dir frontend test -- --run
    frontend/src/lib/__tests__/api.test.ts
    frontend/src/lib/__tests__/plugin-section-contract.test.ts` passed; proves
    frontend types and Plugin UI render plugin-owned capability metadata.
  - `pnpm --dir frontend test -- --run
    frontend/src/lib/__tests__/profile-page-contract.test.ts` passed after the
    Profile overview update; proves overview reads route-backed surface
    availability and broker-visible credential inventory instead of inventing
    profile status text.
  - `pnpm --dir frontend test -- --run
    frontend/src/lib/__tests__/stats-view-contract.test.ts` passed; proves VM
    Stats distinguishes `exec_events` command executions from audit-port
    process observations and no longer renders the vague `Process Audit Events`
    label.
  - `pnpm --dir frontend test -- --run frontend/src/lib/__tests__/profile-page-contract.test.ts`
    passed; proves the Profile UI exposes enforcement and detection as
    first-class tabs instead of a generic policy tab, and renders typed asset
    status rows instead of raw JSON.
  - `uv run python -m pytest tests/test_config.py -q` passed; proves the
    generated frontend mock settings data includes the MCP permission fields
    from the checked-in generator.
  - `cargo test -p capsem-core provider_detection_promotes_unknown_host_by_canonical_model_path -- --nocapture`
    passed; proves canonical OpenAI/Anthropic/Google model paths promote
    unknown hosts into typed model protocol detection.
  - `cargo test -p capsem-core --lib net::mitm_proxy::tests:: -- --nocapture`
    passed; proves the MITM helper suite still keeps unrelated non-AI bodies
    uncaptured while AI and OAuth paths receive bounded previews.
  - `cargo check -p capsem-core` passed.
  - `cargo check -p capsem-core -p capsem-logger -p capsem-service` passed.
- Functional: focused source tests passed; live install not restarted or killed
  per evidence-preservation rule.
- Adversarial: pending; must include AGY activity that bypasses model stats
  today, plus unknown-domain OpenAI/Gemini/Claude-compatible traffic that is
  detected by bounded protocol-shape sniffing and flagged when the endpoint is
  not known or profile/corp-declared.
- E2E/VM: pending; must preserve current VM until destructive actions are
  explicitly approved.
- Telemetry/observability: pending; AGY model/tool activity must be visible
  through ledger-backed stats. Credential broker capture/rewrite must have
  first-class plugin/broker counters and recent-event evidence. Process audit
  must either be a clear process snapshot/inventory with correct timestamp
  labeling, or a real event stream with per-event times. MCP stats must not
  inflate user activity with internal snapshot/health/diagnostic calls. Snapshot
  UI must default to meaningful summaries/deltas, with full inventory available
  only as drill-down/forensics. Snapshot must be proven read-only for workspace
  inspection unless a separate explicit restore/export action is invoked. DNS,
  VSOCK, and MCP pagination findings need security/adversarial tests before the
  release gate is trusted. Profile UI must be tested with at least two profiles
  so singleton assumptions fail loudly. The UI must not invent a generic policy
  abstraction when the backend contract exposes enforcement, detection, and
  plugins. Plugin mode controls should use the enum contract directly and render
  disabled/dummy state clearly. MCP and rule controls should share the same
  mode/status visual vocabulary so users do not have to relearn semantics per
  tab. MCP builtin capability must not be misrepresented as an external stopped
  server. UI must not expose editable controls for backend routes that return
  deliberate 501/not-implemented responses. Disabled visual state and
  policy/mode iconography must be consistent across MCP, rules, and plugins.
  Default MCP behavior must be a visible real rule, not hidden policy. Per-tool
  MCP overrides must also be real rules with clear precedence over the default.
  Asset readiness should be inspectable per asset with check/error indicators.
  Overview should expose profile capability and credential availability first.
  Plugin pages and VM stats must expose plugin-owned info and activity, including
  credential broker support/capture evidence.
- Parse failures are release blockers until their producer/consumer boundary is
  identified and covered by regression tests.
- Performance: not in scope unless the observability fix adds measurable
  latency.
