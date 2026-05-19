# Sprint: Policy, Settings, Profiles

## Where this sprint lives

**One branch, one worktree, one agent.** This sprint is executed
end-to-end on a single development branch in a single working tree.
Do not assume any item is being worked elsewhere unless this section
is updated with the concrete branch and worktree path, verified by
`git worktree list` and the listed branch's `git log`.

- **Branch:** `profile-v2`.
- **Worktree:** `/Users/elie/.codex/worktrees/824d/capsem`.
- **Verifying the state:** `git worktree list` shows every worktree
  on disk. `git log <branch> --oneline | head` shows what each
  branch has actually landed. **Read those two commands before
  trusting any prose in this file** -- prose drifts; git history
  does not.
- **Current git posture:** as of 2026-05-18, this branch is
  `69 ahead / 0 behind` `origin/main` in this worktree after the
  S07c asset-update orchestration planning slice. The rescue
  reconciliation is closed for the active profile sprint; do not
  resurrect the old "main is way ahead" warning unless `git
  rev-list --left-right --count HEAD...origin/main` says it is true
  again.

## Operating Mode

**Rescue is closed; push phase is active.** The S00-S06 audit and the
S07/S07a rescue work brought the branch back to a coherent profile-v2
contract:

- V1 settings/defaults authority is removed from the active runtime path.
- Profile V2 settings, resolver trace, Policy V2 runtime wiring, UDS profile
  and rule routes, package/tool contracts, profile schema artifacts, Pydantic
  admin contracts, and profile-driven VM asset readiness have landed.
- Old asset-only manifests are no longer runtime authority. `assets.manifest.*`
  service settings and setup-time signed asset manifest checks are removed.

The tracker is now a push board, not a rescue board. Work proceeds in this
order:

1. Finish S07a's remaining contract gaps: catalog-driven profile payload
   install/update/revoke, mandatory VM profile/revision/package pins,
   retention, forward-only VM create/fork/persist/resume enforcement, VM
   list/status profile-state reporting, and debug readiness. The next inserted
   stop is telemetry identity: every session must expose the VM id, profile id,
   and user id as a durable session fact before we keep pushing profile
   pinning.
2. Run S07c after S07a so background asset checks, manual `capsem update
   --assets`, status/debug provenance, and structured download logs all use the
   same Profile V2 asset authority.
3. Start S07b only after S07a/S07c are stable enough for `capsem-admin` to
   generate, validate, and check the same shapes.
4. Resume public-surface work in S07/S08/S09/S16 once the profile catalog and
   asset contract are no longer moving underneath them.

Winter readiness rules:

- The old stack is dead and stays dead.
- Profiles are the banner under which VM assets, package assumptions, and
  runtime policy march.
- A VM without explicit profile/revision/package/asset identity is invalid and
  must fail closed; there is no pre-S07a compatibility lane.
- The release gate is the wall: every claim needs tests, status/debug
  explanation, and tracker evidence before it crosses.

## Linear path

Strictly ordered. Finish item N before starting item N+1. No
parallel forks, no "if X then Y" branches, no parking-lot
proposals. If a new concern surfaces, it gets inserted into this
list at a specific position with a written reason -- never as a
side-branch.

Status: `[x]` done, `[~]` in flight, `[ ]` not started. "In flight"
without a verified branch + worktree pinning in
[Where this sprint lives](#where-this-sprint-lives) is **not**
a valid claim -- mark it `[ ]` instead.

1. [x] [S00 - Meta sprint setup](S00-meta-sprint-setup.md)
2. [x] [S01 - Remove v1 settings/policy](S01-remove-v1-settings-policy.md)
3. [x] [S02 - Service settings design](S02-service-settings-design.md)
4. [x] [S03 - Service settings implementation](S03-service-settings-implementation.md)
5. [x] [S04 - Profile design](S04-profile-design.md)
6. [x] [S05 - Profile implementation](S05-profile-implementation.md)
7. [x] [S06-pre - Network contract + confirm wiring](S06-pre-network-contract-and-confirm.md) -- closed. Callback wiring (slices 6a-6e), backoff refactor, adversarial backfill, and slice 6f exit tests all landed; details in [Completed sub-sprints](#completed-sub-sprints). Slice 6f's E2E capsem-doctor ask probe is **deferred** (see [Deferred items](#deferred-items-visible-debt)); slice 7 (`policy_confirm_events` table + remaining deferrals) is tracked separately as future S06-pre+ work.
8. [x] [S06 - Assembly and VM-effective settings](S06-assembly-vm-effective-settings.md) -- six sub-slices closed (parent-chain validation 6.1, layered merge 6.2, resolver trace 6.3, corp directives add/remove/replace 6.4, lock/forbid 6.5, runtime cutover + status/debug exposure 6.6). The in-VM E2E probe is **deferred** (see [Deferred items](#deferred-items-visible-debt)).
9. [x] [S06a - Model request rewrite support](S06a-model-request-rewrite-support.md) -- closed. `evaluate_model_request_policy` now applies the rewrite via `rewrite_model_request_body` against the `request.data` field (unified with the canonical condition vocabulary), forwards the redacted body upstream, and attributes telemetry to the matched rewrite rule. Fail-closed paths: unsupported target, non-UTF-8 body, pattern non-match. The `LastModelPolicyV2Decision::unsupported_rewrite` shim is removed.
10. [x] [S06b - Legacy allowlist migration + rule ownership locks](S06b-legacy-allowlist-migration-and-rule-ownership.md) -- closed. Inventory found that S01's runtime cutover left the legacy v1 settings registry + allowlist builders as test-only dead code, so "migration" boiled down to deletion plus enriching the v2 model. Nine slices landed: 6b.0 deleted v1 (~12k LOC), 6b.1 added ownership metadata fields, 6b.2 enforced priority tiers (corp `[-1000, -1]`, toggle-derived `0`, user `[1, 999]`, catch-all reserved `1000`), 6b.3 added nestable rules under setting hosts, 6b.4 added `http.read` / `http.write` callbacks, 6b.5 added per-type catch-all rules at priority `1000`, 6b.6 added provider-toggle derived rules, 6b.7 added MCP `allowed_tools` derived rules, 6b.8 added the `ensure_rule_editable` mutation gate. 6b.9 documentation scope captured in [S19 spec](S19-documentation-and-site.md).
11. [ ] [S06c - Ablate legacy NetworkPolicy runtime](#s06c---ablate-legacy-networkpolicy-runtime) -- new sprint, see inline brief below; promotes to a standalone spec when it starts.
12. [ ] [Post-S06 cleanup milestone](#post-s06-cleanup-milestone) -- deferred cleanup debt: `git merge origin/main` -> v2 rename -> full verification gate.
13. [~] [S07 - UDS service API](S07-uds-service-api.md) -- started; first
  foundation slice landed `capsem_proto::metrics` plus
  `ServiceToProcess::GetMetricsSnapshot` /
  `ProcessToService::MetricsSnapshot`; read-only profile list/get/resolve
  routes, profile create/fork/update/delete mutation routes, and rules
  list/get/evaluate routes landed. Rules read/evaluate is now hardened with a
  chained service workflow, generated `http.read`/`http.write` dry-run support,
  boolean catch-all CEL support, and a bounded large-profile evaluation test.
  Profile/settings composition has additional service coverage for create-id
  collisions across locked roots and selected-profile settings saves.
  Rules create/delete, confirm listing, skills, profile-backed VM create, and
  full route proof remain open.
14. [~] [S07a - Profile manifest, packages, and assets](S07a-profile-manifest-assets.md)
    -- started. Canonical profile catalog/status parser landed in
    `capsem-core::profile_manifest`; typed profile package/tool contracts and
    per-arch VM asset declarations now parse, validate, serialize through
    VM-effective settings, and merge through profile inheritance. The formal
    `schemas/capsem.profile.v2.schema.json` artifact and Rust golden fixture
    validation gate have landed. Python Pydantic v2 profile/manifest models now
    validate JSON through Pydantic, dump JSON through Pydantic, and bridge TOML
    through immediate Pydantic JSON validation. Rust now validates profile JSON
    and TOML payloads against the production schema artifact. Service startup
    now resolves/downloads VM assets from profile declarations, forwards
    expected profile hashes to `capsem-process`, rejects old asset manifests as
    runtime authority, and no longer exposes `assets.manifest.*` service
    settings. `session.db` now records VM/profile/user telemetry identity, and
    VM metadata now carries a profile pin with resolved profile id, signed
    profile revision, profile payload hash, package-contract hash, and pinned
    asset hashes. Remaining work adds manifest source fetch/scheduling, richer
    catalog clients/debug detail, and first-use selected-profile proof.
15. [ ] [S07c - Profile asset update orchestration](S07c-profile-asset-update-orchestration.md)
    -- unify background asset checks, manual `capsem update --assets`,
    status/debug provenance, structured download logs, and cleanup/create
    concurrency around Profile V2 asset authority.
16. [ ] [S07b - Capsem admin tooling and profile-derived images](S07b-capsem-admin-tooling.md)
    -- unify Python builder/manifest/profile tooling under released
    `capsem-admin`; derive images from profiles; remove hand-edited image
    settings as authority.
17. [ ] [S08 - HTTP gateway API](S08-http-gateway-api.md)
18. [ ] [S09 - CLI integration](S09-cli-integration.md)
19. [ ] [S10 - Credential brokerage](S10-credential-brokerage.md)
20. [ ] [S11 - Status, debug, provenance](S11-status-debug-provenance.md)
21. [ ] [S12 - OpenTelemetry metrics architecture](S12-observability-plugin.md)
22. [ ] [S13 - Remote policy plugin](S13-remote-policy-plugin.md)
23. [ ] [S14 - Rules UI components](S14-rules-ui-components.md) -- rule editor component is consumed by S15.
24. [ ] [S15 - Confirm UX (Ask)](S15-confirm-ux.md)
25. [ ] [S16 - Profile UI](S16-profile-ui.md)
26. [ ] [S17 - Security capabilities UI](S17-security-capabilities-ui.md)
27. [ ] [S19 - Documentation and site](S19-documentation-and-site.md)
28. [ ] [S18 - Full verification and release gate](S18-full-verification-release-gate.md)

## S06c - Ablate legacy NetworkPolicy runtime

Goal: remove the second policy runtime so V1 is gone end-to-end.

S01 removed the V1 settings registry but kept the V1 runtime
plumbing (`crates/capsem-core/src/net/policy.rs`,
`crates/capsem-core/src/net/mitm_proxy/policy_hook.rs`,
`SharedPolicy` type alias). After S01 + S06 + S06b, V1's
domain+method allow/deny is expressible as `dns.request` /
`http.request` rules in V2 with `decision = "block"` and the V1
hook is structurally redundant.

Scope:

- Delete `crates/capsem-core/src/net/policy.rs` (`NetworkPolicy` struct).
- Delete `crates/capsem-core/src/net/mitm_proxy/policy_hook.rs`
  (the V1 hook) and its tests.
- Remove the V1 hook from `make_production_pipeline*` registration.
- Remove the `policy: SharedPolicy` field from `MitmProxyConfig`,
  `DnsHandler`, etc. The V2 `policy_v2` field becomes the only
  policy field.
- Collapse `SharedPolicyV2` -> `SharedPolicy` (single alias).
- Reroute the DNS `is_fully_blocked(qname)` check to V2 rule lookup;
  the `dns.request` callsite already handles this path.
- Regression test: confirm the migrated V1 denial behavior is
  preserved by the equivalent V2 rule (uses the migration tables
  produced by S06b).

When this sprint starts, promote the inline brief above into
`sprints/policy-settings-profiles/S06c-ablate-legacy-networkpolicy.md`.

## Post-S06 cleanup milestone

Originally planned to run before S07. The rescue merge/reconciliation portion is
closed for the active branch: `HEAD...origin/main` is currently `69 ahead / 0
behind`. The remaining cleanup debt is now S06c plus the final V2 naming
collapse and release gate. When executed, keep the order:

1. **Confirm branch remains caught up.** Run `git rev-list --left-right --count
   HEAD...origin/main`. If the right-hand count is non-zero, merge/reconcile
   before rename work.
2. **V2 rename across the crate.** With V1 ablated by S06c the
   rename collapses cleanly:
   - Files: `policy_v2_http_hook.rs` -> `policy_http_hook.rs`,
     `policy_v2_model.rs` -> `policy_model.rs`,
     `benches/policy_v2.rs` -> `benches/policy.rs` (incl. dirs).
   - Types: `PolicyV2HttpHook` -> `PolicyHttpHook`,
     `LastHttpPolicyV2Decision` -> `LastHttpPolicyDecision`,
     `LastModelPolicyV2Decision` -> `LastModelPolicyDecision`.
   - Fields: `policy_v2_rule_name` -> `policy_rule_name`;
     `policy_v2_decision` -> `policy_decision`;
     `policy_v2_snapshot` -> `policy_snapshot`.
   - Helpers: `policy_v2_from_toml` -> `policy_from_toml`;
     `resolve_policy_v2_action` -> `resolve_ask_action`.
   - All `policy_v2_*` test function names drop the prefix.
   - The `policy_v2: SharedPolicyV2` field collapses to
     `policy: SharedPolicy` once S06c removed the V1 field.
3. **Full verification gate.**
   `just test`, `just smoke`, `just run "capsem-doctor"`,
   `just inspect-session`. No warnings.

Public API work has already started, so any rename fallout must be reconciled
against the S07/S07a route contracts before S08 HTTP mirroring.

### Merge conflict guidance (applies in step 1)

Conflicts most likely in:
`crates/capsem-service/src/main.rs` around
`enrich_telemetry_from_session_db` / `handle_list` / `handle_info`,
the new `/list` regression test, and policy code touched by the
parallel hardening work.

Resolve in favor of main where the conflict overlaps with
[S12's](S12-observability-plugin.md) intent (the `/list`
SQL-on-hot-path hotfix and the `attach_list_live_metrics_placeholder`
/ regression test pair). Preserve the S06-pre confirmer plumbing
landed across slices 6a-6e: `crates/capsem-core/src/net/policy_confirm.rs`
(including `confirm_with_backoff` + `default_confirm_backoff`),
the DNS / HTTP / MCP / model ask wiring callsites, and the
per-subsystem `confirm_opts` builders.

## Notes for upcoming work

(Only items that inform a sprint not yet started. Anything tied to
a closed slice/sprint moved to [completed sub-sprints](#completed-sub-sprints).)

- **S07 inherits a proto-types task.** Foundational metrics types
  (`capsem_proto::metrics`) land in S07 so [S12](S12-observability-plugin.md)
  can start with proto already in place. See S12 spec.
- **S07a is a public-contract bridge.** Before HTTP, CLI, and UI harden profile
  create/VM create semantics, the signed manifest must become the profile
  catalog and profiles must carry a closed `capsem.profile.v2` contract backed
  by JSON Schema Draft 2020-12, with package/tool contracts plus per-arch VM
  asset declarations. S07a also defines first-use asset download, profile
  revision status, cleanup retention, and persistent VM profile/revision/asset
  pins.
- **S07c is the asset-update bridge.** The background downloader exists, but
  `capsem update --assets`, status/debug provenance, structured lifecycle logs,
  and cleanup/create concurrency must be unified around the Profile V2 service
  reconciler before profile asset operations are production-grade.
- **S07b is the admin tooling bridge.** The current Python image builder and
  manifest scripts must be unified under a released `capsem-admin` package.
  Profiles become the source of truth for image build plans and manifest
  entries; `capsem-admin profile validate/schema` consumes the shared JSON
  Schema artifact and valid/invalid fixtures; Python admin internals use
  Pydantic v2 models with Pydantic-only JSON input/output instead of raw nested
  dicts; hand-edited `guest/config` image settings are not carried forward as
  compatibility input.
- **S12 architecture: single source of truth.** The in-memory
  per-VM accumulator in `capsem-process` is the only runtime
  source; `session.db` is read on the data path exactly twice in
  a VM's life (seed at launch + cold one-shot in stopped-VM
  `/info`). No `/list` / scrape endpoints / running-VM `/info` /
  gateway status path opens `session.db`. Two open questions
  remain (hypervisor-vs-guest-agent for guest counters; new-counter
  schema migration); decide before [S12](S12-observability-plugin.md)
  starts.
- **S15 release hold.** Do not ship a release that advertises
  `decision = "ask"` while only `PlaceholderConfirmer` is
  registered. Either [S15](S15-confirm-ux.md) lands the UI + CLI
  prompter, or release docs say ask = allow-by-default. The
  same hold is captured in [MASTER Release Holds](MASTER.md#release-holds).

## Completed sub-sprints

One-line each. Detail lives in the corresponding spec file and in
the commit history.

- **S00** (2026-05-14) - Meta sprint setup: board, requirements,
  plan, tracker, all sub-sprint files.
- **S01** (2026-05-14) - V1 settings/policy removal: provision/run
  VM defaults, `/mcp/*`, `capsem-process` runtime, `/settings*`
  cut over to typed `settings_profiles`. Strict payload contract
  (no legacy `tree` / `issues` / `presets` / `policy` keys).
- **S02** (2026-05-14) - Service settings design closed.
- **S03** (2026-05-14) - Service settings implementation: typed
  service settings, profile TOML, built-in Everyday Work profile,
  TOML credentials, profile discovery, descriptors. Asset/manifest
  startup wiring + `/setup/assets` provenance.
- **S04** (2026-05-14) - Profile design closed; canonical v1 rule
  format locked at `security.rules.<type>.<rule_name>` with
  default priority `1`.
- **S05** (2026-05-14) - Profile implementation: parser, validation,
  CRUD primitives, fork, security capabilities, narrowed profile
  types.
- **S06-pre slices 6a-6e** - Confirmer trait + placeholder; DNS,
  HTTP request+response, MCP request+response, model
  request/response/tool-call/tool-response ask wiring.
- **S06-pre adversarial backfill** - Per-subsystem redaction +
  oversized-snapshot + concurrency + panic-isolation tests. TDD
  surfaced two real bugs (HTTP path unbounded; MCP tool_name
  unbounded), fixed via per-field truncation.
- **S06-pre backoff refactor** - Replaced the bespoke
  `Confirmer::timeout()` + `DEFAULT_CONFIRMER_TIMEOUT` constant
  with the shared `capsem_proto::poll::RetryOpts` /
  `crate::poll::poll_until` primitives. New
  `confirm_with_backoff(confirmer, args, &RetryOpts)` wraps each
  attempt in a per-attempt timeout and retries with exponential
  backoff up to the overall deadline. All five callsites (DNS,
  HTTP req/resp, MCP req/resp, model) route through it. Each
  subsystem state has a `confirm_opts: RetryOpts` field with a
  `with_confirm_opts` builder.
- **S06-pre slice 6f - Exit tests** (closed) -
  `confirm_with_backoff` contract tests (accept/deny passthrough,
  hang -> Deny on timeout, panic propagation across the await
  boundary, documented defaults); 200-way concurrent-load smoke
  for HTTP ask resolution; resolved-outcome attribution fix in
  HTTP / DNS / model so `policy_action` reflects `"allow"` /
  `"block"` after the confirmer returns (MCP already correct).
  The capsem-doctor E2E ask probe is deferred (needs doctor
  policy-injection + session-DB read-back fixtures). See
  [Deferred items](#deferred-items-visible-debt) for the
  carry-over.
- **S06 - Assembly and VM-effective settings** (closed,
  2026-05-15 / 2026-05-16) - Six slices: parent-chain
  validation + ancestor-chain helper (6.1), layered profile
  merge with `inherited_from` provenance (6.2), resolver trace
  artifact `vm-effective-trace.json` + service-side attach
  (6.3), corp directives add/remove/replace (6.4), lock /
  forbid + typed `ResolverViolation` (6.5), trace summary in
  status / debug + `Reject` event before violation early
  return (6.6). In-VM E2E probe deferred with same unblock as
  S06-pre slice 6f.
- **S06a - Model request rewrite** (closed, 2026-05-15) -
  `evaluate_model_request_policy` applies rewrite via
  `rewrite_model_request_body` on `request.data` (unified with
  the condition vocabulary). Fail-closed paths: unsupported
  target, non-UTF-8 body, pattern non-match. Removed the
  `unsupported_rewrite` shim. 4 new tests plus 1 repurposed
  integration test.
- **S06b - Legacy allowlist migration + rule ownership locks**
  (closed, 2026-05-16) - Nine slices. Inventory found S01's
  cutover left v1 settings registry + allowlist builders as
  test-only dead code, so the sprint became: 6b.0 deleted
  ~12k LOC of v1 surface; 6b.1 added ownership metadata fields
  (`owner_setting_path`, `owner_setting_label`, `editable`)
  on `EffectiveRule`; 6b.2 enforced priority tiers (corp
  `[-1000, -1]`, toggle-derived `0`, user `[1, 999]`,
  catch-all reserved `1000`) with origin-aware corp-exclusive
  validation; 6b.3 added nestable rule blocks under setting
  hosts (`ai.providers.<name>.rules.*`,
  `mcp.connectors.<name>.rules.*`); 6b.4 split HTTP catch-all
  into `http.read` / `http.write` callbacks dispatched by
  method group; 6b.5 retargeted capability-derived rules from
  priority 100 -> 1000 as proper per-runtime-callback
  catch-alls (`dns.default`, `http.default_read`,
  `http.default_write`, `model.default`, `mcp.default`); 6b.6
  added provider-toggle derived rules at priority 0 from
  `ai.providers.<name>.enabled` (static host map + base_url
  fallback for unknown providers); 6b.7 added MCP
  `allowed_tools` derived rules at priority 0; 6b.8 added
  `ensure_rule_editable` mutation gate returning
  `RuleManagedBySetting { rule_id, owner_setting_path }`. 6b.9
  documentation scope captured in
  [S19 spec](S19-documentation-and-site.md) as a
  decisions-to-document appendix + per-slice docs task list.

## Coverage ledger (sprint-wide rollup)

Current as of 2026-05-16 after S06 / S06a / S06b closed.

- **Unit/contract**: `settings_profiles` carries **118** focused
  tests (resolver, ownership, priority validation, nestable
  rules, catch-alls, provider toggles, MCP allowed_tools,
  mutation gate). `corp/tests.rs` carries **18** corp-directive
  tests. `resolver_trace/tests.rs` carries **9** trace tests.
  HTTP/DNS/MCP/model confirm wiring covered;
  `confirm_with_backoff` covered by 5 dedicated tests.
  `http.read` / `http.write` callback split covered by **5**
  hook-boundary tests in `policy_v2_http_hook/tests.rs`.
  S07 metrics proto foundation adds **36** focused `capsem-proto`
  IPC tests and **18** focused `capsem-process` IPC tests. S07a
  telemetry identity now has focused logger schema/writer/reader,
  core env-resolution, and service serialization/enrichment tests. Profile
  manifest lifecycle gates now have explicit `active` / `deprecated` /
  `revoked` install/new-VM/existing-VM contract tests, plus current/specific
  revision resolution tests in both Rust and Pydantic admin models. Core
  install guards cover active-status, BLAKE3 payload hash, schema validation,
  and manifest/payload id+revision parity in both Rust and Pydantic admin
  models. Runtime conversion/materialization tests prove verified Profile V2
  payloads become resolver-compatible corp TOML while preserving the exact
  signed payload bytes in installed revision storage; `current.json` records
  the installed profile id, revision, and payload hash for later status/debug
  and VM pinning. Profile payload signature verification reuses the existing
  minisign verifier with tamper coverage; fetch tests prove catalog payload/
  signature locations are read and verified before hash/schema/id/revision
  checks. Core profile catalog reconciliation covers active install/update,
  incomplete active re-install, complete active no-op, deprecated installed
  revision keep, and revoked launchable profile plus current-state removal. VM
  profile pins add registry roundtrip, package-contract hash, installed sidecar
  revision/payload-hash capture, API serialization, and fork persistence
  coverage. Service profile catalog reconciliation covers active current
  revision install and revoked installed revision removal through
  `POST /profiles/catalog/reconcile`, including per-revision error summaries.
  The native CLI parser now covers `capsem profile reconcile-catalog
  --manifest --pubkey [--json]`. Absent installed profile cleanup now has a
  core contract test for removing launchable current state while preserving the
  archived payload plus service-route coverage for the `absent_removed`
  summary/outcome. Retention-source coverage now proves installed current
  profile payloads emit hash-derived VM asset filenames, archived payloads
  without `current.json` do not retain assets, persistent VM profile pins feed
  saved-asset retention, and real cleanup preserves the combined profile+VM-pin
  set while deleting an unreferenced hash-named asset. Production cleanup now
  adds a manifest-free hash cleanup helper plus `POST /setup/assets/cleanup`,
  preserving installed-profile and saved-VM retention, deleting stale
  hash-named files and legacy `v1.0.*` directories, and returning
  `409 Conflict` while assets are checking or updating. VM list/status now
  reports pinned profile id/revision plus current/needs_update/deprecated/
  revoked/corrupted/unknown state, and `capsem list`/`capsem info` render the
  typed client enum; missing pins are corrupted. Profile pin construction now
  requires a signed catalog revision, profile payload hash, and pinned asset
  identity, and create-from-source/fork/persist reject missing, revisionless,
  or payload-hash-less pins before durable clone/move work. Fork cloning now
  preserves VM-effective profile attachments, rejects profile and payload-hash
  drift, and has fork-plus-exec IPC coverage for same-profile execution.
- **Functional**: profile CRUD, VM-effective resolve via
  ancestor chain, layered merge, resolver trace artifact
  round-trip, corp directives end-to-end through
  `resolve_effective_vm_settings_with_corp`, debug-report
  rendering with resolver-trace summary, service startup +
  asset settings, verified profile payload materialization into the corp
  profile root and installed revision payload storage, service API profile
  catalog reconcile install/revoke/absent-removal summaries, native
  CLI-to-service wiring for `profile reconcile-catalog`, `/setup/assets`
  provenance, profile-aware cleanup retention source composition, `POST
  /setup/assets/cleanup` cleanup execution with installed-profile/saved-VM
  retention, `/list`/`/info`/`capsem list`/`capsem info` profile-state
  rendering, create-from-source/fork/persist fail-closed profile pin gates,
  fork-plus-exec same-profile IPC coverage, profile payload-hash pin
  enforcement, mitm_proxy integration test for model.request rewrite
  redaction.
- **Adversarial**: profile load (unknown fields, malformed TOML,
  bad endpoint schemes, callback/type mismatches, duplicate
  profile ids, governance toggles). Inheritance graph: unknown
  parent, multi-hop cycles, depth overflow. Confirm wiring:
  redaction, bounds, concurrency, panic isolation, hang
  fail-closed. Corp directives: unknown path, type mismatch,
  add-on-existing, remove-on-missing, lock then re-mutate,
  forbid then add restores (all surface `ResolverViolation`
  with a `Reject` trace event before the early return). Asset
  pipeline: full malformed-input matrix. Priority validation:
  out-of-range high/low, reserved catch-all `1000`, corp
  priority in non-corp profile, corp directive at user-tier
  priority. Model.request rewrite: unsupported target, no
  match, non-UTF-8 body.
- **E2E/VM**: covered for the S03 service-settings asset
  runtime slice (real service + real gateway + malformed TOML
  startup + VM boot/exec) and the S06a mitm_proxy integration
  test forwarding rewritten model bodies. Capsem-doctor ask
  probe remains deferred (see below).
- **Telemetry**: debug report exposes
  profile/settings/rule provenance and now the resolver trace
  summary (event count, corp event count, locked paths,
  rejected paths, last N events). Hook-boundary attribution
  for ask resolves locks the resolved outcome (`allow` /
  `block`). S07a adds a durable `session_identity` row to
  `session.db` with `vm_id`, `profile_id`, and `user_id`, service
  propagation into `capsem-process`, `/info` exposure, and focused
  read-back coverage. VM metadata surfaces the corresponding profile pin for
  status/detail paths without reopening `session.db` on `/list`, and now
  requires the installed profile payload hash for forward VM pin construction
  and source/fork/persist validation.
  Persisted
  policy-decision read-back from a running `session.db` (capsem-doctor E2E ask
  probe) is **deferred**.
  `policy_confirm_events` table remains S06-pre slice 7+ work.
- **Performance**: no benchmarks added by S06/S06a/S06b; the
  resolver runs at provision / reload, not on the hot path,
  so benchmarks would not represent a meaningful budget.
  Performance work remains pending for later sprints (S12
  in-memory metrics accumulator is the next perf-shaped piece).
  The S07 metrics snapshot request is classified as read-only
  `HealthCheck` IPC so it does not enter job/lifecycle dispatch.
- **Test-gate snapshot** (cargo test, updated 2026-05-18 for S07a service
  profile catalog reconciliation and the first native CLI hook):
  `cargo test -p capsem-logger` **100** + **126** passed;
  `cargo test -p capsem-service` **107** + **140** passed;
  after VM profile pins, `cargo test -p capsem-service` **108** + **141**
  passed;
  after installed profile payload identity pins, `cargo test -p capsem-service`
  **108** + **142** passed;
  after the service profile catalog reconcile route, `cargo test -p
  capsem-service` **108** + **144** passed;
  after the native profile catalog reconcile CLI hook, `cargo test -p capsem`
  **240** passed;
  after absent installed profile cleanup, `cargo test -p capsem-core
  reconcile_ --lib` **6** passed and `cargo test -p capsem-service
  handle_reconcile_profile_catalog` **3** passed;
  package gates after absent cleanup: `cargo test -p capsem-service`
  **108** + **145** passed and `cargo test -p capsem` **241** passed;
  `cargo test -p capsem-core --lib` **1612** passed / 0 failed / 1 ignored
  after absent installed profile cleanup;
  after profile-aware asset retention sources, `cargo test -p capsem-core
  installed_profile_asset_filenames --lib` **2** passed, `cargo test -p
  capsem-core settings_profiles --lib` **133** passed, and `cargo test -p
  capsem-service saved_vm_assets` **2** passed;
  package gates after profile-aware asset retention sources: `cargo test -p
  capsem-core --lib` **1614** passed / 0 failed / 1 ignored and `cargo test -p
  capsem-service` **110** + **145** passed;
  after the profile-aware asset cleanup caller, `cargo test -p capsem-core
  cleanup_ --lib` **7** passed, `cargo test -p capsem-core --lib` **1615**
  passed / 0 failed / 1 ignored, `cargo test -p capsem-service
  handle_asset_cleanup` **2** passed, and `cargo test -p capsem-service`
  **110** + **147** passed;
  after forward-only resume pin enforcement, `cargo test -p capsem-service
  resume_saved_vm` **2** passed and `cargo test -p capsem-service` **109** +
  **148** passed;
  after VM list/status profile-state reporting, `cargo test -p capsem-service
  profile_status` **1** passed, `cargo test -p capsem-service
  handle_reconcile_profile_catalog_installs_current_active_revision` **1**
  passed, `cargo test -p capsem format_session_profile_for_list` **1** passed,
  and `cargo test -p capsem list_response_with_entries` **1** passed;
  full package proof for the same slice: `cargo test -p capsem-service`
  **109 + 149** passed and `cargo test -p capsem` **242** passed;
  after forward-only create/fork/persist profile pin enforcement, `cargo test
  -p capsem-service vm_profile_pin_requires_signed_catalog_revision` **1**
  passed, `cargo test -p capsem-service
  provision_from_source_requires_profile_revision_pin` **1** passed, `cargo
  test -p capsem-service handle_fork_rejects_source_without_profile_revision_pin`
  **1** passed, `cargo test -p capsem-service
  handle_persist_rejects_running_vm_without_profile_revision_pin` **1** passed,
  nearby fork/resume positive-path tests passed, and `cargo test -p
  capsem-service` **109 + 153** passed;
  after fork profile-integrity coverage, `cargo test -p capsem-core
  clone_sandbox_state_preserves_vm_effective_profile_attachments` **1** passed,
  `cargo test -p capsem-service handle_fork_preserves_profile_and_fork_exec_works`
  **1** passed, and `cargo test -p capsem-service
  handle_fork_rejects_profile_string_drift_after_clone` **1** passed;
  full package proof after fork profile-integrity coverage: `cargo test -p
  capsem-core --lib` **1616** passed / 0 failed / 1 ignored, `cargo test -p
  capsem-service` **109 + 155** passed, and `cargo test -p capsem` **242**
  passed;
  after mandatory VM profile payload hashes, `cargo test -p capsem-service
  profile_payload_hash` **3** passed, `cargo test -p capsem-service
  vm_profile_pin` **5** passed, `cargo test -p capsem-service handle_fork`
  **8** passed, full `cargo test -p capsem-service` **109 + 158** passed,
  and `cargo test -p capsem` **242** passed;
  `cargo test -p capsem-core profile_manifest --lib` **20** passed;
  `cargo test -p capsem-core settings_profiles --lib` **130** passed after
  core profile catalog reconciliation;
  `cargo test -p capsem-core --lib` **1611** passed / 0 failed / 1 ignored
  after core profile catalog reconciliation;
  `uv run pytest tests/test_profiles.py -q` **12** passed;
  `cargo test -p capsem-core telemetry --lib` **31** passed;
  `cargo test -p capsem-process --no-run` passed; and
  `cargo test -p capsem-mcp-aggregator --no-run` passed.
  Prior full snapshot (2026-05-16):
  capsem-core lib **1590** passed / 0 failed / 1 ignored;
  capsem-service **95** + **119** passed; capsem-process **98**
  passed; capsem-logger **98** + **126** passed. No warnings on
  touched code; rustc `deny(warnings)` clean. `just test`,
  `just smoke`, `just run "capsem-doctor"`,
  `just inspect-session` are folded into the [Post-S06 cleanup
  milestone](#post-s06-cleanup-milestone) full-verification
  step, not re-run per-slice (no slice in S06/S06a/S06b touched
  guest binaries or VM boot path, so the doctor-gated checks
  are not a meaningful regression catcher for what landed).

### Deferred items (visible debt)

- **capsem-doctor E2E ask probe** -- fire one ask rule per
  subsystem from inside a running VM and read the matched
  rule label back out of `session.db`. Unblock requires the
  [S07 Rules API](S07-uds-service-api.md#rules-api) plus the
  [S15 resolve routes](S15-confirm-ux.md). Hook-boundary
  attribution is locked by the Rust-side functional tests so
  this is a coverage-gap item, not a correctness gap.
- **capsem-doctor E2E corp-directive probe** -- launch a VM
  with a multi-level inherited profile + a corp replace
  directive; assert `/debug/report` shows the resolved policy.
  Same unblock as above (S07 Rules API).
- **Streaming sliding-window body inspector**, pattern
  max-match-length parse-time enforcement, structural rewrite
  parse rejection, instant propagation (`ReloadConfig` push +
  `Arc<PolicyState>` swap), per-chunk `Arc` revalidation,
  `policy_confirm_events` + `policy_body_inspection_events`
  tables. These remain S06-pre slice 7+ work.
