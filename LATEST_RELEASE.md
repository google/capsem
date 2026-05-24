version: 1.2.1779658398
---
### Fixed
- Fixed guest `localhost` resolution during boot by restoring a deterministic
  `/etc/hosts`, so CLIs that bind local helper servers such as Google
  Antigravity (`agy`) do not send `localhost` lookups through Capsem DNS.
- Fixed live VM header model counters so VM-scoped model calls update the
  in-memory metrics snapshot used by `/status`, while host-scoped model calls
  remain excluded from VM accounting.
- Fixed Settings loading against the Profile V2 `/settings` contract so the UI
  accepts typed `profile_presets`, `effective_rules`, and `settings_profiles`
  responses without requiring the removed legacy settings tree.
- Fixed Gemini guest setup for Profile V2 sessions: saved Google AI
  credentials now project to `GEMINI_API_KEY`, and non-interactive Gemini
  launches use a real wrapper that defaults to `--yolo` instead of relying on a
  shell alias.
- Fixed dashboard status polling to retry gateway initialization before
  reporting the service offline, avoiding a stale offline state after
  start/install races when the gateway is actually healthy.
- Fixed dashboard connected-state polling to confirm `/status` before showing
  the service offline after a transient gateway health miss.
- Fixed human `capsem status` output to summarize profile assets compactly and
  move profile provenance into a trailing block instead of dumping every asset
  URL and hash inline.
- Fixed the local install harness to restore the packaged `capsem-admin`
  wrapper and Python payload when repairing or simulating an installed layout.
- Fixed frontend gateway API calls to refresh the localhost auth token and
  retry once after a 401, preventing the onboarding Profile step from blocking
  on stale gateway credentials.
- Fixed onboarding provider credentials for the Profile V2 cutover: detected
  service credentials now show as configured, and manually entered keys are
  saved as Profile V2 credential IDs instead of legacy settings keys.
- Fixed the final onboarding screen to use session/profile language and show
  profile cards instead of exposing VM asset readiness internals.
- Fixed profile listing launchability so `/profiles` and `/profiles/catalog`
  mark profiles without an installed signed catalog revision unusable even
  when their VM asset files are present.
- Fixed local setup for packaged Profile V2 installs so `capsem run` and
  temporary `capsem shell` can pin profile/package/asset metadata from the
  packaged base profile without generating a duplicate corp profile.
- Fixed Profile V2 runtime defaults so packaged base profiles emit
  schema-valid profile payload JSON instead of defaulting profile accent colors
  to the service-settings-only `"blue"` value.
- Fixed the local install simulation to codesign macOS Mach-O binaries with the
  Virtualization entitlement, matching package postinstall behavior so release
  smoke tests do not boot unsigned `capsem-process` binaries.
- Fixed `just install` so it reruns non-interactive setup after restoring
  preserved settings and syncing assets, preventing local reinstalls from
  undoing package postinstall setup and leaving profile pins incomplete.
- Fixed `just install` so it no longer restores package-owned `profiles/base`
  or stale profile catalog sidecars over the freshly materialized package
  profiles, preventing VM asset hash drift after initrd repacks.
- Fixed `just install` so the initrd repack runs inside the recipe and repairs
  the existing local profile metadata before any sudo/package step, keeping the
  installed product coherent even if the user cancels or cannot complete sudo.
- Fixed `just install` so local installs rebuild the host-arch profile-derived
  VM assets before repacking/syncing them, preventing an old rootfs from
  surviving after base profile package/tool contracts change.
- Fixed ARM64 guest kernel configuration to use a 48-bit userspace virtual
  address layout, so TCMalloc-based Linux ARM64 CLIs such as Google
  Antigravity (`agy`) can run inside Capsem VMs instead of crashing during
  startup.
- Fixed the local install simulator to tolerate repo `assets/` being the same
  filesystem tree as `~/.capsem/assets`, avoiding same-file copy failures while
  repairing a dev install.
- Fixed the macOS package postinstall hook so it waits for the service socket
  and gateway health endpoint before opening the desktop app, preventing the UI
  from launching into a stale offline screen during install.
- Fixed package postinstall hooks to fail loudly when no target user can be
  determined for per-user setup instead of leaving a package that requires
  manual `capsem setup`.
- Fixed Profile V2 HTTP write enforcement so derived `http.read` and
  `http.write` rules compile into guarded runtime CEL, preserve rule priority,
  let runtime overlays override profile defaults, and resolve profile `ask`
  decisions as allow/pass until S15 ships interactive confirm resolution.
- Fixed in-guest doctor diagnostics to treat positive MCP network probes as
  conditional on the selected profile while still requiring write requests to
  be blocked when `CAPSEM_WEB_ALLOW_WRITE=0`.
- Cleared the local Docker/Colima initrd packaging caveat after restoring the
  half-running Colima VM and proving `just _pack-initrd` with Docker
  cross-compilation, initrd repack, hash-named assets, and manifest signature
  verification.
- Updated developer skills to require a Colima stop/start recovery attempt
  before reporting macOS Docker-backed asset builds as blocked.

### Changed
- Changed default VM sizing to the agent-friendly `4 CPU / 8 GB RAM / 8 active
  VMs` baseline across Profile V2 base profiles, builder defaults, service
  admission defaults, onboarding, and the create-session override UI, and
  removed stale onboarding resource selectors that no longer write through
  Profile V2.
- Bumped the active release line and default stamping recipe from `1.1` to
  `1.2` for the Profile V2/bedrock engine release.
- Expanded human `capsem profile show` and `capsem profile resolve` output with
  package, tool, MCP, VM sizing, and VM asset contract summaries.
- Changed `capsem create`, `capsem resume`, and `capsem restart` to preserve
  typed Profile V2 provision metadata and print profile id/revision/status,
  package contract hashes, pinned VM asset hashes, and asset-health progress
  without changing the first-line VM id output.
- Changed `capsem info <vm>` to preserve and render Profile V2 VM pins,
  including profile payload hash, package contract hash, and pinned
  kernel/initrd/rootfs hashes.
- Changed the onboarding wizard to select Profile V2 profiles through the
  profile catalog/select routes and to show profile identity in the ready
  summary instead of the old security-preset wording.
- Changed frontend VM launch to refresh selected-profile asset status at first
  launch and show a modal download/progress state instead of silently blocking
  creation while assets are checking or downloading.
- Changed profile catalog/status surfaces to report VM asset readiness per
  profile, including missing local paths, so one broken profile cannot hide or
  block usable profiles.
- Changed the frontend profile catalog and launch flows to refuse profiles
  whose VM assets are missing or invalid while still showing the missing asset
  path needed to repair the profile.

### Added
- Added Google Antigravity CLI (`agy`) to the Profile V2 guest tool contract:
  base profiles declare the official `https://antigravity.google/cli/install.sh`
  curl install, `capsem-admin` schemas model it as typed `packages.curl_installs`,
  and image-workspace/rootfs generation materializes and verifies it as a
  required guest tool.
- Added `capsem mcp list` and `capsem mcp show` aliases for the Profile V2 MCP
  connector inspection path.
- Added typed Profile V2 document CLI coverage for `capsem profile create
  --file` and `capsem profile update <id> --file`.
- Added `capsem confirm list` to expose the current disabled S15 ask/confirm
  resolver state through the CLI.
- Added typed Profile V2 mutation CLI coverage for `capsem profile fork` and
  `capsem profile delete`.
- Added read-only Profile V2 CLI inspection with `capsem profile list`,
  `capsem profile show`, and `capsem profile resolve`.
- Added `capsem skills list/show/add/delete` for Profile V2 skill inspection
  and direct user-profile skill mutations through the service `/skills` routes.
- Added broader `capsem enforcement` and `capsem detection` CLI coverage for
  runtime rule compile, update, file-backed backtest, and detection hunt flows.
- Added the first `capsem-file-engine` crate so file activity normalization has
  a first-class Bedrock Engine boundary outside `capsem-core`.
- Added the first `capsem-process-engine` crate so process exec normalization,
  command classification, and inline process Security Engine evaluation have a
  first-class Bedrock Engine boundary outside `capsem-core`.
- Added the first `capsem-network-engine` crate and moved domain/HTTP network
  policy primitives out of `capsem-core`, with process runtime and builtin MCP
  tooling consuming the new boundary directly.
- Moved the DNS wire parser and adversarial fixture/property tests into
  `capsem-network-engine`, with DNS handler, process dispatch, examples, and
  fuzz targets consuming the Network Engine parser directly.
- Moved DNS transport result and DNS SecurityEvent projection into
  `capsem-network-engine`, so DNS runtime blocks, resolved-event rows, and
  legacy `dns_events` projection share the Network Engine boundary.
- Added Network Engine-owned HTTP SecurityEvent projection, with MITM telemetry
  adapting request/response stats into a typed `HttpSecurityEventInput` instead
  of constructing HTTP subjects directly inside `capsem-core`.
- Added Network Engine-owned MCP SecurityEvent projection, with framed MCP
  dispatch adapting JSON-RPC summaries into a typed `McpSecurityEventInput`
  before runtime CEL evaluation and resolved-event journaling.
- Moved the SSE wire parser and parser tests into `capsem-network-engine`, so
  AI/model stream parsing now starts at the Network Engine boundary instead of
  the old `capsem-core::net::parsers` path.
- Moved provider-neutral AI stream events, summaries, provider identity, and
  non-streaming usage parsing into `capsem-network-engine`, leaving
  `capsem-core` to own only MITM provider routing and key injection.
- Moved typed AI request parsing for Anthropic, OpenAI, and Google/Gemini into
  `capsem-network-engine`, including tool-result extraction and malformed-body
  fallback tests.
- Moved canonical AI interaction evidence projection into
  `capsem-network-engine`, so model request/response/tool-call/tool-result
  evidence is built at the Network Engine boundary before core telemetry
  persistence.
- Added Network Engine-owned model SecurityEvent projection, and switched
  session-backed detection hunt reconstruction to build model events through
  that boundary instead of constructing model subjects inside the service.
- Added persisted runtime enforcement/detection overlay recovery: service
  runtime rule mutations now atomically write a typed
  `capsem.runtime-security-rules.v1` store, and startup recompiles the saved
  overlays back into the CEL registries while failing closed on invalid rules.
- Disabled runtime `ask` overlays until the S15 confirm prompter lands, so
  enforcement validate/compile/install/backtest and persisted restore fail
  closed instead of exposing an approval workflow with no resolver.
- Added runtime Security Engine health to `/debug/report`, including the
  persisted runtime-rule store path, enforcement/detection registry counts,
  match counters, rule attribution, and the current confirm resolver state.
- Added runtime Security Engine health to `capsem status`: JSON status now
  carries the typed security summary from `/debug/report`, and text status
  shows compact enforcement/detection rule and match counts.
- Added a resolved Security Event summary to `capsem logs`, so session logs show
  event, block, detection, family, and rule counts before the raw structured
  security-event JSON lines.
- Added a Settings -> Policy Security Engine health panel that renders typed
  `/debug/report` runtime enforcement/detection counts, match totals, runtime
  rule-store state, and confirm resolver availability.
- Added a Settings -> Profiles catalog panel that renders typed profile
  catalog revisions, current/installed drift, and the canonical
  `active`/`deprecated`/`revoked` lifecycle states.
- Added profile selection through `POST /profiles/{id}/select` and surfaced the
  selected/default profile in the Settings -> Profiles UI.
- Added profile-backed VM create requests in the frontend quick-session and
  customize-session flows, forwarding service-reported profile id/revision and
  showing the active profile in the create dialog.
- Added VM profile identity and lifecycle status to the frontend session list,
  including a corrupted marker when a VM lacks an explicit profile pin.
- Added a profile asset readiness panel to the frontend Sessions screen,
  showing the active profile revision, architecture, payload hash, and
  per-asset source/hash/size provenance from `/status`.
- Added runtime rule backtesting to the Settings -> Policy Live Rules editor,
  posting draft enforcement/detection rules with a JSON event corpus and
  rendering deduplicated evidence rows from the service backtest result.
- Added session detection hunting to the Settings -> Policy Live Rules editor,
  letting operators run a draft detection rule against a specific session via
  `/sessions/{id}/detection/hunt` and inspect the returned evidence rows.
- Added the first S08d Security Engine Criterion benchmark harness for
  canonical CEL compile/evaluate, policy-context materialization, 100-rule
  last-match evaluation, and native HTTP lookup comparison.
- Added the first committed Security Engine CEL microbenchmark artifact under
  `benchmarks/security-engine/` and surfaced the host-side numbers in the
  benchmark results docs with explicit non-VM-originated caveats.
- Added the first VM-originated Security Engine benchmark for process
  enforcement: a serial live-service/VM test installs a runtime CEL block rule,
  measures repeated blocked exec decisions, verifies runtime match counters,
  `session.db` resolved-event rows, and `logs` attribution, and archives the
  result under `benchmarks/security-engine/`.
- Expanded the Security Engine Criterion benchmark artifact with runtime
  detection evaluation, backtest evidence deduplication, and runtime rule
  registry operation timings.
- Wired `just bench` to run the Security Engine Criterion microbenchmarks and
  VM-originated process-enforcement benchmark alongside the existing in-VM and
  lifecycle/fork benchmark stages.
- Added a VM-originated HTTP request enforcement benchmark that blocks a
  guest HTTPS request through the MITM/Security Engine path, verifies runtime
  counters, `session.db` security rows, and `logs` attribution, and archives a
  dedicated security-engine benchmark artifact.
- Refined the HTTP request enforcement benchmark to separate guest wall-clock
  latency from curl `time_starttransfer`, with a warmup request so cold
  proxy/TLS setup does not masquerade as Security Engine cost.
- Added curl phase timing deltas to the HTTP request enforcement benchmark so
  DNS, TCP connect, TLS appconnect, post-pretransfer first byte, and response
  tail costs are visible in the committed artifact.
- Added a persistent TLS keep-alive lane to the VM-originated HTTP enforcement
  benchmark so repeated in-connection block decisions prove sub-millisecond
  MITM/Security Engine response timing and one security log row per request.
- Added Security Engine benchmark coverage for runtime compiled-plan rebuilds
  and Detection IR parse/lowering/compile costs, with committed artifacts and
  `just bench` wiring for the `capsem-core` security-pack Criterion harness.
- Added runtime CEL enforcement on the DNS proxy path plus a VM-originated DNS
  request benchmark that blocks guest resolver lookups before upstream
  resolution, verifies `dns_events`, `security_events`, runtime counters, and
  `capsem logs` qname attribution, and archives a dedicated benchmark artifact.
- Added runtime CEL enforcement on the framed MCP endpoint plus a VM-originated
  MCP request benchmark that blocks guest `local__echo` tool calls, verifies
  `mcp_calls`, canonical `security_events`, runtime counters, and `capsem logs`
  server/tool attribution, and archives a dedicated benchmark artifact.
- Expanded `capsem logs` security-event projection with family-specific debug
  fields such as DNS qname, HTTP host/path, MCP server/tool, model provider/
  name, file path, and process operation/class.
- Added the internal "Ledger of the Realm" engineering-quality reference and
  linked the active S08b/canonical-AI-evidence sprint docs to its Lannister,
  Winterfell, Baratheon, and Iron-Bank standards.
- Added the S08 canonical AI interaction evidence side-sprint so model/MCP
  policy, detection, telemetry, timeline, quotas, and plugin work have a
  provider-neutral substrate for OpenAI, Anthropic, and Google/Gemini traffic.
- Added explicit host-versus-VM AI attribution requirements so future
  service-owned model prompts charge host telemetry/counters instead of VM
  health totals.
- Added main sprint release holds for host/service AI counters, resolved-event
  attribution, logger accounting owner fields, and tests proving host prompts
  correlated with a VM do not charge VM metrics.
- Added S08 canonical AI evidence contracts in `capsem-security-engine`,
  including OpenAI/Anthropic/Gemini/host fixtures, host-vs-VM attribution fields
  on security events and quota dimensions, optional model/MCP evidence subjects,
  and tests proving host AI does not charge VM accounting.
- Added the first `capsem-core` AI evidence adapter so existing OpenAI,
  Anthropic, and Gemini request/stream parser summaries project into canonical
  `ModelInteractionEvidence` with tool-call, tool-result, usage, argument
  status, and host-vs-VM attribution tests.
- Added normalized session database tables for canonical AI interaction
  evidence so provider/API/model/tool/linkage fields are queryable directly
  instead of being hidden in an opaque JSON blob.
- Added explicit canonical-AI-evidence enum persistence traits and SQLite
  `CHECK` constraints so session DB evidence rows can only store approved enum
  spellings.
- Added first canonical AI/MCP execution linkage: framed MCP tool calls now
  link to model-emitted MCP tool calls when trace id and normalized tool name
  agree, updating both queryable evidence rows and the legacy tool-call
  projection.
- Added security-engine quota/status projection for canonical AI evidence,
  including API family, parse/evidence status, model tool/result/execution
  counts, linked MCP tool-call counts, and MCP execution link identifiers.
- Closed the canonical AI evidence side sprint with additional fixtures and
  tests for OpenAI Responses, orphan model tool calls, orphan MCP executions,
  and provider unknown-field drift.
- Added the first S08b `capsem-security-engine` contract crate with normalized
  security events, resolved-event actions, detection findings, quota dimensions,
  and throttle-ready serialization tests.
- Added the first S08b Security Engine core pipeline shell, ordering
  preprocessors, enforcement, confirm, detection, postprocessors, and resolved
  event construction with fail-closed enforcement errors.
- Changed Security Engine `ask` decisions without a configured confirm resolver
  to record an applied confirm step and fail closed to a terminal block, so
  inline process decisions do not leave unresolved prompts in logs or jobs.
- Added a real CEL-backed S08b enforcement evaluator in `capsem-security-engine`
  so enforcement rules compile through the `cel` crate before install and
  evaluate against normalized `SecurityEvent` values at runtime.
- Added a real CEL-backed S08b detection evaluator so runtime detection rules
  produce typed findings on normalized `SecurityEvent` values before resolved
  event emission.
- Added lowering from `capsem.detection.ir.v1` into real CEL runtime detection
  rules, with explicit family/field allowlists so unsupported Sigma-derived
  paths fail closed before runtime install.
- Added Security Engine match-stat recording hooks so enforcement and detection
  matches update the runtime rule registry counters that future service stats
  routes will expose.
- Added first service-owned runtime `/enforcement/*` and `/detection/*`
  handlers for validate/compile, live add/update/delete/list, and stats backed
  by real CEL compilation and compile-first registry installs.
- Added deterministic priority ordering to runtime enforcement/detection
  registries and seeded the default effective profile's enforcement rules into
  the service runtime registry at startup, with profile/user/corp attribution
  and typed callback guards around profile CEL conditions; profile-scoped rules
  are kept out of the global runtime-rule broadcast snapshot.
- Added service-owned runtime enforcement and detection backtest handlers that
  evaluate candidate CEL rules against typed normalized `SecurityEvent` inputs
  and return the shared deduplicated `BacktestResult` shape.
- Added the first service-owned detection hunt handler for running multiple
  candidate detection rules over a supplied normalized event corpus.
- Added the first session-backed detection hunt golden path:
  `/sessions/{id}/detection/hunt` reads a hand-built canonical session DB
  corpus, reconstructs HTTP security events from structured journal/projection
  rows, verifies the reconstructed event projects iso-style into
  `capsem_proto::PolicyContext`, and runs real CEL detection rules against
  paths/hosts from the DB.
- Extended session-backed detection hunt reconstruction beyond HTTP so
  canonical `security_events` rows can join existing DNS, MCP, model, file,
  process, and snapshot projections into typed `SecurityEvent` values for CEL
  backtest/hunt rules, with common-row reconstruction for VM, profile, and
  conversation events.
- Added canonical AI evidence reconstruction for session-backed detection hunt:
  model events now prefer `ai_model_interactions` for provider/API family,
  stream, usage, and cost fields, while MCP events attach
  `ai_mcp_execution_evidence` for argument/result status.
- Added raw file path policy projection for normalized file security events,
  so CEL and Detection IR rules can target `file.activity.path` separately from
  classified `file.activity.path_class`.
- Added canonical `security_events` output to `capsem logs`, so resolved
  Security Engine decisions from `session.db` are visible as structured JSONL
  with VM/profile/user/rule/finding attribution alongside process and serial
  logs.
- Added canonical security-log support to the MCP VM log tool's grep/tail
  filtering so agent-side debugging sees the same resolved Security Engine
  events as the CLI.
- Updated HTTP gateway log contract tests and architecture docs so `/logs/{id}`
  is treated as the typed security/process/serial log envelope.
- Enriched `/timeline/{id}` security rows with canonical resolved-event rule,
  pack, finding-count, VM, profile, user, and accounting-owner attribution so
  timeline debugging no longer has to jump straight to SQL for those fields.
- Updated MCP tool metadata and usage docs so `capsem_vm_logs` and
  `capsem_timeline` advertise security-log and security-layer support.
- Changed runtime enforcement/detection backtest evidence rows to report
  canonical enforcement paths such as `http.request.host` instead of an opaque
  whole-subject blob.
- Expanded enforcement/detection backtest evidence rows with common
  attribution, HTTP headers/body, MCP request/response/link evidence, and model
  tool-call/tool-result paths so forensic hunts explain the fields rules
  matched.
- Added HTTP gateway contract coverage for runtime enforcement validation and
  session detection hunt routes so the security API preserves forensic matched
  fields through the gateway.
- Expanded HTTP gateway contract coverage across the S08b enforcement and
  detection route groups, including compile, backtest, list, stats, live
  create/update/delete, inline hunt, and session hunt passthrough.
- Improved `capsem detection hunt-session` human output to show matched event
  ids, rules, packs, outcomes, and canonical evidence fields instead of counts
  only.
- Added typed model tool-call policy projection under
  `model.request.tool_calls`, including name, origin, argument status, status,
  linked MCP call id, and parse confidence, with session-backed detection hunt
  reconstruction from `ai_model_tool_calls`.
- Added typed model tool-result policy projection under
  `model.response.tool_results`, including content kind, previews, error
  status, returned-to-model state, linked MCP call id, and parse confidence,
  with session-backed detection hunt reconstruction from
  `ai_model_tool_results`.
- Added a session policy-context export path:
  `GET /sessions/{id}/policy-contexts` and
  `capsem export-policy-contexts <session>` emit JSONL fixtures from
  `session.db` for admin/runtime corpus work, with live VM proof for blocked
  process enforcement.
- Added the first committed session-export policy-context fixture and matching
  process enforcement pack/expected report so admin offline backtest and Rust
  CEL parity both cover a real `process.exec` block shape.
- Added typed process operation and command-class columns to the canonical
  `security_events` ledger so blocked process decisions preserve policy
  evidence even when no downstream exec projection exists.
- Added a typed frontend API client surface for runtime enforcement and
  detection routes, including validate/compile/install/delete/list/stats,
  backtest, live hunt, and session-backed detection hunt calls.
- Added a Policy settings "Live Rules" UI for runtime enforcement and detection
  overlays, including rule priority, attribution, match counts, validation,
  install, and guarded runtime-only delete actions.
- Added the first S08c shared policy-context/CEL corpus fixtures, with Python
  Pydantic loading and Rust CEL parity coverage over canonical
  `http.request.*` roots plus rejected `event.subject.*` authoring.
- Added `capsem-admin detection backtest` for offline pySigma-backed detection
  checks against typed policy-context fixture JSONL.
- Added `capsem-admin enforcement backtest` for offline enforcement checks against
  typed policy-context fixture JSONL, with golden expected-result artifacts for
  the first shared S08c corpus.
- Added Rust S08c parity coverage proving the real CEL evaluator matches the
  committed admin enforcement backtest expected artifact.
- Added a committed Detection IR artifact for the S08c Sigma corpus and Rust
  parity coverage proving canonical `http.request.*` detection fields match
  the admin detection backtest expected artifact.
- Added `capsem-admin enforcement compile` to fail closed on unsupported or legacy
  enforcement roots before offline backtest.
- Added an explicit admin policy path allowlist so `capsem-admin enforcement compile`
  rejects unknown canonical-looking paths and cross-family policy roots before
  offline replay.
- Fixed `capsem-admin enforcement backtest` to compile-check enforcement packs before
  fixture replay, so an empty corpus cannot report success for invalid policy
  paths.
- Added an S08c drift test proving the committed Sigma-derived Detection IR
  artifact exactly matches current `capsem-admin` compiler output before Rust
  consumes it.
- Extended the real process-enforcement E2E so a VM-originated blocked exec is
  verified in both `capsem logs` and the resolved-event `session.db`
  `security_events` / `security_event_steps` journal.
- Expanded the admin policy-context model and offline enforcement backtest subset
  beyond HTTP so DNS/MCP/model/file/process/profile scalar roots, boolean
  equality, and numeric equality can be tested through `capsem-admin`.
- Added indexed model tool-call/tool-result enforcement paths to admin backtest so
  rules can match roots such as `model.request.tool_calls[0].name` and
  `model.response.tool_results[0].returned_to_model`.
- Added rule-corpus workflow documentation tying policy-context fixtures,
  enforcement/detection expected artifacts, admin commands, and Rust parity
  tests together.
- Expanded the S08c policy-context corpus with detection-only and
  auth-without-secret HTTP rows so enforcement and detection parity tests cover
  divergent outcomes.
- Added a session-backed detection hunt expected artifact for the hand-built
  `session.db` corpus, pinning matched fields and evidence signatures from the
  resolved-event journal path.
- Added session-backed detection hunt projection coverage for DNS, MCP, model,
  file, process, snapshot, VM, profile, and conversation rows, including
  canonical profile activity matched fields.
- Added CLI runtime security commands for enforcement and detection rule
  list/stats/validate/install/delete plus session-backed detection hunt.
- Added typed runtime rule definitions to the rule registry and service/API
  responses so installed enforcement/detection rules can be rebuilt into live
  Security Engine CEL evaluators without losing decision, severity, Sigma, or
  tag metadata.
- Added a service-side runtime Security Engine builder that evaluates installed
  enforcement and detection registries together and records live match counts
  back to the correct registry.
- Added `security_decisions` to session DB triage so normalized
  `security_events` decisions and failed steps surface alongside network, DNS,
  MCP, exec, and audit signals.
- Added production MITM telemetry dual-write for canonical resolved HTTP
  `security_events` while preserving the existing `net_events` projection, so
  Network Engine traffic now starts entering the S08b normalized event journal.
- Added inline Network Engine enforcement for HTTP requests: `capsem-process`
  now builds a CEL-backed runtime Security Engine from effective profile HTTP
  rules, MITM evaluates normalized `http.request` events before upstream
  dispatch, and blocked requests journal both `net_events` and canonical
  `security_events`.
- Added request-body-aware inline HTTP enforcement: when a runtime Security
  Engine is installed, MITM now buffers bounded request bodies before upstream
  dispatch so `http.request.body.text` CEL rules can block without touching the
  network, while preserving the forwarded bytes and telemetry body preview.
- Added response-body-aware inline HTTP enforcement: when a runtime Security
  Engine is installed, MITM can evaluate decoded `http.response.body.text`
  before guest delivery and synthesize a 403 without leaking the upstream body.
- Changed MITM security-event telemetry to persist the actual runtime
  `SecurityResult` when inline enforcement runs, preserving response-phase
  event types, rule ids, findings, and resolved steps instead of rebuilding a
  request-shaped event from `NetEvent`.
- Changed MITM runtime telemetry to persist every resolved request/response
  phase result for a transaction, so an allowed request event is not overwritten
  by a later response-phase block or finding.
- Added canonical MCP Security Engine journaling for framed MCP tool calls so
  allowed and blocked MCP requests write `security_events` alongside the
  existing `mcp_calls` projection.
- Added canonical DNS Security Engine journaling so DNS handler results write
  `security_events` alongside the existing `dns_events` projection.
- Added canonical file Security Engine journaling so file monitor and MCP file
  restore/delete events write `security_events` alongside `fs_events`.
- Added canonical process Security Engine journaling so exec dispatch writes
  typed observe-only `process.exec` events alongside `exec_events`.
- Added inline Process Engine enforcement for exec dispatch: `process.exec`
  events now evaluate through the runtime Security Engine before guest
  delivery, blocked exec calls resolve the pending IPC job with an error, and
  the canonical resolved event records the final decision.
- Added shared Process Engine command classification for session-backed
  detection hunt reconstruction, so historical `process.exec` events use the
  same canonical classes such as `shell`, `python`, and `network` as live exec
  enforcement.
- Added Process Engine runtime rule match stats coverage and subsystem-neutral
  fail-closed wording for runtime Security Engine compile failures.
- Added structured Process Engine decision logging for exec evaluation so
  `capsem logs <vm>` includes event ids, attribution, final action, rule/pack,
  reason, and process command class alongside the session database trail.
- Added JSON serialization coverage for Process Engine decision logs so the
  `security.process` fields that power `capsem logs` remain queryable.
- Added service log endpoint coverage proving structured process security
  decision lines are returned verbatim with VM/profile/user/rule attribution.
- Added testable `capsem logs` formatting so structured process security lines
  survive CLI tailing, and taught shell IPC handling to ignore runtime rule
  match-drain replies.
- Added a real VM e2e for runtime process enforcement: install a shell-blocking
  rule, prove `capsem exec` is blocked, and prove `capsem logs` shows the
  structured `security.process` decision with VM/profile/rule attribution.
- Fixed stale profile-asset test fixtures and child process log filters so
  old `request.*` policy roots no longer fail closed during boot and
  `security.process` lines are not filtered out of `process.log`.
- Added live VM status security metrics from the canonical resolved-event
  stream, including security event counts, block counts, detection counts,
  latest block, and latest detection surfaced through process metrics snapshots
  and service list/info responses.
- Added live VM status counters for canonical HTTP, DNS, model, MCP, file, and
  process security events, with host-attributed model events excluded from VM
  token/cost accounting.
- Added session database seeding for live VM status metrics so resumed
  persistent VM processes start from durable HTTP, DNS, model, MCP, file,
  process, security, block, and detection counters before adding new live
  canonical events.
- Added live profile-policy reload for the Network Engine runtime Security
  Engine: `capsem-process` now shares a swappable engine slot with MITM, so
  `ReloadConfig` can replace profile-derived HTTP enforcement without
  rebuilding the proxy config or restarting the VM process.
- Added typed runtime enforcement/detection rule snapshots to process IPC so
  service-owned `/enforcement/*` and `/detection/*` mutations can push live CEL
  rule state into already-running VM processes and report per-session
  propagation status.
- Added process-to-service runtime rule match draining so live VM enforcement
  and detection matches are folded back into service `/enforcement/stats` and
  `/detection/stats` without relying on stale service-local counters.
- Added VM/session/profile/user identity propagation into Network Engine
  security events and canonical AI evidence, including `CAPSEM_SESSION_ID` and
  `CAPSEM_PROFILE_REVISION` handoff through `capsem-process` and the MCP
  aggregator child environment.
- Fixed local setup-generated profile payloads to include the required UI mode
  when installing a local profile revision from `CAPSEM_ASSETS_DIR`.
- Added the shared `capsem-proto` policy context schema that future CEL and
  high-level DSL rules mirror, with versioned typed roots for common, HTTP,
  DNS, MCP, model, file, process, and profile activity.
- Added canonical policy-context CEL evaluation in `capsem-security-engine`, so
  runtime enforcement/detection rules now use roots such as
  `http.request.host` and reject internal `event.*` paths.
- Added all-family CEL match/pass smoke coverage for the policy context,
  covering dedicated DNS, HTTP, MCP, model, file, process, and profile roots
  plus common-root coverage for credential, VM, conversation, and snapshot
  security events.
- Added typed HTTP request policy projection for canonical CEL rules, including
  request URL/path, case-insensitive headers, and body text predicates such as
  `http.request.body.text.contains("secret")`.
- Added Rust Detection IR evaluation against the new S08b normalized
  `SecurityEvent` contract so Sigma-derived findings can run on the shared
  event model instead of a parallel fixture-only shape.
- Added S08b event identity fields for parent event, stream, activity, sequence,
  source engine, and enforceability so later engine wiring has the correlation
  data needed for timeline, telemetry, and quota work.
- Added S08b security-event schema versions, enforcement/detection pack identity
  fields, and JSON fixtures covering every normalized event family plus resolved
  event findings.
- Added the first S08b resolved-event emitter contract with required versus
  best-effort sink semantics, delivery bookkeeping, and shared event/finding id
  tests.
- Added the first structured resolved-event session ledger:
  `security_events`, `security_event_steps`, `detection_findings`,
  `detection_finding_tags`, and `security_event_links`, with
  `WriteOp::ResolvedSecurityEvent` persistence, canonical enum spelling checks,
  session-schema tooling coverage, and a `/timeline/{id}` `security` layer.
- Added S08b backtest result shaping with full event refs, mismatch outcomes,
  default 100-row match limits, and evidence-signature deduplication.
- Added the first S08b runtime rule registry contract with compile-first
  add/update, previous-plan preservation on compile failure, delete, and live
  match stats.
- Added S08b plugin-groundwork event semantics: first-class ask/block/rewrite/
  throttle decisions, labels/context/history snapshots, findings, declarative
  mutations, mutation target validation, and internal transport projection.
- Added deterministic S08b plugin transform validation with canonical event
  hashes, immutable core event enforcement, and prior label/finding/mutation
  preservation.
- Updated S08b security-event JSON fixtures to include plugin-facing context,
  trace labels, decisions, findings, and declarative mutations.
- Added plugin transform records to resolved security events so replay/audit can
  tie plugin identity to input/output event hashes.
- Added a deferred S22 rate-limit, budget, and quota sprint while keeping S13
  scoped to remote enforcement/observer plumbing and reserving S08/S12
  compatibility points for future throttle decisions.
- Added explicit S12 planning for authoritative in-memory running-VM status with
  enforcement/detection counters, latest detection, latest block, and shared
  `/metrics/json` plus Prometheus scrape sources.
- Added typed `capsem-admin doctor` output that checks admin toolchain
  readiness and optional Profile V2 image-plan derivation without using
  `guest/config` as the operator-facing source of truth.
- Added bootstrap-managed shared skill symlinks for Claude Code, Gemini CLI,
  Codex, and Cursor.
- Added the first S08 Profile V2 HTTP gateway contract coverage for profile
  catalog/revision routes, profile CRUD/resolve, skills, standard MCP servers,
  rules/evaluate, confirm-pending reads, profile-selected VM create response
  pins, and gateway `/status` profile/asset provenance.
- Added S08 gateway coverage for Profile V2 `/setup/assets` download progress,
  `/debug/report` profile asset provenance, exact service typed-error
  passthrough, and service debug-report diagnostics for stale or mismatched
  gateway runtime files.
- Added S08 live HTTP gateway coverage for selected-profile VM creation: real
  service/gateway processes now prove `/provision` accepts profile id/revision,
  reconciles the selected profile's verified VM assets before boot, execs
  through the gateway, and echoes the pinned profile state through
  `/info/{vm_id}`.
- Added S08 adversarial HTTP gateway coverage proving Profile V2 typed-error
  status/body passthrough for malformed profile creation, locked
  skill/MCP/rule mutations, invalid rule evaluation, asset cleanup while
  updating, and revoked profile revision install.
- Added regroup sprint specs for service-settings schema/admin parity and the
  policy-rule versus detection/Sigma architecture decision before CLI,
  telemetry, plugins, rule UI, and Confirm UX continue.
- Added `capsem-admin detection compile|check` with pySigma-backed Sigma
  parsing, typed `capsem.detection.ir.v1` output, JSONL normalized-event
  fixture checks, and fail-closed unsupported Sigma subset coverage.
- Added Rust Detection IR V1 schema/serde/evaluator parity fixtures so
  `capsem-core` consumes the same `capsem.detection.ir.v1` artifact emitted by
  `capsem-admin detection compile`.
- Added corp-facing admin CLI, enforcement, and detection-format docs covering
  PyPI install, developer editable usage, pySigma validation, Detection IR, and
  policy/detection command proofs.
- Added Profile V2 settings/profile provenance to the redacted service debug
  report, including selected profile, profile roots, effective VM summary,
  resolver trace summary, and credential-id-only reporting.
- Added Profile V2 service-settings runtime wiring for service asset locations,
  default VM sizing, and per-session `vm-effective-settings` plus resolver
  trace attachments.
- Added capsem-process consumption of session-attached Profile V2 effective
  settings for network defaults, MCP defaults, and Policy V2 runtime rules.
- Added framed MCP Policy V2 `ask` confirmation resolution through the shared
  confirmer/backoff contract before request dispatch and response surfacing,
  with redacted confirmation snapshots.
- Added HTTP Policy V2 `ask` confirmation resolution through the same
  confirmer/backoff contract before upstream request dispatch or guest response
  surfacing.
- Added model Policy V2 `ask` confirmation resolution through the shared
  confirmer/backoff contract before model request dispatch, model response
  surfacing, and tool-call/tool-response delivery, with redacted metadata-only
  confirmation snapshots.
- Added model Policy V2 `model.request` body rewrite support for
  `request.data` rules, forwarding only the rewritten bytes upstream and
  recording rewritten request previews in telemetry.
- Added a `net::policy_v2` runtime import surface plus CEL, gzip model-response,
  and builder config/defaults tests to keep Profile V2 policy enforcement and
  image-generated settings aligned.
- Added hardening coverage for HTTP gzip decompression, CEL quoted-literal
  parsing, and builder image/defaults alignment.
- Added guard coverage to keep generated builder/frontend settings fixtures from
  being treated as Profile V2 runtime authority.
- Added the first S07 UDS foundation: typed VM metrics snapshot structs plus
  service/process IPC request and response variants for live metrics.
- Added read-only Profile V2 UDS profile routes for listing profiles, fetching
  a profile record, and resolving VM-effective settings with resolver trace.
- Added Profile V2 UDS profile mutation routes for creating, forking, updating,
  and deleting user-owned profiles.
- Added Profile V2 UDS rules routes for listing resolved rules, fetching a
  rule with provenance, and dry-running V2 policy evaluation against synthetic
  subjects without enforcing or prompting.
- Added Profile V2 UDS rule mutation routes for creating user-authored rules
  and deleting direct user rules, including default built-in profile override
  materialization, duplicate-rule rejection, and locked-rule delete failures.
- Added chained functional and bounded performance coverage for the Profile V2
  UDS Rules API before mirroring it through the HTTP gateway.
- Added Profile V2 service tests proving profile creation cannot shadow locked
  profile roots and settings saves follow the currently selected user profile.
- Added the S07 UDS closeout surface: typed `GET /confirm/pending`, Profile V2
  `GET /skills` / `POST /skills` / `DELETE /skills/{id}`, locked/duplicate
  skills mutation coverage including inherited same-kind duplicates, and a
  chained profile/skills/MCP/rules route proof.
- Changed MCP management to use Profile V2 MCP servers: profiles now use the
  standard top-level `mcpServers` map with Capsem governance under
  `mcpServers.<id>.capsem`; `/mcp/connectors` now
  lists/adds servers, `/mcp/connectors/{id}` deletes direct user servers,
  and the old `/mcp/{servers,tools,policy}` plus `/mcp/tools/*` service/CLI
  surface, capsem-mcp debug tools, and service-to-process management IPC are
  removed.
- Added typed Profile V2 package/tool contracts and per-architecture VM asset
  declarations, including canonical BLAKE3 hash validation, path-traversal
  rejection, VM-effective serialization, and inherited resolver merge coverage.
- Added the formal Profile V2 JSON Schema Draft 2020-12 artifact with valid
  and invalid golden fixtures plus a Rust `jsonschema` validation gate.
- Added Pydantic v2 Profile V2 payload and manifest models for admin tooling,
  including Pydantic-only JSON validation/dumping helpers, TOML-to-Pydantic
  validation, and the canonical `active`/`deprecated`/`revoked` status enum.
- Added the first Service Settings V2 admin contract slice: Pydantic v2
  service-settings models, Pydantic-only JSON/TOML validation and dump helpers,
  a committed Draft 2020-12 schema artifact, valid/invalid golden fixtures, and
  Rust/Python fixture parity tests.
- Added the first `capsem-admin settings` commands: schema export,
  TOML/JSON validation, doctor summaries, typed JSON reports, and focused CLI
  coverage over the Service Settings V2 contract.
- Added a shared Service Settings V2 defaults fixture checked by both Python
  and Rust, and aligned Python's default user profile roots with the Rust
  `CAPSEM_HOME` / `$HOME/.capsem` path contract.
- Added `capsem-admin settings init` to emit Pydantic-generated Service
  Settings V2 JSON or TOML drafts with profile-root options, asset cache
  selection, overwrite protection, and validation tests.
- Documented the Service Settings V2 versus Profile V2 boundary, the
  `capsem-admin settings` validation flow, and the split from the guest/UI
  descriptor schema.
- Added `capsem-admin profile schema` and `capsem-admin profile validate`
  for Profile V2 JSON/TOML payloads, including typed JSON reports with profile
  id and revision.
- Added `capsem-admin profile init <profile-id>` to emit a valid Profile V2
  JSON or TOML draft through the Pydantic model, with all-architecture VM asset
  placeholders, package/tool contract defaults, optional file output, and
  parity tests proving init JSON matches init TOML after reparsing.
- Added `capsem-admin image plan <profile>` to derive a typed image build plan
  from Profile V2 package/tool/VM asset contracts, with `--arch all` by default,
  single-arch narrowing, and fail-closed missing-asset checks.
- Added `capsem-admin image verify <profile> --assets-dir <dir>` to verify
  profile-declared local kernel/initrd/rootfs assets by architecture, size, and
  BLAKE3 hash, with typed `capsem.image-verification.v1` JSON output and
  non-zero exits on missing or mismatched assets.
- Added typed `capsem.image-inventory.v1` package/tool inventory checks to
  `capsem-admin image verify --inventory`, comparing apt, Python, node, and
  required guest tool versions against the Profile V2 image plan while
  preserving Pydantic-only JSON input/output.
- Added rootfs build extraction of `image-inventory.json`, collecting installed
  apt, Python, node, and tool versions from the built container and validating
  the artifact through the same Pydantic model used by `image verify`.
- Changed `capsem-admin image verify` to auto-discover per-architecture
  `image-inventory.json` files under the asset directory and report inventory
  contract checks by architecture, rejecting ambiguous all-arch single-file
  inventory input.
- Changed profile image verification to fail closed when any selected
  architecture is missing its `image-inventory.json`, so package/tool contract
  proof is required rather than silently falling back to asset-only checks.
- Added `capsem-admin image verify --doctor-bundle` support for
  `capsem-doctor --bundle` tar files, parsing the JUnit probe result without
  extracting the archive and failing image verification on in-VM test failures.
- Added `capsem-admin image sbom` to generate per-architecture SPDX 2.3 guest
  image SBOM JSON from typed `image-inventory.json` artifacts, including
  profile/revision/package-contract identity and package-manager purl refs.
- Added a profile-backed release-image boot gate that requires host-arch
  `image-inventory.json`, boots the profile image, captures
  `capsem-doctor --bundle`, and verifies the bundle through
  `capsem-admin image verify`; local asset preflight now rebuilds when the
  host-arch image inventory is missing.
- Documented the S08a policy/detection contract: `capsem.enforcement-pack.v1`,
  `capsem.detection-pack.v1`, `capsem.detection.ir.v1`, normalized security
  event taxonomy, typed findings, admin validation/check commands,
  implementation ordering, and test matrix.
- Added typed `capsem-admin enforcement validate|schema` and
  `capsem-admin detection validate|schema` support for strict Pydantic policy
  and detection pack envelopes, including YAML detection envelopes, with
  committed JSON Schema artifacts.
- Added `capsem-admin manifest check <manifest> --fast` with typed
  `capsem.manifest-check.v1` reports, Pydantic manifest validation, local
  `file://` profile payload hash/id/revision checks, remote HTTP(S) `HEAD`
  checks, and non-zero exits on missing or mismatched profile payloads or
  signatures.
- Added `capsem-admin manifest check <manifest> --download` to fetch every
  referenced profile payload, profile signature, VM asset, and VM asset
  signature into a temp or explicit download directory, verifying profile
  payload hashes and profile-declared VM asset sizes and BLAKE3 hashes.
- Added `capsem-admin manifest generate --profiles <dir>` to produce typed
  Profile V2 catalog manifests from local JSON/TOML profile payloads, deriving
  exact payload hashes, `.minisig` URLs, status/current-revision overrides, and
  file or hosted profile URLs without hand-authored manifest JSON.
- Added minisign-backed `capsem-admin manifest sign`,
  `manifest verify-signature`, and `manifest check --download --pubkey`
  cryptographic verification for downloaded profile payload and VM asset
  signatures.
- Added a developer bootstrap proof that `uv sync` exposes the `capsem-admin`
  entrypoint and that `uv run capsem-admin --version` succeeds after Python
  dependencies are installed.
- Added release package layout proof for `capsem-admin`: macOS `.pkg` and
  Linux `.deb` assembly now require the relocatable admin wrapper plus its
  packaged Python payload, and release policy tests verify the helper is
  prepared before OS packages are built.
- Added `capsem-admin image build-workspace` to materialize a profile-derived
  build workspace from the Profile V2 package/tool contract, emitting
  `capsem.image-workspace.v1` reports and generated `guest/config`-compatible
  TOML without reading repo hand-authored image settings.
- Added `capsem-admin image build` as the public profile-derived image build
  entrypoint, routing generated workspaces into the existing kernel/rootfs
  Docker builder with typed `capsem.image-build.v1` JSON reports and dry-run
  support.
- Added the required Profile V2 `ui` contract (`everyday` or `coding`) across
  Pydantic, JSON Schema, Rust profile parsing/effective settings, fixtures, and
  generated built-in profile drafts.
- Added `capsem-admin profile init-builtins` to generate typed
  `everyday-work` and `coding` base profiles, plus committed generated base
  profile TOML drafts under `config/profiles/base/`.
- Changed built-in profile generation to derive package, tool, AI provider,
  MCP server, and VM resource contracts from `guest/config`, preserving the
  current release image inputs while making the profiles the source of truth.
- Added profile-aware `scripts/build-assets.sh --profile` and Justfile
  `build-assets` / `build-kernel` / `build-rootfs` profile arguments so local
  asset builds can route through `capsem-admin image build`.
- Changed VM asset build recipes and PR install CI to require a Profile V2
  payload, using `config/profiles/base/coding.profile.toml` by default and
  removing the unprofiled `capsem-builder build guest/` fallback from live
  build lanes.
- Fixed release SBOM attestation to cover Linux `.deb` packages as well as the
  macOS `.pkg`, and documented that the current `cargo-sbom` artifact is the
  Rust host SBOM while profile-derived guest package/tool SBOMs remain S07b
  image-verification work.
- Added Profile V2 section-level editability gates so profiles can allow user
  skill or MCP edits while locking AI providers, rules, VM assets, package
  contracts, or other sections; service mutations enforce the locks and forks
  preserve them. The editability map itself is immutable through profile update
  routes to prevent unlock-then-edit bypasses.
- Changed service settings reload fallback to reuse the startup settings
  snapshot when `service.toml` is absent or unreadable, preventing profile roots
  from silently falling back to defaults.
- Added Rust Profile V2 payload schema validation helpers for JSON and TOML
  payloads backed by the production Draft 2020-12 schema artifact.
- Changed the signed profile catalog manifest to the canonical
  `ProfileManifest` / `format = 1` contract, removing the transitional
  generation naming and old asset-manifest compatibility language.
- Changed VM asset readiness to be profile-driven: service startup now resolves
  boot assets from the selected profile's per-architecture declarations,
  downloads missing assets from profile URLs, and forwards expected hashes to
  `capsem-process` for boot-time verification.
- Added durable per-session telemetry identity: `session.db` now records the
  VM id, resolved profile id, and local user id, and `/info` exposes those
  fields for support/status flows.
- Added VM profile pins for persistent/running VM metadata, including resolved
  profile id, signed profile revision, profile payload hash,
  package-contract hash, and pinned boot asset identity.
- Changed VM profile pins to read the installed profile revision sidecar and
  include the installed profile payload hash when a verified catalog payload is
  present.
- Added core profile catalog reconciliation so active revisions install/update
  from signed payloads, deprecated installed revisions stay available for
  existing VMs, and revoked installed revisions lose their launchable profile
  plus current state.
- Added `POST /profiles/catalog/reconcile` on the service API so UDS/gateway
  callers can apply signed profile catalog lifecycle state and receive a typed
  install/deprecate/revoke/error summary.
- Added `capsem profile reconcile-catalog --manifest <path> --pubkey <path>`
  so the native CLI can apply a signed profile catalog through the service
  reconciler and print either a compact lifecycle summary or raw JSON.
- Added `capsem profile reconcile-catalog --manifest-url <https-url>` so
  operators can reconcile a signed Profile V2 catalog from a remote source,
  with `http://` accepted only for loopback development/test hosts and a
  bounded manifest body.
- Added typed `[profile_catalog]` service settings plus service-side scheduled
  profile catalog reconciliation from the configured signed catalog URL and
  profile payload public key.
- Added a read-only profile catalog status surface plus `capsem profile
  catalog [--json]` so operators can inspect the persisted signed catalog,
  installed profile revisions, revision lifecycle status, and configured
  catalog source.
- Added per-profile catalog revision inspection through
  `GET /profiles/{id}/revisions` and `capsem profile revisions <id> [--json]`,
  including current/installed revision markers and canonical lifecycle status.
- Added profile revision lifecycle actions through the service and CLI:
  `install`, `update`, and `remove` now operate on signed catalog revisions,
  reject revoked installs, clean revoked installed revisions, and remove local
  launchable state while preserving archived payload material.
- Changed profile catalog reconciliation to remove launchable installed
  profiles whose profile id is absent from the signed catalog while preserving
  the archived installed payload for retention/VM-pin cleanup.
- Added profile-aware asset retention sources so cleanup can preserve VM assets
  referenced by installed profile payloads and by persistent VM profile pins.
- Added `POST /setup/assets/cleanup`, a profile-era asset cleanup endpoint that
  removes unreferenced hash-named/legacy asset files without old manifest
  authority, preserves installed-profile and saved-VM pins, and refuses to run
  while assets are still checking or updating.
- Added `POST /setup/assets/reconcile` so callers can force the service-owned
  Profile V2 asset reconciler to check/download profile VM assets on demand.
- Added explicit profile selection for fresh VM create/provision requests and
  `capsem create --profile [--profile-revision]`, with selected profile asset
  reconciliation and VM-effective profile attachment before process spawn.
- Changed `capsem update --assets` to call the service Profile V2 asset
  reconciler instead of the old asset-manifest downloader.
- Changed VM profile pinning to require complete installed profile revision
  authority when present, including the runtime profile file, archived verified
  payload, and matching payload hash.
- Added structured profile asset check/download lifecycle logs with redacted
  asset URLs, plus status propagation for the service asset check timestamp.
- Added explicit Profile V2 asset provenance to service/CLI asset health,
  including profile id, profile revision, installed profile payload hash, and
  redacted per-asset source/hash metadata in reconcile, list/status, setup
  asset status, and debug-report payloads.
- Added adversarial coverage proving concurrent profile asset reconciles share
  one download run and asset cleanup refuses while a profile asset download is
  active.
- Changed first-use VM create/run to await the service Profile V2 asset
  reconciler before process spawn, and made create-from-source, fork, and
  persist derive boot-asset identity from the VM profile pin while rejecting
  pin/registry drift.
- Added chained service-level coverage proving a profile asset reconcile is
  reflected consistently in `/setup/assets`, `/list`, debug reports, and
  service logs after downloading from a local asset server.
- Added formal `file://` Profile V2 VM asset reconciliation support plus live
  E2E coverage proving `capsem update --assets` can fill an empty asset cache,
  boot a real VM from the reconciled hash-named assets, exec inside it, and
  preserve the installed profile revision pin in `capsem info --json`.
- Added a real-VM fork-lineage E2E proof that writes a file, forks, deletes the
  source, resumes the fork, mutates filesystem state, forks again, deletes the
  middle VM, and proves the final fork preserved only the expected descendant
  state.
- Added current UI baseline screenshots for the marketing-site refresh sprint,
  covering the hero plus the feature, security, how-it-works, and FAQ sections.
- Changed `capsem update --assets` to honor the selected service UDS socket
  instead of assuming the default runtime socket.
- Changed the runtime network policy module names from transitional
  `policy_v2`/`policy_v2_*` paths to the forward `policy` and `policy_model`
  surfaces, with DNS/MITM tests split into focused behavior modules.
- Removed the legacy MITM HTTP policy hook runtime path. Request/response-head
  HTTP enforcement must now move through the S08b canonical Security Engine
  path instead of the old pipeline hook.
- Removed the remaining legacy named-policy runtime: `net::policy`,
  `policy_confirm`, model-policy helpers, Policy Hook Spec0 API/artifact,
  policy-only DNS/MCP/MITM tests, the old policy benchmark, and the
  `policy_hook_events` session table/write path. HTTP, MCP, DNS, model, file,
  and process policy work now has one forward path: canonical Security Engine
  events.
- Removed the old Rust VM asset `ManifestV2` model, verified-manifest loaders,
  manifest-driven downloader, and manifest-driven cleanup path. CLI status and
  service debug reports now rely on Profile V2 asset health instead of legacy
  asset manifests, and cleanup removes stale legacy asset metadata files.
- Changed persistent VM resume to require forward profile pins and pinned asset
  identity; unpinned registry entries no longer fall back to the current
  profile/assets.
- Changed VM profile pinning to require a signed profile catalog revision,
  profile payload hash, and pinned asset identity before create-from-source,
  fork, or persist can produce durable VM state.
- Fixed VM forks to preserve VM-effective profile attachments and fail closed
  on profile drift before the fork is registered or executed.
- Added profile identity and status to VM list/status payloads, `capsem list`,
  and `capsem info`: each VM now reports its pinned profile/revision plus
  `current`, `needs_update`, `deprecated`, `revoked`, `corrupted`, or
  `unknown`.
- Removed legacy `assets.manifest.*` service settings and setup-time asset
  manifest checks; old asset-only manifests are no longer runtime authority.
- Changed `/setup/corp-config` inline and URL installs to accept Profile V2
  corp profile TOML and refresh the typed settings-profile surface.
- Changed guest boot config ownership so `GuestConfig`/`GuestFile` live under
  the VM namespace instead of the legacy policy-config namespace.
- Removed the legacy `net::policy_config` module, v1 settings-file runtime
  fallbacks, v1 install/setup fixtures, and old `user.toml`/`corp.toml`
  support-bundle/uninstall preservation paths in favor of Profile V2
  `service.toml` and profile roots.

### Changed
- Renamed the public admin enforcement-pack surface from `capsem-admin policy`
  to `capsem-admin enforcement`, including the Pydantic model/schema ids
  (`capsem.enforcement-pack.v1`, `capsem.enforcement-compile.v1`, and
  `capsem.enforcement-backtest.v1`), committed fixtures, docs, and tests. The
  old `policy` command group is not kept as a public alias.

### Fixed
- Fixed same-millisecond Security Event ID collisions across HTTP, DNS, MCP,
  and file logging. HTTP now carries a per-request event seed, and DNS/MCP/file
  event IDs use nanosecond timestamps so bursty decisions no longer collapse
  rows in `security_events`.
- Fixed synthetic HTTP block/error telemetry to enqueue Security Engine
  `net_events` and resolved `security_events` at the decision point instead of
  relying on response-body finalization, preserving fast denied keep-alive
  requests in `session.db` and `capsem logs`.
- Fixed settings policy-rule saves to reject unsupported `.match(` condition
  terms before writing a user profile override.
- Fixed HTTP gzip handling so comma-separated `Content-Encoding` token lists are
  recognized case-insensitively and malformed gzip headers with reserved flags
  pass through instead of dropping bytes.
- Fixed Policy V2 CEL parsing so method-looking text inside quoted string
  literals is not mistaken for `.contains()`/`.matches()` calls.
- Fixed Policy V2 dry-run/runtime callback coverage for generated `http.read`
  and `http.write` rules, including boolean `true` CEL catch-all conditions.
- Fixed `POST /profiles` so it rejects ids that already exist in built-in,
  base, corp, or user profile roots instead of writing a shadowing user file.
- Fixed `just smoke`, `just test`, and `build-ui` ordering so Tauri frontend
  assets are built before Rust workspace compile/clippy/test phases that need
  `frontend/dist`.
- Fixed isolated smoke/doctor runs to avoid installed gateway-port collisions
  and to skip persistent service-unit checks when a test-scoped service unit is
  intentionally not required.
- Fixed Profile V2 VM runtime migration compatibility so sessions consume only
  Profile V2 `vm-effective-settings.toml` instead of reopening legacy settings
  files at runtime.
- Fixed running VM reloads to refresh Profile V2 effective policy from each
  session attachment, including MCP builtin domain policy and Policy V2 rules.
- Fixed Profile V2 conditional MCP/HTTP rules so narrow argument/path rules no
  longer collapse into broad legacy tool/domain allow-block lists.
- Fixed default user profile discovery to resolve under `CAPSEM_HOME`/`HOME`
  instead of a literal `./~` directory, keeping local artifacts out of runtime
  and test profile resolution.
- Fixed install E2E asset handling when the repo `assets/` path is a symlink,
  including file-only asset copying so nested/stale arch directories cannot
  poison install fixture refresh.
- Fixed the Profile V2 valid-payload minisign fixture so profile catalog
  install/reconcile tests exercise real signature verification with a matching
  test public key.
- Fixed service test fixtures so profile roots are created consistently and
  asset lifecycle log assertions tolerate equivalent download event ordering.
- Fixed full smoke stability by closing inherited Python fixture log fds,
  provisioning E2E services with Profile V2 asset homes, separating signed MCP
  VM-lifecycle fixtures from editable profile-mutation fixtures, and running
  VM-heavy service/CLI and MCP smoke groups sequentially to avoid Apple VZ
  cleanup starvation.
