# Policy, Settings, Profiles Master

Last updated: 2026-05-18

## Where this sprint lives

**Single branch, single worktree.** Authoritative pinning is in
[tracker.md "Where this sprint lives"](tracker.md#where-this-sprint-lives);
the short version:

- Branch: `profile-v2`
- Worktree: `/Users/elie/.codex/worktrees/824d/capsem`
- Verify with `git worktree list` + `git log <branch> --oneline | head`
  before believing any "in flight elsewhere" claim.

## Mission

Replace Capsem's v1 settings/policy stack with typed service settings and
VM/session profiles. Profiles become the only user-facing "security level"
concept and the product unit for guest package/tool assumptions plus VM asset
requirements. The old ad hoc settings registry, standalone `[mcp]` authority,
and hand-authored `config/defaults.json` runtime/UI source are removed
completely.

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
- **The signed manifest is the profile catalog.** The binary owns the manifest
  signing trust root; the manifest lists profile ids, immutable revisions,
  lifecycle status, payload locations, payload hashes/signatures, and binary
  compatibility. Profiles then declare package/tool contracts and the VM assets
  needed to satisfy them.
- **VMs pin profile revision and assets.** Creating a VM resolves a profile
  revision, downloads/verifies that revision's assets on first use, and pins the
  profile id/revision plus exact asset hashes in the VM registry/session state.
  Profile updates do not silently mutate existing VMs.
- **Admin tooling derives images from profiles.** Corp/admin image and manifest
  workflows use the released `capsem-admin` Python CLI. Profiles are the source
  of truth for package/tool contracts and image build plans; hand-edited image
  settings are not a compatibility surface.
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
| 8 | [S06 - Assembly And VM-Effective Settings](S06-assembly-vm-effective-settings.md) | Done | Resolve profiles/corp governance into VM-attached settings and derived rules. Parent-chain validation, layered merge, resolver trace, corp directives, lock/forbid, runtime cutover, and status/debug exposure have landed; in-VM probe remains visible debt. |
| 9 | [S06a - Model Request Rewrite Support](S06a-model-request-rewrite-support.md) | Done | Implement `model.request` rewrite for `request.data` and remove unsupported fail-closed placeholder behavior. |
| 10 | [S06b - Legacy Allowlist Migration And Rule Ownership Locks](S06b-legacy-allowlist-migration-and-rule-ownership.md) | Done | Delete legacy allowlist/v1 settings dead code and enforce generated-rule ownership (`managed by <setting>`, uneditable). |
| 11 | [S06c - Ablate Legacy NetworkPolicy Runtime](tracker.md#s06c---ablate-legacy-networkpolicy-runtime) | Not Started | Delete `policy.rs` + `policy_hook.rs`; remove the V1 hook from production pipeline; collapse `SharedPolicyV2` -> `SharedPolicy`. Closes the V1 runtime that S01 left behind. |
| 12 | [Post-S06 cleanup milestone](tracker.md#post-s06-cleanup-milestone) | Deferred cleanup debt | `git merge origin/main` -> v2 rename -> full verification gate. Current branch has already advanced into S07; keep the debt visible before release. |
| 13 | [S07 - UDS Service API](S07-uds-service-api.md) | In Progress | Metrics IPC foundation, profile list/get/resolve, profile create/fork/update/delete, and rules list/get/evaluate have landed. Rules create/delete, confirm listing, skills, profile-backed VM create, and full route proof remain open. |
| 14 | [S07a - Profile Manifest, Packages, And Assets](S07a-profile-manifest-assets.md) | Not Started | Make the signed manifest a profile catalog; add profile package/tool contracts, profile-owned asset declarations, first-use download, retention, and VM profile/revision/asset pins. |
| 15 | [S07b - Capsem Admin Tooling And Profile-Derived Images](S07b-capsem-admin-tooling.md) | Not Started | Ship `capsem-admin` Python admin tooling for profile creation, profile-derived image builds, image verification, and manifest generate/check/sign. |
| 16 | [S08 - HTTP Gateway API](S08-http-gateway-api.md) | Not Started | Wire HTTP endpoints to UDS behavior, including profile catalog/revision and profile-backed VM create/readiness. |
| 17 | [S09 - CLI Integration](S09-cli-integration.md) | Not Started | Add `profile`, `mcp`, `skills`, `confirm`, and profile-backed VM create CLI flows. |
| 18 | [S10 - Credential Brokerage](S10-credential-brokerage.md) | Not Started | Define credential release from service settings into sessions. |
| 19 | [S11 - Status, Debug, Provenance](S11-status-debug-provenance.md) | Not Started | Make status/debug explain active settings, profiles, derived rules, MCP, skills, profile catalog state, package contracts, asset readiness, and VM pins. |
| 20 | [S12 - OpenTelemetry Metrics Architecture](S12-observability-plugin.md) | Not Started | Typed per-VM live-metrics architecture: `capsem-proto::metrics`, process-side accumulator, bincode IPC snapshot, service `/metrics/json` + `/metrics`, gateway proxy, UI typed-JSON. Inherits the release-team OTel handoff. |
| 21 | [S13 - Remote Policy Plugin](S13-remote-policy-plugin.md) | Not Started | Add service plugin for remote policy events/decisions. |
| 22 | [S14 - Rules UI Components](S14-rules-ui-components.md) | Not Started | One reusable rule editor/renderer + per-type rule blocks (DNS/HTTP/Model/MCP). **The editor is embedded by [S15](S15-confirm-ux.md) for forward-rule decisions -- design it for embedding from the start, pre-fillable from derived rule input.** |
| 23 | [S15 - Confirm UX (Ask)](S15-confirm-ux.md) | Not Started | Production answer path for `decision = "ask"`: stacked pending-ask queue, UI prompter embedding the S14 rule editor, CLI parity, auto-rule derivation per callback, `policy_confirm_events` integration. Replaces the placeholder confirmer that ships from S06-pre. |
| 24 | [S16 - Profile UI](S16-profile-ui.md) | Not Started | First-class profile catalog, selector, revision, package/asset readiness, create/fork/delete/edit, and VM create flows. |
| 25 | [S17 - Security Capabilities UI](S17-security-capabilities-ui.md) | Not Started | Capability controls above canonical rule editing. |
| 26 | [S19 - Documentation And Site](S19-documentation-and-site.md) | Not Started | Document the engine, corporate deployment, telemetry, remote policy, signed profile catalogs, package contracts, profile-owned VM assets, and `capsem-admin` workflows. |
| 27 | [S18 - Full Verification And Release Gate](S18-full-verification-release-gate.md) | Not Started | Backend/UI/E2E/install proof. Last sprint. |

S15 was previously a "Settings UI Redesign" sprint; that scope is now
folded into the descriptor-driven UI work in S14 / S16 / S17.

## Release Holds

- Do not edit runtime code for this redesign without keeping this board and
  `tracker.md` synchronized.
- Do not preserve old config semantics or fallback behavior.
- Do not ship a backend surface without debug report/status coverage for wrong
  settings and profile resolution.
- Do not wire UI before UDS, HTTP, and CLI contracts are tested.
- Do not lift profile create/VM create to HTTP/UI until S07a defines the signed
  profile catalog, profile package/tool contract, profile-owned asset
  declarations, first-use asset download, and VM profile/revision/asset pinning.
- Do not document or ship corp-admin profile/image/manifest workflows through
  raw Python scripts or hand-edited image settings. S07b must make
  `capsem-admin` the released, bootstrap-installed admin CLI.
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

Current execution is in [S07 - UDS service API](S07-uds-service-api.md).
Before the gateway/UI layers harden the public contract, insert
[S07a - Profile Manifest, Packages, And Assets](S07a-profile-manifest-assets.md).
S07a makes the signed manifest the profile catalog, extends profiles with the
closed JSON Schema Draft 2020-12 `capsem.profile.v2` contract, guest
package/tool contract, and per-arch asset declarations, and makes VM create pin
the selected profile revision plus verified asset hashes.

Immediately after S07a, [S07b - Capsem Admin Tooling And Profile-Derived Images](S07b-capsem-admin-tooling.md)
turns those contracts into operator tooling: `capsem-admin` creates/validates
profiles, exports/validates the shared schema artifact, derives image build
plans from profiles, verifies built images, and generates/checks/signs
manifests.

[S07 - UDS service API](S07-uds-service-api.md), S07a, and S07b are the
public-contract foundation for every later layer. HTTP, CLI, UI, docs, and
release tooling must consume those shapes rather than inventing independent
profile/asset/admin semantics.

**Post-S06 cleanup debt remains visible.** The historical cleanup order is
still tracked in [tracker.md](tracker.md#post-s06-cleanup-milestone): merge
`origin/main`, perform the V2 rename, then run the full verification gate. The
current branch has already advanced into S07, so do not let that cleanup debt
disappear before release.



S00 is complete. A first typed replacement model now exists in
`capsem-core::settings_profiles`: service settings, profile TOML, the built-in
Everyday Work profile, security capabilities, service-scoped telemetry/remote
policy settings, pre-S07a service-scoped asset/manifest/image locations, TOML
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
