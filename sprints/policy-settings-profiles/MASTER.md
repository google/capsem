# Policy, Settings, Profiles Master

Last updated: 2026-05-15

## Where this sprint lives

**Single branch, single worktree.** Authoritative pinning is in
[tracker.md "Where this sprint lives"](tracker.md#where-this-sprint-lives);
the short version:

- Branch: `claude/adoring-joliot-98a4cb`
- Worktree: `/Users/elie/git/capsem/.claude/worktrees/adoring-joliot-98a4cb`
- Verify with `git worktree list` + `git log <branch> --oneline | head`
  before believing any "in flight elsewhere" claim.

## Mission

Replace Capsem's v1 settings/policy stack with typed service settings and
VM/session profiles. Profiles become the only user-facing "security level"
concept. The old ad hoc settings registry, standalone `[mcp]` authority, and
hand-authored `config/defaults.json` runtime/UI source are removed completely.

## Product Contract

- **Service settings are service/app-scoped.** They configure app/service
  behavior, profile roots, telemetry export, remote policy plugin endpoints, and
  credential storage for the cutover.
- **Profiles are VM/session-scoped.** They configure AI providers, MCP and
  connectors, skills, VM settings, security capabilities, canonical rules, and
  derived/generated rules for sessions and VMs.
- **VM-effective settings are attached to a VM/session.** Runtime enforcement,
  debug reports, status, guest materialization, and UI truth read the resolved
  VM-effective profile state.
- **No v1 compatibility.** There is no migration layer and no special diagnostic
  layer for old config shapes.
- **TOML first.** Rust structs plus Serde/TOML parsing and Rust validators define
  syntax, defaults, validation, and semantics.

## Sprint Board

Strictly ordered linear path. Each sprint runs to completion before
the next starts. The `#` column is the execution index;
[tracker.md](tracker.md) is the canonical source.

| # | Sprint | Status | Purpose |
| --- | --- | --- | --- |
| 1 | [S00 - Meta Sprint Setup](S00-meta-sprint-setup.md) | Done | Create durable planning/control artifacts. |
| 2 | [S01 - Remove V1 Settings/Policy](S01-remove-v1-settings-policy.md) | Done | Remove v1 registry/config authority and prove Capsem still boots. |
| 3 | [S02 - Service Settings Design](S02-service-settings-design.md) | Done | Design typed service settings with user review. |
| 4 | [S03 - Service Settings Implementation](S03-service-settings-implementation.md) | Done | Implement typed service settings, validation, defaults, descriptors. |
| 5 | [S04 - Profile Design](S04-profile-design.md) | Done | Design profile TOML and UX/security model with user review. |
| 6 | [S05 - Profile Implementation](S05-profile-implementation.md) | Done | Implement profile files, discovery, validation, CRUD primitives. |
| 7 | [S06-pre - Network Contract + Confirm Wiring](S06-pre-network-contract-and-confirm.md) | Done | Normalize policy network callback/field contracts and wire `ask -> confirm()`. Closed with slices 6a-6e (callback wiring), backoff refactor, adversarial backfill, and [slice 6f - exit tests](tracker.md#slice-6f---exit-tests). Slice 6f's E2E capsem-doctor ask probe is deferred; `policy_confirm_events` table is slice 7+ work. |
| 8 | [S06 - Assembly And VM-Effective Settings](S06-assembly-vm-effective-settings.md) | Foundational slice only | Resolve profiles/corp governance into VM-attached settings and derived rules. `resolve_effective_vm_settings()` + VM-effective persistence is in HEAD; parent-chain resolution, corp directives, layered resolver, and trace artifact are unstarted. Owned by this branch + worktree -- no parallel work elsewhere. |
| 9 | [S06a - Model Request Rewrite Support](S06a-model-request-rewrite-support.md) | Not Started | Implement `model.request` rewrite for `request.data` and remove unsupported fail-closed placeholder behavior. |
| 10 | [S06b - Legacy Allowlist Migration And Rule Ownership Locks](S06b-legacy-allowlist-migration-and-rule-ownership.md) | Not Started | Port legacy allowlist outputs into canonical rules and enforce generated-rule ownership (`managed by <setting>`, uneditable). |
| 11 | [S06c - Ablate Legacy NetworkPolicy Runtime](tracker.md#s06c---ablate-legacy-networkpolicy-runtime) | Not Started | Delete `policy.rs` + `policy_hook.rs`; remove the V1 hook from production pipeline; collapse `SharedPolicyV2` -> `SharedPolicy`. Closes the V1 runtime that S01 left behind. |
| 12 | [Post-S06 cleanup milestone](tracker.md#post-s06-cleanup-milestone) | Not Started | `git merge origin/main` -> v2 rename -> full verification gate, in that order. S07 starts on the post-cleanup codebase. |
| 13 | [S07 - UDS Service API](S07-uds-service-api.md) | Not Started | Expose settings/profile/MCP/skills + `capsem_proto::metrics` types over UDS. |
| 14 | [S08 - HTTP Gateway API](S08-http-gateway-api.md) | Not Started | Wire HTTP endpoints to UDS behavior. |
| 15 | [S09 - CLI Integration](S09-cli-integration.md) | Not Started | Add `profile`, `mcp`, `skills`, and `confirm` CLI command families. |
| 16 | [S10 - Credential Brokerage](S10-credential-brokerage.md) | Not Started | Define credential release from service settings into sessions. |
| 17 | [S11 - Status, Debug, Provenance](S11-status-debug-provenance.md) | Not Started | Make status/debug explain active settings, profiles, derived rules, MCP, skills. |
| 18 | [S12 - OpenTelemetry Metrics Architecture](S12-observability-plugin.md) | Not Started | Typed per-VM live-metrics architecture: `capsem-proto::metrics`, process-side accumulator, bincode IPC snapshot, service `/metrics/json` + `/metrics`, gateway proxy, UI typed-JSON. Inherits the release-team OTel handoff. |
| 19 | [S13 - Remote Policy Plugin](S13-remote-policy-plugin.md) | Not Started | Add service plugin for remote policy events/decisions. |
| 20 | [S14 - Rules UI Components](S14-rules-ui-components.md) | Not Started | One reusable rule editor/renderer + per-type rule blocks (DNS/HTTP/Model/MCP). **The editor is embedded by [S15](S15-confirm-ux.md) for forward-rule decisions -- design it for embedding from the start, pre-fillable from derived rule input.** |
| 21 | [S15 - Confirm UX (Ask)](S15-confirm-ux.md) | Not Started | Production answer path for `decision = "ask"`: stacked pending-ask queue, UI prompter embedding the S14 rule editor, CLI parity, auto-rule derivation per callback, `policy_confirm_events` integration. Replaces the placeholder confirmer that ships from S06-pre. |
| 22 | [S16 - Profile UI](S16-profile-ui.md) | Not Started | First-class profile selector/create/fork/delete/edit flows. |
| 23 | [S17 - Security Capabilities UI](S17-security-capabilities-ui.md) | Not Started | Capability controls above canonical rule editing. |
| 24 | [S19 - Documentation And Site](S19-documentation-and-site.md) | Not Started | Document the engine, corporate deployment, telemetry, remote policy, custom profiles/images. |
| 25 | [S18 - Full Verification And Release Gate](S18-full-verification-release-gate.md) | Not Started | Backend/UI/E2E/install proof. Last sprint. |

S15 was previously a "Settings UI Redesign" sprint; that scope is now
folded into the descriptor-driven UI work in S14 / S16 / S17.

## Release Holds

- Do not edit runtime code for this redesign without keeping this board and
  `tracker.md` synchronized.
- Do not preserve old config semantics or fallback behavior.
- Do not ship a backend surface without debug report/status coverage for wrong
  settings and profile resolution.
- Do not wire UI before UDS, HTTP, and CLI contracts are tested.
- Do not start S06 resolver cutover implementation until S06-pre network and
  confirm wiring gate passes.
- Do not declare model policy rewrite-complete while `model.request` rewrite is
  still unsupported/fail-closed; S06a must pass.
- Do not leave legacy allowlist behavior on old builders; S06b must migrate it
  into canonical rules with ownership locks.
- Do not enter the final release gate while public docs still describe v1
  settings, old security levels, standalone `[mcp]`, or defaults-json authority.
- Do not ship a release that advertises `decision = "ask"` as a
  user-facing capability while the only registered Confirmer is the
  S06-pre `PlaceholderConfirmer`. Either [S15 - Confirm UX](S15-confirm-ux.md)
  must land a real UI+CLI prompter (and the auto-rule derivation that
  feeds the rule editor from S14), or the docs must be explicit that
  ask currently allow-by-default. Silently shipping ask-equals-allow
  is the worst of both worlds.
- Do not build a second rule editor for the Confirm prompter. The
  S14 rule editor component is the single source; the Confirm UI
  embeds it pre-filled from auto-derived rule output.
- Do not call a sprint done without explicit coverage ledger entries.
- Do not reintroduce SQLite reads on hot fan-out paths. The release
  branch removed them from `/list` in the OTel handoff (2026-05-15) and
  added a regression test that must stay green. After S12 lands, the
  contract tightens: `session.db` is read on the runtime data path
  exactly twice in a VM's life -- once at VM launch in `capsem-process`
  to seed the in-memory accumulator with cumulative totals for
  persistent VMs, and once via a cold one-shot read in `/info/{id}`
  when the requested VM's process is gone. No `/list`, no scrape
  endpoints, no gateway status path, and no running-VM `/info` opens
  `session.db`. Support-bundle and `inspect-session` tooling continue
  to read the durable store directly; that is intentional.

## Current Active Work

S06-pre is closed. Slices 6a-6e (Confirmer trait + placeholder, DNS,
HTTP, MCP, model `ask -> confirm()` wiring), the adversarial backfill
(redaction, bounds, concurrency, panic isolation, hang fail-closed
via shared `RetryOpts` backoff), and [slice 6f - exit tests](tracker.md#slice-6f---exit-tests)
(`confirm_with_backoff` contract tests, 200-way concurrent-load smoke,
resolved-outcome attribution fix in the HTTP / DNS / model telemetry
slots) have all landed. Carry-over from slice 6f:

- **Deferred:** capsem-doctor E2E ask probe per subsystem. Unblock
  condition is now explicit: the
  [S07 Rules API](S07-uds-service-api.md#rules-api) (list / get /
  add / remove / evaluate) + the
  [S15 resolve routes](S15-confirm-ux.md) (`GET /confirm/pending`
  + `POST /confirm/pending/{id}/{accept|deny}`), surfaced over the
  gateway by [S08](S08-http-gateway-api.md) and the CLI by
  [S09](S09-cli-integration.md). Python probe shape once that
  lands: `POST /rules` stages an ask rule, traffic from inside the
  VM matches it, `GET /confirm/pending` picks up the queued ask,
  `POST /confirm/pending/{id}/accept` resolves. The Rust-side
  functional attribution test at the hook boundary covers the
  contract until then.
- **Slice 7+:** `policy_confirm_events` /
  `policy_body_inspection_events` schemas, streaming body inspector,
  instant propagation.

Next sprint in the linear path is [S06 - Assembly and VM-effective
settings](S06-assembly-vm-effective-settings.md). Owned by this
branch + worktree (see
[tracker.md "Where this sprint lives"](tracker.md#where-this-sprint-lives)
for the canonical pinning); the foundational slice
(`resolve_effective_vm_settings()` + VM-effective persistence) is
already in HEAD, and the remaining tasks (parent-chain resolution,
corp directives, layered resolver pipeline, trace artifact) are
unstarted.

**Sequencing into S07 is locked.** After S06 / S06b close (and ideally
[S06c](tracker.md#s06c---ablate-legacy-networkpolicy-runtime-proposed)
ablates the legacy `NetworkPolicy` runtime), a focused
[Post-S06 cleanup milestone](tracker.md#post-s06-cleanup-milestone)
runs three steps **in this exact order**:

1. `git merge origin/main` -- closes the long-deferred merge debt
   from the parallel hardening sprint. Doing this first keeps the
   conflict surface mechanical (pre-rename identifiers on both
   sides).
2. V2 rename across the crate (modules, types, files, fields,
   tests). Now safe because V1 is no longer coexisting (or the
   `SharedPolicy` / `SharedPolicyConfig` disambiguation is the
   small worst case).
3. Full verification gate.

[S07 - UDS service API](S07-uds-service-api.md) starts on the
post-cleanup codebase. The rationale is that S07 introduces a
typed public API surface; it should be authored against the
final type names, not against names that get renamed under it.

**Merge with `origin/main` is NOT free-floating any more.** It is
the explicit first step of the Post-S06 cleanup milestone. Until S06
closes, `origin/main` does not get merged. The
conflict-resolution guidance in [tracker Active Notes](tracker.md#active-notes)
still applies when the merge runs: prefer main where it overlaps
with S12's intent (the `/list` SQL-on-hot-path hotfix and related
fixes), preserve the S06-pre confirmer plumbing and backoff
refactor.



S00 is complete. A first typed replacement model now exists in
`capsem-core::settings_profiles`: service settings, profile TOML, the built-in
Everyday Work profile, security capabilities, service-scoped telemetry/remote
policy settings, service-scoped asset/manifest/image locations, TOML
credentials, profile discovery, user profile CRUD/fork, service settings file
load/save, VM-effective settings with provenance and derived capability rules,
VM-effective settings persistence, Rust-owned descriptor metadata, and
debug-report settings/profile/asset summaries that redact credential values.
S03 now also wires service startup through typed service settings for asset
directory and manifest source resolution. `/setup/assets` and the debug report
report the resolved asset, manifest, and image source/provenance so custom
corporate locations are diagnosable. S06 runtime wiring now attaches
`vm-effective-settings.toml` to session directories during sandbox provisioning
and fork, preserving readable attachments and regenerating corrupt ones.
`capsem-process` runtime consumption is now cut over to session-attached
`vm-effective-settings.toml` for startup/reload policy assembly. Remaining v1
runtime callers are primarily deeper core policy-engine surfaces tracked in
S06-pre/S06/S06a/S06b. The S00-S06 accuracy audit is captured in
`sprints/policy-settings-profiles/S00-S06-audit-2026-05-14.md`.
S04 design has now been closed on 2026-05-14 after locking canonical v1 rule
format at `security.rules.<type>.<rule_name>` (priority default `1`) while
keeping capabilities + rules and explicit inheritance requirements. S06 has been
re-scoped as a resolver engine sprint that must deliver explicit inheritance,
corp restriction enforcement, and diff-style resolver traces before the runtime
cutover can be considered complete. The detailed S06 contract is in
`sprints/policy-settings-profiles/S06-resolver-engine-contract.md`.
Latest S05 parser/model checkpoint (2026-05-14) added
`extends_profile_id` parse validation, narrowed v1 profile types to
`everyday-work|coding`, changed default profile rule priority to `1`, and
migrated profile rule parsing to canonical
`security.rules.<type>.<rule_name>` tables with callback/type validation
(including profile-level `dns.query` rejection).
S06-pre is now an explicit prerequisite sprint for S06: it normalizes DNS/HTTP
rule callback+field contracts, wires `ask` through a shared `confirm()` path,
adds dedicated confirm telemetry storage, and enforces 5 MiB conditional
buffering caps for HTTP body-based rule evaluation.
S06a is now explicit as a companion sprint: implement `model.request` rewrite
for `request.body` and remove current unsupported rewrite deny behavior.
S06b is now explicit as a companion sprint: migrate legacy allowlist outputs
into canonical `security.rules` and mark generated rules as managed/uneditable
with source-setting labeling.

Latest focused verification:

- `cargo test -p capsem-core settings_profiles` passed with 51 focused tests.
- `cargo test -p capsem-service --lib debug_report::tests` passed with 5 tests.
- `cargo test -p capsem-service startup_` passed with startup manifest tests.
- `cargo test -p capsem-service
  handle_asset_status_reports_resolved_asset_location_sources` passed.
- `cargo test -p capsem-service ensure_vm_effective_settings_` passed with
  attach/regenerate coverage.
- `cargo test -p capsem-process` passed with 96 unit tests, including
  vm-effective runtime policy conversion/reload behavior.
- `cargo test -p capsem-service handle_` passed with 22 focused service
  handler tests, including `/settings*` typed cutover coverage.
- `cargo test -p capsem-service` passed outside the sandbox on 2026-05-14 with
  95 lib tests and 113 service-bin tests.
- `CAPSEM_ASSETS_DIR=/Users/elie/git/capsem/assets uv run python -m pytest
  tests/capsem-service/test_svc_service_settings_runtime.py -v --tb=short`
  passed with real service, real gateway, malformed-settings startup, and VM
  boot/exec coverage for service.toml-owned assets.
- `CAPSEM_ASSETS_DIR=/Users/elie/git/capsem/assets uv run python -m pytest
  tests/capsem-service/test_svc_setup.py::TestSetupAssets
  tests/capsem-service/test_svc_service_settings_runtime.py -v --tb=short`
  passed with 5 targeted service setup/runtime tests.

S01 closed on 2026-05-14. Service/process runtime paths no longer depend on
v1 settings-policy loaders for `/settings`, `/mcp`, VM defaults, or process
reload assembly. `/settings` now emits strict `settings_profiles_v2` payload
fields only (`settings_profiles`, `profile_presets`, `effective_rules`), setup
corp provisioning accepts canonical profile TOML (legacy corp settings shape
rejected fail-closed), and frontend settings API/model now normalize strict
payloads without backend dependence on legacy tree fields.
First S01 execution checkpoint landed on 2026-05-14: `capsem-service`
provision/run VM defaults no longer read
`net::policy_config::load_merged_vm_settings()` and now resolve from typed
`settings_profiles` effective profile VM settings.
Second S01 service checkpoint landed on 2026-05-14: `/mcp/servers` and
`/mcp/policy` now resolve from typed effective profile state (plus runtime MCP
tool cache) and no longer read merged v1 user/corp settings files.
Third S01 process/runtime checkpoint landed on 2026-05-14: `capsem-process`
startup plus `ReloadConfig`/`McpRefreshTools` no longer read
`net::policy_config::load_settings_files()` or `MergedPolicies`; runtime
policies now derive from session-attached `vm-effective-settings.toml`.
Fourth S01 settings checkpoint landed on 2026-05-14: service `/settings*`
handlers no longer use v1 settings-tree/preset/lint loaders and now read/write
typed `settings_profiles` state (including profile-backed policy rule updates).
Fifth S01 settings contract checkpoint landed on 2026-05-14: `/settings` no
longer emits legacy compatibility keys (`tree`, `issues`, `presets`,
`policy`) and now returns only typed payload fields:
`settings_profiles`, `profile_presets`, and `effective_rules`.
