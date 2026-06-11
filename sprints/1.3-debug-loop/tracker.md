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
  - [ ] Remaining: prove AGY tool-call/activity semantics beyond model HTTP
    rows, and verify against a rebuilt service/VM without destroying the current
    evidence VM until approved.
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
  - [ ] Remaining: verify against a rebuilt service/VM without destroying the
    current evidence VM, expose richer credential-broker capability/status in
    the UI/VM stats, and add a hermetic OAuth/broker flow once the local HTTP
    test server is in the next-gen testing harness.
- [ ] Implement bug 5 after user resumes coding: define what process audit is
  supposed to represent, fix timestamp semantics if it is a snapshot, and rename
  or reshape the UI so it reflects the actual data contract rather than a vague
  audit label.
- [x] Implement bug 6 slice: classify headline MCP stats so user-facing totals
  count only user tool calls (`tools/call`) and exclude protocol handshakes,
  `tools/list`, and builtin snapshot maintenance while raw rows remain in
  session DB for forensics.
- [ ] Implement bug 7 after user resumes coding: define snapshot UX/data
  contract as inventory vs delta vs evidence, add filters/summaries around
  changed/high-value files, and ensure raw thousands-of-files output is not the
  default user-facing state.
- [ ] Implement bug 8 after user resumes coding: non-destructively trace file
  provenance from paths, mtimes, process/security logs, and session DB evidence;
  prove whether snapshot is read-only or mutating the workspace; then add a
  regression test that snapshot cannot create workspace files unless explicitly
  requested.
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
- [ ] Implement bug 13 after user resumes coding: burn/rename the generic
  `Policy` UI surface and replace it with route-backed enforcement, detection,
  and plugin views that list rules/plugins from the contract, show source files
  and defaults, and expose allowed edits with enum/select/toggle controls.
- [ ] Implement bug 14 after user resumes coding: default dummy plugins to
  disabled, render disabled plugins as inactive/greyed out, and add consistent
  iconography for ask/block/pass-or-allow/rewrite/disable modes using the
  plugin contract values rather than UI-invented labels.
- [ ] Implement bug 15 after user resumes coding: apply the same contract-backed
  visual language to MCP and rules: grey out disabled MCP servers/tools/resources
  and disabled rules, group default rules visibly without making them a separate
  engine, and use consistent icons/select boxes/toggles for enum/boolean
  controls.
- [ ] Implement bug 16 after user resumes coding: define MCP source/lifecycle
  vocabulary (`builtin` vs external/server-backed), make the UI display that
  exact contract, and prevent builtin/static MCP entries from being shown as
  stopped servers unless there is a real stopped process.
- [ ] Implement bug 17 after user resumes coding: either implement profile
  persistence for MCP server/tool edits through the profile object/mutation
  ledger, or remove/disable the edit affordance and route until it is real; add
  tests so UI cannot expose unsupported 501 edit paths.
- [ ] Implement bug 18 after user resumes coding: create shared row/icon
  semantics for disabled entries across plugins, MCP, enforcement rules, and
  detection rules: grey/inactive styling for disabled state, plus policy/mode
  icon from the underlying enum.
- [ ] Implement bug 19 after user resumes coding: expose the default MCP rule
  as a visible, editable rule/policy selector where allowed by profile/corp
  constraints; test that changing the selector mutates the same rule contract
  used by enforcement, not a separate MCP policy field.
- [ ] Implement bug 20 after user resumes coding: add route/UI support for
  per-tool MCP overrides backed by specific enforcement rules, with tests for
  precedence over the default MCP rule and no reintroduction of a separate MCP
  decision engine.
- [ ] Implement bug 21 after user resumes coding: expose/render per-profile
  asset readiness as a checklist: asset name/kind, resolved source, expected
  hash, local path/status, downloaded/verified/missing/error state, and action
  where applicable.
- [ ] Implement bug 22 after user resumes coding: reshape overview to show
  profile capability/readiness: available surfaces, enabled plugins, credential
  broker status and credential reference list, plus blockers that prevent using
  a surface.
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
  - [ ] Remaining: render the credential-broker-specific panel in the UI,
    implement grant mutation/constraints, and connect those grants to broker
    replay/substitution decisions.
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
  - `pnpm --dir frontend test -- --run frontend/src/lib/__tests__/api.test.ts`
    passed; proves frontend API helpers understand plugin detail routes and
    the credential broker detail endpoint.
  - `pnpm --dir frontend check` passed with zero Svelte/TypeScript warnings.
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
