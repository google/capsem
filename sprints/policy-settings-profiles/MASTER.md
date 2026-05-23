# Policy, Settings, Profiles Master

Last updated: 2026-05-23

## Where this sprint lives

**Single branch, single worktree.** Authoritative pinning is in
[tracker.md "Where this sprint lives"](tracker.md#where-this-sprint-lives);
the short version:

- Branch: `profile-v2`
- Worktree: `/Users/elie/.codex/worktrees/824d/capsem`
- Verify with `git worktree list` + `git log <branch> --oneline | head`
  before believing any "in flight elsewhere" claim.

## Legacy Sprint Retirement

Older sprint directories remain in the repository as historical context, but
they are not active planning authority for Profile V2. See
[RETIRED-LEGACY-SPRINTS.md](RETIRED-LEGACY-SPRINTS.md). Any useful requirement
from a retired directory must be promoted into this board before it affects
scope, sequencing, public surfaces, or release claims.

## Mission

Replace Capsem's v1 settings/policy stack with typed service settings and
VM/session profiles. Profiles become the only user-facing "security level"
concept and the product unit for guest package/tool assumptions plus VM asset
requirements. The old ad hoc settings registry, standalone `[mcp]` authority,
and hand-authored `config/defaults.json` runtime/UI source are removed
completely.

## Execution Mode

**Rescue complete; S07 foundation closed.** As of 2026-05-21, the profile-v2
branch is coherent and sits `138 ahead / 0 behind` `origin/main` in this
worktree. S00-S07, S07a, S07c, S07d, and S07b are closed. S08a is closed.
S08b is the next implementation gate, after this closeout audit records that
remaining work is owned by later sprints rather than left as pre-S08 debt.

**Bedrock release mode.** As of 2026-05-23, this sprint is no longer a rescue
or prototype track. The Profile V2 release must ship a fully working, usable,
and documented bedrock: the Network Engine, File Engine, Process Engine,
Security Engine, Resolved Event Emitter, profile contract, enforcement/
detection runtime, CLI, HTTP/UDS endpoints, and UI entry points stand together.
Later sprints may add credentials, quotas, remote plugins, richer workbench
views, marketing polish, and product integrations, but they must build on the
frozen engine/event/profile terms instead of changing them.
The release contract is pinned in
[BEDROCK-RELEASE-CONTRACT.md](BEDROCK-RELEASE-CONTRACT.md).

**Winter readiness.** The wall is the release gate. Nothing crosses it unless
the profile trust chain is signed, profile payloads are installed from the
catalog, VMs pin exact profile/revision/package/asset identity, old config stays
dead, and every public surface can explain what happened.

**Latest verification.** `just smoke` passed on 2026-05-20 in 272s. The S07b
closeout gate on 2026-05-21 also passed the focused admin/profile/image/
manifest/security/doc/doctor suite (`174 passed, 1 skipped`), `uv run python
-m compileall src/capsem`, and the docs build. The 2026-05-21 S07 closeout
audit re-ran profile manifest/schema checks, profile asset service probes,
fork/pin service probes, Python admin/profile/image tests, `git diff --check`,
and the docs build after fixing asset supervisor finish-event logging.

## Product Contract

- **Service settings are service/app-scoped.** They configure app/service
  behavior, profile roots, telemetry export, remote enforcement plugin endpoints, and
  credential storage for the cutover.
- **Profiles are VM/session-scoped.** They configure AI providers, MCP and
  connectors, skills, VM settings, security capabilities, canonical rules, and
  derived/generated rules for sessions and VMs.
- **VM-effective settings are attached to a VM/session.** Runtime enforcement,
  debug reports, status, guest materialization, and UI truth read the resolved
  VM-effective profile state.
- **Enforcement and detection are separate runtime surfaces.** Enforcement is
  blocking-capable CEL policy exposed through `/enforcement/*`. Detection is
  Sigma-compatible finding logic exposed through `/detection/*`. Both support
  validate, compile, backtest, live list/mutation, and stats; detection also
  supports forensic hunt. Backtest returns event-level evidence by default.
- **Policy rules use canonical roots, not event internals.** Authored CEL and
  the future high-level DSL mirror a typed policy context with roots such as
  `http`, `dns`, `mcp`, `model`, `file`, `process`, `profile`, and `common`.
  Rules such as `http.request.host.contains("google")` are valid; `event.*` is
  internal-only and must be rejected by runtime/admin validators. The shared
  object typing lives in `capsem-proto` with an explicit schema version, while
  CEL injection/evaluation stays in `capsem-security-engine`.
- **Activity engines must be separated.** Network transport, file/snapshot
  mechanics, process/audit mechanics, security decisions, and resolved-event
  emission are separate engines. S08b creates those boundaries so
  network/file/process code parses and applies typed responses, while the
  Security Engine owns policy, ask/confirm, detection, postprocessing, and the
  complete resolved-event journal.
- **The bedrock contract freezes in this release.** After S08b/S09/S16/S19/S18
  pass, public and internal extension points are the contract: typed engine
  inputs/outputs, canonical policy roots, profile-owned rule packs, resolved
  event journal, runtime registry routes, CLI commands, and UI models. Future
  work can add plugins, credentials, quotas, reporting, and new engines through
  those extension points; it must not require another rewrite of the engine
  vocabulary, event identity, profile pinning, route names, or rule-authoring
  roots.
- **Session DB must become a resolved-event store.** Existing domain tables are
  useful read models, but S08b must add a canonical resolved-event journal and
  route migrated event-family writes through the emitter instead of direct
  subsystem SQLite writes.
- **AI/model/MCP evidence is canonical before policy.** Guest-originated model
  requests, model responses, model tool calls, tool results returned to models,
  and MCP executions must project into a provider-neutral evidence model before
  CEL, Sigma, backtest, telemetry, quotas, timeline, plugins, or UI rely on
  them. OpenAI, Anthropic, and Google/Gemini are first-class first-slice
  providers; Bedrock is later adapter coverage, not a release blocker.
- **Host AI accounting is separate from VM accounting.** Service-owned model
  prompts such as VM naming, session summarization, support-bundle summaries,
  and admin/workbench helpers may correlate with a VM/profile/session, but they
  must carry host/service attribution and increment host counters, host
  telemetry, and host quota dimensions. They must not inflate VM health,
  running-VM model call counts, VM MCP/tool counts, VM token/cost totals, or VM
  quota dimensions unless the call actually originates from that VM runtime
  path.
- **Everyday work UI is a follow-on sprint.** S16a owns the Conversation
  Engine, SDK/terminal adapters, and structured `/timeline/{id}` workbench. The
  bedrock release must preserve canonical event ids and links so S16a can build
  without changing the engine contract.
- **The signed manifest is the profile catalog.** The binary owns the manifest
  signing trust root; the manifest lists profile ids, immutable revisions,
  lifecycle status, payload locations, payload hashes/signatures, and binary
  compatibility. Profiles then declare package/tool contracts and the VM assets
  needed to satisfy them.
- **VMs pin profile revision and assets.** Creating a VM resolves a profile
  revision, downloads/verifies that revision's assets on first use, and pins the
  profile id/revision plus exact asset hashes in the VM registry/session state.
  Profile updates do not silently mutate existing VMs.
- **Admin tooling derives images and rule artifacts offline.** Corp/admin image
  and manifest workflows use the released `capsem-admin` Python CLI. Profiles
  are the source of truth for package/tool contracts and image build plans;
  hand-edited image settings are not a compatibility surface. `capsem-admin`
  must also produce valid enforcement CEL and Sigma detection artifacts without
  requiring Capsem to be installed; S08c proves parity against the Rust runtime
  using shared event/rule corpora.
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
| 11 | [S06c - Ablate Legacy NetworkPolicy Runtime](S06c-ablate-legacy-networkpolicy.md) | Done | Deleted legacy `NetworkPolicy` and the first V1 hook path. A later S08b cleanup removed the remaining named `PolicyConfig` runtime, confirm shim, policy-hook spec, and policy-hook telemetry table so policy returns only through the Security Engine path. |
| 12 | [S06d - Core Structure And Test Boundaries](S06d-core-structure-and-test-boundaries.md) | Done | Split oversized MITM/DNS modules and tests inside `capsem-core` before the rename and S08b engine contracts; defer new crate boundaries to S08b. |
| 13 | [Post-S06 cleanup milestone](tracker.md#post-s06-cleanup-milestone) | Done | Closed after the singular `policy` rename, S06c/S06d structural cleanup, `just smoke`, S07/S07a route proof, S07c live asset boot proof, and S07b admin closeout gates. Remaining confirm/journal/release hardening is owned by S08b/S15/S18, not Post-S06 cleanup. |
| 14 | [S07 - UDS Service API](S07-uds-service-api.md) | Done | Metrics IPC foundation, profile list/get/resolve, profile create/fork/update/delete, profile-backed VM create request shape, standard `mcpServers` profile format plus Profile V2 MCP server list/create/delete across service/CLI/capsem-mcp, old MCP management API/IPC removal, rules list/get/create/delete/evaluate, typed `GET /confirm/pending`, Profile V2 skills list/create/delete, and chained S07 route proof have landed. HTTP, CLI, production confirm resolution, and UI lift remain in S08/S09/S15/S16. |
| 15 | [S07a - Profile Manifest, Packages, And Assets](S07a-profile-manifest-assets.md) | Done | Canonical signed profile catalog, status enum, Profile V2 schema/Pydantic models, package/tool contracts, per-arch VM assets, profile-driven download/reconcile, cleanup retention, signed payload checks, revision/payload/package/asset pins, forward-only VM identity gates, VM list/status profile state, CLI/service/gateway catalog and revision actions, scheduled `[profile_catalog]` reconciliation, and old asset-manifest authority removal have landed. UI richness and deeper post-engine provenance are owned by S11/S16/S18. |
| 16 | [S07c - Profile Asset Update Orchestration](S07c-profile-asset-update-orchestration.md) | Done | Manual service asset reconcile endpoint, `capsem update --assets` service trigger, status checked-at/profile/payload/per-asset provenance propagation, structured check/download logs, service debug Profile V2 asset-health reporting, old Rust asset-manifest parser/loader/downloader removal, duplicate-download/active-cleanup race proof, first-use VM create reconciliation, profile-pin asset authority for source/fork/persist, chained service-level reconcile/status/debug/log proof, formal `file://` asset reconciliation, explicit UDS socket selection, and a live real-VM boot/exec proof from freshly reconciled profile assets have landed. |
| 17 | [S07d - Service Settings Schema And Admin Contract](S07d-service-settings-schema-admin-contract.md) | Done | Pydantic v2 `ServiceSettingsV2`, Pydantic-only JSON/TOML helpers, committed Draft 2020-12 schema artifact, valid/invalid fixtures, Rust/Python fixture parity, `capsem-admin settings init|schema|validate|doctor`, cross-runtime defaults drift proof, and closeout docs have landed. |
| 18 | [S07b - Capsem Admin Tooling And Profile-Derived Images](S07b-capsem-admin-tooling.md) | Done | Profile-admin validation/schema, required Profile V2 `ui`, `profile init`, guest-config-derived `profile init-builtins` generated `everyday-work`/`coding` base profiles, `settings init`, typed section editability gates, `capsem-admin image plan`, profile-derived `image build-workspace`, public `image build` routing, profile-required local/release asset build recipes, rootfs-generated package/tool inventory, local asset plus per-arch inventory-backed `image verify`, typed guest SPDX SBOM generation, typed doctor-bundle probe ingestion, profile-backed release-image boot gate, `manifest generate`, fast/download `manifest check`, minisign manifest signing/verification, profile/asset signature verification, developer bootstrap proof, OS package layout proof for the `capsem-admin` wrapper/Python payload, typed enforcement/detection pack validate/schema commands, pySigma-backed `detection compile|check` with Detection IR output, Rust Detection IR parity fixtures, corp admin/detection/enforcement docs proof, `capsem-admin doctor`, raw JSON boundary hygiene guards, and bootstrap symlinks for Claude/Gemini/Codex/Cursor have landed. |
| 19 | [S08 - HTTP Gateway API](S08-http-gateway-api.md) | In Progress | Profile V2 gateway contract slices landed: catalog/revision, profile CRUD/resolve, skills, standard MCP servers, rules/evaluate, confirm-pending read, profile-selected VM create response payloads, `/status` and `/setup/assets` profile asset provenance/progress, `/debug/report` profile provenance, exact typed-error passthrough, debug-report gateway runtime mismatch diagnostics, live selected-profile HTTP create/download/boot/exec with `/info` pin echo, and adversarial typed-error passthrough for malformed, locked, invalid, updating, and revoked Profile V2 cases. Remaining: S15 confirm resolution/stream. |
| 20 | [S08a - Rule Abstraction And Detection Architecture](S08a-rule-abstraction-detection-architecture.md) | Done | Enforcement and detection are separate profile-owned rule families; enforcement uses real CEL via the Rust `cel` crate family; Sigma is a detection authoring/import format, not a blocking language; `capsem.policy-pack.v1`, `capsem.detection-pack.v1`, `capsem.detection.ir.v1`, normalized event taxonomy, typed finding shape, admin validate/schema/compile/check commands, implementation ordering, testing matrix, and downstream S07b/S08b/S12/S13/S14/S15/S16a/S19 deltas are locked. |
| 21 | [S08b - Bedrock Engine](S08b-bedrock-engine.md) | In Progress / Release-Blocking | Bedrock engine contract. First slice added the `capsem-security-engine` contract crate with normalized security events, resolved-event actions, detection findings, quota dimensions, reserved throttle action, and serialization/strictness tests. Uses [S08 Side Sprint - Canonical AI Interaction Evidence](S08-side-canonical-ai-interaction-evidence.md) as the model/MCP evidence substrate. The shared typed `capsem-proto` policy context schema, direct CEL roots such as `http.request.host`, `event.*` rejection, HTTP request projection, service enforcement/detection routes, UI runtime-rule exposure, and full legacy named-policy runtime removal have landed. Remaining release blockers: split/wire Network/File/Process/Security engines, emitter-owned projections, canonical `session.db` journal, and real runtime dispatch for every shipped event family. |
| 22 | [S08c - Rule Corpus, Backtest, And Admin Parity](S08c-rule-corpus-admin-parity.md) | Done | Shared policy-context/enforcement/detection corpus, offline `capsem-admin` backtest parity, Rust CEL/Detection IR expected-artifact parity, negative `event.*` fixtures, session-backed hunt artifacts, installed-service policy-context export, and the first stable session-export process fixture have landed. |
| 23 | [S08d - Security Engine Performance Benchmarks](S08d-engine-performance-benchmarks.md) | In Progress | Security Engine CEL Criterion microbench harness, Detection IR/security-pack Criterion harness, host-side microbenchmark artifacts, VM-originated process/HTTP/DNS/MCP enforcement benchmark artifacts, detection/dedupe/runtime-registry/rebuild microbench coverage, and `just bench` wiring landed. The VM-originated slices prove live service/process IPC, guest network/MITM dispatch, guest DNS proxy dispatch, framed MCP dispatch, runtime CEL block, match counters, `session.db` resolved-event rows, and `logs` attribution at ~9.4ms mean blocked exec latency, ~4.0ms mean curl HTTP blocked-response first-byte timing, ~0.68ms mean post-pretransfer Security Engine/MITM response timing after warmup, ~0.55ms mean first-byte timing on persistent keep-alive blocked requests, ~1.11ms mean blocked DNS lookup timing, and ~0.31ms mean blocked MCP request timing. The latest slices also fixed same-millisecond Security Event ID collapse across HTTP/DNS/MCP/file logging, measured 100+100 rule engine rebuild at ~0.63ms plus 100-rule Detection IR lower+compile at ~2.76ms, expanded security logs with family-specific subject fields, and fixed MCP log subject projection to match by request id. Remaining: model/file VM-originated benchmarks, concurrency cases, backtest/hunt scan-rate artifacts, release-grade artifacts, and broader regression gates. |
| 24 | [S09 - CLI Integration](S09-cli-integration.md) | Release-Blocking | Add the usable CLI surface for the bedrock contract: profile/catalog/revision, profile-backed VM create, enforcement/detection validate/compile/backtest/live registry/stats/hunt, logs/status/debug. Confirm CLI is release-blocking only if `ask` is enabled in shipped rules. |
| 25 | [S10 - Credential Brokerage](S10-credential-brokerage.md) | Standalone Extension | Define credential release from service settings into sessions. It is explicitly split from the bedrock release and must use the frozen Security Engine/profile/resolved-event contracts. |
| 26 | [S11 - Status, Debug, Provenance](S11-status-debug-provenance.md) | Release-Blocking | Make status/debug/logs explain the shipped bedrock truth: active settings, profiles, derived rules, MCP, skills, profile catalog state, package contracts, asset readiness, VM pins, enforcement/detection matches, engine provenance, and honest live-health fields without false S12 claims. |
| 27 | [S12 - OpenTelemetry Metrics Architecture](S12-observability-plugin.md) | Post-Bedrock / Release-Truth Only | Typed per-VM live-metrics architecture: `capsem-proto::metrics`, process-side accumulator, bincode IPC snapshot, authoritative running-VM status health, model/provider/token/cost counters, enforcement/detection counters, latest detection/latest block summaries, service `/metrics/json` + `/metrics`, gateway proxy, UI typed-JSON. The bedrock release must preserve the event/counter attachment points and avoid false OTel claims; full export polish can follow. |
| 28 | [S13 - Remote Enforcement Plugin](S13-remote-policy-plugin.md) | Not Started | Add bounded remote enforcement decisions and observer export after S08b defines the engine/emitter boundary; rate limits, budgets, and centralized quota design are deferred to S22. |
| 29 | [S14 - Rules UI Components](S14-rules-ui-components.md) | Not Started | Shared enforcement-rule editor/renderer plus detection rule/finding/backtest components from S08b/S08c; the enforcement editor is embedded by [S15](S15-confirm-ux.md) for forward-rule decisions. |
| 30 | [S15 - Confirm UX (Ask)](S15-confirm-ux.md) | Conditional Release-Blocking | Production answer path for `decision = "ask"` inside the Security Engine: stacked pending-ask queue, UI prompter embedding the S14 enforcement-rule editor, CLI parity, auto-rule derivation per callback, confirm event integration. Required before any shipped profile/rule exposes ask as user-facing behavior; otherwise ask must be disabled or clearly unavailable. |
| 31 | [S16 - Profile UI](S16-profile-ui.md) | Release-Blocking | First-class usable UI for the new endpoint contract: profile catalog/selector/revision, package/asset readiness, profile-backed VM create, VM profile state, and runtime enforcement/detection overlay visibility/actions sufficient to operate the bedrock engine. Rich workbench polish remains S16a/S17. |
| 32 | [S16a - Unified Timeline And Agent Workbench](S16a-unified-timeline-and-agent-workbench.md) | Not Started | Friendly everyday-work UI for Codex/Claude SDK-backed and terminal-fallback sessions. Owns the Conversation Engine and structured `/timeline/{id}` API, consuming the S08b canonical resolved-event store. |
| 33 | [S17 - Security Capabilities UI](S17-security-capabilities-ui.md) | Not Started | Capability controls above canonical enforcement-rule editing plus detection finding/backtest views. |
| 34 | [S19 - Documentation And Site](S19-documentation-and-site.md) | Table Stakes / Release-Blocking | Document the bedrock engine contract, corporate deployment, signed profile catalogs, package contracts, profile-owned VM assets, settings schema, CLI/UI usage, enforcement how-to, detection how-to/backtest/hunt, and corp/admin `capsem-admin` workflows. Docs must clearly split shipped bedrock behavior from later credentials, quotas, remote plugins, OTel polish, and workbench polish. |
| 35 | [S19a - Marketing Site Refresh](S19a-marketing-site-refresh.md) | Not Started | Refresh the landing page around four pillars with claims for realtime CEL enforcement, Sigma-compatible detection/backtest/forensics over unified events, fast matching, VM health/OTel, and S08d-backed performance aligned to the sprint tracker. |
| 36 | [S18 - Full Verification And Release Gate](S18-full-verification-release-gate.md) | Table Stakes / Release-Blocking | Backend/CLI/UI/E2E/install proof for the core Profile V2 bedrock release path. |
| 37 | [S20 - OpenAPI To MCP](S20-openapi-to-mcp.md) | Proposed | Standalone product sprint for turning OpenAPI-described HTTP services into profile-owned MCP tools with review, provenance, diagnostics, and UI visibility. |
| 38 | [S21 - Local LLM](S21-local-llm.md) | Proposed | Standalone product sprint for making local model services first-class profile/VM AI providers governed by the same enforcement, detection, audit, diagnostics, and UI model as remote providers. |
| 39 | [S19b - Reporting Setup](S19b-reporting-setup.md) | Proposed / Non-blocking | Standalone operations sprint for reporting setup docs, collector examples, privacy guidance, and dashboard packaging. Core runtime can ship without it. |
| 40 | [S22 - Rate Limits, Budgets, And Quotas](S22-rate-limits-budgets-and-quotas.md) | Proposed / Later | Later full design sprint for HTTP/MCP/model/file/process rate limits, token/cost budgets, throttle actions, quota APIs, UI, telemetry, and plugin or centralized quota-provider integration. Not part of S08/S13 ship scope. |
| 41 | [S23 - Post-Bedrock Improvements](S23-post-bedrock-improvements.md) | Proposed / Later | Improvement track after the bedrock release. Adds richer product capabilities through the frozen engine/profile/API/UI contracts instead of reopening the rescue architecture. |

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
  remote enforcement plugin event contracts until S08a decides the enforcement-
  rule versus detection-rule abstraction, the real CEL runtime, the real
  Sigma-compatible detection path, and how enforcement/detection packs live in
  signed profiles.
- Do not build CLI/UI/telemetry/plugin contracts directly against today's mixed
  HTTP/DNS/MCP/file telemetry paths. S08b must split the Network Engine, File
  Engine, Process Engine, Security Engine, and Resolved Event Emitter first,
  with file/snapshot/process activity represented as normalized security events.
- Do not write CEL/Sigma/model/MCP rules, telemetry, timeline blocks, or quota
  dimensions directly against provider-specific model JSON. S08b must consume
  the canonical AI interaction evidence side sprint for OpenAI, Anthropic, and
  Google/Gemini before policy surfaces freeze.
- Do not expose service-owned AI prompts through VM metrics. S08b/S12 must add
  explicit host/service AI attribution, counters, logger fields, and tests
  proving host calls linked to a VM/session do not charge VM health totals.
- Do not add more independent `session.db` tables as security authority. S08b
  must define the canonical resolved-event journal and decide which existing
  domain tables remain projections/read models.
- Do not build the everyday-work UI directly against raw `pty.log`, `/inspect`
  SQL, or legacy telemetry tables. S16a consumes S08b's canonical
  resolved-event journal and owns the structured `/timeline/{id}` API, with
  cursor pagination over typed timeline blocks. Conversation, turn, process,
  activity, trace, finding, and artifact views are client-side filtering/
  formatting modes over those blocks. Raw transcript is only a forensic
  artifact/fallback input.
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
- Do not call the Profile V2 bedrock release done until S08b's Network/File/
  Process/Security/Emitter split is implemented and tested, S09 CLI and S16 UI
  can operate the new endpoint contract, S19 documents the shipped contract and
  deferrals, and S18 proves install/VM/E2E behavior. This is the sharp split:
  later sprints extend the bedrock; they do not redefine it.
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

Current execution is [S08b - Bedrock Engine](S08b-bedrock-engine.md).
S07/S07a/S07c/S07d/S07b are closed, S08 mirrors the Profile V2 service
contracts through the authenticated local HTTP gateway, and S08a has locked the
enforcement/detection architecture. No remaining item is allowed to float as
"S07 debt"; it must be closed here or assigned to a named later sprint.

[S08b - Bedrock Engine](S08b-bedrock-engine.md)
is the next implementation gate. It turns the S08a rule/detection decision
into real crate/module boundaries: Network Engine for transport, File Engine
for file/snapshot mechanics, Process Engine for process/audit mechanics and
attribution, Security Engine for preprocessors, enforcement, ask/confirm,
detection and postprocessing, and a Resolved Event Emitter for telemetry/audit/
logging/detection export. Conversation/timeline work is owned by S16a.

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

S07a/S07c/S07d/S07b closeout:

1. Catalog-driven profile payload install/update/remove/revoke from manifest
   records is closed in service, CLI, gateway, scheduled reconcile, and
   `capsem-admin` manifest workflows.
2. VM profile identity is closed: create, source, fork, persist, resume, list,
   info, and telemetry carry explicit profile id, revision, payload hash,
   package-contract hash, and pinned asset hashes.
3. Asset reconciliation is closed for S07: profile-aware cleanup, duplicate
   reconcile sharing, cleanup-while-updating fail-closed behavior,
   first-use selected-profile downloads, structured logs, status/debug
   provenance, and live profile-asset boot proof all use Profile V2 authority.
   Cross-process/per-asset lock hardening, if still required after S08b
   engine boundaries, belongs to S18 release verification.
4. In-guest package/tool proof is closed through S07b's image inventory,
   doctor-bundle verification, and profile-backed release-image boot gate.
5. UI-rich catalog/profile editing belongs to S16. Post-engine provenance and
   deeper debug presentation belongs to S11. Release-scale replay and upgrade
   probes belong to S18.

[S07 - UDS service API](S07-uds-service-api.md), S07a, S07c, S07d, S07b, and
[S08a](S08a-rule-abstraction-detection-architecture.md) are the
public-contract foundation for every later layer. HTTP, CLI, UI, docs,
marketing, telemetry, plugins, and release tooling must consume those shapes
rather than inventing independent profile/settings/rule/admin semantics.

**Deferred work remains visible and owned.** S06c legacy NetworkPolicy
ablation, S06d structure, and the final V2 naming collapse are closed. The
remaining release holds are not hidden cleanup debt: real confirm UX is S15,
the canonical resolved-event journal and engine split are S08b, richer
status/debug is S11, profile UI is S16, OTel metrics/finding propagation is
S12, docs/site are S19/S19a, and final replay/doctor/release gates are S18.

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
typed `settings_profiles` state (including profile-backed enforcement rule updates).
Fifth S01 settings contract checkpoint landed on 2026-05-14: `/settings` no
longer emits legacy compatibility keys (`tree`, `issues`, `presets`,
`policy`) and now returns only typed payload fields:
`settings_profiles`, `profile_presets`, and `effective_rules`.
