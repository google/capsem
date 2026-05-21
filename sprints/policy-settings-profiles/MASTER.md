# Policy, Settings, Profiles Master

Last updated: 2026-05-20

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

## Execution Mode

**Rescue complete; push phase active.** As of 2026-05-19, the profile-v2 branch
is coherent again and is expected to sit `92 ahead / 0 behind` `origin/main` in this
worktree after the S07d/S08a regroup planning commit. The tracker is now a push
board:

- Keep S07a as the active contract sprint until profile catalog install/update,
  mandatory VM profile/revision/package pins, retention, forward-only
  resume/create/fork/persist enforcement, and VM list/status profile-state
  reporting are landed and tested.
- Do not start S07b implementation until S07d gives service settings the same
  formal schema/Pydantic/admin-validation footing as Profile V2 payloads.
- Do not resume CLI/UI/telemetry/plugin lift work until S08a settles the
  policy-rule versus detection-rule abstraction.

**Winter readiness.** The wall is the release gate. Nothing crosses it unless
the profile trust chain is signed, profile payloads are installed from the
catalog, VMs pin exact profile/revision/package/asset identity, old config stays
dead, and every public surface can explain what happened.

**Latest verification.** `just smoke` passed on 2026-05-20 in 272s after the
smoke harness was made less host-starved: service/CLI and MCP VM-heavy suites
no longer overlap, log fds are closed in parent fixtures, e2e services receive
Profile V2 asset homes, and MCP tests now separate signed VM lifecycle fixtures
from editable unsigned profile-mutation fixtures.

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
- **Policy rules and detection rules are under review.** Runtime blocking rules
  currently remain Capsem-native synchronous policy. S08a decides the real CEL
  enforcement language, the real Sigma-compatible detection path, and whether
  detection rules become a separate signed/profile-scoped family before
  telemetry, plugins, rule UI, and Confirm UX freeze the model.
- **Activity engines must be separated.** Network transport, file/snapshot
  mechanics, process/audit mechanics, security decisions, and resolved-event
  emission are separate engines. S08b creates those boundaries so
  network/file/process code parses and applies typed responses, while the
  Security Engine owns policy, ask/confirm, detection, postprocessing, and the
  complete resolved-event journal.
- **Session DB must become a resolved-event store.** Existing domain tables are
  useful read models, but S08b must add a canonical resolved-event journal and
  route migrated event-family writes through the emitter instead of direct
  subsystem SQLite writes.
- **Everyday work needs a unified structured timeline.** SDK-backed
  Codex/Claude work and terminal fallback workflows must feed a first-class
  Conversation Engine and the single `/timeline/{id}` read model so sessions
  can be reviewed, searched, grouped, and explained without spelunking raw
  tables or PTY logs.
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
| 11 | [S06c - Ablate Legacy NetworkPolicy Runtime](S06c-ablate-legacy-networkpolicy.md) | Done | Deleted `policy.rs` + `policy_hook.rs`; removed the V1 hook from production pipeline; collapsed DNS/MITM/runtime policy authority to Profile V2 `PolicyConfig`. |
| 12 | [S06d - Core Structure And Test Boundaries](S06d-core-structure-and-test-boundaries.md) | Done | Split oversized MITM/DNS modules and tests inside `capsem-core` before the rename and S08b engine contracts; defer new crate boundaries to S08b. |
| 13 | [Post-S06 cleanup milestone](tracker.md#post-s06-cleanup-milestone) | In Progress | Branch is `92 ahead / 0 behind` `origin/main`; code/docs now use the singular `policy` runtime name, with focused cargo gates passing. Remaining: decide/run heavyweight `just test`/doctor release gate before marking closed. |
| 14 | [S07 - UDS Service API](S07-uds-service-api.md) | Done | Metrics IPC foundation, profile list/get/resolve, profile create/fork/update/delete, profile-backed VM create request shape, standard `mcpServers` profile format plus Profile V2 MCP server list/create/delete across service/CLI/capsem-mcp, old MCP management API/IPC removal, rules list/get/create/delete/evaluate, typed `GET /confirm/pending`, Profile V2 skills list/create/delete, and chained S07 route proof have landed. HTTP, CLI, production confirm resolution, and UI lift remain in S08/S09/S15/S16. |
| 15 | [S07a - Profile Manifest, Packages, And Assets](S07a-profile-manifest-assets.md) | In Progress | Canonical profile catalog/status parser, typed profile package/tool contracts, per-arch VM asset declarations, Draft 2020-12 schema + Rust validation, Python Pydantic v2 profile/manifest models, profile-driven service asset resolution/download, profile-aware cleanup caller, complete installed-payload trust checks, signed revision/payload-hash/asset VM pins, forward-only resume/create-from-source/fork/persist pin enforcement, VM list/status profile-state reporting, first-use selected-profile asset reconciliation, file/HTTPS catalog reconcile sources, and scheduled `[profile_catalog]` service reconciliation have landed; old asset-manifest service settings/setup/runtime authority are removed. Remaining scope adds richer catalog clients/debug detail. |
| 16 | [S07c - Profile Asset Update Orchestration](S07c-profile-asset-update-orchestration.md) | Done | Manual service asset reconcile endpoint, `capsem update --assets` service trigger, status checked-at/profile/payload/per-asset provenance propagation, structured check/download logs, service debug Profile V2 asset-health reporting, old Rust asset-manifest parser/loader/downloader removal, duplicate-download/active-cleanup race proof, first-use VM create reconciliation, profile-pin asset authority for source/fork/persist, chained service-level reconcile/status/debug/log proof, formal `file://` asset reconciliation, explicit UDS socket selection, and a live real-VM boot/exec proof from freshly reconciled profile assets have landed. |
| 17 | [S07d - Service Settings Schema And Admin Contract](S07d-service-settings-schema-admin-contract.md) | Done | Pydantic v2 `ServiceSettingsV2`, Pydantic-only JSON/TOML helpers, committed Draft 2020-12 schema artifact, valid/invalid fixtures, Rust/Python fixture parity, `capsem-admin settings init|schema|validate|doctor`, cross-runtime defaults drift proof, and closeout docs have landed. |
| 18 | [S07b - Capsem Admin Tooling And Profile-Derived Images](S07b-capsem-admin-tooling.md) | In Progress | Profile-admin validation/schema, required Profile V2 `ui`, `profile init`, guest-config-derived `profile init-builtins` generated `everyday-work`/`coding` base profiles, `settings init`, typed section editability gates, `capsem-admin image plan`, profile-derived `image build-workspace`, public `image build` routing, profile-required local/release asset build recipes, rootfs-generated package/tool inventory, local asset plus per-arch inventory-backed `image verify`, typed guest SPDX SBOM generation, typed doctor-bundle probe ingestion, profile-backed release-image boot gate, `manifest generate`, fast/download `manifest check`, minisign manifest signing/verification, profile/asset signature verification, developer bootstrap proof, OS package layout proof for the `capsem-admin` wrapper/Python payload, and typed policy/detection pack validate/schema commands have landed. Remaining: detection compile/check with pySigma/Rust parity fixtures and docs/release proof. |
| 19 | [S08 - HTTP Gateway API](S08-http-gateway-api.md) | In Progress | Profile V2 gateway contract slices landed: catalog/revision, profile CRUD/resolve, skills, standard MCP servers, rules/evaluate, confirm-pending read, profile-selected VM create response payloads, `/status` and `/setup/assets` profile asset provenance/progress, `/debug/report` profile provenance, exact typed-error passthrough, debug-report gateway runtime mismatch diagnostics, live selected-profile HTTP create/download/boot/exec with `/info` pin echo, and adversarial typed-error passthrough for malformed, locked, invalid, updating, and revoked Profile V2 cases. Remaining: S15 confirm resolution/stream. |
| 20 | [S08a - Rule Abstraction And Detection Architecture](S08a-rule-abstraction-detection-architecture.md) | Done | Policy and detection are separate profile-owned rule families; enforcement uses real CEL via the Rust `cel` crate family; Sigma is a detection authoring/import format, not a blocking language; `capsem.policy-pack.v1`, `capsem.detection-pack.v1`, `capsem.detection.ir.v1`, normalized event taxonomy, typed finding shape, admin validate/schema/compile/check commands, implementation ordering, testing matrix, and downstream S07b/S08b/S12/S13/S14/S15/S16a/S19 deltas are locked. |
| 21 | [S08b - Security Event Engine, Network Engine, File Engine, And Process Engine](S08b-security-event-engine-and-file-engine.md) | Not Started | Split Network Engine, File Engine, Process Engine, Security Engine, and Resolved Event Emitter into clear contracts/crates before public surfaces consume the event model; consume S08a's locked policy/detection contracts; reshape `session.db` around a canonical resolved-event journal plus projections. |
| 22 | [S09 - CLI Integration](S09-cli-integration.md) | Not Started | Add `profile`, `mcp`, `skills`, `confirm`, settings validation, and profile-backed VM create CLI flows. |
| 23 | [S10 - Credential Brokerage](S10-credential-brokerage.md) | Not Started | Define credential release from service settings into sessions. |
| 24 | [S11 - Status, Debug, Provenance](S11-status-debug-provenance.md) | Not Started | Make status/debug explain active settings, profiles, derived rules, MCP, skills, profile catalog state, package contracts, asset readiness, VM pins, live VM health metrics, and policy/detection/engine provenance after S08a/S08b/S12. |
| 25 | [S12 - OpenTelemetry Metrics Architecture](S12-observability-plugin.md) | Not Started | Typed per-VM live-metrics architecture: `capsem-proto::metrics`, process-side accumulator, bincode IPC snapshot, VM status health, model/provider/token/cost counters, detection finding metrics, service `/metrics/json` + `/metrics`, gateway proxy, UI typed-JSON. Inherits the release-team OTel handoff and depends on S08a's event/finding taxonomy plus S08b's emitter contract. |
| 26 | [S13 - Remote Policy Plugin](S13-remote-policy-plugin.md) | Not Started | Add service plugin for remote policy events/decisions after S08a separates event streaming from synchronous decision authority and S08b defines the engine/emitter boundary. |
| 27 | [S14 - Rules UI Components](S14-rules-ui-components.md) | Not Started | One reusable policy-rule editor/renderer plus detection-rule/finding UX decision from S08a and resolved-event provenance from S08b. **The policy editor is embedded by [S15](S15-confirm-ux.md) for forward-rule decisions.** |
| 28 | [S15 - Confirm UX (Ask)](S15-confirm-ux.md) | Not Started | Production answer path for `decision = "ask"` inside the Security Engine: stacked pending-ask queue, UI prompter embedding the S14 policy-rule editor, CLI parity, auto-rule derivation per callback, confirm event integration. Replaces the placeholder confirmer that ships from S06-pre. |
| 29 | [S16 - Profile UI](S16-profile-ui.md) | Not Started | First-class profile catalog, selector, revision, package/asset readiness, create/fork/delete/edit, and VM create flows. |
| 30 | [S16a - Unified Timeline And Agent Workbench](S16a-unified-timeline-and-agent-workbench.md) | Not Started | Friendly everyday-work UI for Codex/Claude SDK-backed and terminal-fallback sessions, backed by S08b's structured `/timeline/{id}` API and resolved-event store. |
| 31 | [S17 - Security Capabilities UI](S17-security-capabilities-ui.md) | Not Started | Capability controls above canonical policy-rule editing and any S08a-approved detection/finding views. |
| 32 | [S19 - Documentation And Site](S19-documentation-and-site.md) | Not Started | Document the engine, corporate deployment, telemetry/VM health, remote policy, signed profile catalogs, package contracts, profile-owned VM assets, settings schema, enforcement page, detection-format page, unified timeline, and corp/admin `capsem-admin` workflows. |
| 33 | [S19a - Marketing Site Refresh](S19a-marketing-site-refresh.md) | Not Started | Refresh the landing page around four pillars: Ship Fast With AI, Ship Safely, Scale Your Productivity Without Drag, and Enterprise Ready, with claims aligned to the new profile/security/observability architecture. |
| 34 | [S18 - Full Verification And Release Gate](S18-full-verification-release-gate.md) | Not Started | Backend/UI/E2E/install proof. Last sprint. |

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
- Do not build `capsem-admin` on raw service settings. S07d must give service
  settings a formal JSON Schema, Pydantic v2 models, Rust/Python fixture parity,
  and admin validation commands first.
- Do not build rules UI, Confirm promotion, OTel detection/finding metrics, or
  remote policy plugin event contracts until S08a decides the policy-rule versus
  detection-rule abstraction, the real CEL runtime, the real Sigma-compatible
  detection path, and how policy/detection packs live in signed profiles.
- Do not build CLI/UI/telemetry/plugin contracts directly against today's mixed
  HTTP/DNS/MCP/file telemetry paths. S08b must split the Network Engine, File
  Engine, Process Engine, Security Engine, and Resolved Event Emitter first,
  with file/snapshot/process activity represented as normalized security events.
- Do not add more independent `session.db` tables as security authority. S08b
  must define the canonical resolved-event journal and decide which existing
  domain tables remain projections/read models.
- Do not build the everyday-work UI directly against raw `pty.log`, `/inspect`
  SQL, or legacy telemetry tables. S16a consumes S08b's structured
  `/timeline/{id}` API, with cursor pagination over typed timeline blocks.
  Conversation, turn, process, activity, trace, finding, and artifact views are
  client-side filtering/formatting modes over those blocks. Raw transcript is
  only a forensic artifact/fallback input.
- Do not call profile asset updates production-ready until S07c proves debug
  provenance plus duplicate-download and cleanup/create concurrency behavior
  around the Profile V2 service asset reconciler.
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

Current execution is regrouping after the S08 gateway slices. S08 now mirrors
the S07/S07a/S07c Profile V2 service contracts through the authenticated local
HTTP gateway, proves live selected-profile create/download/boot/exec, and
preserves adversarial typed-error status/body responses for malformed, locked,
invalid, updating, and revoked Profile V2 cases.

The next inserted foundation sprint is
[S07d - Service Settings Schema And Admin Contract](S07d-service-settings-schema-admin-contract.md).
It exists because profiles now have a stronger formal contract than service
settings. `capsem-admin` now has its first typed settings boundary:
schema export, TOML/JSON validation, and doctor reports all flow through the
Service Settings V2 Pydantic model. The Python defaults now follow Rust's
`CAPSEM_HOME` / `$HOME/.capsem` profile-root contract, with a committed
defaults fixture checked by both runtimes. S07d is closed; S07b can now expand
the admin package.

[S08a - Rule Abstraction And Detection Architecture](S08a-rule-abstraction-detection-architecture.md)
is the next architecture discussion gate. It decides how Capsem-native
synchronous policy rules relate to detection/Sigma-compatible rules before
logging, telemetry, plugins, rule UI, and Confirm UX harden around that model.

[S08b - Security Event Engine, Network Engine, File Engine, And Process Engine](S08b-security-event-engine-and-file-engine.md)
is the implementation architecture gate immediately after S08a. It turns the
rule/detection decision into real crate/module boundaries: Network Engine for
transport, File Engine for file/snapshot mechanics, Process Engine for
process/audit mechanics and attribution, Conversation Engine for SDK/terminal
conversation capture and timeline normalization, Security Engine for
preprocessors, enforcement, ask/confirm, detection and postprocessing, and a
Resolved Event Emitter for telemetry/audit/logging/detection export.

S07b remains a release-blocking admin-tooling sprint. It now consumes both the
Profile V2 contract and the S07d service-settings contract. Once S08a chooses
the final enforcement/detection formats, `capsem-admin` must also validate,
schema-export, and report policy/detection packs through typed Pydantic models
before S19 documents those corp workflows as supported.

S07a/S07c foundation carried into S08:

- Canonical signed profile catalog parser/model (`ProfileManifest`, format
  `1`) with `active|deprecated|revoked` lifecycle status.
- Closed Profile V2 JSON Schema Draft 2020-12 artifact plus Rust schema
  validation helpers and Pydantic v2 admin models.
- Typed package/tool contracts and per-arch VM asset declarations in profile
  TOML, resolver merge, VM-effective serialization, and tests.
- Profile-driven service asset readiness/download. Service startup resolves VM
  assets from the selected profile, `capsem-process` verifies profile-provided
  expected hashes, and old asset-only manifests are not runtime authority.
- Legacy `assets.manifest.*` service settings and setup-time signed asset
  manifest checks are removed.
- Durable session telemetry identity. `session.db` records `vm_id`,
  `profile_id`, and `user_id`; service passes those facts to
  `capsem-process`; process/aggregator logs include them; `/info` surfaces the
  stored identity.
- VM profile pins. Running and persistent VM metadata now carries resolved
  `profile_id`, signed `profile_revision`, profile payload hash,
  package-contract hash, and pinned boot asset hashes; fork/persist/list/info
  preserve and expose that pin.
- Core profile payload install guard. Catalog-selected revisions now verify
  active status, BLAKE3 payload hash, Profile V2 schema validity, and
  manifest/payload id+revision parity before an install/update path can write
  the payload.
- Verified profile payload materialization. Profile V2 payloads now convert
  into the runtime resolver profile shape, materialize into the corp profile
  root, and preserve the exact verified payload under the installed revision
  catalog path.
- Installed revision sidecar. Materialization now writes
  `.catalog/profiles/<id>/current.json` with profile id, revision, and payload
  hash for status/debug and mandatory VM revision pinning.
- Installed payload identity pins. VM pin construction now reads the installed
  profile revision sidecar, records the installed profile payload hash, and
  rejects create/inherit paths that lack that signed payload proof.
- Core profile catalog reconciler. A typed core API now installs/updates
  complete `active` revisions, re-installs incomplete active state, keeps
  installed `deprecated` revisions for existing VMs, and removes the launchable
  profile plus current state for installed `revoked` revisions.
- Service profile catalog reconcile route. `POST /profiles/catalog/reconcile`
  applies the lifecycle reconciler through the service UDS surface and returns
  typed per-revision outcomes plus summary counts. The gateway fallback exposes
  the same route to authenticated local HTTP callers.
- Native profile catalog reconcile CLI. `capsem profile reconcile-catalog
  --manifest <path> --pubkey <path> [--json]` now calls the service reconciler
  and renders either a compact install/deprecate/revoke summary or raw JSON.
  It also accepts `--manifest-url <https-url>` for remote signed catalog
  sources, with cleartext HTTP restricted to loopback development/test hosts.
- Read-only profile catalog status. `GET /profiles/catalog` and `capsem
  profile catalog [--json]` expose configured catalog source state, persisted
  manifest presence, profile ids, current/installed revisions, installed
  payload hashes, and canonical revision lifecycle status.
- Per-profile revision inspection. `GET /profiles/{id}/revisions` and `capsem
  profile revisions <id> [--json]` expose current/installed revision markers,
  installed payload hash, and canonical lifecycle status for one catalog
  profile, with missing manifests/unknown profiles failing as absence errors.
- Per-profile revision lifecycle actions. `POST
  /profiles/{id}/revisions/{install,update,remove}` and `capsem profile
  install|update|remove <id> [--revision <rev>] [--json]` install only active
  signed revisions, reconcile lifecycle updates, clean revoked installed
  revisions, and remove local launchable state while preserving archived
  payload material.
- Absent installed profile cleanup. Catalog reconciliation now removes
  launchable current state for installed profile ids missing from the signed
  manifest and reports `absent_removed`, while preserving archived payloads for
  the retention/VM-pin cleanup slice.
- Profile-aware asset retention sources. Cleanup can now derive preservation
  filenames from installed current profile payloads and persistent VM profile
  pins before deleting hash-named assets.
- Profile-aware production asset cleanup. `POST /setup/assets/cleanup` now
  runs a manifest-free cleanup path through installed-profile and saved-VM
  retention, removes stale hash-named files plus legacy `v1.0.*` directories,
  preserves metadata/temp files, and refuses cleanup while assets are checking
  or updating.
- Forward-only persistent VM resume. Resume now requires a profile pin and
  pinned asset identity before process spawn; unpinned registry entries no
  longer fall back to the current catalog/default assets.
- Forward-only VM creation boundaries. Profile pin construction now requires a
  signed catalog revision, profile payload hash, and pinned asset identity, and
  create-from-source, fork, and persist fail closed before cloning/moving
  durable state when the source/running VM lacks that full pin.
- Fork profile integrity. Fork cloning now preserves the VM-effective profile
  settings/trace attachments, verifies the forked pin still matches the source
  VM's profile id/revision/payload-hash/package/assets, and has service
  coverage that the fork can still execute through IPC with the same profile
  identity.
- VM list/status profile state. `/list`, `/info`, `capsem list`, and `capsem
  info` now expose each VM's profile id/revision plus `current`,
  `needs_update`, `deprecated`, `revoked`, `corrupted`, or `unknown` based on
  the persisted profile catalog snapshot and installed current revision
  sidecar.
- Profile payload signature verification. The profile catalog path now has a
  profile-specific minisign verification wrapper with tamper coverage, reusing
  the existing Capsem signature verifier.
- Installable profile payload fetch. Catalog payload/signature locations are
  read together, signature is verified before parsing, then hash/schema/id/
  revision checks produce the verified payload for materialization.

Remaining S07a push order:

1. Catalog-driven profile payload install/update/delete/revoke from manifest
   records, including `deprecated` and `revoked` fail-closed semantics.
   Core verification/fetch/materialization/signature primitives and the typed
   lifecycle reconciler have landed; service UDS/gateway reconciliation has
   landed for current active revisions plus deprecated/revoked local-state
   handling, and the first native CLI hook can apply a catalog file or bounded
   HTTPS catalog URL through the service. Typed `[profile_catalog]` service
   settings now persist the catalog URL, profile payload public key, and check
   interval; service startup schedules the same reconcile path and logs summary
   counts. `GET /profiles/catalog`, `GET /profiles/{id}/revisions`, `capsem
   profile catalog [--json]`, and `capsem profile revisions <id> [--json]`
   expose source, manifest, current/installed revision, and lifecycle status.
   Revision install/update/remove actions now exist in both service and CLI.
   Absent installed profile ids now lose launchable state during reconcile. UI
   clients remain.
2. Persistent VM `profile_id`, `profile_revision`, profile payload hash,
   package contract hash, and pinned asset metadata. Landed for
   runtime/registry/API with installed revision/payload-hash capture; profile
   pin construction now requires a signed catalog revision, profile payload
   hash, and pinned asset identity on every create/inherit path.
3. Retention and cleanup that preserve active/deprecated installed revisions,
   in-progress downloads, and existing VM pins. Retention filename extraction
   has landed for installed current profile payloads and persistent VM profile
   pins; `POST /setup/assets/cleanup` now uses that retention set and fails
   closed while assets are checking/updating. Duplicate manual reconcile and
   active-cleanup races are covered; first-use VM create now uses the same
   reconciler before spawn. Remaining: cross-process/per-asset download locks.
4. Forward-only VM identity enforcement on every create/fork/persist/resume
   path. Resume now rejects registry entries without profile pins or pinned
   asset identity. Create-from-source, fork, and persist now reject
   missing/revisionless pins or missing profile payload hashes before asset
   resolution, clone, or session move work. Fork now preserves VM-effective
   profile attachments and rejects profile or payload-hash drift before
   registry state is created. First-use selected-profile create now validates
   the selected installed revision, rejects missing/hash-drifted archived
   payloads, downloads missing profile assets, and attaches the selected
   VM-effective profile before process spawn.
5. Status/debug readiness for profile catalog state, installed revisions,
   package contracts, asset verification, VM pins, and drift/revocation.

Immediately after S07a, [S07c - Profile Asset Update Orchestration](S07c-profile-asset-update-orchestration.md)
turned the asset pieces into a production operator workflow: background checks,
manual `capsem update --assets`, status/debug provenance, structured download
logs, cleanup/create concurrency, and live boot proof all use the same Profile
V2 asset authority. After S07c, [S07d - Service Settings Schema And Admin Contract](S07d-service-settings-schema-admin-contract.md)
brings service settings up to the same schema/Pydantic/admin-validation level
as profiles. After S07d, [S07b - Capsem Admin Tooling And Profile-Derived Images](S07b-capsem-admin-tooling.md)
turns those contracts into operator tooling: `capsem-admin` creates/validates
settings and profiles, exports/validates the shared schema artifacts, derives
image build plans from profiles, verifies built images, and generates/checks/
signs manifests. Python admin internals use Pydantic v2 models for those data
shapes, with JSON entering through Pydantic validation and leaving through
Pydantic dumping, not raw nested dicts.

[S07 - UDS service API](S07-uds-service-api.md), S07a, S07c, S07d, S07b, and
[S08a](S08a-rule-abstraction-detection-architecture.md) are the
public-contract foundation for every later layer. HTTP, CLI, UI, docs,
marketing, telemetry, plugins, and release tooling must consume those shapes
rather than inventing independent profile/settings/rule/admin semantics.

**Deferred cleanup debt remains visible.** S06c legacy NetworkPolicy ablation
and the final V2 naming collapse are still tracked in
[tracker.md](tracker.md#s06c---ablate-legacy-networkpolicy-runtime) and
[tracker.md](tracker.md#post-s06-cleanup-milestone). They are not blockers for
the immediate S07a push, but they remain release blockers.

Historical S00-S06 rescue context: a first typed replacement model now exists in
`capsem-core::settings_profiles`: service settings, profile TOML, the built-in
Everyday Work profile, security capabilities, service-scoped telemetry/remote
policy settings, service-scoped asset/image locations, TOML
credentials, profile discovery, user profile CRUD/fork, service settings file
load/save, VM-effective settings with provenance and derived capability rules,
VM-effective settings persistence, Rust-owned descriptor metadata, and
debug-report settings/profile summaries that redact credential values.
S03 wired service startup through typed service settings for asset/image
location resolution; S07a later removed old asset manifest authority and made
profile payloads own VM asset declarations. S06 runtime wiring now attaches
`vm-effective-settings.toml` to session directories during sandbox provisioning
and fork, preserving readable attachments and regenerating corrupt ones.
`capsem-process` runtime consumption is now cut over to session-attached
`vm-effective-settings.toml` for startup/reload policy assembly. Remaining v1
runtime callers are primarily deeper core policy-engine surfaces tracked in
S06c. The S00-S06 accuracy audit is captured in
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

Latest focused verification after the rescue/push transition:

- `cargo test -p capsem-logger` passed with 100 unit tests + 126 roundtrip
  tests.
- `cargo test -p capsem-service` passed with 107 library tests + 140 service
  tests.
- `cargo test -p capsem-service` passed with 108 library tests + 141 service
  tests after VM profile pins.
- `cargo test -p capsem-service` passed with 108 library tests + 142 service
  tests after installed profile payload identity pins.
- `cargo test -p capsem-service` passed with 108 library tests + 144 service
  tests after the service profile catalog reconcile route.
- `cargo test -p capsem` passed with 240 tests after the native profile
  catalog reconcile CLI parser/client hook.
- `cargo test -p capsem-core reconcile_ --lib` passed with 6 focused
  reconciliation tests and `cargo test -p capsem-service
  handle_reconcile_profile_catalog` passed with 3 service tests after absent
  installed profile cleanup.
- `cargo test -p capsem-service` passed with 108 library tests + 145 service
  tests and `cargo test -p capsem` passed with 241 tests after the absent
  cleanup and CLI summary coverage.
- `cargo test -p capsem-core --lib` passed with 1612 tests + 1 ignored after
  absent installed profile cleanup.
- `cargo test -p capsem-core installed_profile_asset_filenames --lib` passed
  with 2 tests, `cargo test -p capsem-core settings_profiles --lib` passed with
  133 tests, and `cargo test -p capsem-service saved_vm_assets` passed with 2
  tests after profile-aware asset retention sources.
- `cargo test -p capsem-core --lib` passed with 1614 tests + 1 ignored and
  `cargo test -p capsem-service` passed with 110 library tests + 145 service
  tests after profile-aware asset retention sources.
- `cargo test -p capsem-core cleanup_ --lib` passed with 7 tests,
  `cargo test -p capsem-core --lib` passed with 1615 tests + 1 ignored,
  `cargo test -p capsem-service handle_asset_cleanup` passed with 2 service
  tests, and `cargo test -p capsem-service` passed with 110 library tests +
  147 service tests after the profile-aware asset cleanup caller.
- `cargo test -p capsem-service resume_saved_vm` passed with 2 service tests,
  and `cargo test -p capsem-service` passed with 109 library tests + 148
  service tests after forward-only resume pin enforcement.
- `cargo test -p capsem-service profile_status`, `cargo test -p capsem-service
  handle_reconcile_profile_catalog_installs_current_active_revision`, `cargo
  test -p capsem format_session_profile_for_list`, and `cargo test -p capsem
  list_response_with_entries` passed after VM list/status profile-state
  reporting.
- `cargo test -p capsem-service` passed with 109 library tests + 149
  service tests, and `cargo test -p capsem` passed with 242 CLI tests after the
  VM list/status profile-state reporting slice.
- `cargo test -p capsem-service vm_profile_pin_requires_signed_catalog_revision`,
  `provision_from_source_requires_profile_revision_pin`,
  `handle_fork_rejects_source_without_profile_revision_pin`,
  `handle_persist_rejects_running_vm_without_profile_revision_pin`, and nearby
  fork/resume positive-path tests passed after forward-only
  create/fork/persist pin enforcement.
- `cargo test -p capsem-service` passed with 109 library tests + 153 service
  tests after forward-only create/fork/persist pin enforcement.
- `cargo test -p capsem-core
  clone_sandbox_state_preserves_vm_effective_profile_attachments`, `cargo test
  -p capsem-service handle_fork_preserves_profile_and_fork_exec_works`, and
  `cargo test -p capsem-service
  handle_fork_rejects_profile_string_drift_after_clone` passed after fork
  profile-integrity coverage.
- `cargo test -p capsem-core --lib` passed with 1616 tests + 1 ignored, `cargo
  test -p capsem-service` passed with 109 library tests + 155 service tests,
  and `cargo test -p capsem` passed with 242 CLI tests after fork
  profile-integrity coverage.
- `cargo test -p capsem-core telemetry --lib` passed with 31 tests.
- `cargo test -p capsem-process --no-run` passed.
- `cargo test -p capsem-mcp-aggregator --no-run` passed.
- `cargo test -p capsem-core settings_profiles --lib` passed with 122 tests.
- `cargo test -p capsem-core settings_profiles --lib` passed with 130 tests
  after core profile catalog reconciliation.
- `cargo test -p capsem-core --lib` passed with 1611 tests + 1 ignored after
  core profile catalog reconciliation.
- `cargo test -p capsem-core profile_manifest --lib` passed with 12 tests after
  adding lifecycle gates and current/specific revision resolution.
- `cargo test -p capsem-core profile_manifest --lib` passed with 20 tests after
  adding the installable profile payload guard, signature wrapper, and fetch
  primitive.
- `uv run pytest tests/test_profiles.py -q` passed with 10 Pydantic
  profile/manifest tests after mirroring lifecycle gates and revision
  resolution in admin models.
- `uv run pytest tests/test_profiles.py -q` passed with 12 Pydantic
  profile/manifest tests after adding installable payload verification.
- `cargo test -p capsem-core --test profile_schema` passed with 6 tests.
- `cargo test -p capsem-service` passed with 245 tests.
- `cargo test -p capsem-process --no-run` passed.
- `cargo test -p capsem profile_catalog` passed with 7 tests,
  `cargo test -p capsem parse_profile_reconcile_catalog` passed with 3 tests,
  and `cargo test -p capsem` passed with 251 tests after adding file/URL
  profile catalog reconcile sources.
- `cargo test -p capsem-service handle_profile_catalog` passed with 2 tests,
  `cargo test -p capsem parse_profile_catalog` passed with 1 test, and `cargo
  test -p capsem profile_catalog_summary` passed with 1 test after adding
  read-only catalog status API/CLI wiring.
- `cargo test -p capsem-service handle_profile_revisions` passed with 3 tests,
  `cargo test -p capsem parse_profile_revisions` passed with 1 test, and
  `cargo test -p capsem profile_revisions_summary` passed with 1 test after
  adding per-profile revision inspection API/CLI wiring.
- `cargo test -p capsem` passed with 255 tests and `cargo test -p
  capsem-service` passed with 112 lib tests, 174 service-bin tests, and doc
  tests after the revision inspection slice; the service gate also now keeps
  the profile asset operator-flow log capture on one dispatcher-bound runtime
  so verification/install log assertions are stable under the full package run.
- `cargo test -p capsem-service handle_install_profile_revision` passed with 2
  tests, `cargo test -p capsem-service handle_update_profile_revision` passed
  with 1 test, `cargo test -p capsem-service handle_remove_profile_revision`
  passed with 1 test, `cargo test -p capsem
  parse_profile_install_update_remove` passed with 1 test, `cargo test -p
  capsem profile_revision_action_summary` passed with 1 test, and `cargo test
  -p capsem-core remove_installed_profile_revision --lib` passed with 1 test
  after adding selected revision lifecycle actions.
- Widened gates after the selected revision lifecycle slice: `cargo test -p
  capsem` passed with 257 tests, `cargo test -p capsem-service` passed with
  112 lib tests, 178 service-bin tests, and doc tests, and `cargo test -p
  capsem-core settings_profiles --lib` passed with 137 tests.
- `uv run python -m pytest tests/capsem-e2e/test_winterfell_fork_lineage.py
  -q -s` passed with 1 real-VM fork-lineage test, and `uv run python -m pytest
  tests/capsem-e2e/test_profile_asset_boot.py -q -s` re-passed after extracting
  the shared Profile V2 asset-backed E2E fixture.
- `cargo test -p capsem setup::tests` passed with 16 tests.
- `uv run python -m pytest tests/test_profiles.py` passed with 8 tests.

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
startup plus `ReloadConfig` no longer read
`net::policy_config::load_settings_files()` or `MergedPolicies`; runtime
policies now derive from session-attached `vm-effective-settings.toml`. The
old `McpRefreshTools` management IPC was deleted later by S07's connector
replacement.
Fourth S01 settings checkpoint landed on 2026-05-14: service `/settings*`
handlers no longer use v1 settings-tree/preset/lint loaders and now read/write
typed `settings_profiles` state (including profile-backed policy rule updates).
Fifth S01 settings contract checkpoint landed on 2026-05-14: `/settings` no
longer emits legacy compatibility keys (`tree`, `issues`, `presets`,
`policy`) and now returns only typed payload fields:
`settings_profiles`, `profile_presets`, and `effective_rules`.
