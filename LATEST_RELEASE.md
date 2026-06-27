version: 1.3.1782571508
---
### Added
- Added a logger DB correctness test proving the same HTTP/security/model/tool
  and body-blob query results match before flush, after flush, and after
  restart rehydration.
- Added an adversarial logger DB flush test proving interrupted disk flushes
  leave memory truth coherent, disk state transactionally valid, and recovery
  flushes exact.
- Added public session-telemetry documentation for the `turn_id` /
  `model_call_id` / `tool_call_id` identity graph and mirrored the same
  debugging contract in the session DB developer skill.
- Added a public `DbHandle::write` contract test proving security ledger
  events persist exact rule/action/detection/trace/turn/credential fields.
- Added a public `DbHandle::query` contract test proving bind parameters,
  deterministic column/row JSON, and the DB-owned route output cap.
- Added a logger DB startup rehydration contract test proving existing disk
  rows are visible through `query()` before `ready()` can be trusted by routes.
- Added explicit logger DB contract tests with failure messages that point back
  to the DB boundary rationale when ready/query/write exactness regresses.
- Added public `capsem-logger` DB handle contract docs and type aliases that
  pin caller-owned query intent, DB-owned execution/storage, and loud
  missing-schema failures.
- Added the logger-owned async DB handle contract with `ready`, `query`, and
  `write` execution APIs plus structured operation/duration logging for
  session ledger reads and writes.
- Added service route DB-boundary structured logging and rewired stats,
  history, timeline, security, detection, plugin, credential, and triage ledger
  reads through `DbHandle::ready/query` instead of route-owned SQLite readers.
- Added service-owned session DB handle registration so session routes resolve
  persistent `DbHandle`s from runtime state instead of reopening ledger handles
  on every request.
- Added startup session DB handle hydration plus structured readiness logging
  and `/vms/{id}/info` DB readiness status for session ledgers.
- Added the unified tool-call ledger contract: MCP `tools/call` observations
  now write to `tool_calls origin = 'mcp'` with request/response payloads,
  protocol-only MCP messages remain in `mcp_calls`, old SQLite constraints are
  migrated to allow orphan/direct tool evidence, and the UI/Inspector/Ironbank
  contracts read user-facing tools from the single `tool_calls` table.
- Added a frontend vocabulary guard that scans product source for retired 1.3
  UI/API strings such as VM dashboard wording, policy labels, preview fields,
  raw credential hashes, and 404/501 placeholders.
- Added a release evidence collector that writes a timestamped 1.3 audit bundle
  with git, manifest, benchmark, file-inventory, and pending manual-gate facts.
- Added provider-specific Ironbank release gate entrypoints for OpenAI, AGY,
  Gemini, and Claude so the Sprinty 1.3 provider matrix runs the named
  hermetic replay tests directly instead of relying on consolidated test files.
- Added a release evidence guard that parses Ironbank tests and fails the
  bundle if release-proof tests use disabled-test markers such as `skip`,
  `skipif`, `slow`, optional markers, or `pytest.skip()`.
- Added a release evidence guard that scans installed Capsem helper binaries
  when present and fails if any active `~/.capsem/bin/capsem*` executable still
  carries retired native Keychain credential-store markers.
- Added a dedicated optional live-provider compatibility canary suite for
  OpenAI, Gemini, and Claude outside Ironbank; it reuses the model-client
  ledger assertions when operator credentials are present and stays inert when
  no live keys are configured.
- Added an Ironbank guard that keeps Anthropic/Claude replay fixtures on the
  release-target `claude-sonnet-4-6` model across mock-server responses and SDK
  ledger assertions.
- Added an Ironbank guard that keeps the hermetic Gemini replay path on the
  release-target `gemini-3.5-flash` model across script generation, mock-server
  routing, pricing, and ledger assertions.
- Added strict capsem-doctor Ironbank acceptance checks for functional package
  manager proof, hermetic doctor fixtures, and no retired escape markers in the
  installed diagnostic suite.
- Added a remote HTTPS apt Ironbank package-manager gate that installs and
  runs a Debian package through Capsem, then verifies process, file, HTTP, DNS,
  and security ledger rows.
- Added bootstrap and Justfile contract tests that prove release gates keep
  checking project skills, site structure, profile-owned asset materialization,
  ruff/ty/skill validation, and retired escape-path names.
- Added the explicit `just test-frontend` release gate so Sprinty, docs, and
  local checks all use the same frontend check/test/build path.
- Hardened macOS and Linux postinstall scripts so missing packaged helper
  binaries fail installation instead of leaving stale tools in `~/.capsem/bin`.
- Added a dedicated Ironbank Claude CLI ledger gate that runs `ollama launch claude` through the VM profile and proves the model, tool, file, credential, and security ledger path.
- Added a dedicated Ironbank Codex CLI ledger gate that runs direct Codex and
  `ollama launch codex` through the VM profile and proves the model, tool,
  file, credential, and security ledger path.
- Added fresh 1.3 release benchmark artifacts and docs for the VM-path
  mock-server protocol, lifecycle, fork, disk, and EROFS/LZ4HC performance
  gates.
- Refreshed the 1.3 release benchmark baselines from the green full
  non-manual release gate.
- Refreshed the 1.3 release benchmark baselines again after the install/profile
  asset rail fix so release evidence matches the current branch state.
- Added benchmark report output for sample counts, error rates, and a generated
  1.3 release latency/throughput graph.
- Added an Ironbank mock-server contract proving the single reusable local
  mock server serves the HTTP, HTTPS/SSE, DNS, OAuth, MCP, OpenAI, Anthropic,
  Gemini/AGY, and Ollama fixture surfaces used by release gates.
- Added a stable Ironbank capsem-doctor acceptance contract that ties the
  named release gate to the full VM doctor ledger proof and shared mock server.
- Added an Ironbank profile asset readiness gate proving profile cards can be
  built from route-owned asset status for `code` and `co-work`, including
  missing, ensure/download, shared cache reuse, hash-named assets, and manifest
  provenance.

### Fixed (service control)
- Fixed DNS load latency by replacing per-query guest vsock connect/close
  with a persistent worker pool and matching host-side framed DNS sessions,
  while keeping every DNS query on the single security/logging rail before
  the response is returned.
- Fixed logger DB read latency by reusing the DB-owned reader connection,
  moving read-query timeouts to SQLite's progress handler, and caching DB
  readiness after validation; the Ironbank route gate now measures `/stats`
  at about 0.8ms p95 through the service and about 1.1ms p95 through the
  gateway.
- Fixed stats-detail session routes so a missing or broken session ledger fails
  loudly through the logger DB handle instead of fabricating empty model, tool,
  HTTP, DNS, file, process, credential, and security data.
- Fixed logger DB readiness so session ledgers validate required route-critical
  tables/columns and fail loudly on broken schemas instead of accepting any
  open reader, including preserving `tool_calls.turn_id` through legacy table
  rebuild migrations.
- Fixed session DB handle registration so malformed existing session ledgers
  surface explicit `/vms/{id}/info` readiness errors with structured
  `vm_id`/`db_path`/operation/duration/error logging instead of degrading into
  fake empty route data.
- Removed service-owned telemetry projections from stats, timeline, triage,
  security, detection, and history routes. Logged-data routes now read through
  the logger DB boundary, and a source guard rejects raw service DB opens or
  route-owned logged-data projection state.
- Fixed the gateway route table so the Stats view can reach
  `/vms/{id}/stats/detail` through the installed app instead of receiving a
  404, and added a route-health gate that exercises the stats-detail contract
  through both service and gateway.
- Fixed profile shell bootstrap so fresh VMs keep `/opt/ai-clis/bin` on PATH
  ahead of durable `/usr/local/bin` and `/root/.local/bin`, restoring
  out-of-the-box Gemini, Codex, and global npm package-manager diagnostics.
- Fixed service startup projection hydration so incomplete or stale per-session
  ledgers degrade to empty live counters instead of preventing the service from
  accepting routes; snapshot routes remain file-backed and ignore session DB
  activity.
- Tightened the Ironbank route-health gate so profile enforcement and detection
  evaluate routes are benchmarked for latency and CPU along with status/list
  routes, preventing decision-path regressions from hiding outside hot-route
  tests.
- Fixed exec-route latency by moving session-ledger projection refreshes off
  the async route worker and returning command results without waiting for a
  full SQLite projection rebuild; serial provision-to-exec gates now pass at
  about 1s, including the three-live-VM gate.
- Fixed live MCP tool-call security projection so `capsem_mcp_call` emits
  through the existing security DB writer and mirrors the exact
  `security_rule_events` rows into service memory for `/security/latest` and
  `/security/status`, without route-time SQLite reads or a second writer.
- Fixed the session stats Tools tab to count all model-origin tool calls
  (`native`, `builtin`, and `local`) separately from protocol-origin tool
  transport (`mcp`/`mcp_proxy`) instead of presenting an MCP-only activity
  metric.
- Removed retired `capsem-mcp` lifecycle semantics: the host MCP server no
  longer registers `capsem_persist`, no longer sends create-time lifecycle
  flags, and its tool descriptions now reflect profile-owned sessions.
- Fixed the frontend VM provision contract so profile-owned CPU/RAM defaults
  stay service-owned when creating sessions from the dashboard or quick-create
  paths.
- Fixed stats detail drawers so nested event objects are compacted before
  rendering, keeping file/security rows focused on present ledger facts instead
  of showing null-only branches.
- Fixed HTTP/model body recording so compact preview fields stay separate from
  full bounded `event_body_blobs` storage while still using the single
  `DbWriter` security-event/session ledger.
- Fixed the Stats UI ledger path so HTTP, DNS, model, tool, file, process,
  credential, and body-detail rows come from a typed per-session route
  projection instead of sending raw SQL through `/vms/{id}/inspect`.
- Removed the frontend SQL Inspector surface so session UI tabs can only use
  typed route projections instead of arbitrary session database queries.
- Removed the service, gateway, and MCP raw SQL inspection surface so product
  callers cannot route arbitrary session database reads through Capsem.
- Fixed the per-session timeline route so it filters the service's in-memory
  timeline projection instead of opening `session.db` on the request path.
- Fixed the triage route so session-scoped diagnostics are served from a
  route-owned memory projection instead of querying `session.db` per request.
- Fixed the one-shot `/run` route so it stops the session without reopening
  `session.db` to synthesize counters on the request path.
- Tightened the route-authored rule ledger regression so latest security and
  detection routes are asserted against an already-hydrated projection after
  `session.db` has been moved away.
- Fixed Ironbank plugin ledger assertions so hot plugin-list routes stay
  config-only while per-plugin detail routes prove runtime execution counters.
- Fixed profile shell bootstrap payloads so shipped profiles put
  `/root/.local/bin` ahead of `/usr/local/bin`, keeping Claude/Codex/AGY
  user-local CLI installs on the path used by interactive shells and doctor
  checks.
- Fixed the dashboard session list so it refreshes explicitly, keeps long
  session lists scrollable, groups broken sessions below healthy sessions, and
  exposes a route-backed purge action instead of leaving stale broken rows mixed
  into the main list.
- Fixed TUI and tray session creation so default new-session actions route
  through profile-scoped service naming (`code-1`, `co-work-1`, etc.) instead
  of legacy `tmp-*` session creation, while preserving explicit custom names.
- Fixed the tray menu so a stopped service status cannot expose dashboard,
  session list, new-session, or connect actions that would route into dead
  service state.
- Fixed the stats Tools panel so model-native tool calls from active minimal
  `tool_calls` ledgers render from the unified tool table instead of appearing
  empty in AGY/Claude sessions that already recorded the tool rows.
- Fixed TUI terminal responsiveness under bursty keyboard/paste input by
  bounding per-tick input draining and coalescing adjacent terminal bytes before
  websocket sends, matching the browser terminal's coalescing rail.
- Fixed gateway terminal input relay latency by forwarding client keystrokes to
  the VM immediately while preserving coalesced VM-output rendering.
- Fixed Codex sandbox prerequisites in shipped profiles by adding Bubblewrap
  to profile-owned apt packages and capsem-doctor checks, preventing Codex from
  falling back to bundled sandbox helpers because `bwrap` is missing.
- Fixed runtime `/root/.local/bin` shims for curl-installed AI CLIs so
  Claude/AGY doctor checks see the same user-local command path that survives
  the writable `/root` mount.
- Fixed runtime apt HTTPS installs by keeping `/etc` traversable for apt's
  `_apt` sandbox after profile-root projection, and added a capsem-doctor gate
  that fetches and runs a real remote Debian package.
- Fixed default Codex profile seeds so shipped profiles no longer force a
  hidden local Ollama provider or startup update checks, and added release
  doctor/profile-payload guards to keep that drift from returning.
- Fixed shipped AGY/Gemini profile root seeds and image-workspace
  materialization so production profiles cannot silently force local Ollama or
  mock-provider endpoints.
- Fixed tool-call ledger contract tests so tool responses must reference both
  their matching tool-call ID in the same trace and the exact model exchange
  row that consumed the tool response.
- Fixed file-event credential attribution so ordinary model-created files keep
  their trace correlation without inheriting unrelated HTTP/OAuth credential
  references from the same agent turn.
- Fixed the session stats detail drawer so body ledger metadata is separated
  from event fields, long hashes wrap cleanly, and security snapshots render
  compact non-null JSON instead of dense null-heavy projections.
- Fixed the profile plugin list hot path so UI/TUI polling reads cached plugin
  configuration instead of scanning session databases for runtime counters;
  per-plugin detail routes still hydrate live ledger counters.
- Fixed the DNS security ledger unit test so it drains the async DB writer
  before reading joined DNS/security-rule rows, avoiding Linux coverage timing
  flakes without weakening the ledger assertion.
- Fixed Linux CI coverage so the KVM/unit lane has bounded timeout guards
  instead of hanging indefinitely without a named test failure.
- Fixed Linux release-test regressions in the KVM pause/stop harness and PTY
  bridge concurrency proof so CI verifies the intended lifecycle and full-duplex
  contracts without racy or unbounded test behavior.
- Fixed terminal WebSocket burst preservation, deterministic Linux PTY bridge
  multi-chunk transfer coverage, and macOS CI coverage upload prerequisites so
  the release gate no longer depends on runner scheduling, excessive
  coverage-load socket traffic, or missing checksum tools.
- Fixed the IPC schema-mismatch handshake regression test so it keeps the
  responder socket alive until the initiator observes the typed mismatch under
  the macOS nextest runner.
- Fixed the CI install-test asset preparation rail so placeholder initrds are
  valid gzip-compressed cpio images and the macOS bootstrap asset hash suite
  installs `b3sum` before verifying `B3SUMS`.
- Fixed the macOS CI Python coverage gate so it no longer references the
  deleted `tests/test_mcp.py` file and includes the existing image-build
  backend test needed to keep the builder coverage threshold meaningful.
- Fixed runtime profile materialization on CI hosts whose CPU architecture is
  not present in the checked-in asset manifest by falling back to the
  manifest's sole available architecture while keeping explicit `CAPSEM_ARCH`
  overrides strict.
- Fixed stale frontend development guidance that still described the gateway as
  a transparent fallback proxy instead of the explicit route allowlist used by
  the 1.3 UI/TUI contract.
- Fixed package and simulated installs to ad-hoc sign helper binaries with
  stable `org.capsem.*` identifiers, preventing rebuilds from producing
  hash-derived macOS code identities that can trigger repeated authorization
  prompts.
- Fixed the dev service/install asset rail so local service starts materialize
  a real installed-style assets directory and profile catalog instead of
  symlinking `~/.capsem/assets` to the worktree, preventing stale profile pins
  from mixing with fresh assets in the UI.
- Removed the retired global `/vms/list` asset-health payload so service and
  gateway status cannot report flat `vmlinuz/initrd/rootfs` readiness that
  contradicts profile-owned asset status.
- Removed the retired top-level gateway `/status.assets` payload and added an
  Ironbank route guard so profile asset readiness can only come from
  profile-owned asset routes.
- Removed the retired service `ListResponse.asset_health` compatibility field
  so `/vms/list` can only report sessions, never stale global VM asset health.
- Fixed profile launcher cards so missing-asset and ready states expose one
  route-owned action row instead of duplicating `Download`/`Start` in the card
  header and footer.
- Fixed package, Debian, and simulated installs so retired per-user config
  artifacts are removed before the service starts, keeping profile/corp/config
  ownership on the 1.3 rails.
- Fixed package, Debian, and simulated installs so the retired
  `capsem-admin-python` bundle is removed from `~/.capsem/bin`, preventing old
  1.2 builder/keychain code from lingering in installed trees.
- Fixed `capsem shell` input handling so bursty keypresses and paste input are
  drained in one TUI cycle instead of being throttled to one event per 16ms
  redraw tick.
- Strengthened gateway terminal relay coverage so text and binary bursts keep
  their coalesced frame contract across size limits and frame-type changes.
- Fixed the gateway architecture docs and developer skills to state the
  explicit-route/404 contract instead of describing a generic gateway
  forwarding path.
- Clarified the 1.3 manual bug hotlist against the Sprinty release ledger so
  closed work such as body blobs, MCP proof, overlay panic handling, and
  session naming/action fixes are not accidentally reopened during final smoke.
- Gitignored `.env.local` and `.env.ironbank` so live-provider canary
  credentials stay out of source control alongside the default `.env`.
- Fixed gateway startup so an explicit `--run-dir` controls the log, token,
  port, pid, and lock artifacts even when `CAPSEM_RUN_DIR` is set, restoring
  isolated Ironbank gateway logging under parallel release tests.
- Fixed isolated `just test` and `just smoke` cleanup so any test-home
  service started through the release gate is stopped by pidfile on exit,
  preventing failed or interrupted runs from leaving hidden source daemons.
- Fixed the unknown model-endpoint integration gate so it asserts provider
  identity and wire protocol separately: undeclared OpenAI-shaped traffic stays
  provider `unknown` while still recording protocol `openai`.
- Disabled the macOS Keychain-backed credential broker store for 1.3 and
  routed durable credential storage through the same file-backed store used on
  Linux, preventing repeated native credential prompts during normal service and
  TUI use.
- Removed the last runtime credential-store vault namespace vestige so 1.3
  exposes only the file-backed durable credential store, with no Keychain-shaped
  `org.capsem.credentials` runtime contract left behind.
- Fixed the macOS LaunchAgent install contract so installed services explicitly
  pin `CAPSEM_CREDENTIAL_STORE_PATH` to the file-backed credential store.
- Fixed macOS package preinstall so it no longer invokes the previously
  installed `capsem stop` binary, preventing stale Keychain-backed installs from
  prompting before the file-backed 1.3 payload replaces them.
- Fixed macOS package preinstall so it removes stale per-user
  `~/Applications/Capsem.app` bundles, preventing old GUI builds from surviving
  alongside the package-owned `/Applications/Capsem.app`.
- Strengthened the installer and release-doctor credential-store guards so the
  retired Keychain/test-store selector cannot be regenerated by package
  metadata.
- Fixed macOS package and simulated install cleanup so stale
  `~/.capsem/bin.backup*` helpers carrying retired Keychain credential-store
  code are removed and fail the release evidence guard if they reappear.
- Fixed macOS package assembly so companion binaries are rejected before
  packaging if they still carry retired native Keychain credential-store
  markers, preventing stale payloads from reintroducing credential prompts.
- Removed the desktop app's hidden native updater check and switched
  Capsem-owned HTTP clients to webpki roots so startup/status paths do not
  touch macOS platform trust or Keychain APIs outside the service contract.
- Fixed the release docs gate by restoring `just docs` as the single command
  that builds both the docs site and the marketing site.
- Fixed release telemetry docs and developer skills to identify
  `event_body_blobs` as the forensic HTTP/model/MCP body source, with preview
  fields documented only as compact UI display fields.
- Fixed the CLI service-boundary regression guard so its test-only helper no
  longer trips release clippy as dead production code.
- Added a CLI boundary guard proving `capsem stop` and the other service-control
  commands are handled before UDS/service API construction, so they cannot
  depend on profile, status, or credential-store readiness.
- Stabilized credential broker telemetry-hook tests under the full coverage
  gate by waiting for DB-visible ledger rows instead of relying on a
  one-second sleep window.
- Fixed session trace detail reads for older ledgers whose `tool_responses`
  table predates `credential_ref`, preserving tool response inspection instead
  of failing the release fixture gate.
- Fixed the Gemini/AGY stream parser so function-call argument JSON is parsed
  from raw response bytes instead of a normalized serde value, preserving the
  exact tool-call payload that enters the ledger.
- Fixed unknown-endpoint model sniffing so OpenAI/Anthropic/Gemini model names
  infer the provider from the bounded request body while generic compatible
  traffic remains `unknown`.
- Updated the frontend Astro/Svelte integration dependencies to patched
  Astro/Vite versions so the release `pnpm audit` gate is clean during
  `just test`.
- Fixed bootstrap's Colima readiness check so `colima is not running` no longer
  matches the word `running` and skips the Docker VM startup path during
  `just test`.
- Fixed the generated settings/schema rail so it reads the current
  `config/docker/image` authority instead of the removed `guest/config` tree,
  keeping `just test` and CI on the same profile-derived build inputs.
- Fixed CLI status/debug health checks so they use the same `CAPSEM_RUN_DIR`
  socket and gateway files as the service client, preventing source and
  installed runs from checking different Capsem runtimes.
- Fixed the service file API control-channel contract so 1 MiB file
  read/write round trips no longer tear down the guest agent stream, and
  restored the initrd repack path to build guest agents from
  `config/docker/image` instead of the removed `guest/config` tree.
- Fixed `capsem stop` and other service-control commands so they stay pure
  local control operations and no longer start the background update/network
  refresh before dispatch.
- Fixed explicit service stops so installed clients remember the user stopped
  Capsem and refuse to auto-launch the service from status/session requests
  until `capsem start` is run, preventing surprise credential-store hydration
  and Keychain prompts during stop flows.

### Fixed (terminal throughput)
- Coalesced desktop terminal output to one xterm write per animation frame and
  batched bursty terminal input before WebSocket send, preventing high-volume
  agent output from starving keyboard responsiveness.
- Coalesced gateway terminal relay bursts in both directions, so adjacent
  terminal WebSocket/UDS frames are batched without losing byte order while
  preserving a short interactive flush deadline.

### Fixed (session lifecycle)
- Fixed MCP snapshot reverts that reported `action: deleted` through the tool
  result while leaving the created file visible inside the guest workspace.
- Fixed stale persistent sessions whose preserved boot logs show overlayfs
  `Stale file handle` / kernel panic failures so they are reconciled as
  `Defunct`, cannot be resumed, keep the original boot-failure reason in
  route JSON, and are removed by default purge.
- Fixed session ledger inspection for incompatible persistent sessions so
  stats, timeline, and forensic views can still read the preserved
  `session.db` while the session remains non-resumable and delete-only.
- Replaced ad hoc temporary session names with profile-scoped session names
  such as `code-1` and `co-work-1` across service provisioning, the TUI create
  dialog, and the desktop UI, while preserving focus handoff to newly created
  sessions.

### Changed (route surfaces and diagnostics)
- Rewired `/stats` to read the main session ledger through the logger
  `DbHandle::ready/query` path with structured query/error logging, and added
  a guard against route-time `SessionIndex::open` regressions.
- Changed logger DB writes to acknowledge after the DB-owned memory commit,
  then batch-flush dirty memory tables to disk on threshold, interval,
  explicit flush, and shutdown without exposing dirty-set mechanics to routes.
- Clarified the shared agent, testing, debugging, architecture, and Rust
  guidance for the logger DB boundary: routes may own query intent, but only
  the logger DB object owns SQLite execution, connection threads, mem/disk
  layout, batching, flush, rehydration, and schema failures.
- Clarified the release architecture and developer skills so the documented
  service routes use the explicit `/vms/...` contract, VM asset manifests use
  BLAKE3/origin reporting instead of local minisign theater, and `tool_calls`
  is named as the canonical tool ledger while `mcp_calls` is transport
  evidence.
- Added a release compliance gate for SBOM, OBOM, and build-ledger evidence,
  clarifying that OBOMs describe base VM images while build ledgers remain
  debug evidence.
- Renamed the private mock-server implementation and benchmark artifact
  directory so release tests and docs refer to the single reusable
  mock-server/protocol rail instead of retired MITM-local wording.
- Exposed model request/response/tool-call validity facts in serialized
  security events so route JSON matches the first-party CEL model facts used
  by enforcement.
- Added a config-layout gate that makes the settings/corp/profiles/docker/data
  source contract executable and rejects host metadata or generated pins in
  checked-in profile config.
- Moved image build defaults out of checked-in `guest` source config and into
  `config/docker/image`, with `capsem-admin` generating the backend image
  workspace from the selected profile plus Docker image defaults.
- Added an Ironbank Gemini API ledger gate proving public Gemini
  `streamGenerateContent` and `generateContent` traffic through the hermetic
  mock server records Google provider/protocol rows, tool calls, non-stream
  output, brokered credentials, DNS/HTTP evidence, and security decisions.
- Fixed installed asset cleanup so `manifest-origin.json` survives service
  startup, preserving manifest origin/hash reporting while profile asset
  readiness and `capsem update --assets` hydrate through the hash-named asset
  rail.
- Tightened the TUI session contract so profile launch options come only from
  `/profiles/list`, no fallback profile is synthesized from stale session
  rows, and user-facing TUI controls say sessions rather than VMs.
- Removed retired frontend policy vocabulary from settings origins and dead
  network-policy IPC types so profile UI surfaces speak enforcement,
  detection, plugins, MCP, and assets directly.
- Removed the visible frontend build timestamp from the main toolbar; build and
  version evidence remain available through debug/status surfaces.
- Replaced raw toolbar status colors with semantic UI tokens so service chrome
  follows the Capsem design contract.
- Added frontend route-contract gates for the Sessions dashboard and profile
  surfaces so the UI must keep using route-owned profile/session terminology,
  asset readiness, enforcement, detection, plugins, MCP, and canonical detail
  payloads.
- Removed the retired MCP tool `approved` field from profile MCP route
  responses; the UI/TUI contract now exposes only route-backed
  `permission_action` / `permission_source` decisions.
- Cleaned the desktop stats/detail panes so HTTP/model bodies are loaded from
  the blob ledger rather than preview columns, credential broker rows display
  verbs/origins instead of substitution refs, and inspector presets use the
  same broker vocabulary as the session UI.
- Tightened the desktop stats contract so user-facing detail controls say
  session ledger instead of database, and MCP protocol cards no longer surface
  credential-reference counts that belong to the credential broker view.
- Added a service and gateway route-matrix gate for profile UI surfaces so
  `code` and `co-work` profile pages must expose assets, enforcement,
  detection, plugins, credential broker, and MCP routes without 404/501
  fallbacks.
- Fixed gateway forwarding for session snapshot status/list routes and added
  route-contract coverage so the stats UI reads snapshot state through the
  explicit service route instead of hitting a gateway 404.
- Added service-level plugin route contract coverage so profile plugin list,
  info, edit, credential-broker detail, retry, and unknown-plugin responses
  prove the typed pre/post/logging stage surface through UDS.
- Fixed profile plugin edits so `/profiles/{profile_id}/plugins/{plugin_id}/edit`
  persists to the profile file, refreshes route-visible policy immediately, and
  records a `profile_mutation_events` ledger row instead of using a runtime-only
  override.
- Added credential store lifecycle route coverage proving startup hydration,
  explicit broker retry, memory-only hot reads, empty-versus-ready status, and
  raw-secret absence from service/plugin route JSON.
- Tightened the profile plugin UI contract so plugin rows render route-owned
  stage, version, mode, detection level, counters, latency, and broker
  capabilities, while credential inventory uses provider/last-seen/counts
  instead of exposing raw BLAKE references as the primary identity.
- Added service-side snapshot and DbWriter contract coverage proving snapshot
  status/list routes are file/IPC-backed, ignore toxic `session.db` rows, and
  keep per-session SQLite writes on the capsem-process `DbWriter` rail.
- Added a session dashboard route gate proving defunct and incompatible
  sessions remain delete-only across list/status/info/resume/delete routes,
  and cleaned frontend session wording checks so stale VM labels cannot hide in
  test noise.
- Cached profile route summaries in service memory so `/profiles/list` no
  longer reloads profile files or recompiles rule sets on every UI/TUI poll;
  the Ironbank route-health gate now shows profile list p95 in single-digit
  milliseconds with negligible service CPU.
- Renamed the local protocol benchmark internals from the retired
  `mitm-local` escape-hatch wording to the shared mock-server protocol rail;
  `capsem-bench protocol` remains the public command and now emits
  `mock_server_protocol` benchmark JSON.
- Fixed profile route summaries so `code` and `co-work` expose route-owned
  rule, plugin, MCP, and asset metadata without leaking host profile paths or
  falling back to default-only profile assumptions.
- Refreshed the 1.3 benchmark artifacts and docs from the canonical
  `just bench` rail, including mock-server HTTP/protocol throughput plus
  lifecycle and fork timings used by the S05 route-latency gate.
- Hardened the Ironbank HTTP body ledger proof so upstream transcript
  assertions ignore non-HTTP records instead of failing on unrelated DNS
  rows emitted by the hermetic mock server.
- Added strict model wire-protocol recording to the session ledger so model
  traffic can preserve both the endpoint owner (`provider`) and the recognized
  protocol (`protocol`) without collapsing OpenAI-compatible local traffic into
  a fake provider.
- Changed `just bench` to use the artifact-recording release benchmark path
  with the shared local mock server, so HTTP, proxy throughput, and protocol
  benchmarks fail on skips and publish local numbers alongside lifecycle/fork
  artifacts.
- Fixed security decision ledgers so visible default catchall rules remain
  recorded in `security_rule_events` without emitting a second effective
  decision after a more specific profile/corp enforcement rule wins. The code
  and co-work profiles now include an explicit hermetic mock-server allow rule
  for `127.0.0.1:3713`, so doctor, benchmark, and Ironbank traffic does not
  trip the default local-network ask rule.
- Tightened the CEL fact contract exposed by profile enforcement routes:
  evaluate requests now materialize typed `http`, `dns`, `mcp`, `model`,
  `file`, `process`, `ip`, `tcp`, and `udp` facts, default rules include
  unknown-model and unknown-MCP detections, and provider endpoint aliases are
  rejected in favor of explicit `allowed_remote_targets`.
- Fixed Ironbank route contracts for MCP tools and file listings so profile
  MCP routes assert the current permission-action shape and `.txt` uploads are
  reported deterministically as text/plain instead of Magika-dependent
  octet-stream.
- Strengthened `/vms/create` and `/vms/{id}/resume` responses so provision
  routes return the session profile ID, lifecycle state, persistence bit,
  resumability, and valid action enum list alongside the VM ID and UDS path.
  Ironbank route-health now proves create/status/info/list/exec/fork/pause/
  resume/stop/delete/purge state and latency budgets through service and
  gateway routes.
- Strengthened the Ironbank route-health gate so profile enforcement evaluate
  routes must prove exact `allow`, `ask`, and `block` decisions, detection
  rows, and plugin execution stages while keeping hot control-route CPU and
  latency budgets under test.
- Added a first-class `event_body_blobs` ledger for HTTP, model, and MCP
  request/response bodies with a 10 MiB bounded capture, original/stored byte
  counts, BLAKE3 body hash, content type, trace ID, and truncation flag. Stats
  details now load `request_body`/`response_body` from that ledger instead of
  treating preview fields as forensic truth.
- Strengthened the Claude/Anthropic Ironbank ledger proof to cover
  non-streaming HTTP, streaming SSE, and SDK client paths through the same
  model/tool/file/security/broker ledger assertions. Repeated same-path model
  checks now anchor tool rows and tool responses to the current model-call IDs
  and trace IDs so provider proofs cannot pass on stale rows.
- Extended the OpenAI/Codex Ironbank ledger proof to cover Responses,
  embeddings, and image-generation traffic through the same VM/session DB
  path. OpenAI image endpoints are now classified as model traffic and their
  generated payloads are recorded in `model_calls.text_content` while brokered
  credentials remain opaque and raw secrets stay out of DB/log output.
- Strengthened the Codex CLI Ironbank proof so tool-call IDs are derived from
  the per-run nonce and local OpenAI-compatible traffic asserts
  `provider = unknown`, `protocol = openai`, and the unknown-provider
  detection rule instead of relying on stale fixed identifiers.
- Added a host `capsem-mcp` Ironbank proof that exercises the real stdio MCP
  server against `capsem-service`, verifies every advertised tool, calls the
  session/file/exec/MCP/log/triage routes with deterministic inputs, and
  reconciles MCP, file, exec, security, route, snapshot, and structured-log
  ledger output. Host-triggered exec events now carry trace IDs so MCP-driven
  command activity stays attributable through the session ledger.
- Added a reusable Ironbank two-turn model ledger assertion surface that
  computes expected trace/cardinality from externally meaningful client facts
  and proves exactly matched model item, tool call, tool response, file, DNS,
  HTTP, security, credential, and upstream transcript rows through a dedicated
  black-box VM test.
- Removed the remaining network-side HTTP port denial from the MITM path so
  routing/capture mechanics no longer issue security verdicts outside the CEL
  security-event rail. The former `NetworkPolicy` type is now named
  `NetworkMechanics`, and Ironbank now guards old policy-v2, MCP decision,
  fallback logger, side-write, and retired policy authoring strings from
  reappearing in live code.
- Added dedicated Ironbank credential broker and plugin ledger proof. Broker
  coverage now has its own release-gate entry point for capture, brokered
  rewrite, injection rows, and raw-secret absence, while plugin route coverage
  proves profile-scoped list/info/edit, broker inventory/reload, dummy
  pre/post mode changes, serialized security-event detections, plugin
  executions, and evaluation decisions.
- Removed the old settings-tree MCP server rail. Settings metadata and
  settings responses now expose UI/application preferences only, while MCP
  remains profile-owned through `/profiles/{profile_id}/mcp/...` routes.
  Default security-rule catchalls also remain visible in the security ledger
  after specific rules match, so forensic rows show both the specific verdict
  and the late default rule.
- Removed the dead MCP server merge rail that auto-detected host AI CLI MCP
  configs and merged manual/corp/user inputs outside the profile contract.
  Runtime MCP server construction is now guarded to use profile-owned
  `build_profile_server_list()` only, with docs and skills updated to remove
  the stale fallback language.
- Renamed the MCP configuration contract from `McpUserConfig` to
  `McpProfileConfig` and added a no-legacy guard so profile/corp-owned MCP
  config cannot regress to user-config terminology.
- Hardened profile parsing so `assets` is a required profile-owned section
  instead of silently defaulting to the first built-in profile's asset release.
  Profile contract and admin profile-check tests now prove malformed profiles
  cannot inherit Code assets by omission.
- Aligned the shared settings conformance fixture with the 1.3 contract that
  settings are UI/application preferences only. Python, Rust, and frontend
  settings schema tests now reject stale AI-provider, credential, profile-file,
  and `enabled_by` provider surfaces instead of requiring them.
- Split model wire protocol from endpoint-provider identity so Ollama,
  OpenAI-compatible, Anthropic-compatible, and unknown model endpoints can be
  parsed without pretending protocol and provider are aliases. Recognized model
  protocol traffic on undeclared endpoints now emits `model.provider =
  "unknown"` and hits a default informational detection rule.
- Fixed local model enforcement so explicit profile/corp allow rules win over
  the built-in local-network `ask` default while the default rule remains
  visible in the security ledger. Model request/response events now carry the
  same `tcp.port`/`ip.value` transport facts as HTTP events, and Ironbank
  proves UDS and HTTP latest routes expose the same unknown-provider detection
  row.
- Tightened credential brokerage for unknown OpenAI-compatible and
  Anthropic-compatible model endpoints: `Authorization` and `x-api-key` headers
  are brokered from protocol/header shape without relabeling the provider, and
  async file attribution keeps the first credential seen for a trace.
- Fixed the AGY hermetic replay fixture so Google Code Assist
  `listExperiments` matches the recorded 68 experiment IDs and 250 flags, and
  `/log` accepts protobuf play-log telemetry with the recorded empty text/plain
  acknowledgement instead of fake JSON.
- Refactored the Ironbank model-client proof into composable script-builder
  and ledger-assertion helpers, and made the Codex CLI fixture use the same
  brokered OpenAI credential path as the SDK/API clients instead of a
  non-secret marker shortcut.
- Tightened the shared Ironbank AI-client harness so every credentialed model
  client proof must show broker capture, brokered request rewrite, one shared
  `credential_ref` across HTTP/model/tool-call/tool-response/file rows, exact
  substitution ledger verbs, and raw-secret absence from DB/log output. The
  OpenAI API, OpenAI two-turn, Codex CLI, Claude HTTP, and Claude SDK proofs
  now all run through that same broker contract.
- Tightened the OpenAI-compatible Ironbank double-turn ledger so repeated
  model history is deduplicated by persisted BLAKE3 item hashes, model tool
  calls register workspace file-path trace hints, and subsequent fs-monitor
  events plus security-rule rows are attributed to the same model trace. The
  focused proof now asserts two random tool calls produce exactly two traces,
  ten model item rows, four model calls, four HTTP rows, one DNS row, two tool
  calls, two tool responses, and two created file events.
- Tightened the HTTP Ironbank ledger path so active profiles carry corp network
  mechanics into `capsem-process`, HTTP security events expose `http.query`,
  `http.body`, `tcp.port`, and `ip.value` to CEL and forensic rows, and the
  first plain-JSON HTTP full-chain test reconciles client output, upstream
  transcript, `net_events`, `security_rule_events`, UDS inspect, gateway
  inspect, timeline, security status/latest, VM status counters, and structured
  service/gateway logs.
- Fixed blocked HTTP telemetry so CEL-denied requests now keep request byte
  counts, request previews, and client-visible denial response previews in the
  same ledger path as allowed requests, with Ironbank proof that the denied
  request never reaches the upstream fixture.
- Fixed pending HTTP `ask` decisions so clients see an approval-required 403
  instead of a generic block message, while Ironbank proves the pending
  `security_ask_events` lifecycle row, `policy_action = ask`, security status,
  UDS inspect, gateway inspect, counters, and logs all agree.
- Fixed brokered HTTP credential rewrite accounting so OAuth captures emit
  exact `captured`/`brokered`/`injected` ledger verbs, broker refs replay into
  upstream header/query bytes without leaking raw credentials to DB, routes, or
  logs, and credential inventory merges injected rows with their captured
  provider identity. Grouped CEL rule matches such as `a && (b || c)` now
  compile through the same profile rule path used by the HTTP rewrite proof.
- Changed the credential broker durable store to the same file-backed backend on
  macOS and Linux for 1.3, so service startup/reload hydrates captured
  credentials without native credential prompts.
- Tightened HTTP body-handling ledger proof for gzip, chunked, SSE, truncated
  preview, and HTTPS override traffic. Decoded gzip responses now log the same
  materialized headers and body bytes delivered to the guest instead of stale
  compressed response metadata.
- Added DNS Ironbank ledger proof for allowed and blocked UDP DNS traffic.
  Allowed DNS rows now carry the matched security rule and policy fields just
  like blocked rows, hermetic DNS upstream transcripts prove blocked
  exfiltration never leaves the VM boundary, and security status exposes
  detection-level counters regenerated from `session.db`.
- Added MCP Ironbank ledger proof for profile-owned builtin MCP and observed
  remote MCP traffic. MCP security events now carry request arguments,
  response content, trace IDs, and transport facts through CEL, DB rows, UDS
  inspection, gateway inspection, latest/status routes, and structured logs.
- Added Ironbank file/process/snapshot and package-manager ledger proofs.
  The new black-box coverage exercises file import/export/create/modify/delete
  rows, symlink escape rejection, process audit versus exec semantics,
  snapshot route hermeticity, package-manager functional probes, route
  serialization, and DB-backed security rows.
- Tightened Ironbank model/client coverage so the mock server replays an
  Ollama-compatible OpenAI chat-completion shape with native tool calls, the
  OpenAI SDK/Anthropic SDK/LiteLLM/Ollama SDK/Codex CLI paths assert full
  model, HTTP, security, file, exec, credential, and session DB ledger fields,
  and the tests now fail on any public HTTP or DNS side traffic. This caught and
  closed Codex plugin/OTLP side calls and LiteLLM's default public cost-map
  fetch during hermetic release proof.
- Added a full mock-server JSONL request ledger and upgraded the Codex CLI
  Ironbank proof to drive the OpenAI Responses API through a native
  `exec_command` tool call, require Codex to write a random UUID4 hex value to a
  random filename, return only the successful tool status to the model, and
  reconcile exact HTTP bodies with
  `model_calls`, `tool_calls`, `fs_events`, `net_events`, and
  `security_rule_events`.
- Upgraded the mock server and Ironbank launcher proof for
  `ollama launch claude`: the mock now replays Anthropic streaming `tool_use`
  and final-message SSE shapes, structurally detects real `tool_result` blocks,
  and the ledger proof covers Claude's real `Bash` tool call, tool response,
  token usage, file write, HTTP/model rows, DNS, and security rules. AI request
  capture is now bounded at 1 MiB by default so large real agent continuations
  are parseable instead of clipping away trailing tool results.
- Tightened the config authority guard so `config/` can only contain the
  declared `settings/`, `corp/`, `profiles/`, `docker/`, and `data/` roots;
  active docs and skills now explicitly reject admin/default/guest/preset/
  registry/template roots, clarify that settings have schemas while profiles
  have catalogs, and describe `capsem-admin` as a validation/materialization
  tool rather than a product authoring surface.
- Tightened the profile-derived image/config contract in docs and developer
  skills: `config/` is now documented as settings/corp/profiles/docker/data,
  `capsem-admin` is explicitly a validator/materializer/build tool rather
  than a config authority, stale `guest/config` authoring and source-profile
  pin language is removed from active docs/skills, and `capsem-admin image
  build --dry-run` is no longer a public product rail. The internal settings UI
  metadata parser no longer calls itself a registry, preserving the rule that
  profiles and corp own runtime truth while settings only describe
  UI/application preferences; private capsem-admin scaffold helpers are now
  burned by a guard test too.
- Burned the public `capsem-builder build`, `validate`, `inspect`, `mcp`, and
  `--dry-run` rails so product image/config work can only enter through
  profile-owned config plus `capsem-admin`; docs, skills, and CLI tests now
  document and enforce `capsem-builder` as a backend helper only.
- Kept profile image builds behind the `capsem-admin image build` rail while
  moving Docker/template execution to a private Python backend module, and
  tightened partial asset generation so rootfs-only or kernel-only outputs
  cannot mint a bootable manifest or delete unrelated arch assets.
- Fixed PR CI Python coverage so the schema/builder coverage step runs the
  explicit Python contract suite that exercises `src/capsem`, instead of
  replaying VM, serial, install, MCP, service, and Ironbank suites under one
  monolithic `pytest tests/ --cov` command; the gate now also covers malformed
  dev skill frontmatter, symlink, empty-root, and bad-entry cases so remote
  runner coverage drift no longer drops the Python gate below threshold.
- Fixed PR CI non-VM Python integration setup so bootstrap, codesign, and
  rootfs artifact tests generate their ignored local test assets through
  `capsem-admin`, build the exact debug host binaries under inspection, and
  ad-hoc sign them with the canonical entitlement before asserting the package
  and signing contracts.
- Fixed PR CI frontend coverage by moving generated settings/mock fixture
  creation onto a shared `scripts/generate-settings.sh` rail, running that rail
  before frontend build/check in CI, declaring the Vitest coverage provider,
  uploading the actual `frontend/coverage/coverage-final.json`, and excluding
  generated coverage output from later frontend type checks.
- Fixed PR CI Rust coverage so `cargo llvm-cov` reports and uploads coverage
  without aborting the rest of the release gate on a local percentage
  threshold; Codecov remains the coverage ledger while Python, frontend,
  schema, cross-compile, and artifact checks now still run.
- Fixed the Docker install e2e package path so Linux `.deb` repacking
  materializes profile-owned runtime config before copying profiles into the
  package, using the same shared materializer as local dev recipes instead of
  assuming `just` exists inside the package-test container.
- Fixed Docker install e2e asset bootstrap so the ignored local `assets/`
  working tree is prepared with tiny test boot files and a `capsem-admin`
  generated manifest before profile materialization.
- Fixed CI regressions where macOS Rust coverage compiled the Tauri app before
  `frontend/dist` existed, and Linux ARM agent exec tests selected `/root` as
  cwd for a non-root runner user simply because the directory existed.
- Fixed ARM Linux CI compilation for KVM checkpoint tests by keeping portable
  checkpoint header decode coverage on every target while gating x86 KVM vCPU,
  IRQ, PIT, and MMIO serialization tests to x86_64 where those structs exist.
- Fixed CI release gates so Rust coverage no longer references the deleted
  `capsem-debug-upstream` crate and Python lint validates the top-level
  `skills/` library instead of the retired `config/skills` path.
- Made the credential broker memory-first behind an opaque `CredentialStore`:
  captures update runtime memory before durable storage, replay/status checks
  no longer hit Keychain or disk, real substitutions can hydrate on cache
  miss, service `/status` reports only ready/degraded state, and
  `/profiles/{id}/plugins/credential_broker/credentials/{info,reload}` exposes
  the detailed broker store object plus explicit retry.
- Routed the profile-scoped credential broker retry endpoint through the HTTP
  gateway and pinned it in the explicit route allowlist so the UI cannot see a
  404 for a service-supported profile/plugin operation.
- Added a real-service gateway contract test for the profile overview route
  bundle so profile info, credential broker status/retry, asset status,
  enforcement rules, and detection rules must all survive the HTTP gateway with
  the UI-facing JSON field shape intact.
- Extended file-boundary IPC so plugin `rewrite` decisions can return mutated
  bytes to the service for import/export/read/write boundaries; the service
  now writes or returns only the bytes approved by the plugin-aware security
  rail, while block still fails closed.
- Fixed file-boundary rewrite materialization so logging-stage sanitizers and
  large-content security previews cannot truncate or replace guest file bytes;
  data-plane rewrites now require a complete payload and an applied
  non-logging `rewrite` plugin.
- Fixed the Linux installed-package build by scoping the Keychain credential
  index type to macOS, keeping the non-macOS credential store warning-clean
  under the package e2e `-D warnings` gate.
- Tightened plugin route regression coverage so `rewrite` mode proves an
  actual event mutation and `block` mode remains the only plugin mode that
  denies the evaluated security event.
- Tightened Ironbank plugin matrix coverage so postprocess plugin detections
  must appear in the security event detection vector, closing the explicit
  allow/ask/block/disable/rewrite/pre/post/detection-level proof item.
- Removed fake confidence from broker-created credential observations and
  injections; substitution rows keep the historical nullable column, but
  broker emissions now record `NULL` confidence.
- Hardened file import/export security boundaries so explicit file writes run
  through the plugin-aware security rail, plugin `block` decisions deny the
  VM-facing file operation before bytes are written or returned, and profile
  plugin edits reload matching active VMs before returning. Ironbank now proves
  the denied EICAR import, live plugin disable, allowed import, and exact
  session DB plugin decision/execution ledger.
- Split security plugins into explicit preprocess, postprocess, and logging
  stages while preserving the single `SecurityEvent -> SecurityEvent` plugin
  contract; the credential broker now owns credential observation/storage as a
  security plugin, and the log sanitizer owns the ledger-safe projection before
  emission. The profile/corp plugin policy and route-visible plugin catalog now
  expose all three stages instead of hiding logging plugins behind a
  compatibility bucket.
- Renamed the core security plugin stage contract to
  `preprocess`/`postprocess`/`logging` and extended the security action
  benchmark matrix to cover all three plugin kinds, including the logging
  sanitizer.
- Extended credential broker replay so broker refs in HTTP headers or queries
  are treated as preprocess injection events, materialized only for upstream
  runtime bytes, and recorded in the substitution ledger as `injected` without
  leaking raw secrets or broker refs through sanitized header payloads.
- Expanded the Ironbank credential broker ledger proof to cover query replay,
  JSON request bodies, form request bodies, OAuth response token bodies, and
  generic credential response bodies through the real VM path and hermetic
  mock server.
- Added route-visible plugin execution counters and latency totals for
  security plugins, and moved MITM rule-ledger emission onto the plugin-aware
  security event path so broker and log-sanitizer executions are preserved in
  session DB forensic payloads and `/profiles/{id}/plugins/list`.
- Documented the runtime-vs-ledger materialization split across security
  policy, network isolation, MITM architecture, and developer skills so future
  work keeps credential capture/injection in the broker plugin and ledger
  materialization in logging plugins instead of network formatters, routes, DB
  readers, frontend transforms, or test harnesses.
- Hardened the local OpenAI-compatible model path: bounded request sniffing now
  promotes unknown localhost model traffic before CEL/plugin evaluation, the
  credential broker uses the parsed provider hint for SDK bearer headers, and
  Ironbank proves the VM-visible OpenAI SDK response, tool call, file write,
  broker reference, substitution ledger, route counters, raw-secret absence,
  explicit model allow rules, and the default local-network `ask` guard end to
  end.
- Removed provider-aware credential brokering from MITM header formatting so
  network helpers no longer create credential refs or credential observations.
- Replaced the Rust mock-server crate with the shared Python mock server
  runtime for doctor, integration, recorder, benchmark, and Ironbank tests, so
  there is one hermetic protocol lab and no duplicate fixture implementation.
- Extended `capsem-mock-server` with deterministic DNS fixtures over UDP and
  TCP, reported in its ready JSON, so doctor, recorder, benchmark, and
  Ironbank work can exercise DNS without public resolvers or a second fixture
  server.
- Extended `capsem-mock-server` with a real local HTTPS listener that serves
  the same deterministic fixtures as HTTP, giving doctor, recorder, benchmark,
  and Ironbank work one protocol lab for HTTP, HTTPS/MITM, DNS, SSE,
  WebSocket, MCP, OAuth, and model replay.
- Extended the protocol fixture recorder to capture and replay DNS fixtures
  from `capsem-mock-server`, keeping DNS in the same sanitized fixture corpus
  as model, MCP, OAuth, credential, and HTTP-like flows.
- Removed the env-gated local MITM benchmark skip from the serial release
  tests and restored its default load to 50,000 requests at concurrency 64, so
  `just test` always produces meaningful local HTTP/SSE/WebSocket MITM
  baseline numbers through the shared mock server.
- Hardened the in-VM network doctor so missing or unroutable
  `CAPSEM_MOCK_SERVER_BASE_URL` fails the local HTTP/SSE/WebSocket/OAuth/model
  proof instead of silently skipping deterministic protocol coverage.
- Clarified the shared skills contract for profile `build.sh`: it is a
  rootfs-only build hook, not an installer/runtime/config path, and changes
  require profile descriptor updates, asset rebuilds, and black-box VM proof.
- Routed service-initiated profile MCP tool calls through the logged MCP
  JSON-RPC security rail instead of calling the aggregator directly, so
  `capsem_mcp_call` now writes `mcp_calls`, built-in MCP HTTP `net_events`,
  and matching `mcp.tool_call` security-rule rows through the process
  `DbWriter`.
- Added an Ironbank-native profile MCP ledger proof for `capsem_mcp_call` that
  drives `capsem-mcp`, profile MCP routes, a fresh VM, the shared mock server,
  and read-only session DB checks in one black-box release gate.
- Hardened agent bootstrap packaging: profile build hooks now remove
  installer-created OAuth/token/history/cache/log residue before rootfs
  packaging, AGY runs through the Capsem sandbox wrapper by default, and Gemini
  is wrapped without copying its npm entrypoint so relative JS chunk imports
  still work. Ironbank now boots a fresh VM and proves AGY, Claude, Codex, and
  Gemini bootstrap commands plus route/session ledgers from the outside.
- Extended the Ironbank model ledger proof to drive real Anthropic, LiteLLM,
  and native Ollama Python SDK clients through the shared mock server, and
  fixed native Ollama `/api/chat` classification so session DB rows, security
  ledgers, route output, token counts, byte counts, and file writes agree.
- Extended gateway `/status` to preserve the service profile catalog and
  installed asset manifest provenance, including profile readiness, manifest
  origin/source/hash, validation status, and current asset/binary versions.
- Included installed asset manifest provenance in support bundles so debug
  reports preserve the manifest origin/source/hash trail alongside the active
  asset manifest.
- Extended support-bundle debug diagnostics with the current profile route
  inventory and profile OBOM descriptors, including `/profiles/{id}/obom`,
  BLAKE3 hash, generator metadata, size, and base-image scope.
- Added support-bundle supply-chain references for the host SPDX SBOM release
  artifact, GitHub attestation source, profile CycloneDX OBOM routes, and
  manifest provenance paths.
- Hardened package artifact tests so local and remote manifest overrides prove
  the packaged manifest payload and `manifest-origin.json` provenance instead
  of only checking installer script text.
- Added the manifest file BLAKE3 to `capsem-admin manifest check --json` and
  logged manifest report/provenance events during package postinstall.
- Tightened the Ironbank doctor ledger gate so local-network `ask` decisions,
  informational detections, serialized detection payloads, and security plugin
  execution timings are proven from session DB rows instead of only counted.
- Renamed the deterministic local fixture upstream to `capsem-mock-server` and
  made `CAPSEM_MOCK_SERVER_BASE_URL` the shared contract for doctor,
  integration, recorder, benchmark, and Ironbank-style black-box tests.
- Added an Ironbank package-manager ledger proof that boots a VM through public
  service routes, verifies apt, npm, uv, pip, and node packages perform real
  work, and audits session history plus `exec_events`/`fs_events` fields.
- Hardened VM fork cloning so `session.db` is snapshotted through SQLite
  instead of copied as a raw file. Forks of forks now preserve WAL-backed
  committed ledger rows as a standalone quick-check-clean database, preventing
  boot failures from malformed copied session DBs.
- Hardened Apple VZ suspend/resume and benchmark gates: checkpoint files now
  require an fsynced completion marker before a VM can be considered
  suspended, save/restore remain exclusive across service workers, cold starts
  stay concurrent, and timing probes run isolated after the `-n 4` integration
  canary so published boot/lifecycle numbers remain meaningful.
- Replaced fork-package proof in MCP and lifecycle benchmarks with a hermetic
  local `.deb` probe installed through the public VM file/exec routes, so fork
  preservation no longer depends on public `apt` repositories while still
  proving rootfs overlay package state survives the fork.
- Pointed the injection test runner at the materialized profile catalog and a
  short `/tmp` CAPSEM_HOME so injection scenarios exercise package/CI-style
  profile config without tripping macOS Unix-socket path limits.
- Made `doctor --fix` rebuild VM assets for every checked-in profile through a
  named profile loop instead of a default-only asset build, with a release
  contract test guarding the recipe.
- Aligned support-bundle and gateway test fixtures with the current
  profile/settings layout and VM `available_actions` contract, and cleaned up
  Rust formatting debt from the release cleanup branch.
- Hardened profile routing assumptions by passing the full release gate under
  temporary arbitrary profile ids before restoring the shipping `code` and
  `co-work` profile identities. This keeps profile-aware routes, UI/TUI
  helpers, admin materialization, and install packaging from silently depending
  on a single hardcoded profile.
- Added a real checked-in `co-work` profile as source profile data, and
  tightened Profile UI/TUI/service tests so profile-aware surfaces consume
  route-provided profile ids instead of silently falling back to `code`.
- Advanced the 1.3 release metadata to `1.3.1781205836`, pinned the frontend
  `esbuild` override through the lockfile, and archived fresh lifecycle, fork,
  in-VM storage, and parallel benchmark ledgers for the current build.
- Fixed the gateway profile MCP surface so the UI/TUI route for reading and
  editing a profile's default MCP permission forwards to the service instead
  of returning a route-level 404.
- Moved dashboard session creation controls onto each profile card: ready
  profiles expose a primary `New` action, profiles with missing assets expose
  `Download`, and `Customize` opens the session dialog preselected to that
  profile.
- Added a compact route-backed VM asset checklist to each profile launcher
  card so users can see which kernel/initrd/rootfs assets are present or
  missing before starting or downloading a profile.
- Fixed dashboard session actions so incompatible or defunct sessions remain
  non-openable and expose only the delete action even if a stale status payload
  includes start, resume, or fork actions.
- Tightened the MCP profile UI so default and per-tool permission controls use
  the same typed allow/ask/block option list as the route contract.
- Fixed credential broker stats so captured, brokered, injected, and error
  events are counted independently instead of treating every broker row as a
  captured credential.
- Made credential capture write the full durable verb trail: observed secrets
  now emit `captured` and `brokered`, while replayed references emit
  `injected`.
- Fixed the hermetic credential broker test store so concurrent captures cannot
  corrupt the store or lose refs before replay.
- Added Ironbank coverage for unknown-host OpenAI-compatible body-shape
  detection: neutral-path model traffic now proves model rows, broker refs, and
  detection-rule ledger output.
- Added Ironbank coverage for unknown remote MCP-over-HTTP JSON-RPC activity:
  observed initialize/list/tool-call traffic now proves MCP DB rows, timeline
  route evidence, and `mcp.tool_list`/`mcp.tool_call` security ledger entries.
- Added Ironbank coverage for declaration-only model tools: an
  OpenAI-compatible request may advertise tools without creating executed
  `tool_calls` rows unless the model response actually emits a tool call.
- Tightened Ironbank tool-call ledger coverage so executed model tool calls
  must have exact row counts, declaration-only tools stay absent, and observed
  MCP `tools/call` rows correlate by trace and tool name without protocol
  chatter becoming phantom executions.
- Added Ironbank coverage for Gemini/Google and Claude/Anthropic streaming
  model traffic through hermetic SSE fixtures, proving client-visible bytes,
  parsed model rows, security-ledger entries, and brokered API-key references.
- Fixed the credential broker so Google `x-goog-api-key` headers are captured
  as Google credentials even before a provider hint exists.
- Hardened profile root bootstrap packaging: `capsem-admin profile check` now
  rejects unpinned files under a profile root seed, profile payload tests prove
  AGY/Claude/Codex/MCP non-secret bootstrap files are pinned exactly, and
  OAuth tokens, logs, conversations, history, and cache payloads cannot be
  baked into checked-in profile roots silently.
- Tightened the VM Stats Process panel so it reports command executions and
  observed processes as separate ledgers, replaces the unrelated credential-ref
  counter with unique binary counts, and removes tutorial prose from the app UI.
- Made Stats detail payload rendering content-aware: HTTP header fields use an
  HTTP grammar, JSON previews are parsed and formatted as JSON, and non-JSON
  payloads stay as escaped text instead of being forced through a JSON view.
- Cleaned up Profile overview credential inventory so it shows provider,
  last-seen, observed, and injected counts without rendering raw broker
  credential references in the primary UI.
- Moved frontend MCP controls off settings-backed `mcp.servers.*` mutation and
  onto profile-scoped MCP routes. Settings now stays focused on UI/app
  preferences, while the Profile surface owns rules, plugins, MCP, and assets.
- Moved `capsem-process` and the built-in MCP server onto the materialized
  runtime profile directory. Runtime rules, plugins, MCP, model endpoints, and
  service-supplied corp overlays now load from the profile contract instead of
  global settings/user config files.
- Updated the Sessions launcher to render profile-owned icon/name/description
  from `/profiles/list`, check assets per profile, show a download action while
  assets are missing/downloading, and pass the selected `profile_id` on VM
  creation.
- Unified the frontend VM list around one profile-owned VM model: profile
  launches, keyboard creation, and the custom VM dialog now create named
  retained VMs, and both the list and active-VM toolbar expose pause/resume,
  stop/start, fork, and delete without temporary-vs-persistent UI branches.
- Rebuilt the VM Stats tab around the current session database and VM-scoped
  ledger routes. It now surfaces Model, MCP, HTTP, DNS, Files, Process,
  and Security evidence, links directly to raw session DB inspection, and uses
  DB-backed security/detection/enforcement rows for forensic details. Hypervisor
  snapshot internals no longer appear as a generic Stats tab; explicit snapshot
  MCP calls still surface through MCP activity, but host snapshot state is no
  longer written to or exposed from `session.db`.
- Hardened the black-box integration gate so credential-broker tests use an
  isolated file-backed broker store instead of the developer's native keychain,
  and bounded the VM model fixture call so model/credential regressions fail
  quickly with ledger evidence instead of hanging the release test.
- Hardened the integration service startup wait so a clean `capsem-service`
  idempotent exit during a compatible peer-start race keeps probing the UDS
  route instead of failing the release gate before `/list` becomes ready.
- Isolated each integration gate invocation under its own test CAPSEM_HOME so
  focused and full runs do not share stale service sockets, pidfiles, or broker
  stores; `CAPSEM_INTEGRATION_HOME` remains available as an explicit debug
  override.
- Pinned integration-test `CAPSEM_RUN_DIR` and `capsem-service --uds-path` to
  the same process-scoped runtime directory so inherited test environment
  cannot redirect service startup to a foreign singleton socket.
- Made package postinstall hydrate VM assets through `capsem update --assets`
  after copying the selected manifest/profile ledgers. Local dev/corp manifests
  now use `manifest-origin.json` to hydrate from the source asset tree with the
  same hash-named layout and blake3 verification as remote downloads, while the
  package payload remains free of rootfs/initrd/kernel blobs.
- Made `bootstrap.sh` frontend dependency installation non-interactive by
  running `pnpm install` with `CI=true`, matching the full test gate contract
  and avoiding TTY-only confirmation prompts during unattended bootstrap.
- Added VM-scoped snapshot status/list routes backed by the running
  `capsem-process` in-memory snapshot scheduler. Stopped VMs reconstruct
  snapshot status from that VM's snapshot metadata only when requested, and
  migrated session databases drop the old `snapshot_events` table.
- Compact `snapshots_list` output now defaults to created/edited/deleted counts
  so AI-facing MCP responses stay small; callers must pass
  `include_changes=true` to request full per-file snapshot diffs.
- Hardened workspace snapshot storage so capture, compaction, deletion, and
  eviction refuse to operate when snapshot storage or a slot resolves inside
  the live workspace. Regression tests prove snapshot capture/compaction leave
  live workspace entries unchanged and reject symlinked storage back into the
  workspace.
- Hardened `snapshots_revert` against symlink escape/pull-in regressions:
  restore now rejects symlinked parent components in checkpoint storage, avoids
  following live workspace symlinks during no-op checks, and reads regular
  snapshot sources with no-follow file opens. Regression tests cover the old
  “symlink out of workspace, pull outside file bytes into restore” class.
- Clarified the VM Stats process tab by separating command execution rows from
  audit-port process observations, removing the vague “Process Audit Events”
  label from the user-facing table.
- Updated public architecture docs and internal development skills to reflect
  the 1.3 contract: profile-owned assets/rules/MCP/plugins, settings as UI/app
  preferences only, explicit gateway routes, ledger-backed Stats/Inspector,
  and the single SecurityEvent/CEL rule rail.
- Added a `capsem debug` CLI alias for redacted support bundles and expanded
  `capsem status` with profile catalog readiness and corp config
  presence/source/hash information when the service is running.
- Expanded `capsem debug` support bundles with a machine-readable runtime
  boundary contract covering first-party host VSOCK services, explicitly closed
  raw ports, and diagnostic/status routes for bug reports.
- Updated package installation diagnostics: macOS and Linux package scripts now
  write a durable `~/.capsem/logs/install.log`, package builders accept local
  paths plus `file://`, `http://`, and `https://` manifest overrides, and
  service status reports the installed manifest hash and package provenance.
- Hardened macOS `.pkg` and Linux `.deb` package composition so closed
  packages contain the app/binaries, profile config, and selected
  `manifest.json`/`manifest-origin.json` only; VM asset payloads are never
  embedded and are reconciled by the service from the installed manifest.
- Reorganized checked-in config source into `config/settings`, `config/corp`,
  `config/profiles`, `config/docker`, and `config/data`, documented the layout,
  and made source profiles unpinned by contract. `config/settings` owns only
  UI/application preferences; profile/corp own runtime behavior.
- Added per-install timestamped logs under `~/.capsem/logs/install-*.log` plus
  `install-latest.log`, while preserving the aggregate `install.log`.
- Expanded manifest status reporting with mutable-manifest semantics:
  `/profiles/status`, `/profiles/{id}/assets/status`, and CLI status output now
  report the current manifest hash, source, refresh timestamp, and validation
  result instead of treating the install-time hash as immutable.
- Hardened doctor/Ironbank diagnostics so credential-shaped model and OAuth
  probes no longer place synthetic secrets in process argv, and removed the
  guest `shutdown` sysutil alias now that VM shutdown is owned by the TUI.
- Made `capsem-admin manifest generate <assets_dir>` the documented manifest
  production rail for local, release, and corp custom builds; package builders
  consume the selected manifest but no longer document or rely on direct
  generator internals.
- Added a route-backed frontend debug snapshot:
  `window.__capsemDebug.snapshot()` now returns frontend version/log context,
  websocket tail, gateway status, profile catalog status, and corp info for
  pasteable bug reports.
- Updated the session UI to display each VM's backend-provided `profile_id` and
  replaced hard-coded About runtime/kernel claims with live diagnostic status.
- Updated the Profile overview to render route-backed surface availability
  (web, shell, mobile) and broker-visible credential inventory/grant status, so
  profile readiness is visible before users dig into Plugins or raw stats.
- Removed the mistaken checked-in `config/skills/` mirror and restored
  repository `skills/` as the developer skill source; profile/product skills
  must be introduced through the profile ledger instead of a global config
  escape hatch.
- Moved the code profile ledger to `config/profiles/code/profile.toml` and
  materialize generated/installed profiles with the same directory shape, so
  source and runtime config use one profile path contract.
- Added profile-owned VM base-image OBOM evidence: materialized profiles can
  pin `obom.cdx.json` with BLAKE3 hash, size, cdxgen generator metadata, and
  the rootfs hash it describes, and `/profiles/{id}/info` plus
  `/profiles/{id}/obom` expose that base-image-only contract.
- Added profile-owned image payload declarations for the code profile: MCP
  config, apt/Python/npm package lists, build-time hook script, tips, and
  packaged guest-root seed files are now declared from `profile.toml`.
  `capsem-admin profile check` verifies those source payloads plus the root
  seed manifest, and `capsem-admin image build` materializes a pinned,
  self-contained generated guest workspace before invoking the backend builder.
- Renamed profile image hooks from `install.sh`/`files.install` to
  `build.sh`/`files.build` and added Ollama to the shipped Code and Co-work
  profile images through that builder rail, with `zstd` included for the
  official Ollama installer.
- Pruned Ollama CUDA libraries from profile-built images and added the Python
  Ollama SDK to Code and Co-work profiles so local Ollama client tests do not
  require ad-hoc VM package repair or waste guest disk on unused GPU payloads.
- Added non-secret Claude MCP approval state to Code and Co-work profile roots
  so fresh profile-built sessions do not prompt users to trust the built-in
  `capsem` MCP server before agents can use it.
- Added OpenAI, Anthropic, and LiteLLM Python SDKs to the Code and Co-work
  profile package ledgers so Ironbank real-client model tests can run from the
  VM without ad-hoc guest installs.
- Added an Ironbank `capsem-doctor` ledger proof that boots a VM through public
  service routes, runs the hermetic mock protocol lab, and verifies HTTP, DNS,
  MCP, model, tool-call, file, exec, security-rule, and credential broker rows
  agree in `session.db`.
- Made the VirtioFS doctor pip probe hermetic by installing a generated local
  wheel with `--no-index` instead of reaching out to PyPI for `cowsay`.
- Expanded per-architecture VM build ledgers with a `rootfs.config_inputs`
  stage that records declared package config, rendered rootfs install inputs,
  profile root/build-script inputs, and EROFS settings. Installed package
  names and versions remain OBOM evidence, not build-ledger claims.
- Cleaned active architecture/development docs and internal skills around the
  profile/admin image contract: public guidance now points at profile-owned
  package/MCP/rule/root files, generated `target/config`, `capsem-admin image
  build`, build ledgers, and OBOM evidence instead of retired builder
  scaffolding or image-owned provider configuration.
- Added the first profile mutation rail: enforcement and detection rule files
  are now profile-owned files, `Profile` owns core status/check/download and
  MCP tool permission mutation, backend-managed rules carry typed ownership
  annotations, and profile mutations have a DB-writer ledger event.
- Wired service profile routes onto that rail: profile status now verifies
  pinned profile files plus asset hashes, profile asset ensure repairs corrupt
  hash-prefixed assets, MCP tool permission edits write managed profile
  enforcement rules and profile mutation ledger rows, and enforcement/detection
  route listing and authoring compile from profile files plus corp overlays
  without reading or writing user settings.
- Made MCP tool permissions round-trip through the same profile enforcement
  contract: tool list responses now include the effective `allow`/`ask`/`block`
  action and source rule, the frontend edits tools with `{ action }` instead of
  the retired `{ approved: true }` cache shape, and unsupported server
  add/toggle/delete controls are no longer exposed in the MCP UI.
- Clarified MCP builtin display semantics: the profile-owned `local` Capsem MCP
  entry is rendered as built-in capability, not as a stopped external server,
  and frontend runtime counts exclude static builtin MCP entries.
- Split the Profile UI's retired generic `Policy` section into explicit
  `Enforcement` and `Detection` route-backed tabs, with a frontend contract
  test guarding against reintroducing the old policy tab.
- Replaced the Profile UI's raw asset JSON dump with a route-backed asset
  checklist that shows manifest status, VM assets, profile files, verified/
  missing/invalid/downloading state, paths, and size details from
  `/profiles/{profile_id}/assets/status`.
- Disabled debug-only dummy plugins by default and updated the plugin UI to
  show enum-backed mode badges/icons for allow, ask, block, rewrite, and
  disabled states without hiding inactive plugins.
- Added plugin-owned capability metadata to `/profiles/{profile_id}/plugins/*`.
  The credential broker now reports watched event families, supported
  providers, and credential source shapes, and the Plugin UI renders those
  fields alongside broker inventory/counters instead of guessing.
- Updated the Profile rule lists and MCP tool list to use the same
  enum-backed visual language for allow/ask/block/rewrite/detection levels,
  while keeping MCP tool permission changes on the route-backed selector.
- Added an explicit `enabled` field to the security rule contract. Disabled
  rules remain visible in profile enforcement/detection inventories but are
  skipped by `SecurityRuleSet` evaluation and rendered inactive in the UI.
- Grouped Profile enforcement and detection rule lists into `default_rule`
  and profile/corp sections so built-in catchalls are visible without creating
  a second rule engine.
- Added a visible MCP default permission selector backed by `default.mcp`.
  The UI reads and edits `/profiles/{profile_id}/mcp/default/*`, while the
  service mutates the pinned enforcement file and writes the same profile
  mutation ledger used by per-tool MCP overrides.
- Cleaned the admin/doctor/status/debug rails so diagnostics follow the profile
  contract: builder doctor delegates profile validation to `capsem-admin
  profile check`, Justfile asset builds no longer pass legacy guest-config
  knobs, `capsem status`/default health read profile readiness from the service,
  and support bundles collect `settings.toml`/corp diagnostics without
  preserving `user.toml` as a config contract.
- Added structured `capsem.profile_mutation` logs for profile mutation routes
  and ledger writes. MCP tool edits plus enforcement/detection rule upserts and
  deletes now log route requests, validation rejections, ledger-open failures,
  and applied mutations with the same stable profile, target, operation, rule,
  hash, size, status, and mutation identifiers stored in the mutation ledger.
- Updated in-VM diagnostics to validate that the profile-owned Gemini,
  Antigravity, Claude, Codex, and MCP config files are actually projected into
  runtime `/root`, point at the canonical Capsem MCP bridge where applicable,
  and do not contain obvious credential-shaped secrets. The arm64 code-profile
  EROFS rootfs and initrd pins were refreshed from the rebuilt assets.
- Added a coverage-infra guard for release prep: PR Rust coverage now includes
  every workspace crate across the macOS/Linux jobs, Codecov components map
  each crate, and build-chain tests fail if a future crate is left out.
- Hardened AGY/manual-loop diagnostics: missing `capsem-mcp-aggregator` now
  fails loud instead of returning an empty MCP tool stub, unknown private
  model gateways are promoted from bounded JSON protocol shape while preserving
  the original HTTP body, broker credential inventory reports whether a stored
  reference is actually replayable, unknown remote MCP-over-HTTP JSON-RPC is
  promoted into first-party MCP ledger/security events, and boot/dispatch
  consume one typed host VSOCK service registry.

### Added (kernel 7.0 + EROFS)
- Added a stable-kernel upgrade path for guest builds: `kernel_branch = "7.0"`
  now resolves against kernel.org stable releases, while `auto` remains
  LTS-only for conservative release automation.
- Restored Linux KVM guest-memory hardening from the lost Linux line:
  guest memory reads/writes now reject offset overflow, and virtio-blk validates
  complete guest physical ranges before exposing raw host pointers to vectored
  I/O.
- Added experimental EROFS rootfs image generation with `lz4`, `lz4hc`, and
  `zstd` compression. EROFS zstd uses a newer `erofs-utils` container image,
  both guest defconfigs enable kernel-side EROFS zstd decompression, and
  `capsem-init` mounts EROFS when the VM cmdline carries `capsem.rootfs=erofs`.
- Added an opt-in Mac/VZ EROFS DAX probe lane:
  `CAPSEM_EXPERIMENTAL_EROFS_DAX=1` forwards to `capsem-process`, appends
  `capsem.rootfs=erofs-dax`, and makes `capsem-init` attempt an EROFS
  `ro,dax` mount so we can verify whether the VZ block transport can support
  the Linux-style DAX win locally.
- Moved guest NAT setup for the kernel 7.0 lane to `iptables-nft`: defconfigs
  enable nf_tables with the required nft/xt compatibility objects, legacy
  `IP_NF_*` tables are forbidden by tests, `capsem-init` fails closed on NAT
  rule insertion errors, and the rootfs build strips Debian's legacy iptables
  frontend binaries.
- Promoted EROFS lz4hc rootfs assets into the normal asset contract:
  `just build-assets code [arch]`, manifests, service resolution, setup status,
  release attestation, and installer download tests now use `rootfs.erofs` as
  the 1.3 runtime rootfs.
- Removed squashfs as a runtime/build fallback for 1.3 assets: the builder emits
  only `rootfs.erofs`, manifests require EROFS rootfs entries, service/core
  asset resolution no longer selects `rootfs.squashfs`, and in-VM doctor checks
  require `/dev/vda` to be EROFS.
- Added per-architecture VM asset `build-ledger.log` JSONL output from the real
  builder path, covering rendered Dockerfile/build-context hashes, rootfs tar,
  EROFS, kernel assets, tool-version output, compression settings, git revision,
  and project version; release CI uploads the ledger separately for retraceable
  failures.
- Added Python quality gates: Ruff now runs across the repository, and `ty`
  type-checks `src/capsem` in CI plus the local `just test`/`just smoke`
  fast-fail stages.

### Added (benchmarks)
- Added a deterministic `/model/response` fixture to `capsem-mock-server`
  and wired `capsem-bench protocol` to exercise both SSE model streams and
  JSON model responses without public-network dependencies.
- Added a shared `capsem-bench` load harness for MITM, MCP, DNS, and local
  mock-server tests: `CAPSEM_BENCH_CONCURRENCY`,
  `CAPSEM_BENCH_DURATION_S`, `CAPSEM_BENCH_TOTAL_REQUESTS`, and
  `CAPSEM_BENCH_SCENARIOS` now drive one tested config path, and load rows
  share the same request/error/rps/p50/p95/p99/p999/RSS schema.
- Added `scripts/benchmark_report.py`, a Pydantic-validated host reporter that
  renders benchmark JSON as Markdown and can produce matplotlib PNG graphs for
  committed load artifacts.
- Expanded the security-action Criterion benchmark to cover runtime event
  classification for HTTP, DNS, MCP, model, file, and process events in
  addition to rule matching, plugin dispatch, broker substitution, and MCP
  brokered OAuth credential-reference resolution.
- Refreshed the VM `mitm-local` release artifact so the local fixture corpus now
  includes JSON model responses, credential-shaped responses, WebSocket control,
  and session DB/no-secret verification through the profile-selected VM path.
- Added a retired security-rail guard test that fails if old Policy V2,
  domain-policy, or MCP decision-provider code paths reappear in live crates or
  configuration.

### Fixed (install/setup)
- Fixed `capsem stop` on macOS so it unloads the LaunchAgent instead of sending
  SIGTERM to a `KeepAlive` job that launchd immediately restarts. The command
  now verifies the service is no longer loaded before reporting success, so
  stopping Capsem no longer re-enters service startup or prompts for Keychain.
- macOS package postinstall now adds `~/.capsem/bin` to fish shell startup via
  an idempotent `fish_add_path --path "$HOME/.capsem/bin"` entry.
- Rebuilt install/startup flow around service readiness and asset state instead
  of setup wizard state: package installs surface postinstall failures, assets
  resolve through the manifest contract, and the UI waits on the service rather
  than opening against a dead daemon.
- Removed the old setup/onboarding authority path. Provider credentials are now
  discovered or brokered by the credential broker plugin through runtime
  security events and broker-owned references instead of being copied through a
  setup wizard.
- Removed the dead host credential detection module that could scan raw host
  API keys/OAuth files and write them into settings. Credential capture now
  stays behind the credential broker/plugin path, and the retired settings key
  validation surface remains fail-closed at the gateway.
- Stopped settings-derived guest config from materializing brokered provider
  credentials, repository tokens, generated `.git-credentials`, provider allow
  env vars, or AI CLI config files into VM boot env/files. Settings can still
  provide UI/app preferences and explicit non-secret `guest.env.*`; credential
  materialization is broker/plugin-owned.
- Removed the generated/UI `settings.ai.*` provider registry and the stale
  settings-based API-key injection tests. Retired flat AI setting IDs now fail
  validation for both settings file loads and inline corp config installs;
  provider control remains profile/corp rule-owned and credential handling
  remains plugin-owned.
- Removed the retired settings preset subsystem and cleaned root `config/` so
  MITM CA key material lives under `security/keys/` instead of looking like
  editable runtime configuration. Profile assets are selected by URL and
  verified by BLAKE3 hash/size, while release evidence stays in SBOM and
  provenance attestations.
- Fixed local install/package asset materialization so literal build outputs
  and already hash-prefixed assets both install through the same
  manifest-driven hash-prefixed layout, and package/simulated installs now
  include the full host tool set including `capsem-admin`,
  `capsem-tui`, `capsem-mcp-aggregator`, and `capsem-mcp-builtin`.
- Updated the built-in code profile's arm64 asset pins to the current
  EROFS/LZ4HC release artifacts so profile-owned VM boot resolution and the
  installed asset manifest agree.
- Fixed EROFS asset generation to disable the internal superblock CRC feature;
  BLAKE3 remains the release/boot integrity contract, and the repaired LZ4HC
  rootfs now passes `fsck.erofs` before install.
- Hardened the install test harness so the Linux package/systemd user unit is
  stopped before scoped process cleanup, and renamed the internal dev-readiness
  just helper away from setup wording while keeping `capsem setup` removed.

### Changed (release proof)
- Added shared runtime config materialization through
  `capsem-admin profile materialize`: local dev, smoke/test/install recipes,
  and release package jobs now generate `target/config` from checked-in
  `config/` plus `assets/manifest.json` instead of hand-editing source
  profiles. Service test helpers and `just _ensure-service` load
  `target/config/profiles` fail-closed.
- Updated docs and developer skills to document the same generated-config rail:
  checked-in `config/` is source/support material, current-build runtime config
  lives under `target/config`, and EROFS/LZ4HC level 12 is the 1.3 rootfs
  contract rather than a best-effort fallback.
- Restored the Linux-team KVM/FUSE performance work and storage benchmark
  harness into the current EROFS/LZ4HC rail, including bounded VM proof for
  `capsem-bench storage` from the generated profile-selected asset chain.
- Replaced public-service release proof with deterministic local fixtures:
  `capsem doctor` now starts/passes a local `capsem-mock-server`, doctor MCP
  content checks use local text/HTML fixtures, integration tests use local
  allowed/throughput/blocked HTTP paths, and session DB row-generation tests no
  longer curl public services.
- Routed local release-proof network traffic through the normal guest
  iptables-nft redirect rail. The local fixture is only the upstream target;
  doctor, integration, and benchmark paths no longer inject proxy environment
  variables or explicit WebSocket proxy sockets.
- Expanded the shipped plain-HTTP redirect/allowlist mechanics to
  `80`, `3128`, `3713`, `8080`, and `11434`, with doctor and local release
  proof pinned to `127.0.0.1:3713` to avoid colliding with real Ollama.

### Changed (service/API)
- Routed profile mutation ledger writes through the service-owned logger
  `DbHandle::write` path with structured DB failure logging, removing the
  service-side `DbWriter` side path for profile/MCP/rule/plugin edits.
- Updated architecture docs and local development skills to match the 1.3
  contract: settings endpoints are `/settings/info|edit` and expose only
  `tree`/`issues`, install is service/profile-asset readiness rather than a
  setup wizard, and EROFS/LZ4HC is the rootfs contract.
- Moved VM APIs under the explicit `/vms/...` contract. VM creation, listing,
  info, stop, pause, delete, resume, save, fork, exec, logs, inspect, history,
  timeline, and file read/write/list/content routes now live under
  `/vms`/`/vms/{vm_id}`; the retired top-level routes fail closed in the
  service/gateway route contract.
- Tightened the Python service, gateway, and E2E harnesses around the
  profile-owned VM contract: every VM creation and one-shot run test now passes
  the real `code` profile id explicitly, and the gateway mock rejects missing
  profile ids instead of accepting old default-profile payloads.
- Fixed runtime config loading so env-supplied corp/profile config preserves
  direct `corp.rules`, `profiles.rules`, `default`, `plugins`, and refresh
  groups when materializing `MergedPolicies`. Negative-priority corp rules now
  survive into VM processes and are covered by deterministic local MITM
  telemetry proof.
- Added `GET /vms/{vm_id}/status` as the runtime-state endpoint for one VM so
  UI state reads no longer need to treat `/vms/{vm_id}/info` as a status API.
- Added `PATCH /vms/{vm_id}/edit` as a fail-closed VM edit gate: attempts to
  mutate immutable `profile_id` or unknown fields are rejected, and resource
  edits return explicit unsupported status until live edit semantics are
  implemented.
- Added `GET /vms/{vm_id}/save/status` and
  `GET /vms/{vm_id}/fork/status`; because save/fork are synchronous today,
  existing VMs report explicit `idle` operation state rather than fake progress.
- Added VM action route coverage for `POST /vms/{vm_id}/start`,
  `POST /vms/{vm_id}/restart`, and `POST /vms/{vm_id}/reload-profile`.
  `start` uses the existing resume/start path; restart and reload-profile
  verify the VM exists and fail explicitly until real semantics land.
- Added profile inventory routes `GET /profiles/list` and
  `GET /profiles/status`, `POST /profiles/reload`, and
  `GET /profiles/{profile_id}/info`. Profile identity now comes from the typed
  profile catalog: the built-in `code` profile is a real `ProfileConfigFile`,
  route validation no longer uses a hard-coded `default` profile stub, and
  catalog reload/status reports profile readiness through the profile asset
  contract.
- Removed the `ProfileConfigFile::builtin_default()` compatibility alias and
  updated built-in profile validation/tests to name the real `code` profile.
- Fixed CLI and `capsem-mcp` MCP commands to use the real built-in `code`
  profile instead of the retired `default` profile when listing servers/tools,
  refreshing tools, calling profile-scoped MCP tools, or creating one-shot VMs.
  “Default” now refers only to visible default rules, not a hidden profile id.
- Restored the terminal control UI as the `capsem-tui` host binary and made
  `capsem shell` launch it. The TUI is wired to the current `/profiles/list`,
  `/status`, and `/vms/...` contracts, restores Alt-owned shortcuts,
  create/fork/pause/resume/stop/delete/recovery flows, vt-backed terminal
  reconnect behavior, and deterministic text/SVG snapshot inspection.
- Moved the service route table into a single shared router builder so startup
  and route-level tests exercise the same mounted API contract, including
  detection-rule authoring through `/profiles/.../detection/rules/...` and
  ledger readback through `/vms/.../security/latest`.
- Tightened gateway and service release fixtures around the explicit API
  contract: generic fallback proxy paths stay rejected, body-limit tests use
  real file-content routes, MCP credential status remains opaque, and macOS
  process leak detection survives `KERN_PROCARGS2` permission denials.
- Expanded mounted service route contract tests across fail-closed profile/VM
  stubs, profile/settings/corp reads, corp edit/reload, plugin edit/evaluate,
  MCP profile scoping, service-wide security ledgers, and file import/export
  boundary logging.
- Moved remote MCP auth onto the credential broker contract. MCP profile/corp
  config now carries `auth.kind` plus opaque `auth.credential_ref` for bearer
  or OAuth material; raw `bearer_token`/`bearerToken` imports are rejected or
  skipped, secret-bearing MCP headers fail validation, and UI status reports
  `has_auth_credential` instead of token presence.
- Replaced internet-backed MCP manager proof with local recording test
  infrastructure. The normal MCP manager suite now uses a local Streamable
  HTTP MCP server and HTTP recorder to prove broker-owned auth resolution,
  tool discovery, tool dispatch, and fail-closed missing credentials without
  contacting public services.
- Replaced builtin MCP HTTP tool tests that fetched `elie.net` and Wikipedia
  with local static HTTP fixture responses. `fetch_http`, `grep_http`, and
  `http_headers` still exercise the real reqwest/tool/security path, but
  normal tests no longer require public network availability.
- Added a profile-owned rule-file compilation guard: profile enforcement TOML
  and Sigma detection YAML now materialize as `SecurityRuleProfile` and compile
  only through the unified `SecurityRuleSet`/CEL rail, rejecting old policy
  syntax and profile-file attempts to smuggle `corp.rules`.
- Restored the `capsem-admin` executable as a Rust admin front door. Its
  product surface is intentionally narrow: profile validate/check/materialize,
  settings validate, enforcement/detection validate, manifest check/generate,
  and profile-derived image build.
- Added `capsem-admin manifest check|generate` for the current format-2 asset
  manifest. The commands validate top-level `refresh_policy`, report asset
  releases/arches, and regenerate the canonical `assets/manifest.json` from
  built assets without restoring manifest signing or a second asset path.
- Added profile-derived `capsem-admin image build` and moved
  `just build-assets` onto that rail. Asset builds now require an explicit
  profile, validate the profile and rule files first, preserve the Code profile
  defaults, build EROFS `lz4hc` level 12 rootfs assets, and reject raw
  no-profile build attempts.
- Updated the release workflow to call the profile-derived asset build rail
  explicitly (`code` profile) and to package/sign the full restored host binary
  set, including `capsem-admin`.
- Replaced the temporary flat profile asset triplet with per-architecture
  profile asset declarations. `config/profiles/code/profile.toml` now parses as
  the checked-in contract for EROFS/LZ4HC kernel, initrd, and rootfs assets with
  URL/hash/size metadata.
- Made `/profiles/{profile_id}/assets/status` report the selected profile's
  current-architecture asset contract instead of a service-global asset guess,
  including profile id, revision, profile payload hash, expected hashes,
  sizes, source URLs, and present/missing state from the same hash-prefixed
  resolver used by boot.
- Made VM creation profile-explicit. `POST /vms/create`/provision and
  one-shot `run` payloads now require `profile_id`; unknown profiles fail
  before boot state is created, persistent registry rows store `profile_id`,
  fork/save/resume preserve it, and list/info responses expose it. A VM's
  `profile_id` remains immutable after creation.
- Made VM boot preflight and process spawn resolve kernel, initrd, and rootfs
  from the selected profile asset contract. Profile resolution supports the
  approved hash-prefixed downloaded layout and logical-name dev layout, but
  both are derived from profile asset descriptors instead of the old
  service-global file guess.
- Made `/profiles/{profile_id}/assets/ensure` profile-owned. It downloads the
  selected profile's current-architecture kernel, initrd, and rootfs URLs into
  hash-prefixed asset files, verifies each file with the profile BLAKE3 hash,
  updates reconcile status, and skips already-verified profile assets.
- Made `capsem assets status` and `capsem assets ensure` profile-aware. Both
  commands now target the real `code` profile by default, accept `--profile`,
  and call `/profiles/{profile_id}/assets/...` instead of the burned
  `/profiles/default` path; gateway route coverage also forwards
  `/profiles/status` and `/profiles/reload` explicitly.
- Updated the frontend MCP and plugin settings surfaces to target the real
  `code` profile instead of the burned `default` profile id.
- Made startup asset cleanup preserve profile catalog assets and persistent VM
  boot asset pins. Hash-prefixed files referenced by active profile
  descriptors or saved VM pins are retained even when they are not listed in
  the release manifest.
- Made persistent VM lifecycle state pin the selected profile revision, profile
  payload hash, and boot asset descriptors. Create/save/fork/resume preserve
  the pinned profile revision, typed profile payload BLAKE3 hash, and
  kernel/initrd/rootfs name+hash pins; save/fork/resume fail closed when the
  current profile revision, profile payload hash, or boot asset pins drift.
- Added profile management route gates:
  `POST /profiles/create`, `PATCH /profiles/{profile_id}/edit`,
  `DELETE /profiles/{profile_id}/delete`, `POST /profiles/{profile_id}/clone`,
  and `POST /profiles/{profile_id}/validate`. Validation is real over the
  typed `ProfileConfigFile`; mutation routes fail explicitly until profile file
  persistence is implemented instead of writing through settings.
- Added `GET /profiles/{profile_id}/enforcement/rules/list`, returning the
  compiled profile rule inventory with source, default-rule, priority, action,
  detection level, and lock metadata so the UI can reflect backend rule
  truth instead of inventing grouping state.
- Added `GET /profiles/{profile_id}/enforcement/info`, returning compiled
  enforcement configuration counts by source/action plus default/custom,
  detection, and corp-lock totals. Runtime counters remain table-backed under
  VM enforcement status.
- Added profile-scoped detection rule routes
  `/profiles/{profile_id}/detection/info`,
  `/profiles/{profile_id}/detection/rules/list`,
  `/profiles/{profile_id}/detection/evaluate`,
  `/profiles/{profile_id}/detection/rules/{rule_id}/edit`,
  `/profiles/{profile_id}/detection/rules/{rule_id}/delete`, and
  `/profiles/{profile_id}/detection/reload`. They reuse the same compiled
  security-rule contract as enforcement and only list/write rules with an
  explicit `detection_level`.
- Moved asset readiness/reconciliation to profile-owned routes
  `/profiles/{profile_id}/assets/status` and
  `/profiles/{profile_id}/assets/ensure`; retired global `/assets/status` and
  `/assets/ensure` so asset selection stays under the profile contract.
- Removed the retired service-global asset status helper from the service
  binary and converted its reconcile-progress unit coverage to the
  profile-owned asset status contract.
- Added profile-scoped skills route surfaces. Skills `info|list` reflect the
  typed profile manifest; add/edit/delete fail explicitly until profile
  persistence is implemented.
- Removed the profile credential API surface before release: there is no
  `/profiles/{profile_id}/credentials/*` route and no `[credentials]` profile
  block. Credential capture/substitution state belongs to the credential broker
  plugin runtime contract.
- Added profile-scoped assets `info|edit`, plugins `info`, and MCP `info`
  routes. Info routes summarize existing profile/config state; asset edits
  fail explicitly until profile persistence lands.
- Made profile MCP inventory profile-owned. `/profiles/{profile_id}/mcp/...`
  now reads the selected profile's MCP section instead of settings/corp MCP
  sections, `config/profiles/code/profile.toml` explicitly enables the real
  built-in `local` MCP server, and unknown profile server ids fail closed.
- Added service-wide runtime ledger routes `/security/latest|status`,
  `/enforcement/latest|status`, and `/detection/latest|status`. These aggregate
  per-VM `session.db` security-rule ledger rows through `DbReader`; detection
  routes filter to rows with an explicit detection level.

### Added (security event rule spine)
- Replaced callback-shaped Policy V2 authoring with one native rule contract
  over canonical `SecurityEvent`: `[corp.rules.*]`, `[profiles.rules.*]`, and
  provider convenience `[ai.<provider>.rules.*]` all compile into the same
  `SecurityRuleSet`.
- Added typed rule actions `allow`, `ask`, `block`, `preprocess`, `rewrite`,
  and `postprocess`, plus optional `detection_level` metadata for
  `informational`, `low`, `medium`, `high`, and `critical` detections.
- Added source-aware priority discipline: built-in defaults use the named
  `default` priority sentinel after the numeric user range, user/plugin rules
  default to `10`, corp-locked rules default negative, and non-corp rules
  cannot use negative priorities.
- Added shared external rule files: both user and corp settings can reference
  native enforcement TOML with `[rule_files].enforcement` and Sigma YAML with
  `[rule_files].sigma`; both compile into the same runtime rules. Corp settings
  also carry the future `corp_rule_files.sigma_output_endpoint` integration
  field for SIEM/export delivery.
- Hardened security rule validation with adversarial parser/compiler tests:
  malformed CEL, stale callback fields, callback/table mismatches, invalid
  rule names, invalid priorities, invalid plugin shapes, and atomic rejection
  now fail closed before settings are written.
- Added strict CEL validation against first-party `SecurityEvent` roots
  (`http`, `dns`, `mcp`, `model`, `file`, `process`, and `security`) so stale
  callback-local fields fail before rules persist. Credential substitution
  remains a ledger event type, while snapshot lifecycle state is host recovery
  state exposed through VM snapshot routes rather than CEL roots or
  `session.db`.
- Added typed runtime-family markers for first-party CEL roots versus
  ledger-only `credential.substitution` rows, with regression tests tying the
  markers to `SECURITY_EVENT_CEL_ROOTS`.
- Replaced legacy `[profiles.defaults.*]` rule authoring with the visible
  `[default.<domain>]` contract. Default rules still compile into ordinary late
  CEL rules under `profiles.rules.default_<domain>`, and the old namespace is
  rejected instead of aliased.
- Removed static `tool_config_sources` from settings/profile contracts and the
  settings UI response. Tool config observations now belong to runtime
  plugin/security-ledger evidence with BLAKE3 references, and static
  `tool_config_sources` tables fail closed.
- Removed static credential/config-file metadata from `[ai.*]` provider
  endpoint records. Provider records now carry routing/rule/discovery
  information only; `credential_setting_id`, provider-level `credential_ref`,
  and provider `files` fail closed, and settings provider cards no longer expose
  brokered credential refs.
- Removed provider status from `/settings/info` and the settings UI/model.
  Provider-like behavior is no longer a settings object: profile/corp rules own
  enforcement and credential/plugin runtime status owns credential evidence.
- Stopped the credential broker from writing brokered references into settings.
  Observed credentials are stored behind the credential-store boundary, emitted
  to the substitution/security ledger, and can record provider discovery;
  settings files no longer become a credential-reference inventory.
- Added a security-event engine that runs configured preprocess plugins before
  detection/enforcement, evaluates CEL once against the canonical event, then
  runs configured postprocess plugins only after the decision allows
  materialization.
- Added the typed plugin contract `plugin(SecurityEvent) -> SecurityEvent`;
  plugins own their filtering and runtime state, plugin failures fail closed,
  and plugin effects are recorded in the security rule ledger.
- Added typed profile/corp plugin policy with `mode` and `detection_level`.
  Enabled plugins append `SecurityDetectionEvent` records onto
  `SecurityEvent.detections`, rules with `detection_level` append the same
  reporting vector, and `rewrite` is the canonical mutation mode.
- Extended profile plugin API responses with backend-owned plugin metadata and
  runtime status: stage, version, counters, errors, and brokered credential
  references. The settings UI now reads brokered credential refs only from the
  credential-broker plugin runtime status shape.
- Hardened plugin edit requests so unknown fields are rejected instead of
  ignored. Invalid modes, invalid detection levels, unknown plugins/profiles,
  and credential-reference smuggling attempts fail closed.
- Hardened profile skill mutation routes with typed, strict payloads. Add/edit
  requests now reject unknown fields and empty paths before the current
  profile-persistence gate returns `501 Not Implemented`.
- Added the plugin/detection/enforcement endpoint taxonomy:
  `/profiles/{profile_id}/plugins/list`,
  `/profiles/{profile_id}/plugins/{plugin_id}/info`, and
  `/profiles/{profile_id}/plugins/{plugin_id}/edit` report and update
  profile-owned plugin config,
  `/profiles/{profile_id}/enforcement/evaluate` sends a profile-scoped test
  event through the real engine, and
  `/vms/{vm_id}/detection/latest|status` plus
  `/vms/{vm_id}/enforcement/latest|status` remain table-backed ledger views.
- Added enforcement rule-management endpoints:
  `PUT /profiles/{profile_id}/enforcement/rules/{rule_id}/edit` and
  `DELETE /profiles/{profile_id}/enforcement/rules/{rule_id}/delete`
  validate profile rules against the native `SecurityRuleProfile` compiler
  before writing `user.toml`, and
  `POST /profiles/{profile_id}/enforcement/reload` reloads that profile's
  enforcement rules.
- Replaced the retired `/corp-config` provisioning route with
  `PUT /corp/edit`; the gateway and service now reject the old route instead
  of forwarding it.
- Added the rest of the corp plane routes: `GET /corp/info`,
  `POST /corp/validate`, and `POST /corp/reload`, all forwarded explicitly by
  the gateway.
- Replaced the ambiguous `GET|POST /settings` route with
  `GET /settings/info` and `PATCH /settings/edit`; the old magic settings
  route now fails closed in the service and gateway.
- Split core config mutation by owner: `PATCH /settings/edit` now uses the
  UI-settings writer, while VM/security/AI behavior uses profile-owned config
  writers. Credential brokerage state belongs to the broker plugin runtime
  contract.
- Added a first-class profile manifest contract covering profile identity,
  description, icon SVG, web/shell/mobile availability, VM asset selection,
  VM defaults, rule files/default rules, plugins, MCP servers, skills,
  AI/provider convenience rules, and tool config source metadata.
- Profile inventory now sources the built-in `default` profile summary from
  the profile manifest contract instead of service-local placeholder text.
- Removed retired settings utility routes `/settings/lint` and
  `/settings/validate-key`; settings now expose only `info` and `edit` until
  profile/corp validation and credential broker endpoints own those workflows.
- Removed retired settings preset endpoints and UI selector; security/profile
  defaults no longer mutate behavior through `/settings/presets`.
- Removed preset metadata from `/settings/info`; settings responses now carry
  settings tree/issues plus status fields only, not behavior presets.
- Replaced the global `POST /reload-config` route with
  `POST /profiles/{profile_id}/reload`; the old global reload route now fails
  closed in the service and gateway.
- Added `SerializableSecurityEvent` as the public evaluated-event wire DTO:
  every first-party event root is present, absent roots serialize as `null`,
  and raw credential observation buffers are excluded.
- Added credential broker plugin support with file-backed durable storage and
  BLAKE3 `credential:blake3:<hex>` references in broker runtime status, logs,
  and `session.db`; raw credentials stay broker-private.
- Added brokered credential capture from observed HTTP headers/body responses
  and `.env` files, plus upstream-only substitution of broker references for
  allowed HTTP materialization.
- Added a closed runtime security-event identity contract and routed HTTP/net,
  model, MCP, DNS, file, process exec/audit/completion, broker substitution,
  and snapshot session DB rows through the security-engine emitter handoff.
- Removed the old MITM PolicyHook/Policy V2 runtime rails and the MCP built-in
  legacy domain bridge. HTTP request, model request/response, framed MCP
  request/response, MCP built-in HTTP tools, and DNS query blocking now enforce
  through the canonical `SecurityEvent` + CEL rule path before dispatch.
- Added contract tests proving built-in default rules match HTTP, DNS, MCP,
  model, file, and process security events as ordinary late-priority CEL rules;
  specific rules run first, and editing a default rule changes evaluation
  without any hidden network fallback.
- Removed retired web decision settings (`security.web.allow_read`,
  `security.web.allow_write`, `security.web.custom_allow`, and
  `security.web.custom_block`) from defaults, presets, builder schemas,
  frontend fixtures, guest diagnostics, and integration fixtures. Network
  settings now expose only mechanics such as `security.web.http_upstream_ports`;
  HTTP/DNS allow/block behavior belongs to profile security rules.
- Replaced global MCP service/gateway/frontend routes with profile/server
  routes: servers live under `/profiles/{profile_id}/mcp/servers/list`, tools
  live under `/profiles/{profile_id}/mcp/servers/{server_id}/tools/list`, and
  tool edit/call/refresh operations are scoped to the same profile/server path.
- Replaced global enforcement authoring routes with profile-owned routes:
  `/profiles/{profile_id}/enforcement/evaluate`,
  `/profiles/{profile_id}/enforcement/rules/{rule_id}/edit`,
  `/profiles/{profile_id}/enforcement/rules/{rule_id}/delete`, and
  `/profiles/{profile_id}/enforcement/reload`.
- Routed explicit file import/export/read/write boundaries through the
  process-owned security-event emitter so `fs_events` and
  `security_rule_events` share the same primary event id without a service-side
  DB writer or fallback logger.
- Added a release guard that keeps session event writes behind
  `capsem_logger::DbWriter`: production protocol, plugin, security, service,
  and process code may not open ad-hoc SQLite writers or insert event rows
  directly.
- Added a security rule forensic ledger: `security_rule_events` stores the
  triggering event id/type, rule id/name/action/detection level, rule snapshot,
  matched `SecurityEvent` payload, and trace id. `security_ask_events` records
  append-only pending/approved/denied ask lifecycle rows.
- Added DB-backed security endpoints: `/vms/{vm_id}/security/latest` returns
  full stored rule ledger rows and `/vms/{vm_id}/security/status` regenerates
  counters from `session.db`.
- Replaced retired top-level VM lifecycle routes with the profile-era VM
  namespace across service, gateway, CLI, MCP, tray, frontend, and tests:
  `POST /vms/{vm_id}/pause`, `DELETE /vms/{vm_id}/delete`,
  `POST /vms/{vm_id}/resume`, `POST /vms/{vm_id}/save`, and
  `POST /vms/{vm_id}/fork`. The gateway now rejects the old
  `/suspend`, `/delete`, `/resume`, `/persist`, and `/fork` route family.
- Moved core VM create/list/info/stop routes into the same VM namespace across
  service, gateway, CLI, MCP, tray, frontend, status aggregation, docs, and
  tests: `POST /vms/create`, `GET /vms/list`,
  `GET /vms/{vm_id}/info`, and `POST /vms/{vm_id}/stop`. The gateway now
  rejects retired `/provision`, `/list`, `/info/{id}`, and `/stop/{id}` paths.
- Added built-in provider-owned AI rules for OpenAI/Codex, Anthropic/Claude,
  Google/Gemini, and Ollama. The rules live under `[ai.<provider>.rules.*]`,
  merge as defaults < user < corp, enforce corp-only negative priorities, and
  compile into deterministic `profiles.rules.*` security-event rules whose
  matches are written to the `security_rule_events` session DB ledger and
  exposed through `/vms/{vm_id}/security/latest`.
- Added Sigma import support that parses Sigma YAML into typed `SecurityRule`
  entries, derives valid rule ids/names, validates generated CEL against
  `SecurityEvent` roots, and keeps security-team detection authoring on the
  same ledger/enforcement rail as native rules.
- Added `capsem-core` security-action microbenchmarks for rule matching,
  action-chain overhead, runtime event classification, and brokered HTTP
  credential materialization.

### Added (observability and benchmarks)
- Added OpenTelemetry-style spans and local-only metrics around MITM/network
  stages, security-event emission, DB enqueue/write behavior, and launch paths
  for benchmark/debug use without exposing upstream telemetry by default.
- Added a local MITM debug benchmark server with HTTP, gzip, SSE/model-like,
  credential-response, deny-target, and WebSocket scenarios so network/security
  hot paths can be measured without public internet variance.
- Added logger-owned DB writer pressure benchmarks and metrics for enqueue
  latency, batch writes, shutdown flushes, and coalesced event pressure.

### Changed (security policy enforcement)
- Unified HTTP, DNS, MCP, model, file, and process detection/enforcement on
  the security-event rule engine. Producers now emit canonical security events,
  evaluate the active `SecurityRuleSet`, and write matched rule rows with the
  same primary event id as the underlying `session.db` event. Credential
  substitution and snapshot lifecycle writes remain canonical ledger event
  types, not fake rule roots.
- Removed the global MCP policy API/UI/CLI surface (`/mcp/policy`,
  `capsem mcp policy`, and frontend MCP policy mutators). MCP runtime endpoints
  now report mechanics only; MCP decisions must be expressed as security rules.
- Removed the old `McpPolicy`/`ToolDecision` decision object from core config.
  Security presets no longer write MCP tool permissions, retired
  `mcp.global_policy`, `mcp.default_tool_permission`, and
  `mcp.tool_permissions` keys fail closed at settings load, and MCP blocking
  tests now use profile security rules.
- Removed `NetworkPolicy::evaluate`, `PolicyDecision`, and
  `NetworkPolicy::is_fully_blocked` from the network engine. Network policy
  code now carries only mechanics such as DNS redirects, HTTP port metadata,
  and body-capture settings; HTTP/DNS allow, ask, block, and default behavior
  must come from profile/corp security rules.
- Removed the remaining domain allow/read/write/default fields from
  `NetworkPolicy` itself. The network object can no longer carry hidden
  domain enforcement state; tests now assert default and provider behavior
  through compiled `SecurityRuleSet` entries.
- Stopped exporting retired web default toggles as guest authority env vars
  (`CAPSEM_WEB_ALLOW_READ` and `CAPSEM_WEB_ALLOW_WRITE`). The guest now relies
  on security events and rules for HTTP/DNS behavior rather than stale
  settings-derived hints.
- Replaced the old callback-demux rule authoring language with CEL over
  first-party event roots. Admin-visible rules use `match = ...` and typed
  actions rather than callback-local `on`/`if`/`decision` fields.
- Preserved enforcement semantics for real boundaries: HTTP/model dispatch,
  DNS handling, framed MCP calls/notifications, file import/export/read/write,
  process exec/audit/completion, credential substitution, and snapshot events
  all pass through the shared security-event emitter and rule ledger.
- Added VM and integration coverage proving configured security rules block,
  ask, or log HTTP, DNS, MCP, model, file, and process events without leaking
  denied request/response payloads into previews.
- Updated the policy product surface and docs around the new
  `SecurityEvent` rule contract, Sigma import, DB-backed latest/info
  endpoints, and forensic `session.db` ledger instead of generated
  callback-specific policy stanzas.

### Fixed (policy rules)
- Fixed model telemetry parsing for explicit/local OpenAI-compatible
  provider paths by carrying the request's provider classification through
  the MITM chunk-hook metadata, so enforcement and SSE interpretation use
  the same provider decision instead of relying only on the network domain.
- Fixed builtin MCP HTTP policy propagation: `capsem-process` now passes
  merged domain allow/block lists to `capsem-mcp-builtin`, so configured
  builtin HTTP denials fail at the policy boundary, avoid upstream
  resolution, and write both `mcp_calls` and `net_events`.
- Fixed a model tool-response policy bypass found during adversarial unit
  testing: an allow rule matching one tool result can no longer let a
  separate secret-bearing tool result in the same provider request bypass a
  block rule.
- Fixed a policy evaluator safety bug found during adversarial testing:
  a missing field no longer satisfies a negative comparison such as
  `provider != "local"`.
- Fixed policy settings UI crashes found during browser verification by
  tolerating omitted live metadata arrays and deduplicating generated rule
  keys before rendering Svelte keyed rows.
- Fixed MITM integration fixture discipline: fake upstreams now drain the
  full `Content-Length` body and upstream task panics fail the test instead
  of only printing noisy background panics.
- Fixed warnings-as-errors issues found during policy verification by
  removing a redundant setup detection closure and switching settings
  endpoint env-serialization tests to an async mutex.
- Fixed an MCP telemetry leak: pre-dispatch block/ask denials now avoid
  writing raw denied request arguments into `mcp_calls.request_preview`.
- Fixed MITM body handling regressions found during T6 verification:
  HTTP decompression now honors `Content-Encoding: gzip` instead of raw
  gzip magic bytes, and decoded responses drop stale compressed
  `Content-Length`/size hints so guest delivery cannot truncate.
- Fixed suspend/resume recovery hardening found during T7: `.vzsave`
  checkpoints are fsynced before process exit, service registry suspended
  state is cleared only after resume readiness, and failed Apple VZ warm
  checkpoints are archived before a persistent cold-boot fallback recovers
  workspace/overlay state.
- Fixed the smoke leak-detector false positive where concurrent pytest
  invocations shared one leak-attribution file and could report another
  still-running pytest process's service fixture as a leak; `just smoke`
  now gives each pytest phase a distinct leak-log namespace.

### Fixed (mitm-mcp-unification T4 coverage hardening)
- Preserved all JSON-RPC request id shapes in framed MCP telemetry:
  string, numeric, and null ids now populate `mcp_calls.request_id`
  instead of only unsigned numeric ids.
- Corrected the sprint tracker T5 scope: configured external MCP tool
  calls are inspected at the framed MITM MCP boundary; any remaining
  downstream host-side egress concern must be named separately.
- Expanded framed MCP coverage across Rust, VM E2E, and in-VM doctor
  diagnostics for malformed JSON recovery, oversized guest requests,
  corrupted-frame recovery after an established MCP frame stream,
  notification interleaving, non-`tools/call` timeout telemetry, and
  persistent stop/resume reconnect.
- Updated the T4 coverage review notes and benchmark log with the bugs
  found during review, `session.db` sanity evidence, and fresh
  `mcp-load` numbers after the hardening pass.

### Changed (mitm-mcp-unification T4 cutover)
- **Guest MCP now uses framed MITM transport by default.**
  `/run/capsem-mcp-server` relays stdio JSON-RPC over bounded MCP
  frames on `vsock:5002`, carries per-frame process attribution, emits
  explicit disconnect errors for in-flight JSON-RPC requests, and avoids
  automatic replay of non-idempotent `tools/call` requests after a
  transport drop.
- Removed the legacy guest MCP router on `vsock:5003`: deleted
  `capsem-core/src/mcp/gateway.rs`, removed `VSOCK_PORT_MCP_GATEWAY`,
  removed 5003 vsock dispatch/classification, and updated guest
  diagnostics, docs, skills, and benchmarks to describe the MITM MCP
  endpoint as the canonical guest MCP path.
- Added the `mitm.mcp_disconnects_total` metric and VM E2E coverage
  proving the default guest relay writes populated `mcp_calls` rows,
  live policy reload affects an existing connection, concurrent parent
  processes preserve `mcp_calls.process_name`, tool timeouts record
  terminal errors, external stdio MCP tools still dispatch, and legacy
  `vsock:5003` refuses guest connections.
- Fixed `scripts/check_session.py` so `just inspect-session <id>` works
  with current run-session directories and older system Python versions.

### Changed (development process)
- Strengthened the Capsem sprint/testing skills to require an explicit
  functional-slice proof matrix for non-trivial work: unit/contract,
  functional, adversarial, E2E/VM, telemetry, and performance evidence
  must be named in sprint trackers, with any missing coverage recorded
  as visible debt instead of implied by benchmarks or unit tests.
- Expanded the MCP development skill with the framed MITM MCP hardening
  matrix: parser/interpreter adversarial cases, dispatch coverage,
  policy rule enforcement, telemetry assertions, VM E2E checks, and the
  aggregator DB-free boundary.

### Fixed (mitm-mcp-unification T3 hardening)
- **Framed MCP now consumes request stream ids before JSON parsing,**
  so a valid frame with invalid JSON cannot reuse the same stream id for
  a later request. Parser-level failures still return JSON-RPC parse
  errors, complete the stream id, and avoid writing misleading
  `mcp_calls` rows.
- **`capsem-service` now forwards framed MCP runtime knobs to
  `capsem-process`.** The child-process env allowlist includes
  `CAPSEM_HOME`, `CAPSEM_MCP_DEFAULT_TIMEOUT_SECS`,
  `CAPSEM_MCP_TOOL_CALL_TIMEOUT_SECS`, and
  `CAPSEM_MCP_TOOL_CALL_TIMEOUT_CEILING_SECS`, keeping service/process
  config roots aligned and allowing E2E tests to exercise real timeout
  limits.
- Added framed MCP VM E2E coverage for builtin `tools/call`, configured
  external stdio tools, live policy reload on an already-open connection,
  concurrent process attribution, slow-tool timeout telemetry, and
  `session.db` policy/preview assertions.
- Added a static regression guard proving the low-privilege
  `capsem-mcp-aggregator` crate remains free of session DB dependencies
  and audit writes.

### Added (mitm-mcp-unification T3 MITM MCP endpoint)
- **Framed MCP now dispatches through a real MITM-owned endpoint
  instead of borrowing the legacy MCP gateway handler.** `MitmProxyConfig`
  owns `McpEndpointState`; the framed path routes initialize,
  tool/resource/prompt list, tool calls, resource reads, and prompt gets
  through the low-privilege `AggregatorClient`; and the MITM frame layer
  writes `mcp_calls` telemetry directly through the session `DbWriter`.
  The aggregator remains DB-free.
- Added method-aware framed MCP timeouts: non-`tools/call` methods default
  to 60s, `tools/call` defaults to 300s, tool-call catalog timeout
  overrides are clamped by a 300s ceiling, and timeout failures return
  JSON-RPC errors while recording terminal `mcp_calls` rows with
  `decision=error`.

### Added (mitm-mcp-unification T2 decision provider)
- **Framed MCP calls now record audit-only policy decisions in
  `mcp_calls`.** The MITM MCP frame path builds an owned decision
  request from the interpreter summary, preserving process name,
  method classification, request preview, and BLAKE3 request hash for
  future remote corp forwarding. The local v1 provider emits only
  `allow` or `deny` actions, maps warning policy to `allow`, evaluates
  tool calls at per-tool granularity, evaluates resource/prompt reads
  at server granularity, and stores `policy_mode`, `policy_action`,
  `policy_rule`, and `policy_reason` through the logger schema,
  writer, reader, and session triage output.
- Added the T2 policy test matrix for exact tool name, exact MCP
  resource URI, prompt/tool argument name, prompt/tool argument value,
  nested return value, deny-over-allow precedence, live policy mutation,
  response-time decisions, actual framed request blocks, and sanitized
  framed response blocks. The framed tests now drive the MCP frame
  transport into a real `session.db` and assert both telemetry previews
  and policy fields on the resulting `mcp_calls` rows.
- Framed MCP deny decisions now enforce as well as log: request-rule
  denies short-circuit before aggregator dispatch, and return-value
  denies replace the original MCP result with a policy error before it
  reaches the guest.

### Added (mitm-mcp-unification T1 parser/interpreter)
- **Framed MCP over `vsock:5002` now has a bounded parser and
  interpreter instead of relying on the T0 spike shape.** The MITM
  MCP frame path validates frame length/flags, enforces monotonic
  nonzero request `stream_id`s while reserving `stream_id=0` for
  notifications, bounds JSON-RPC payload parsing before deserialize,
  classifies MCP request/notification methods, extracts server/tool/
  resource/prompt names for the known MCP call families, emits method
  metrics, and recovers from corrupt-but-bounded frames by returning
  JSON-RPC invalid-request errors before continuing the stream.

### Changed (exec timeout contract)
- **`capsem exec` and `capsem run` no longer impose a hidden default
  command timeout.** Omitting `--timeout` now waits for command
  completion, which matches long-running user jobs such as builds,
  installs, migrations, and `capsem-bench mcp-load`. Explicit
  `--timeout <seconds>` still applies a service-side deadline. The
  process-layer exec watchdog was removed; transport delivery remains
  covered by the control bridge's Ack/AckReply replay layers.

### Changed (rustfmt sweep)
- Ran a one-time workspace `cargo fmt` sweep while landing T1 so future
  sprint diffs start from the same formatter baseline.

### Added (mitm-mcp-unification T0 wire gate)
- **Framed MCP-over-MITM transport is now benchmark-gated for the
  MCP unification sprint.** Added a bounded `MC` frame envelope in
  `capsem-proto`, a MITM classifier branch for framed MCP on
  `vsock:5002`, and an explicit `CAPSEM_MCP_TRANSPORT=framed`
  mode in the guest MCP relay. The T0 spike still routes through
  the existing aggregator/policy/MCP telemetry path so the wire
  comparison stays fair. Fresh same-hardware `mcp-load` artifacts
  are recorded at
  `benchmarks/mcp-load/baseline-pre-mitm-unification.json` and
  `benchmarks/mcp-load/baseline-framed-mitm-unification-t0.json`.
  Framed selected: rps +8.6% / +4.8% / -6.4% / +5.4% and p99
  -31.9% / -23.9% / +7.8% / -31.0% at concurrency 1/10/50/200,
  with zero errors on both transports.

### Fixed (mcp/file_tools: truncate_path panic on non-ASCII paths -- AB-007)
- **`truncate_path` no longer panics on paths whose suffix
  byte offset lands inside a multibyte UTF-8 sequence.** The
  legacy implementation used `path.len()` (bytes) and
  `&path[path.len() - (max - 3)..]` (byte slice). For example,
  a path of 40 `日` chars + 1 ASCII char (121 bytes) with
  max = 33 panicked with `start byte index 91 is not a char
  boundary; it is inside '日'`. Both call sites
  (`render_changes` and the snapshot list renderer) walk
  user-supplied paths, so any non-ASCII path could crash
  snapshot rendering for the whole VM. The new implementation
  counts and slices by character, falling back to a
  no-ellipsis suffix for `max <= 3` so ill-typed callers
  cannot bring down the tool. Eight regression tests cover
  ASCII-under, ASCII-over, Unicode-under (keeps as-is even
  when byte length exceeds max), Unicode-boundary panic
  repro, Unicode-over (correct char count), empty path,
  `max == 3`, and `max == 0`.

### Fixed (security: deep-link JS injection -- AB-003)
- **`capsem-app::dispatch_deep_link` no longer interpolates
  `--connect` / `--action` values into JavaScript that runs in
  the desktop webview.** The previous code only escaped single
  quotes and embedded the values into a single-quoted JS
  literal that was passed to `window.eval`. A trailing
  backslash, a newline, or a payload like
  `x\'); alert(1); //` broke out of the string and ran as
  code -- in a webview that holds the gateway auth token, so
  effective full local capsem control. New helpers
  `build_deep_link_payload` (returns a `serde_json::Value`)
  and `build_deep_link_script` embed the payload via JSON
  serialization, which is a strict subset of valid JS object/
  string literals; every backslash, quote, control char, and
  high-bit code point is escaped by construction. Tests added
  cover plain values, single quote, backslash, newline, the
  injection-payload repro, and a JSON round-trip across a
  high-entropy input string.

### Fixed (mitm-redesign T3 closure -- production bug, dns-load reveal)
- **DNS cache returned the original query id for every cache hit.**
  The TTL-honoring answer cache (T3.f) stored wire-format response
  bytes verbatim, including the 16-bit DNS transaction id in
  bytes 0-1. Cache hits returned those bytes without rewriting the
  id, so subsequent queries to the same `(qname, qtype, qclass)`
  always echoed the FIRST query's id. Downstream resolvers (which
  match responses to outstanding queries by id, RFC 1035 sec 4.1.1)
  would discard the cached response as not-mine, causing 100%
  query failure once the cache warmed up. Surfaced by the
  `capsem-bench dns-load` in-VM run during T3 closure: the run
  reported ~99.999% errors, and an inline diagnostic showed the
  exact pattern -- 5 sequential queries with random ids all
  returned the same id (the first query's). Fix: `DnsAnswerCache::get`
  takes a new `query_id: u16` parameter and patches the response
  bytes' id field on every hit before returning. New regression
  tests `cache_hit_patches_query_id_into_response` (asserts the
  patch happens with two different ids on the same key) and
  `cache_hit_with_zero_query_id_zeroes_bytes` (defensive: id=0
  must overwrite, not skip the patch). Existing 18 cache tests
  updated to pass through the new arg. capsem-core lib at 1693
  tests now (+2 regression). Workspace clippy clean.

### Fixed (mcp: corp precedence -- AB-002)
- **Corp-defined MCP servers can no longer be shadowed by a
  same-name user manual entry.** The build pipeline in
  `crates/capsem-core/src/mcp/mod.rs::build_server_list_with_builtin`
  used a first-wins HashSet but processed entries in the order
  builtin → auto-detected → user → corp, so corp was last and
  was silently skipped on collision. A user typing the same
  name as a corp-injected server would win the URL, headers,
  and bearer token, contradicting the documented `corp > user
  > defaults` policy in `docs/architecture/settings.md` and
  the "corp_locked" model. Corp definitions are now processed
  first, so the first-wins rule enforces the documented trust
  order. Same-name user entries are skipped; unique-name user
  and auto-detected entries are unaffected. Tests added:
  `build_server_list_corp_shadows_user_on_same_name`,
  `build_server_list_unique_user_server_survives_with_corp_present`,
  `build_server_list_corp_enabled_override_on_user_server`.
  `docs/src/content/docs/architecture/mcp-aggregator.md`
  reordered to match the new processing order.

### Fixed (security: gateway CORS -- AB-001)
- **Gateway CORS now does an exact-host check on the Origin
  header instead of a string prefix match, closing a path that
  could leak the gateway auth token to attacker-controlled
  pages.** The previous predicate accepted any origin starting
  with `http://localhost`, `http://127.0.0.1`, `https://...`,
  or `tauri://`, so origins like `http://localhostevil.com`,
  `http://127.0.0.1.evil.example`, and `tauri://evil.example`
  passed CORS. Combined with `GET /token` being exempted from
  the auth middleware (it is gated only by loopback peer IP --
  which a victim's own browser satisfies), a malicious page
  could read `gateway.token` cross-origin and drive the local
  capsem service. The new
  `crates/capsem-gateway/src/cors.rs::is_allowed_origin` parses
  the Origin as a URI and accepts only exact matches for
  `http`/`https` to `localhost`, `127.0.0.1`, or `::1`, plus
  `tauri://localhost`; any path/userinfo/query/fragment, any
  unknown scheme, and any host suffix attack are rejected.
  22 unit tests cover the positive and negative matrix and the
  predicate is now shared between production and the
  integration test in `main.rs` so they cannot drift.

### Fixed (mitm-redesign T3 closure -- in-VM gate)
- **Host vsock listener registration was missing
  `VSOCK_PORT_DNS_PROXY` (5007) and `VSOCK_PORT_AUDIT` (5006).**
  In-VM smoke surfaced the DNS half: `capsem-dns-proxy` queries
  failed with "Connection reset by peer (os error 104)" because
  the host kernel had no listener for vsock port 5007 to accept
  on. `crates/capsem-core/src/vm/boot.rs::vsock_ports` now
  includes both 5006 and 5007 alongside the existing 5000-5005,
  so the Apple VZ + KVM hypervisor backends register listeners
  on every port `dispatch_aux_connection` knows how to handle.
  The audit case was a latent bug -- `audit_events` had been
  silently empty in every session since the audit feature
  landed -- now incidentally fixed alongside the DNS one.
- **Diagnostics: `test_dns_resolves_to_local` (test_sandbox.py)
  and `test_allowed_domain` still asserted the legacy
  `10.0.0.1` dnsmasq sentinel.** Updated to match the T3.4
  cutover: DNS now resolves to a real upstream IP via the
  capsem proxy (accepting either IPv4 or IPv6 first-token
  shape, since some upstreams return AAAA-only). The
  `test_allowed_domain` step-by-step diagnostic now uses the
  resolved hostname for TCP/TLS steps instead of hard-coding
  10.0.0.1. `test_dns_blocked_domain_returns_nxdomain` was
  policy-dependent (the user's `~/.capsem/user.toml` may
  override `api.openai.com.allow`); replaced with
  `test_dns_nxdomain_propagates_from_upstream` which uses an
  RFC 2606 `.invalid` TLD that no upstream can resolve --
  a clean policy-independent NXDOMAIN E2E test that pre-T3
  dnsmasq would have wrongly answered with 10.0.0.1.
- **In-VM E2E gate result.** With the boot.rs fix + diagnostic
  updates: `capsem-doctor -k 'dns or proxy_listening or
  iptables_redirect'` returns 14/14 PASS in a temp VM. The
  full DNS path is validated end-to-end: libc -> iptables nat
  53 -> 1053 -> capsem-dns-proxy -> vsock 5007 -> host hickory
  handler -> upstream forward (1.1.1.1) OR NXDOMAIN
  short-circuit -> answer back. `dns_events` rows populate with
  `trace_id`, source_proto, upstream_resolver_ms.
  `pgrep dnsmasq` returns nothing.

### Added (mitm-redesign T3 follow-up `f.proptest`)
- **proptest property-based tests for the DNS wire codec.** New
  `crates/capsem-core/src/net/parsers/dns_parser/proptests.rs`
  with 7 properties (256 random cases each by default) closing
  the loop alongside the cargo-fuzz targets:
  - `parse_query_round_trip`: build a query with arbitrary
    name + qtype + id, parse it back, assert id / qname / qtype
    / qclass / extra_questions match.
  - `build_nxdomain_preserves_question`: NXDOMAIN response built
    from an arbitrary query parses back to a question with the
    same id / qname / qtype / qclass.
  - `build_servfail_preserves_question`: same shape, ServFail
    rcode.
  - `build_redirect_preserves_question_for_a`: redirect response
    with N arbitrary IPv4 IPs lands all N as A records (no
    cross-family filter loss).
  - `build_redirect_filters_cross_family`: redirect with
    1 IPv4 + 1 IPv6 + an A query yields exactly 1 answer
    (the IPv4) -- the cross-family filter holds.
  - `parse_query_does_not_panic_on_arbitrary_bytes`: 0..2000
    arbitrary bytes never panic. Mirrors the cargo-fuzz target's
    safety contract so a regression surfaces in `cargo test`
    even without nightly + cargo-fuzz installed locally.
  - `build_nxdomain_does_not_panic_on_arbitrary_bytes`: same.
  Strategies: `dns_name_strategy()` produces 2-3 label
  syntactically-valid lowercase DNS names; `qtype_strategy()`
  picks from A/AAAA/TXT/MX/CNAME/SRV/CAA/NS/SOA/PTR/HTTPS/ANY.
  New dev-dep `proptest = "1"` (test-only, no production
  surface). capsem-core lib at 1691 tests now (was 1684).

### Added (mitm-redesign T3 follow-up `f.cache`)
- **TTL-honoring LRU answer cache for the DNS proxy.** New
  `crates/capsem-core/src/net/dns/cache.rs` shipping
  `DnsAnswerCache`: bounded LRU (default 1024 entries) keyed on
  `(qname, qtype, qclass)`, value is the wire-format answer bytes
  + `expires_at` derived from `min(answer_TTL, max_cache_ttl)`
  with `[60s, 300s]` clamp (DEFAULT_MAX_TTL_SECS / MIN_TTL_SECS).
  Lazy expiry: an expired entry is popped on the next lookup +
  counted as a miss. Cache **eligibility**: only `Decision::Allowed`
  responses with rcode=0 are inserted -- block + redirect
  re-evaluate every query (admin can change either at any moment),
  and SERVFAIL / NXDOMAIN from upstream are not persisted (avoids
  amplifying a transient upstream blip into 5 minutes of wrong
  answers). Cache **coherence**: `cache.get()` re-checks
  `is_fully_blocked` AND `find_dns_redirect` on every hit -- a
  domain that becomes blocked or redirected after we cached its
  answer is invalidated lazily on the next access (the entry is
  popped + counted as a miss). Three new metrics:
  `mitm.dns_cache_hits_total`, `mitm.dns_cache_misses_total`,
  `mitm.dns_cache_evictions_total`. New `lru = "0.18"` capsem-core
  dep (small pure-Rust crate). Wired into `DnsHandler` via the
  new `with_cache` constructor; `with_default_resolver` enables
  it by default with default config. `new` (no cache) constructor
  is preserved so existing tests can assert the upstream path
  always runs without cache-hit interference. 18 cache unit tests
  (insert/get round-trip, qtype/qclass key independence, capacity
  eviction with LRU order, TTL clamps to MIN/MAX bounds,
  garbage-input falls back to MIN, NoData answer falls back to
  MIN, min-across-records, clear, default constants pinned) + 8
  handler integration tests (cache hit short-circuits upstream
  via blackhole-after-warmup, policy-now-blocks invalidates
  lazily, policy-now-redirects invalidates lazily, block path
  still NXDOMAINs without consulting cache, cache_hits_total +
  cache_misses_total metrics fire, NXDOMAIN-from-upstream is not
  cached, with_default_resolver enables caching, new() leaves
  cache=None). capsem-core lib at 1684 tests now (was 1658).
  Workspace clippy clean.

### Added (mitm-redesign T3 follow-up `f.observability`)
- **DNS path metrics + structured tracing span.** Three new
  metric names registered alongside the existing MITM ones:
  `mitm.dns_queries_total{decision}` (allowed / denied /
  redirected / error), `mitm.dns_handle_duration_ms` (histogram,
  end-to-end), `mitm.dns_upstream_duration_ms` (histogram,
  upstream-forward path only -- absent on policy short-circuit),
  `mitm.dns_upstream_failures_total`. `DnsHandler::handle` is now
  wrapped in a `mitm.dns.query` info-span recording `qname`,
  `qtype`, `decision`, `rcode`, and `upstream_ms` on exit so a
  single `RUST_LOG=capsem::net::dns=debug` traces one query from
  parse to answer. The handler was refactored to a thin
  `handle()` (span + metric emission) wrapping `handle_inner()`
  (the decision tree) so every exit path goes through the same
  observability stamp -- no drift between block / redirect /
  forward / error branches. 5 new tests against
  `metrics_util::DebuggingRecorder` assert the right counter
  fires per decision label, the upstream histogram is absent on
  policy short-circuit but present on the forward path, and
  `dns_upstream_failures_total` increments on resolver error.
  `metrics_util` was already a dev-dep from the T1 sprint;
  facade-only emission means a no-op overhead in production
  until T5 wires the OTel exporter (same shape as the existing
  MITM metrics).

### Added (mitm-redesign T3 follow-up `e`)
- **`capsem-bench dns-load` harness.** New
  `guest/artifacts/capsem_bench/dns_load.py` mirrors the
  mitm-load shape: drives the DNS proxy at concurrency
  1/10/50/200, measures rps + p50/p95/p99/p999 latency, counts
  errors, and reports a per-level rcode distribution
  (`{"denied": 1234}` for the policy-block path,
  `{"allowed": 1234}` for the upstream-forward path) so the
  output dovetails with `dns_events.decision` for cross-checks.
  Defaults to `api.openai.com` (a fully-blocked domain in the
  dev policy) so every query hits the NXDOMAIN short-circuit
  path -- isolates the proxy's per-query cost from real upstream
  variance. Override via `CAPSEM_BENCH_DNS_QNAME` /
  `_QTYPE` / `_DURATION` / `_TIMEOUT`. The harness builds DNS
  wire-format queries by hand (no dns-python dep needed) so the
  guest's bundled python is enough; the encoder helpers
  (`_encode_qname`, `_build_query`, `_decode_rcode`,
  `_RCODE_DECISION` map) come with 7 host-side unit tests
  pinning the wire format + the rcode-to-Decision lock-step.
  Wired into `__main__.py` as the new `dns-load` mode (gated
  off `all` like mitm-load -- 40s of pure proxy stress would
  dominate a casual `capsem-bench all` run). Baseline JSON
  capture deferred to junior who owns the bench runner this
  session per the resume prompt.

### Added (mitm-redesign T3 follow-up `d`)
- **`DnsRedirect` policy rule -- admin-configured DNS overrides.**
  New `DnsRedirect { matcher, qtype, answers, ttl }` rule kind on
  `NetworkPolicy::dns_redirects` lets an admin override DNS
  resolution for a specific qname (and optionally a specific
  qtype). The DNS handler checks redirects AFTER `is_fully_blocked`
  (a blocked domain stays NXDOMAIN; redirect never weakens block)
  and BEFORE the upstream forward (no network round-trip when the
  answer is pinned locally). Use cases: redirect telemetry domains
  to a local trap, simulate an unreachable name with a deterministic
  IP for test runs, /etc/hosts-style overrides without modifying
  the guest. New `Decision::Redirected` variant on
  `capsem_logger::events::Decision` (string `"redirected"`) so
  `dns_events` rows surface override hits via
  `WHERE decision = 'redirected'`. Builder
  `dns_parser::build_redirect_response(query_bytes, &[IpAddr],
  ttl) -> Result<Vec<u8>>` synthesizes A/AAAA answer records
  filtered by qtype (cross-family IPs silently skipped, yielding
  the standard "name exists, no record of that type" NoError +
  zero-answers shape). 9 new policy unit tests + 11 new handler
  integration tests + 8 new builder unit tests covering exact /
  wildcard match, qtype filter, qtype=None matches anything,
  cross-family filtering, mixed-family yields only matching,
  block-overrides-redirect (block path runs first), TTL
  propagation, multiple IPs, empty-answers nodata, and
  no-match-falls-through-to-upstream. capsem-core lib at 1653
  tests now (was 1591). Workspace clippy clean.

### Added (mcp-concurrency T3 angle 2)
- **Pooled rmcp stdio peers for the local builtin MCP server.** The
  gateway can now spawn N independent stdio subprocesses for one
  MCP server and round-robin tool calls across them, removing
  rmcp 1.6's per-`Peer` mpsc → driver-task → stdin funnel as a
  singleton bottleneck. New fields on `McpServerDef`: `pool_size`
  (None / 0 / 1 = no pool, current behavior; >1 = N peers) and
  `pool_safe_tools` (allowlist of tool names safe to round-robin;
  others pin to `peers[0]` so per-process state stays consistent).
  HTTP servers ignore `pool_size` (HTTP/2 multiplexes natively).
  Builtin pool defaults to `min(available_parallelism, 4)` (matches
  the inflight-cap rule from `d88a714`). `CAPSEM_MCP_BUILTIN_POOL`
  overrides for tuning / debugging (set to 1 to force pre-pool
  behavior; clamped [1, 16]). `pool_safe_tools = [echo, fetch_http,
  grep_http, http_headers]`; snapshot tools stay pinned to
  `peers[0]` (their `AutoSnapshotScheduler` is per-process and N
  peers would diverge silently). Single-shot smoke at the dynamic
  default on M5 Max (pool=4): c=200 mcp-load p99 = 28.2 ms (vs
  sprint gate ≤ 35 ms), rps = 9591 (vs sprint gate ≥ 8000); c=10
  rps 3628 → 8794 (+143 %) — the rmcp stdio funnel disappearing
  at low contention.
- **`CAPSEM_BUILTIN_PEER_INDEX` env var** on `capsem-mcp-builtin`.
  Peer 0 keeps the original `mcp-builtin.lock` singleton; peers
  1..N use `mcp-builtin-{idx}.lock` so the `capsem_guard::install`
  per-session-dir guard doesn't make pool peers exit 0 with
  "another instance holds the lock".
- **`CAPSEM_MCP_BUILTIN_POOL` added to capsem-service env-allowlist**
  (both create and resume paths) so ops/bench can tune without
  rebuilding.

### Added (mitm-redesign T3 follow-up `c`)
- **cargo-fuzz harnesses for the DNS wire-format codec.** Four
  libFuzzer targets at `crates/capsem-core/fuzz/fuzz_targets/`:
  `parse_query`, `build_nxdomain`, `build_servfail`, and
  `round_trip` (asserts that if `parse_query` succeeds then
  `build_nxdomain` succeeds AND the response re-parses to the
  same qname/qtype/qclass -- catches divergence between the parse
  and rebuild paths that would let malformed queries escape
  NXDOMAIN gating). Each `corpus/<target>/` is pre-seeded with
  the T3.b `.bin` fixtures for fast structural coverage. The
  `fuzz/` directory is a standalone cargo workspace so libFuzzer's
  instrumentation flags don't leak into the parent workspace's
  normal builds. Plan acceptance from `T3-dns-proxy.md`: each
  target must survive `cargo +nightly fuzz run <target> --
  -max_total_time=60` clean (run path documented in
  `crates/capsem-core/fuzz/README.md` alongside the triage
  workflow for any crash artifact).

### Added (mitm-redesign T3 follow-up `b`)
- **dns_parser on-disk wire-format fixture corpora.** 13 raw DNS
  wire-byte `.bin` fixtures live at
  `crates/capsem-core/src/net/parsers/dns_parser/fixtures/`,
  covering simple A / AAAA / TXT / MX / CAA / HTTPS queries, the
  multi-question case, NXDomain + ServFail synthetic responses,
  truncated query, header-only, lying-qdcount, and the
  compression-self-loop adversarial case. Loaded via
  `include_bytes!()` at compile time so test runs don't hit the
  filesystem. 13 round-trip tests + an `all_fixtures_have_nonzero_length`
  pin (catches "include_bytes! pointed at an empty file" failure
  modes) wire them into the existing dns_parser test suite.
  Bootstrapped + regenerated by a new
  `crates/capsem-core/examples/dns_fixture_gen.rs` (separate
  compilation unit so the include_bytes! / regen chicken-and-egg
  doesn't bite). Plain English: a hickory-proto upgrade that
  changes the on-the-wire encoding of any of these query shapes
  lights up in the test diff before it bites a real query, and
  cargo-fuzz can corpus-seed from these exact bytes.

### Added (mitm-redesign T3 follow-up `a`)
- **dns_parser test breadth: record types + adversarial.** 32 new
  unit tests covering CNAME / NS / SOA / PTR / SRV / CAA / HTTPS /
  ANY / NULL / HINFO / AXFR / IXFR record types, all five DNS
  classes (IN / CH / HS / NONE / ANY), and risk-shape inputs:
  empty / single-byte / header-only / lying-qdcount / oversized
  qdcount=65535 / label compression self-loop / forward pointer
  past EOF / label > 63 bytes / NUL byte in label / truncated
  question section / max-label (63 bytes) accepted / NXDOMAIN
  preserves obscure qtype (CAA) and non-IN qclass / SERVFAIL
  rejects undecodable input. Total dns_parser tests: 46 (was 14).
  No production code changed -- pure additive coverage so a
  hickory-proto upgrade that quietly drops a record-type variant
  or breaks compression-bomb defense lights up before it bites a
  real query.

### Changed (mitm-redesign T3.4)
- **Guest cutover from dnsmasq to capsem-dns-proxy.** The
  in-guest dnsmasq fake (which resolved every name to the sentinel
  `10.0.0.1` so the MITM proxy could intercept connections) is
  gone. `capsem-init` now launches `capsem-dns-proxy` (T3.2) and
  installs iptables nat rules redirecting UDP/TCP port 53 to the
  proxy's `127.0.0.1:1053` listener. DNS queries now traverse the
  vsock envelope to the host's hickory-backed handler (T3.1)
  which applies the shared `NetworkPolicy` and forwards to a real
  upstream nameserver. `dig anthropic.com` from a guest returns a
  real answer; `dig api.openai.com` returns NXDOMAIN with the
  decision logged in `dns_events` (T3.3). The `dnsmasq` package
  is dropped from `guest/config/packages/apt.toml`, so the next
  rootfs rebuild leaves the binary out of the squashfs entirely.
  Diagnostics updated: `test_sandbox::test_dnsmasq_running` is
  replaced with `test_dns_proxy_running` plus a new
  `test_dnsmasq_not_running` that pins the cutover.
  `test_network` swaps the dnsmasq sentinel checks for two new
  acceptance tests: `test_dns_resolves_via_capsem_proxy` (a
  policy-allowed name resolves to a real IP, not the legacy
  10.0.0.1) and `test_dns_blocked_domain_returns_nxdomain` (the
  host policy short-circuits api.openai.com to NXDOMAIN before
  hitting the upstream resolver). Boot-stage marker added:
  `dns_proxy` between `net_proxy` and the rest of the boot
  sequence.

  End-to-end VM validation + `mitm-load` regression check still
  pending: the dev `capsem` binary needs codesigning (handled by
  the `just` recipes) and the `~/.capsem/assets/` install needs
  a `just install` to pick up the rebuilt initrd. Both fall
  under the junior-dev-owned bench runner this session, so the
  final acceptance gate is staged but not yet executed -- code,
  cross-compile, initrd repack (validated end-to-end via the
  Docker `agent` recipe), workspace clippy, and full Rust test
  suite are all green.

### Added (mitm-redesign T3.3)
- **`dns_events` telemetry table + per-query event row + trace_id
  correlation.** New `dns_events` schema in `capsem-logger`
  (timestamp, qname, qtype, qclass, rcode, decision, matched_rule,
  source_proto, process_name, upstream_resolver_ms, trace_id) with
  indexes on `(timestamp, qname, trace_id, decision)` for the
  inspect-session join. New `DnsEvent` event struct +
  `WriteOp::DnsEvent` + `insert_dns_event` writer; idempotent
  schema migration so existing DBs pick up the new table without a
  rebuild. New free function
  `capsem_core::net::dns::build_dns_event(result, source_proto,
  process_name, trace_id) -> DnsEvent` (pure, sqlite-free) +
  `serve_dns_session` in `capsem-process::vsock` calls it after
  every handler invocation and pushes the row through the shared
  `DbWriter` via `try_write` (matches the audit-event back-pressure
  pattern). `trace_id` is the ambient capsem trace id, so a single
  agent action joins across `dns_events` and `net_events` -- a
  `curl https://anthropic.com/` shows up as one `dns_events` row
  ("anthropic.com" allowed, qtype=A, rcode=0) plus one `net_events`
  row, both stamped with the same trace_id. 6 new
  capsem-core::net::dns::telemetry tests (allowed, denied,
  undecodable, decision strings round-trip with logger convention,
  source_proto optional, process_name passthrough) + 2 new
  capsem-logger writer tests (dns_event_insert_populates_row,
  dns_events_indexed_by_trace_id_for_join) + 3 new schema tests
  (create includes dns_events, migrate idempotent, indexes
  present). Bench gate still deferred to T3.4 (zero MITM hot-path
  code touched).

### Added (mitm-redesign T3.2)
- **vsock DNS envelope + guest `capsem-dns-proxy` listener.** New
  vsock port `VSOCK_PORT_DNS_PROXY = 5007` (`capsem-proto`)
  carries length-framed `rmp-serde` `DnsRequest` / `DnsResponse`
  envelopes between the guest agent and the host's `DnsHandler`.
  The host side (`capsem-process::vsock::serve_dns_session`)
  performs one envelope round-trip per vsock connection: read a
  `DnsRequest`, run `DnsHandler::handle` (T3.1), write a
  `DnsResponse`, close. The guest side is a new agent binary
  `capsem-dns-proxy` that listens on `127.0.0.1:1053` (UDP + TCP
  on the same port; iptables NAT will redirect 53 -> 1053 in
  T3.4) and opens a fresh vsock conn per query. The `DnsHandler`
  was retrofitted to take the same `Arc<RwLock<Arc<NetworkPolicy>>>`
  hot-swappable shape as `MitmProxyConfig` so an admin policy
  edit propagates to both protocols at once. The agent crate
  stays hickory-free -- it forwards raw bytes only. 9 new
  capsem-proto envelope tests (port-distinctness, request /
  response roundtrip, no-process-name path, compactness,
  garbage rejection, IPC-frame disjointness) + 5 new agent-bin
  unit tests pinning the listen port (1053), vsock port (5007),
  EDNS payload size, proto labels. Pre-T3.4 the `capsem-dns-proxy`
  binary is built and packaged but NOT launched -- T3.4 wires it
  into `capsem-init` alongside the iptables redirect for port 53
  and removes the dnsmasq invocation. Until then dnsmasq is still
  the guest's DNS server.

### Added (mitm-redesign T3.1)
- **Host-side DNS handler + UDP forwarder + wire-format parser.**
  New `capsem-core::net::dns` module (`server`, `resolver`) plus
  `capsem-core::net::parsers::dns_parser`. The `DnsHandler` is the
  bytes-in / bytes-out async processor that decodes a DNS query,
  consults the shared `NetworkPolicy::is_fully_blocked` rule, and
  either synthesizes an NXDOMAIN response (`Decision::Denied`),
  forwards the bytes verbatim to one of N upstream nameservers
  (default `1.1.1.1:53`, `8.8.8.8:53`; `Decision::Allowed`), or
  returns a synthetic SERVFAIL when every upstream is
  unreachable (`Decision::Error`). Read-only domains still
  resolve so the MITM proxy keeps its verb-level audit trail.
  Built on `hickory-proto = "0.26"` (workspace dep,
  `default-features = false, features = ["std"]`) -- the agent
  crate stays hickory-free; it'll forward raw bytes when T3.2
  wires the vsock envelope. 14 parser unit tests + 10 handler
  end-to-end tests against a fake `127.0.0.1:0` UDP upstream.
  Not yet wired into anything; T3.2 brings the vsock bridge,
  T3.3 the `dns_events` schema + telemetry hook, T3.4 cuts the
  guest image over from dnsmasq to iptables redirect.

### Performance (mcp-concurrency)
- **MCP gateway in-flight cap now scales with host CPU.** Default
  `DEFAULT_MCP_INFLIGHT` constant replaced with
  `default_inflight_cap()` = `available_parallelism * 4`. Anchors
  to the empirical sweet spot we measured on Apple M5 Max (18 cores,
  64 permits optimal) and tracks host shape automatically.
  `CAPSEM_MCP_INFLIGHT` continues to override the computed default.
  Sample mappings: 8-core -> 32, 16-core -> 64, 18-core (M5 Max) ->
  72, 32-core -> 128. Fallback when `available_parallelism()` itself
  fails: 8 cores -> 32 permits.
- **mcp-load throughput +62 % at concurrency 200; tail -24 %.**
  Three changes shipped together so the regression we measured when
  T1.2 + T1.3 were tried alone (p99@200: 40 → 358 ms, mitm rps -40 %)
  cannot land on its own again:
  1. **T1.2: aggregator subprocess pipelined.** `capsem-mcp-aggregator`
     no longer reads-then-handles-then-writes in one task; the reader
     spawns `handle_request` per incoming msgpack frame and a single
     writer task drains an `mpsc<AggregatorResponse>(256)` to stdout.
     `Shutdown` is acked synchronously on the reader path before the
     drain so we can't lose the ack to a stuck handler.
  2. **T1.3: hot manager lock eliminated.** `McpServerManager` now
     exposes `dispatch_call_tool` / `dispatch_read_resource` /
     `dispatch_get_prompt` that perform the lookup synchronously and
     return owned `impl Future + Send + 'static` futures. The
     aggregator wraps the manager in `std::sync::RwLock`; the sync
     read guard drops before the rmcp RPC is awaited, so concurrent
     dispatches never serialise on the manager.
  3. **T1.5: bounded concurrency at the gateway.** The MCP gateway in
     `capsem-core::mcp::gateway::serve_mcp_session` now acquires a
     `tokio::sync::Semaphore` permit BEFORE `tokio::spawn`-ing each
     handler. Default cap 64 (override via `CAPSEM_MCP_INFLIGHT`,
     forwarded through the capsem-service env-allowlist). Without
     this cap, T1.2 + T1.3 turn the MCP path into a CPU-starvation
     source for the rest of capsem-process (notably the MITM proxy on
     the same tokio runtime).
  Bench (Apple M5 Max, 2 vCPU bench VM, vs T1.1-only baseline at
  HEAD): mcp-load c=10 rps 3370 → 9160 (+172 %), c=50 rps 3081 →
  8633 (+180 %), c=200 rps 5224 → 8464 (+62 %), p99@200 57.1 →
  43.4 ms (-24 %), p999@200 67.9 → 53.4 ms (-21 %). mitm-load
  c=200 rps 2845 → 2968 (+4.3 %), p99 177 → 170 ms (-3.8 %) — both
  paths better, neither path regressed. Sprint MCP rps@200 gate
  (≥ 8000) cleared; the 35 ms p99@200 gate is still 8 ms over and
  is tracked as T3 in `sprints/mcp-concurrency/tracker.md`.

### Added (mitm-redesign)
- **T2 plain-HTTP coverage: adversarial / risk-shape tests.**
  Five more tests on top of the parsing-correctness ones, each
  hitting a real failure mode the proxy could plausibly meet in
  the wild:
    * `…body_larger_than_preview_cap_forwards_full_but_caps_preview`
      -- 16 KB request body (4x default `max_body_capture`).
      Asserts upstream receives the full body byte-for-byte,
      `NetEvent.bytes_sent == 16384`, but
      `NetEvent.request_body_preview` length <= 4096 and starts
      with the first 4 KB block (no later block leaked through
      the cap).
    * `…ipv6_host_header_does_not_silently_succeed` -- inbound
      `Host: [::1]:8080`. The host parser explicitly bails on
      `[`-prefixed hosts; the proxy must NOT 200 on the implicit
      ("", 80) fallback. Asserts response is 502 or 403, never
      200, with a non-Allowed `Decision`.
    * `…corrupted_gzip_response_doesnt_crash` -- upstream sends
      `Content-Encoding: gzip` plus a valid 10-byte gzip header
      followed by 61 bytes of garbage payload. With a 5s read
      deadline, the test asserts: (a) the proxy still emits
      exactly one `NetEvent` (= `on_response_end` fired = no
      panic on the response path), and (b) `bytes_received == 0`
      because `flate2::Decompress` yields nothing on a
      fully-corrupt deflate body. Future regressions that would
      leak pre-decode bytes here get caught.
    * `…truncated_upstream_response_doesnt_hang` -- upstream
      advertises `Content-Length: 1000` but writes only 33 bytes
      then closes. With a 5s read deadline. Asserts the proxy
      doesn't hang AND `bytes_received <= 33` AND `< 1000` (i.e.
      we record the actual bytes received, not the lying
      Content-Length).
    * `…zero_length_response_body_emits_netevent` -- 200 OK with
      `Content-Length: 0`. Asserts the chunk-hook chain still
      fires `on_response_end` on an empty body and emits exactly
      one `NetEvent` with `bytes_received == 0`.
  26 mitm_integration tests pass (17 plain-HTTP + 8 TLS + 1
  ignored throughput); 1542 lib tests pass; clippy clean.
- **T2 plain-HTTP coverage: verbs, query strings, header
  passthrough + secret redaction.** Four more integration tests
  on top of the structural ones, closing the parsing-correctness
  gap:
    * `mitm_proxy_plain_http_records_every_http_method` -- sends
      GET / HEAD / OPTIONS / POST / PUT / DELETE / PATCH on one
      keep-alive connection, asserts seven separate `NetEvent`
      rows each with the right `method` + `path` + `204` status.
      Validates verb parsing across both read-classified and
      write-classified methods.
    * `mitm_proxy_plain_http_records_query_string_with_parameters`
      -- `GET /search?q=hello%20world&page=2&filter=active&tag=a&tag=b`.
      Asserts the upstream sees the full request line verbatim
      AND `NetEvent.path == "/search"` (no `?`) +
      `NetEvent.query == "q=hello%20world&page=2&filter=active&tag=a&tag=b"`.
      Repeated keys, equals signs, and percent-encoded values
      preserved verbatim.
    * `mitm_proxy_plain_http_forwards_custom_headers_to_upstream`
      -- sends `User-Agent` (allowlisted), `X-Trace-Id`,
      `X-Custom-Flag`, `Authorization: Bearer ...` (custom).
      Asserts the upstream receives every header by name + value
      verbatim, and that `accept-encoding` was rewritten to `gzip`
      (we only forward what we can decompress).
    * `mitm_proxy_plain_http_telemetry_hashes_non_allowlisted_headers`
      -- security-focused. Sends real-shaped secrets:
      `Authorization: Bearer SUPER-SECRET-...`,
      `X-Api-Key: live_pk_DEADBEEF_...`,
      `Cookie: session=ROTATE_ME_...`. Asserts
      `NetEvent.request_headers` does NOT contain any of those
      verbatim values (each is replaced with `hash:<12-hex>`),
      while the header NAMES still appear and allowlisted
      `User-Agent` + `Host` appear verbatim. Locks down the
      "secrets in telemetry" surface.
  Also tightened the keep-alive test's response reader to drain
  head + body per request rather than relying on one-shot
  `tcp.read()` (was order-flaky on a busy CI). 21 mitm_integration
  tests pass; 1542 lib tests pass; clippy clean.
- **T2 plain-HTTP integration coverage extended.** Five new
  integration tests close the "ad-hoc verification" gap left by
  the earlier Ollama smoke. The new tests share a
  `spawn_fake_upstream(serve)` helper + a `read_http11_request`
  drainer so each test parameterizes the upstream's behavior:
    * `mitm_proxy_plain_http_post_forwards_body_and_records_bytes_sent`
      -- POST with body. Asserts the upstream sees the JSON body
      verbatim + `NetEvent.bytes_sent` covers the body.
    * `mitm_proxy_plain_http_chunked_streaming_response_aggregates_bytes`
      -- fake upstream sends `Transfer-Encoding: chunked` with 4
      data frames. Asserts the client sees every chunk +
      `NetEvent.bytes_received` equals the concatenated payload
      length (proves the ChunkDispatchBody runs the sync
      ChunkHook chain across multiple frames and the
      end-of-stream NetEvent emission fires).
    * `mitm_proxy_plain_http_keep_alive_emits_one_netevent_per_request`
      -- single client TCP connection, three back-to-back GETs to
      `/a`, `/b`, `/c`. Asserts three separate `NetEvent` rows,
      each with the right path/method/status/port/conn_type.
      Validates the per-connection cached upstream sender +
      keep-alive on the plain-HTTP branch.
    * `mitm_proxy_plain_http_preserves_host_header_to_upstream`
      -- captures the bytes the upstream observed. Asserts the
      inbound `Host: 127.0.0.1:<port>` header is forwarded
      verbatim. (TLS path rewrites Host from SNI; HTTP must not.)
    * `mitm_proxy_plain_http_unresolvable_upstream_emits_502_netevent`
      -- targets `nonexistent.invalid` (RFC 6761). Asserts 502
      back to the client + one `NetEvent` with `Decision::Error`,
      status 502, conn_type http-mitm, and the dial error in
      `matched_rule`. No silent drop on dial failure.
  17 mitm_integration tests pass (8 plain-HTTP + 8 TLS + 1
  ignored throughput); 1542 lib tests pass; clippy clean.
- **T2 verified end-to-end against real Ollama on
  `127.0.0.1:11434`.** From inside an air-gapped VM, `curl
  http://127.0.0.1:11434/api/tags` rides the full new pipeline:
  iptables redirect (port 11434 → 10080), agent listener on
  10080, vsock bridge, host first-byte sniff (T2.1), Host header
  parse + port allowlist (T2.2), plain TCP upstream dial, 357-byte
  JSON response forwarded verbatim to the guest. NetEvent recorded
  with `port=11434, conn_type=http-mitm, decision=allowed,
  status=200`. As part of the verification,
  `DEFAULT_HTTP_UPSTREAM_PORTS` is bumped from `[80]` to
  `[80, 3128, 3713, 8080, 11434]` so the host policy default mirrors the iptables
  rules in `capsem-init` -- otherwise port 11434 traffic gets
  redirected to 10080, hits the host proxy, and is rejected by
  the policy gate, which is the wrong default for the canonical
  local-LLM workflow this protocol path was designed for. New
  ports get added by editing the shared policy config and guest redirect lists
  in tandem.
- **T2 (agent-side): plain-HTTP listener + iptables redirects.**
  `capsem-net-proxy` now listens on `127.0.0.1:10080` in addition to
  the original `:10443`; a `run_listener(port)` helper drives the
  per-port accept loop, and both targets the same vsock port
  `VSOCK_PORT_SNI_PROXY` (5002) -- the host's first-byte sniff
  (T2.1) classifies on wire bytes, so the guest-side listener split
  is just an iptables-target convenience. `capsem-init` adds two
  `iptables -t nat -A OUTPUT -p tcp --dport <N> -j REDIRECT
  --to-port 10080` rules for `:80` (plain HTTP) and `:11434`
  (Ollama default); the post-launch readiness poll waits for both
  `:10443` and `:10080` before declaring the proxy ready. Three
  new in-VM diagnostics cover the wiring:
  `test_iptables_redirect_80_to_10080`,
  `test_iptables_redirect_11434_to_10080`, and
  `test_net_proxy_http_listening`. Three new agent unit tests pin
  the new constant + cross-port distinctness. Cross-compile
  (`aarch64-unknown-linux-musl`) clean. The configurable
  guest-side allowlist (read from `policy_config`) is deferred --
  the host-side `NetworkPolicy.http_upstream_ports` is the
  authoritative gate, and adding a config plumb to the guest-side
  iptables list is its own follow-up.
- **T2.3: Ollama-shaped end-to-end test for the plain-HTTP path.**
  `mitm_proxy_plain_http_ollama_shape_records_telemetry` spins a
  fake plain-HTTP upstream on `127.0.0.1:0`, configures the proxy
  with that OS-assigned port on its `http_upstream_ports` allowlist
  + `127.0.0.1` on the domain allowlist, sends `POST /api/generate`
  with the typical Ollama request shape (model + prompt JSON body),
  and asserts: (a) the upstream's response body is forwarded
  verbatim, (b) the resulting `NetEvent` records
  `method=POST`, `path=/api/generate`, `status=200`,
  `domain=127.0.0.1`, `port=<upstream_port>`,
  `conn_type=http-mitm`, `decision=Allowed`, with non-zero
  `bytes_sent` / `bytes_received`. Adds `make_proxy_config_full`
  helper to override the `http_upstream_ports` allowlist
  (existing tests stay on the default `[80]`). 12 mitm_integration
  tests pass.
- **T2.2 (host-side): plain HTTP serves through the same hyper
  pipeline as TLS.** When the first-byte sniff (T2.1) classifies a
  connection as `Protocol::Http`, the listener now skips rustls
  entirely and runs `hyper::server::conn::http1::Builder::new()
  .serve_connection(io, svc)` directly on the vsock stream
  (`ReplayReader` carries the buffered first bytes). Per-request
  domain + upstream port are parsed from the inbound `Host` header
  by `parse_http_host_target` (T2.2 helper in `mitm_proxy/util.rs`)
  and threaded through `handle_request` as a new `upstream_port:
  u16` parameter; the inbound `host` header is preserved (it's
  authoritative for plain HTTP), unlike the TLS path which still
  rewrites it from the SNI domain. The hyper service closure runs
  the same PolicyHook and ChunkHook chain as TLS, so domain
  policy, decompression, SSE parsing, AI interpreters and
  Telemetry all apply uniformly. Upstream dials branch on
  `protocol`: TLS does TCP+rustls+http1::handshake, HTTP does
  TCP+http1::handshake (no TLS step). Telemetry: every
  `TelemetryRequestContext` carries `port: u16` + `conn_type:
  &'static str` (`https-mitm` / `http-mitm`); `NetEvent` rows now
  reflect the actual upstream port and transport label so
  operators can split HTTPS vs plain-HTTP traffic in `session.db`.
  `MitmProxyConfig::handle_inner` is split into `serve_tls`,
  `serve_plain_http`, and a shared `serve_pipeline` helper that
  drives the hyper server over either an `IO: hyper::rt::Read +
  hyper::rt::Write`. New `NetworkPolicy::http_upstream_ports:
  Vec<u16>` (default `[80]`) gates plain-HTTP upstream ports
  before the dial -- a request whose `Host` header carries an
  allowlist-missing port is rejected with a 403 + Decision::Denied
  + `matched_rule = "http-port-not-allowlisted({port})"`. The TLS
  path is unaffected by the allowlist (always uses 443).
  Two new integration tests cover the path:
  `mitm_proxy_plain_http_denies_disallowed_host` (PolicyHook 403
  on a disallowed Host) and
  `mitm_proxy_plain_http_denies_port_not_in_allowlist` (port-gate
  403). 1539 lib tests + 11 mitm_integration tests pass; clippy
  clean. Agent-side multi-port listener and iptables rules ship
  separately so the in-VM test (T2.3) can drive them.
- **T2.1: first-byte protocol sniff (TLS vs plain HTTP) on the vsock
  listener.** New `mitm_proxy::protocol` module with `Protocol` enum
  (`Tls` / `Http` / `Unknown`) and `detect(&[u8]) -> Option<Protocol>`
  classifier. The `vsock:5002` accept path now peeks the first
  post-meta payload byte: `0x16` -> TLS (existing path, unchanged);
  uppercase ASCII (`0x41..=0x5A`, the HTTP method set) -> plain HTTP
  classified but routed to a "T2.2-pending" connection-level error
  (the actual hyper plain-HTTP server lands in T2.2); other bytes ->
  `Unknown` connection-level error. The `mitm.connections_total`
  counter, previously hard-coded to `protocol="tls"` on every accept,
  is now incremented post-sniff with the correct label so operators
  can distinguish TLS / HTTP / unknown traffic. `mitm.requests_total`
  + the upstream-error increments propagate the same label.
  `ConnMeta` carries a `protocol: Protocol` field set from the sniff;
  every hook reads it through `ctx.conn().protocol`. 8 unit tests in
  `protocol/tests.rs` cover the byte-level rules (record types
  `0x14`/`0x15`/`0x17` rejected; lowercase methods rejected; high-bit
  junk rejected) plus 2 integration tests in `mitm_integration.rs`
  asserting the plain-HTTP and unknown-byte paths each emit the
  right `NetEvent`.

### Changed (mitm-redesign)
- **T1 closes -- legacy async body chain deleted; sync ChunkHook
  pipeline owns the response path end-to-end.** Slice 9 cleanup.
  Removes `mitm_proxy/telemetry.rs` (`TelemetryEmitter` +
  `TelemetryBody`, ~390 lines), `ai_traffic/ai_body.rs`
  (`AiResponseBody`, ~155 lines), `body::DecompressBody` +
  `body::BodyStream` + `body::RespStatsKind` (one
  `async_compression::tokio::bufread::GzipDecoder` adapter, one
  `tokio_util::io::StreamReader`, one `Body→Stream` shim). The
  inline `if is_gzip { DecompressBody::new(...) }` block in
  `handle_request` is gone -- the inline `if is_gzip` now only
  strips Content-Encoding / Content-Length headers (a few field
  accesses on the parts struct, kept inline because moving it to
  an async hook would re-introduce the same plumbing the slice
  removed). All four ChunkHooks are pure sync: `DecompressionHook`
  (`flate2::Decompress::new(false)`), `SseParserHook`, three
  `InterpreterHook`s, `TelemetryHook` -- per-chunk work runs inline
  from `poll_frame` with no `.await`, no channel hop, no async
  wrapper. `TelemetryHook` is wired into
  `make_production_pipeline` + reads its per-request context out
  of a `HookState` slot seeded by `handle_request` (new
  `HookState::set::<T>()` + `ChunkDispatchBody::seed::<T>()`
  builder). `MitmProxyConfig` is refactored to hold
  `Arc<TelemetryDeps> { db, pricing, trace_state }` instead of
  by-value `pricing` + `Mutex<TraceState>` -- the `Arc` breaks
  the would-be config↔pipeline↔hook reference cycle (the hook
  points at `TelemetryDeps`, not the surrounding config).
  `make_production_pipeline` signature now takes the
  `Arc<TelemetryDeps>`; `capsem-process` construction site +
  in-tree test fixtures + the integration test in
  `crates/capsem-core/tests/mitm_integration.rs` updated. The
  redundant `TelemetryEmitter` / `TelemetryBody` / `DecompressBody`
  / `emit_model_call` / `trace_chains_across_tool_use` test
  fixtures in `mitm_proxy/tests.rs` are deleted -- the same
  surfaces are covered by the per-hook tests in
  `telemetry_hook/tests.rs` (NetEvent + ModelCall builders),
  `decompression_hook/tests.rs` (gzip streaming), and the
  remaining integration tests still exercise the full path
  end-to-end via `handle_connection`.

  **Bench: SSE parser microbench at 478-488 MiB/s (up from 449-472
  MiB/s in the T0 pre-rewrite baseline; criterion reports
  "Performance has improved" with p<0.05).** Sync ChunkHooks are
  structurally faster than the async wrappers they replace.
  `capsem-bench mitm-load` against
  `benchmarks/mitm-load/baseline.json` is the integration gate;
  it requires a built VM image and is run on a real-machine
  session (this commit's verification rests on the criterion
  micro-bench + the 8 in-tree integration tests through the
  full MITM path).

  1531 capsem-core lib tests pass (down from 1547 -- the deleted
  redundant fixtures); 8/8 mitm_integration tests pass; clippy
  clean.

### Performance (mcp)
- **Pipelined the MCP gateway loop**
  (`crates/capsem-core/src/mcp/gateway.rs`). The per-vsock-connection
  serial `read → handle → write` loop is replaced with a reader that
  spawns one `tokio::spawn(handle_json_rpc)` per request and a
  dedicated writer task that drains an `mpsc::Receiver<Vec<u8>>`(256).
  Out-of-order responses are fine — JSON-RPC `id` lets the client
  demux. mcp-load (single fastmcp Client over one vsock) gains
  **+30 % rps@200 (4 252 → 5 551) and -44 % p99@200 (70.95 → 39.73 ms)**;
  mitm-load unchanged (±2.6 %). Next ceiling is the aggregator
  subprocess loop (T1.2 in `sprints/mcp-concurrency/`).

### Fixed (mcp)
- **`capsem_host_logs` / `capsem_panics` / `capsem_triage` /
  `capsem_timeline` no longer corrupt query values with reserved
  characters.** Each tool built its URL via raw
  `format!("k={}&", value)` interpolation. Two failure modes,
  both reproduced via live MCP:
  1. Any value containing whitespace (e.g. `grep="capsem-gateway
     spawned"`) failed with `invalid uri character` because the
     URL parser rejects unencoded spaces. **Multi-word grep was
     completely broken.**
  2. Any value containing `&` (e.g. `grep="foo&bar"`) was silently
     truncated to `foo` because the server's query parser saw the
     unescaped `&` as a separator and treated `bar` as a stray
     empty param.
  Same risk on `=`, `+`, `#`, `%`, `?`, and other reserved chars
  in `since`, `id`, `trace_id`, `layers`. Fix in
  `crates/capsem-mcp/src/main.rs`: new `query_string` helper
  builds the query from a list of `(key, Option<value>)` pairs,
  percent-encoding each value with an explicit RFC 3986
  query-value set (CONTROLS plus all reserved/unsafe ASCII;
  ALPHA/DIGIT and the unreserved `-._~` round-trip plain).
  Refactored the 4 tools to use it; trailing-`&` cosmetic issue
  fixed as a side effect. 8 new unit tests cover empty/single/
  multiple/None-skipping/space/`&`/multi-reserved-chars/unreserved-
  passthrough. `capsem_service_logs` was unaffected (does
  client-side filtering); the other 21 tools use JSON bodies or
  path-only URLs and don't take untrusted query values.

### Fixed (build)
- **`just _pack-initrd` no longer corrupts the hash-named hardlink
  while a stress run is mid-`VmConfig::build`.** The recipe wrote
  the gzipped cpio archive via shell redirect (`gzip > "$INITRD"`),
  which truncates the existing inode in place. `create_hash_assets.py`
  later gives `initrd.img` a hash-named hardlink (e.g.
  `initrd-<hex16>.img`, sharing the inode). An in-place rewrite
  mutates that hardlink's content too, so any concurrent VM mid-
  `VmConfig::build` reading the old hash-named path computes a hash
  of the NEW bytes and rejects with `hash mismatch for ...img:
  expected X, got Y` -- a stress run hit by a parallel `just
  _pack-initrd` lost two cycles per race (observed in
  `target/stress-acceptance-logs/iter-6.log` cycles 48-49 with
  unified-log evidence of `cpio` running at the exact failure
  timestamp). Fix in `Justfile`: write to `${INITRD}.tmp.$$` and
  `mv` to the final path. The atomic rename leaves the old inode
  (and its hash-named hardlink) intact until `_cleanup_stale` in
  `create_hash_assets.py` explicitly unlinks the old alias.

### Fixed (resume,protocol)
- **Stress-cycle "doesn't have entitlement" cascade now self-recovers
  via launchd-cleanup-aware retry.** Apple's
  `Virtualization.framework` runs a per-VM XPC helper
  (`com.apple.Virtualization.VirtualMachine.<UUID>`); when
  capsem-process dies, launchd schedules that XPC's cleanup with a
  9-second delay (observed in `log show`: `scheduling cleanup in 9
  sec after sending Killed: 9` followed by `internal event:
  PETRIFIED`). Under rapid VM churn (~3s/cycle) the cleanup queue
  grows; once `syspolicyd` saturates (`Unable to get certificates
  array: (null)` in the unified log just before the failure
  window), the next freshly-spawned capsem-process's
  `VZVirtualMachineConfiguration.validateWithError()` returns
  NSError code 2 with the misleading
  `localizedDescription = "...The process doesn't have the
  'com.apple.security.virtualization' entitlement."` -- even though
  the binary IS entitled. We saw this fire as 2-cycle cascades at
  ~cycle 37-40 of the 50-cycle stress (iter-2 cycles 37-38; iter-6
  cycles 39-40 post-Bug-C-fix). Two-part fix in
  `crates/capsem-service/src/main.rs`:
  (1) New `is_launchd_cleanup_transient` helper pattern-matches
  the full VZ-specific phrase (`com.apple.security.virtualization`
  + `entitlement`) on the failed-attempt's process.log tail. Does
  NOT match a bare `entitlement` mention so a real codesign
  regression still surfaces.
  (2) `handle_provision` extracts the per-attempt logic into
  `provision_attempt` and wraps it in `capsem_core::poll::poll_until`
  with `timeout=8s, initial_delay=200ms, max_delay=500ms`. On
  `LaunchdTransient` outcome the loop unregisters the failed
  attempt's persistent-registry entry + clears the instances map,
  then retries; everything else (`BootCrash`, `ProvisionError`,
  `Ready`) bails or succeeds immediately. Retry-decision routing
  is a pure function (`classify_attempt_decision`) so the retry
  logic is unit-testable without spawning a real VM. Worst-case
  user-visible latency on a healthy launchd is unchanged
  (single attempt, ~3-5s); under cascade the retry adds ~500ms-1s
  of backoff per failed attempt, amortized against the launchd
  drain. Unit coverage: 4 matcher tests + 6 routing tests covering
  Ready/StillBooting/LaunchdTransient/BootCrash/already-exists
  /generic-provision-error.
- **Post-resume `vsock_connect` ECONNRESET no longer poisons the agent's
  exec dedup cache.** After `restoreMachineStateFromURL` the host's
  vsock listener for the EXEC port (5005) is registered but the
  kernel-side accept queue can briefly reset incoming connections
  while VZ attaches it. The agent's `run_exec` opened that connection
  with a single-shot `vsock_connect`; one ECONNRESET → `run_exec`
  returned 126 → `exec_done` cached `id → 126` → every host-watchdog
  retry of the same Exec id replayed `ExecDone {exit_code: 126}`,
  even after the transport recovered. Captured in serial.log as
  `exec[N] vsock connect failed: Connection reset by peer (os error
  104)` followed by `exec[N] duplicate (already done, exit=126);
  replaying ExecDone`. Two-part fix in
  `crates/capsem-agent/src/main.rs`: (1) new
  `vsock_connect_with_econnreset_retry` helper retries on
  `ErrorKind::ConnectionReset` only (5 attempts × 20ms backoff =
  ~100ms ceiling, well under the host's 1s watchdog window);
  non-ECONNRESET errors bail immediately so misconfiguration
  (refused / address-family-unsupported) is not papered over. (2)
  `run_exec` now returns `ExecOutcome::{Done(i32), TransportFailed}`;
  `control_loop` only inserts into `exec_done` when
  `outcome.should_cache()` -- transport failures stay uncached so
  the next host-watchdog retry gets a fresh attempt against the
  recovered vsock. The host still receives `ExecDone {exit_code:
  126}` so its watchdog resolves with a real ExecResult instead of
  hanging. Verified in real-VM stress: pre-fix cycle 1 hit this in
  the very first failure; post-fix 39 consecutive cycles pass before
  a different (separately-tracked) failure mode appears. Unit
  coverage: 7 new tests covering retry-success, retry-recovery,
  bail-on-other-kinds, exhaustion, cache-decision matrix.
- **Symmetric guest-side replay buffer with `HostToGuest::AckReply`.**
  Closes the bidirectional silent-drop hole: the prior bridge replay
  layer covered the host→guest forward path; this adds the matching
  guest→host return path. The agent now keeps every ackable
  `GuestToHost` response (`ExecDone` / `FileOpDone` / `FileContent` /
  `Error`) in a `pending_responses` map keyed by `id`, lifted to
  outer scope in `capsem-agent/src/main.rs` so it survives
  reconnects (the writer thread is per-`run_bridge`). On every
  fresh control conn the writer thread first replays every entry
  still in the map, then resumes normal writes. The host bridge in
  `capsem-process/src/vsock.rs` emits `HostToGuest::AckReply { id }`
  immediately on receipt of an ackable response; `control_loop`
  removes the entry. Without this, an ExecDone (or FileContent --
  worse, since the agent doesn't cache file bytes) lost on the
  Apple VZ silent-drop path was unrecoverable except via the
  host's watchdog re-sending the original `Exec`, which only worked
  for `Exec` (cached `exit_code`) and not for `FileRead`'s
  `FileContent`. Verified directionally with a 50-cycle
  `CAPSEM_STRESS=1 test_stress_suspend_resume.py` run, 50/50 passed.
- **Bridge replay layer with `GuestToHost::Ack` for ackable
  HostToGuest messages.** The control bridge in
  `capsem-process/src/vsock.rs` now keeps every ackable outbound
  message (`Exec` / `FileWrite` / `FileRead` / `FileDelete`) in a
  pending map keyed by `id` (`JobStore::pending_acks`). The agent
  emits `GuestToHost::Ack { id }` immediately on receipt, *before*
  any processing -- the bridge clears the entry. On every fresh
  control conn after a re-key, the bridge re-writes every entry
  still in the map. This is the protocol-level cover for Apple
  VZ's post-restoreState silent-drop pattern: the host's
  `write_control_msg` returns success while the bytes never
  propagate, so the previous single-slot `held: Option<HostToGuest>`
  (which only fired on write *errors*) couldn't catch them. The
  multi-slot map also recovers a message whose Ack was lost on the
  return path -- the message stays pending across reconnects until
  an ack actually lands. Agent dedup ensures a re-sent message that
  did land twice doesn't double-execute.
- **Watchdog recalibrated to 1s × 16 retries (16s budget)** -- with
  the bridge replay layer now handling forward-path losses, the
  watchdog only exists to cover the asymmetric return-path case
  (agent processed and sent ExecDone / FileOpDone, those bytes were
  silently dropped). 1s gives ~6× headroom over the longest
  observed healthy round-trip (~150ms for `bash -c "mkdir+echo+cat"`)
  without sitting idle for 3s of dead time.
- The earlier "8 × 3s = 24s budget" config (commit `8cc76e2`) is
  superseded -- the storm-derivation-based number was correct in
  intent but the bridge replay layer is the structurally right fix
  for forward-path drops.

### Added (mitm-redesign)
- **`TelemetryHook` -- per-request `NetEvent` + optional `ModelCall`
  emission as a sync `ChunkHook`.** T1 slice 8 (additive). Carries
  the entire emit surface that lives in `telemetry::TelemetryEmitter`
  today, packaged as a `ChunkHook` that fires on `on_response_end`.
  The hook owns its own response-side byte counting + preview, so
  once the legacy chain is removed in the cleanup slice it
  replaces both `TelemetryEmitter` (the per-request scratch
  struct) and `TelemetryBody` (the body wrapper that decided
  *when* to fire). Per-request context is read out of a typed
  `HookState` slot (`Option<TelemetryRequestContext>`); a missing
  slot puts the hook in shadow mode (no allocation, no emit). The
  per-call `LlmEventStream` populated by the interpreter hooks is
  read at end-of-stream and folded into the `ModelCall` via the
  existing `collect_summary` path. Pure builder helpers
  (`build_net_event` and `maybe_build_model_call`) are split out
  so tests verify the field-mapping logic without spinning up an
  async runtime or a real `DbWriter`. Trace-correlation
  (tool-use chains across requests) goes through a shared
  `Arc<Mutex<TraceState>>` exactly the way `TelemetryEmitter`
  does today, so existing trace-grouping behavior is preserved
  byte-for-byte. Hook is **not** yet registered in
  `make_production_pipeline` and `handle_request` is **not** yet
  rewired; those changes ship together with the deletion of
  `telemetry.rs`, the legacy `AiResponseBody` /
  `DecompressBody` wrappers, and the benchmark gate in slice 9
  cleanup. Eight unit tests covering: `NetEvent` field mapping,
  HEAD probe filter, non-LLM path filter, non-AI provider
  filter, `LlmEvent` flow into `ModelCall`, tool-use trace
  chaining across two requests, shadow-mode skip when context
  unseeded, byte counting + preview tally with seeded context.
  1547 capsem-core lib tests pass; clippy clean.

### Added (mitm-redesign)
- **`DecompressionHook` -- streaming gzip decompression as a sync
  `ChunkHook`.** T1 slice 7. Replaces the
  `async_compression::tokio::bufread::GzipDecoder` driving
  `body::DecompressBody` with the lower-level
  `flate2::Decompress` raw-deflate state machine plus a small
  hand-rolled gzip-header parser. gzip streaming-decode is
  fundamentally sync, so the async wrapper was plumbing-only
  (one `tokio::io::AsyncRead` adapter, one `StreamReader`, one
  `Body -> Stream` shim) -- removing it is the goal of the cleanup
  slice. The hook detects gzip from the first two bytes' magic
  prefix (`0x1f 0x8b`) since the per-request `HookState` slot map
  carried by `ChunkDispatchBody` isn't shared with async
  `Hook::on_event`'s state, so a `Content-Encoding: gzip` flag
  can't bridge from `RawResponseHead` into the chunk pass through
  that map. Magic detection sidesteps the issue without changing
  the hook trait. The header parser handles the standard 10-byte
  prefix plus FEXTRA / FNAME / FCOMMENT / FHCRC optional fields
  (RFC 1952 §2.3.1). After the header, the deflate body streams
  through `flate2::Decompress::new(false)` (`zlib_header=false` =
  raw deflate); the decoder retains state across chunks so partial
  blocks split anywhere decode correctly. Registered in
  `make_production_pipeline` BEFORE the SSE parser hook so the
  hook order is correct once the legacy inline `DecompressBody` is
  removed in slice 9 (today the hook is essentially a no-op
  because `DecompressBody` decompresses upstream of the
  `ChunkDispatchBody` and the hook sees plaintext bytes; that's
  intentional -- this slice ships the surface, the cleanup slice
  flips the switch). Six unit tests: single-chunk decompress,
  decompressed-bytes split across two chunks, plain non-gzip
  passthrough, classification stickiness (a chunk that happens to
  start with `0x1f 0x8b` after a non-gzip first chunk is left
  alone), byte-by-byte chunking, and one-byte-first-chunk
  classification deferred. 1539 capsem-core lib tests pass;
  clippy clean.

### Added (mitm-redesign)
- **Provider interpreter `ChunkHook`s -- Anthropic / OpenAI /
  Google.** T1 slice 6. Three concrete `ChunkHook`s that consume
  parsed `SseEvent`s from the upstream `SseEventStream` slot and
  emit provider-agnostic `LlmEvent`s into a shared `LlmEventStream`
  slot. Each interpreter gates on its provider's domain
  (`api.anthropic.com`, `api.openai.com`,
  `generativelanguage.googleapis.com`) so registering all three in
  the production pipeline is essentially free for non-AI traffic --
  the unmatched hooks short-circuit on a single string compare
  before touching state. Internally, each hook reuses the existing
  `ProviderStreamParser` impl
  (`AnthropicStreamParserWithState` / `OpenAiStreamParser` /
  `GoogleStreamParser`) -- no parsing logic is duplicated, so all
  the existing per-provider tests still cover the parse semantics.
  The interpreter takes the parser out of its slot via
  `mem::take`, drains `SseEventStream`, runs each event through
  the parser, then puts the parser back -- this releases the slot
  map for the SSE/LLM slot accesses inside (single-borrow at a
  time on the slot map). `LlmEventStream` carries an optional
  `provider: ProviderKind` set by the matching interpreter on
  first push, so downstream consumers can dispatch on provider
  without re-parsing the domain. `on_response_end` runs the same
  drain so trailing SSE events flushed by `SseParserHook` reach
  the interpreter. All three registered in
  `make_production_pipeline` after `SseParserHook`. Six unit tests
  covering: end-to-end Anthropic SSE → text delta + summary,
  OpenAI text delta, Google multi-part chunk, three-hooks-coexist
  routing (only matching one drains), wrong-domain skip leaves
  queue untouched, on_response_end trailing flush. 1533
  capsem-core lib tests pass; clippy clean.

### Added (mitm-redesign)
- **`SseParserHook` -- the first concrete `ChunkHook` consumer.** T1
  slice 5. Wraps the existing `parsers::sse_parser::SseParser` as a
  sync `ChunkHook` and writes parsed `SseEvent`s into a public
  per-request `SseEventStream` slot via `ChunkCtx::state`. The slot
  is the bridge to the provider-specific interpreter hooks landing
  in the next slice -- they drain new events on every chunk pass to
  build `ModelCall` summaries. The hook gates internally on AI
  domains (`api.anthropic.com`, `api.openai.com`,
  `generativelanguage.googleapis.com`) so registering it in the
  production pipeline is free for non-AI traffic: the `is_ai` check
  caches in the parser-state slot on first chunk and a non-AI
  connection bails before allocating the parser. `on_response_end`
  flushes any trailing event without a terminating blank line --
  matches the behavior of the inline `AiResponseBody` path that
  this hook is replacing. Now registered in
  `make_production_pipeline`. Six unit tests cover single-chunk,
  multi-chunk-split, multi-event accumulation, non-AI bypass,
  trailing-event flush, and the `[DONE]` sentinel filter for
  OpenAI. 1527 capsem-core lib tests pass; clippy clean.

### Fixed (resume,protocol)
- **Host-side watchdog around HostToGuest::Exec / FileWrite / FileRead
  with j_rx-based retry and 24s budget.** Apple VZ post-restoreState
  occasionally drops a successfully-written vsock frame (the host's
  `write_control_msg` returns success; the bytes never reach the
  guest), and the existing single-slot replay buffer in the control
  bridge can't catch this -- it only triggers on a write *error*.
  The watchdog re-sends the payload every 3s if the host hasn't seen
  the result oneshot resolve. Direct measurement of one stress-suite
  failure (`process.log` from
  `20260503-220608/.../susp-10f1a6c7`) showed the storm lasted 9.13s
  before any message arrived end-to-end, so the budget is set to 8
  attempts × 3s = 24s, leaving 6s of headroom under the 30s IPC
  envelope. The watchdog's signal is the j_rx oneshot resolving
  (i.e. ExecResult / FileOp ack), not ExecStarted -- the latter
  fires while ExecDone is still in flight, and ExecDone can be lost
  on the same torn return path the original Exec was lost on.
- **Agent-side dedup with cached ExecDone replay.** Exec ids
  observed during a session are tracked in two maps shared across
  reconnects: `exec_inflight` (still running -- skip duplicate, the
  original will send ExecDone) and `exec_done: HashMap<id,
  exit_code>` (finished -- replay GuestToHost::ExecDone with the
  cached code so the host's j_rx resolves even when the original
  reply was lost on the return path). The maps are hoisted out of
  `control_loop` into the parent's outer reconnect scope so a retry
  that lands on a *new* control conn after the previous one was
  torn still hits the dedup logic. File ops are intentionally not
  deduped -- write/read/delete are idempotent enough to re-process
  and re-ack on every receipt, which is correct for a FileOpDone
  that was lost on the return path (dedup-with-skip there would
  deadlock the host watchdog).

### Known limitations (resume,protocol)
- **Stress-suite flakiness floor: ~30% iteration fail rate remains.**
  10x runs of the back-to-back stress suite
  (`test_svc_resume_paths.py` + `test_svc_suspend_corruption.py` +
  `TestSuspendResume`) score 6-7/10 with these fixes, vs 7/10 for
  the unfixed baseline at HEAD~1 -- within the same noise band.
  Direct measurement (one ovl-test failure) showed the post-resume
  storm can last 21s of constant vsock re-keying, dropping
  bidirectional traffic for the entire window. Neither host-side
  retries nor guest-side response replay survive a storm that
  spans the whole 30s IPC envelope, because the bytes for the
  retried Exec *and* its replayed ExecDone are both subject to
  silent-drop on every conn. Closing this requires either: (a)
  application-level reliability (per-message ACKs over vsock with
  exponential backoff and a longer envelope), (b) a guest-side
  replay buffer for GuestToHost messages analogous to the host's
  bridge replay buffer (held across the agent's reconnect rather
  than dropped when the writer thread breaks), or (c) detecting and
  pausing sends during a storm. Followup beyond this sprint's scope.

### Fixed (test-infra)
- **`/delete` now routes through `preserve_failed_session_dir`.**
  Previously the only paths that preserved `process.log` /
  `serial.log` / `session.db` for post-mortem were three
  host-detected failure routes; a Python-side test assertion that
  fired after `/exec` but before the test's `finally:
  client.delete()` left only `service.log` archived, which doesn't
  show what the per-VM process or the guest were doing. The cull
  is bumped from 5 to 32 most-recent failed sessions so a
  10-iteration stress run that creates 1-3 VMs per iteration
  doesn't lose earlier failures to the LRU. Disk usage stays
  bounded by the cull regardless.

### Added (mitm-redesign)
- **Pipeline observability contract: every hook call is logged,
  timed, and counted.** Closes the "what is blocking?" gap. Async
  `Hook::on_event` is now wrapped in a `mitm.hook` info-span carrying
  fields `hook`, `kind`, `layer`, `decision` (recorded after the
  future resolves -- one of `continue`/`rewrote`/`stop_drop`/
  `stop_reject`/`stop_dns_reject`), and `duration_ms`. Counter
  `mitm.hook_invocations_total{hook}` increments per call;
  histogram `mitm.hook_duration_ms{hook}` samples the wall time.
  Trace events bracket the call: `on_enter` + `on_exit` at trace!
  level (filter via `RUST_LOG=mitm.hook=trace`). Stop-outcomes
  promote to debug! at target `mitm.hook.cause` so triage tooling
  surfaces them at default RUST_LOG=info filtering. Sync
  `ChunkHook` iteration gets the same counter + histogram (no span,
  trace! events at `mitm.hook.chunk` -- per-chunk spans would
  dominate the bench budget). New unit test installs a
  `metrics_util::DebuggingRecorder` via `set_default_local_recorder`
  and asserts the counter + histogram both fire on a single
  dispatch. 1521 tests pass; clippy clean.

### Added (mitm-redesign)
- **`ChunkHook` -- sync per-body-chunk hook trait + pipeline
  registration.** T1 slice 3 foundation. `ChunkHook` is a sync
  companion to the async `Hook` trait: methods
  `on_request_chunk(&mut Bytes, &mut ChunkCtx)` /
  `on_response_chunk(...)` / `on_request_end` /
  `on_response_end`. Body wrappers iterate registered ChunkHooks
  inline from `poll_frame` -- no async overhead, no channel hop.
  Sync is correct here because per-chunk work is fundamentally
  CPU-bound byte transformation: decompression, regex
  match-and-replace, streaming parsers, byte counting. None need
  `.await`. Per-connection state lives in the same typed slot
  map the async `Hook`s use, accessed via `ChunkCtx::state::<T>()`.
  `Pipeline` gains `register_chunk(ArcChunkHook)` builder method,
  `has_chunk_hooks()` short-circuit predicate, and
  `dispatch_request_chunk` / `dispatch_response_chunk` /
  `dispatch_request_end` / `dispatch_response_end` iteration
  helpers. Two new unit tests prove the surface: registration-order
  iteration with one hook rewriting bytes that the next hook then
  observes, and the empty-pipeline short-circuit. Slices 3b
  (DecompressionHook), 3c (TelemetryHook), 3d (SseParserHook) are
  now unblocked. 1520 tests pass; clippy clean.

### Added (mitm-redesign)
- **`RawResponseHead` dispatch + per-request `mitm.request` span.**
  T1 slice 3a (observer surface). After upstream returns headers,
  `handle_request` now dispatches `Event::RawResponseHead(&mut parts)`
  through the pipeline so future hooks can observe the response head
  before any wrapping (decompression, telemetry, AI parsing) takes
  place. Hooks that want to react to status codes or content-encoding
  / content-type live here. Today observer-only -- the Stop outcome
  is intentionally dropped because handing the upstream sender
  partially-used would leak. Plus a `#[instrument(target="mitm.request")]`
  decoration on `handle_request` itself recording fields domain,
  method, path, decision, status; every log line in a request now
  carries those as structured fields. Pure addition; no behavior
  change. 1518 tests pass; clippy clean.

### Added (mitm-redesign)
- **Metrics + tracing decision contract wired on the hot path.** T1
  slice 4. Every TLS connection now increments
  `mitm.connections_total{protocol="tls"}` and the
  `mitm.active_connections` gauge (RAII-decremented on drop, even on
  panic). Every request increments
  `mitm.requests_total{protocol="tls", decision}` partitioned by
  outcome (`allow` / `deny` / `upstream_error`). TLS handshake time
  histograms via `mitm.tls_handshake_ms`; full upstream-dial path
  (TCP + TLS) via `mitm.upstream_dial_ms`. `handle_connection` now
  in a `#[instrument(target="mitm.connection")]` span. No recorder
  registered yet, so each emission is one relaxed atomic add against
  the global no-op recorder (~4 ns per call per the T0 baseline).
  Two new smoke tests assert the metric names are unique and
  `describe_all` is idempotent. 1518 capsem-core lib tests pass;
  clippy clean.

### Fixed (virtio-blk-overlay-migration)
- **System overlay moved off loop-on-VirtioFS onto a real virtio-blk
  device.** rootfs.img is now attached to the guest as `/dev/vdb` and
  mounted directly as the overlayfs upper, bypassing the prior
  loop-device-on-VirtioFS sandwich whose closed-source virtiofsd
  returned EIO under writeback pressure on resume. Closes
  `loop-device-io-after-resume`: heavy directory churn + suspend +
  resume no longer leaves `EXT4-fs (loop0): failed to convert
  unwritten extents` / `I/O error, dev loop0` in dmesg. Universal --
  ephemeral and persistent VMs both use the new path; legacy
  loop-on-VirtioFS fallback removed from `capsem-init`. Snapshot
  (APFS clonefile) path validated byte-for-byte against the
  virtio-blk-attached file. `BootOptions::scratch_disk_path` renamed
  to `system_overlay_disk` to reflect its new role.

### Fixed (resume-stability)
- **Resume API no longer hangs 30s when capsem-process dies during
  restore.** `wait_for_vm_ready` now races the `.ready` sentinel poll
  against an instance-presence check; when the resume-side child
  exits before signalling ready, the API fails fast (~5ms-50ms)
  instead of spinning out the full readiness budget. The exit
  handler also logs the child's `exit_status` so future failures are
  diagnosable from `service.log` alone (previously the resume-side
  exit silently dropped the status).
- **Apple VZ post-restoreState handshake EOF is now retryable.**
  `is_retryable_handshake_error` accepts `UnexpectedEof` alongside
  `BrokenPipe` / `ConnectionReset` -- empirically the dominant
  fingerprint when Apple VZ tears the new vsock conn down between
  guest frames. The host re-accepts a fresh terminal+control pair
  and re-runs the handshake within the existing
  `HANDSHAKE_RETRY_MAX` budget. Prior behaviour: process exited with
  code 1, leaving the resume API to time out at 30s.
- **Control bridge holds in-flight `HostToGuest` messages across
  re-key.** When Apple VZ kills the control vsock mid-write, the
  message that was being sent (often an `Exec` or `FileWrite`
  command) used to be silently dropped, and the corresponding
  `/exec` or `/write_file` call timed out at 30s waiting for a reply
  that would never come. The bridge now stashes the failed message
  and replays it on the next successfully re-keyed connection.

### Changed (mitm-redesign)
- **Inline `policy.evaluate` deny path removed; PolicyHook is now the
  source of truth.** T1 slice 2d. PolicyHook stashes its
  PolicyDecision (allowed + matched_rule + reason) in HookCtx::state
  via the typed slot mechanism. After dispatch, handle_request reads
  the record back and uses it to populate the TelemetryEmitter (allow
  + deny paths both). On Stop(Reject(_)) the hook's response is
  wrapped with TelemetryBody so a NetEvent still fires for denies
  (no telemetry regression). Test fixtures upgraded from
  make_default_pipeline() to make_production_pipeline(policy) so
  policy actually fires in unit + integration tests. 1516 lib tests +
  8 integration tests pass; clippy clean. Slice 2d closes T1's
  rewire of the policy stage; the pipeline now owns it end-to-end.

### Added (mitm-redesign)
- **Pre-rewrite `mitm-load` baseline captured.** T0 closes:
  `benchmarks/mitm-load/baseline.json` holds the live numbers from
  `capsem-bench mitm-load` against the un-redesigned proxy at
  concurrency 1/10/50/200 (10s per level). Highlights: rps
  1109/2862/2995/2701, p99 2.2/8.4/45.4/175.2 ms, 0 errors,
  RSS 26-230 MB. T5's CI gate compares against this file -- any
  level >2x p99 regression fails the build.

### Added (mcp + bench)
- **`local__echo` MCP tool + `capsem-bench mcp-load` mode + baseline.**
  New zero-I/O diagnostic tool: returns its `text` parameter verbatim.
  Lives in `capsem-mcp-builtin`; reachable as `local__echo` through
  the in-guest MCP server -> vsock:5003 -> aggregator -> builtin
  subprocess chain. New `capsem-bench mcp-load` mode hammers it from
  the guest with concurrent fastmcp Client calls (asyncio.gather over
  N workers per concurrency level) so we get a number for the MCP
  path's scaling shape, isolated from the MITM path. Pre-rewrite
  baseline at `benchmarks/mcp-load/baseline.json`: rps
  2162/3792/4061/3965 across concurrency 1/10/50/200, p99
  1.1/4.4/17.4/70.8 ms, 0 errors. Sub-linear scaling -- plateaus at
  ~4000 rps from concurrency 10 onwards. There IS a serialization
  point in the MCP path that needs debugging (suspect:
  stdio-framing in capsem-mcp-server, single vsock:5003 stream, or
  JSON-RPC dispatch in the aggregator). Sister to the MITM baseline,
  which plateaus around ~3000 rps with worse tails.

### Added (capsem CLI)
- **`capsem cp` -- file transfer between host and a session's
  workspace.** The service has had `GET/POST /files/{id}/content`
  upload/download endpoints for a while (used by the desktop app's
  Files tab). The CLI never exposed them. Now: `capsem cp foo.txt
  my-vm:foo.txt` (upload) / `capsem cp my-vm:bench.json
  ./bench.json` (download) / `capsem cp my-vm:log.txt -` (stdout).
  Exactly one of `<src>`/`<dst>` must be `SESSION:PATH`; PATH is
  relative to `/root` (workspace bind-mount in the guest). Errors
  loud: `guest-to-guest copy not supported`,
  `neither argument is SESSION:PATH`. New
  `UdsClient::request_bytes` returns raw response bytes + content-type
  for endpoints that don't speak JSON (the existing `request` method
  always tries to deserialize JSON, so couldn't be used for binary
  downloads).

### Added (mitm-redesign)
- **Pre-rewrite `mitm-load` baseline captured.** T0 closes:
  `benchmarks/mitm-load/baseline.json` holds the live numbers from
  `capsem-bench mitm-load` against the un-redesigned proxy at
  concurrency 1/10/50/200 (10s per level). Extracted via the new
  `capsem cp` command (write bench output to `/root/baseline.json`
  in the guest, `capsem cp` it to host). Highlights: rps
  1037/3043/3029/2699, p99 2.3/8.4/53.4/191.3 ms, 0 errors,
  RSS 27-260 MB. T5's CI gate compares against this file -- any
  level >2x p99 regression fails the build.

### Added (mitm-redesign)
- **Hook pipeline now dispatches from `handle_request`
  (parallel-deploy).** T1 slice 2c: every HTTPS request through the
  MITM now runs `pipeline.dispatch(Event::RawRequestHead, ...)` with
  the per-connection `ConnMeta` (domain + process_name + port=443)
  and the ambient `trace_id`. Production builds use
  `make_production_pipeline` so `PolicyHook` fires for every request,
  emitting the `mitm.policy_decisions_total` counter and the
  structured `mitm.policy` tracing event with rule + reason fields.
  The hook's `Stop(Reject(_))` outcome is intentionally dropped this
  slice -- the inline `policy.evaluate()` call below remains the
  source of truth for the actual stop/continue decision so behavior
  is provably unchanged. Subsequent slices land TelemetryHook +
  RejectHook plumbing that lets us safely remove the inline path.

### Added (mitm-redesign)
- **`PolicyHook` + `ConnMeta` + `make_production_pipeline`.** T1
  slice 2b: first concrete `Hook` impl. `mitm_proxy/policy_hook.rs`
  subscribes to `Event::RawRequestHead` (priority -1000 so it runs
  before any other L1 consumer), evaluates `NetworkPolicy::evaluate`
  against `ConnMeta::domain` + the request method, returns
  `Stop(Reject(403))` on deny. Tracing target `mitm.policy` records
  `decision` (allow|deny) + `rule` + `reason`; metric
  `mitm.policy_decisions_total{decision}` increments. New
  `ConnMeta` (`domain`, `process_name`, `port`) carried read-only
  through `HookCtx::conn()` so hooks can reach per-connection
  metadata not present in `RawRequestHead`. `make_production_pipeline`
  builds the registered set; `handle_request` does not yet dispatch
  through it (slice 2c). 4 new tests cover allow / deny / default-allow
  and the `evaluate_decision` rendering helper. 1516 passing.

### Added (mitm-redesign)
- **`MitmProxyConfig` carries a `pipeline: Arc<Pipeline>` field.**
  T1 slice 2a: the `Pipeline` from slice 1 is now plumbed through the
  proxy config so subsequent slices can dispatch from `handle_request`
  without changing the public type again. `make_default_pipeline()`
  returns an empty pipeline -- the inline call graph in
  `handle_request` still drives policy / decompression / AI parsing /
  telemetry. T1 slice 2b will register the production hooks; slice 3
  wires the metrics + tracing decision contract. Three call sites
  updated: `mitm_proxy/tests.rs`, `tests/mitm_integration.rs`,
  `capsem-process/src/main.rs`.

### Added (mitm-redesign)
- **Single `Hook` trait + `Event<'_>` ladder + dispatcher.** T1 slice
  1: pure-additive infrastructure for the new pipeline. Three new
  modules under `mitm_proxy/`: `events.rs` (15-variant `Event<'a>`
  enum across L1 raw transport / L2 protocol / L3 semantic, plus
  `EventKind` discriminator + `EventLayer` ordering + bitset
  `EventMask`), `hooks.rs` (the single `Hook` trait, `HookOutcome` =
  `Continue | Rewrote | Stop(StopAction)`, `StopAction` =
  `Drop | Reject(http::Response) | DnsReject(rcode)`, `HookCtx` with
  per-connection typed slot map for cross-call carry-over and
  `ctx.emit()`), `pipeline.rs` (registration-time-sorted dispatcher
  with O(1) per-kind plan, recursive `emit()` re-entry, layer-cycle
  prevention enforced at runtime: an L3 hook cannot emit L1/L2;
  `EmitError::CycleAttempt` returned). 16 new unit tests including:
  hook ordering by `(priority, registration_order)`, `Stop`
  short-circuit, L1->L2 emit dispatch, L3->L1 cycle rejection, typed
  state slot persistence across multiple chunk dispatches (the
  contract the future credential-rewrite hook will use), trace-id
  visibility. No production code wires the pipeline yet -- T1 slice 2
  rewires policy / decompression / AI parsing / telemetry as Hook
  impls.

### Added (mitm-redesign)
- **`capsem-bench mitm-load` mode.** New
  `guest/artifacts/capsem_bench/mitm_load.py` drives the MITM proxy at
  configurable concurrency levels (default 1 / 10 / 50 / 200) for
  `CAPSEM_BENCH_MITM_DURATION` seconds each (default 10s) against
  `CAPSEM_BENCH_MITM_TARGET` (default a non-routable domain so every
  request fails fast at upstream-dial, isolating proxy cost from
  upstream variance). Reports per-level rps, p50/p95/p99/p99.9 latency,
  RSS peak, and error count. T5's CI gate compares to
  `benchmarks/mitm-load/baseline.json`: any concurrency level >2x p99
  regression fails the build. Baseline JSON itself is deferred --
  requires `just run "capsem-bench mitm-load"` against the
  un-redesigned proxy and commit of the result.

### Added (mitm-redesign)
- **Criterion bench harness + pre-rewrite baselines.** `criterion`
  (dev-dep) plus four new benches under `crates/capsem-core/benches/`:
  `parser_sse`, `parser_jsonrpc`, `interp_anthropic`, `mitm_pipeline`.
  First-run numbers committed to `benches/baselines/T0-pre-rewrite.md`
  -- T5's regression gate compares against this file via `critcmp` and
  fails CI on >5% slower medians. Baseline highlights: SSE parser
  449-472 MiB/s on 1MB corpora (plan budget 500 MiB/s), Anthropic
  interpreter end-to-end 233 MiB/s on tool-use response, metrics-facade
  counter emission 3.89 ns with no recorder installed.
- **`metrics` facade dependency + `mitm_proxy::metrics` module.**
  All counter / histogram / gauge names from the plan declared with
  `describe_*` calls in `mitm_proxy/metrics.rs`. No recorder registered
  this sprint -- T5 wires an exporter (likely OTel via
  `opentelemetry-otlp`); until then emission is a single relaxed atomic
  add against the global no-op recorder. T0 slice 4 of
  `sprints/mitm-redesign/`.

### Fixed (observability)
- **W3 IPC handshake: respect tokio's non-blocking sockets.**
  `tokio::net::UnixStream::into_std()` returns the std handle still in
  non-blocking mode. The W3 handshake's `read_exact`/`write_all` then
  bailed with WouldBlock instantly, manifesting as 95 integration tests
  failing with "peer did not send Hello within 5000ms" the first time
  any IPC channel was used. `negotiate_initiator`/`negotiate_responder`
  now flip the socket to blocking mode for the handshake (saving the
  previous flag) and restore the original mode afterward so the bincode
  channel inherits the same tokio-non-blocking shape it expects. Builds
  + 1273 integration tests now pass.

### Added (observability follow-ups)
- **W6 writer-side population.** `trace_id` is now a column AND a
  field on every event struct. Writer INSERTs the column on every row.
  Construction sites populate via
  `capsem_core::telemetry::ambient_capsem_trace_id()`. `tool_calls` /
  `tool_responses` fall back to the parent `model_calls.trace_id`.
- **`capsem_triage --id <vm>` queries session.db** for `denied_net`,
  `mcp_errors`, `exec_failures` alongside the host-log scan.
- **`capsem_timeline` joins tool_calls -> mcp_calls** so a model
  tool_use shows its servicing MCP call inline.
- **`capsem support-bundle --max-session-bytes`** (default 50MB) drops
  oldest sessions when their session.db total exceeds the cap.
- **Hot-path `#[instrument]` coverage** on `wait_for_vm_ready`,
  `pause`, `resume`, `attach_disk`, `attach_virtiofs_share`.
- **`dump_frontend_logs` Tauri command + `recordWsEvent` wiring.**
  `__capsemDebug.dumpLogs()` now returns a real jsonl path;
  `__capsemDebug.lastWsEvents` actually fills as WS events arrive.
- **Triage panic parser + redactor adversarial fixtures.**
- **`capsem-app` emits `service.start`** so cross-version-mix detection
  covers all 9 binaries (adds capsem-proto leaf dep; capsem-core
  invariant preserved).
- **Skill updates: dev-mcp** (4 new tools in tool table), **dev-debugging**
  (MCP triage trio workflow + schema_hash hint),
  **references/mcp-wire.md** (W5 `_meta` envelope + BootConfig.traceparent).
- **C1: T3 timeline SQL allowlist** enforced before `format!()`.
- **C2: `app_error_logged!`** used in fork's clone-task error path.
- **T1 (test): `tests/capsem-service/test_protocol_handshake.py`**
  exercises the W3 handshake regression.
- **CLI parity: `support-bundle` added to `CLI_ONLY` allowlist.**

### Added (observability)
- **In-band W3C trace context on the host->guest control bridge and
  on MCP JSON-RPC.** `BootConfig` now carries an optional
  `traceparent: String` so the guest agent learns the host's trace_id
  on message #1 of boot; capsem-agent stamps every subsequent
  `blog_line` log line with `trace_id=<lower 16 hex>` so guest-side
  panics, kernel errors, and init script output correlate with
  host-side spans for the same VM boot.
  `JsonRpcRequest` and `JsonRpcResponse` gain an optional `_meta`
  envelope with `traceparent` + `tracestate` (W3C Trace Context) so a
  per-tool-call trace can ride alongside the JSON-RPC payload. Both
  fields are optional with serde defaults -- third-party MCP clients
  and pre-W5 capsem peers continue to round-trip cleanly.
  Also reorganizes the post-mitm-redesign rename: `net::ai_traffic::
  {anthropic,google,openai,sse}` are now re-exports of the new
  `net::interpreters::*_interpreter` and `net::parsers::sse_parser`
  modules so existing call sites compile while new code can use the
  fully-qualified path.

### Changed (mitm-redesign)
- **`mitm_proxy.rs` decomposed into submodules.** The 1421-line file
  is now `mitm_proxy/mod.rs` (614 lines: handle_connection +
  handle_inner + handle_request + MitmProxyConfig + helpers) plus four
  sibling submodules: `body.rs` (BodyStats, RespStatsKind, ProxyBoxBody,
  TrackedBody, BodyStream, DecompressBody), `telemetry.rs`
  (TelemetryEmitter + TelemetryBody + emit_model_call), `fd_stream.rs`
  (AsyncFdStream + ReplayReader + set_nonblocking), `util.rs`
  (is_llm_api_path + split_path_query + format_headers +
  HEADER_ALLOWLIST). Each submodule keeps `pub(super)` visibility so
  the public API of `crate::net::mitm_proxy::*` is unchanged. T0
  slice 3 of `sprints/mitm-redesign/`; zero behavior change.

### Changed (mitm-redesign)
- **All remaining inline `mod tests { }` blocks in `net/` extracted to
  sibling `tests.rs` per CLAUDE.md.** `mitm_proxy.rs` shrinks from
  2847 to 1421 lines (1426 lines of tests now in
  `mitm_proxy/tests.rs`); `ai_traffic/{events,pricing,ai_body,provider,
  mod}.rs` similarly cleaned. Production code is no longer buried under
  scroll-past test fixtures; every grep / Read of a parser shows just
  the parser.

### Changed (observability)
- **W6 trace_id wiring completed across capsem-logger / capsem-core /
  capsem-process.** The `trace_id` column on `net_events`, `mcp_calls`,
  `tool_calls`, `tool_responses`, `fs_events`, and
  `audit_events` is now populated end-to-end. Write-side: every event
  emitter (`mitm_proxy`, `mcp/{gateway,builtin_tools,file_tools}`,
  `fs_monitor`, and `capsem-process` audit paths) calls
  `capsem_core::telemetry::ambient_capsem_trace_id()`. INSERT statements
  in `writer.rs` now include the new column. `tool_calls.trace_id` and
  `tool_responses.trace_id` fall back to the parent `model_calls.trace_id`
  when the per-row value is None (same agent turn). Read-side defaults
  to `None` until the SELECT clauses are extended in a follow-up.

### Changed (mitm-redesign)
- **AI parser tests extracted to sibling `tests.rs` per CLAUDE.md.**
  `parsers/sse_parser.rs`, `interpreters/anthropic_interpreter.rs`,
  `interpreters/openai_interpreter.rs`, and
  `interpreters/google_interpreter.rs` no longer carry inline
  `mod tests { }` blocks; their ~1100 lines of tests now live next to
  each prod file (e.g., `parsers/sse_parser/tests.rs`). Same pattern
  established by the obs sprint's earlier 18-file extraction.
- **Backwards-compat re-exports removed.** The transitional aliases
  `net::ai_traffic::{anthropic,google,openai,sse}` are gone; all
  internal callers (mitm_proxy, ai_body, events, provider, interpreter
  tests) reference the canonical
  `net::parsers::sse_parser` / `net::interpreters::<provider>_interpreter`
  paths. T0 slice of `sprints/mitm-redesign/`.

### Added (mitm-redesign)
- **`sprints/mitm-redesign/` scaffolded.** Meta-sprint plan to decompose
  the 2847-line `mitm_proxy.rs` monolith into a hookable pipeline with
  first-class plain HTTP, a real DNS proxy (hickory-server replaces the
  fake dnsmasq), MCP protocol awareness, and a single `Hook` trait + L1/
  L2/L3 `Event` ladder. Six phases (T0..T5) covering reorganization,
  hook traits, plain HTTP, DNS, MCP awareness, and hardening with
  performance regression CI gates. The future security engine
  (credential rewrite via regex body replace) is explicitly out of scope
  but the hook surface is shaped to host it without trait changes.

### Added (observability)
- **`capsem doctor --bundle` -- in-VM diagnostic tar wired into the
  support bundle.** `guest/artifacts/capsem-doctor` now accepts
  `--bundle [PATH]` and packages pytest output + junit XML, /var/log,
  dmesg, /proc/{mounts,cmdline}, /tmp/capsem-init.log, and
  session.db (when present) into a single tar at
  `/shared/doctor-bundle.tar` (default) or a caller-supplied path.
  Host-side `capsem doctor --bundle` lifts that file out of virtiofs
  to `~/.capsem/run/doctor-latest.tar` before the VM is destroyed.
  `capsem support-bundle` then embeds it as `doctor/bundle.tar`.
  Closes the "guest-side bug, but the bundle has only host context"
  gap in T1's bundle.

- **CI uploads `test-artifacts/` on red runs.** Both the `test-linux:`
  and `test:` jobs now have `upload-artifact@v4` steps gated on
  `if: failure()`. Reviewers get a downloadable bundle of
  `service.log`, `process.log`, `serial.log`, and `session.db` from
  every failed job without rerunning. Existing `preserve_tmp_dir_on_failure`
  in `tests/helpers/service.py` already populates the directory.
- **`just test-artifacts`** -- one recipe that finds the latest
  preserved failure dir under `test-artifacts/` and prints the file
  list with sizes. Saves digging through `ls -lt` after a red local
  run.
- **Frontend `window.__capsemDebug` console handle.** Exposed when
  the URL contains `?debug=1`. Methods: `versions()` (build_ts +
  version), `dumpLogs()` (returns the path to the latest jsonl via a
  reserved `dump_frontend_logs` Tauri command), `lastWsEvents` (small
  ring buffer; populated by api.ts when a WS event arrives via
  `recordWsEvent`). Console-only -- the visual HUD is punted to the
  frontend-rebuild sprint.

- **`capsem_timeline` MCP tool -- one tool call renders the unified
  time-ordered event stream for a session.** UNION across exec_events,
  mcp_calls, net_events, fs_events, and model_calls, ordered by
  timestamp. Filter by `traceId` to follow a single logical operation
  across layers (W6 added trace_id to every table; W4 propagates the
  id through the host process tree). Filter by `since` to scope the
  window. Optional `layers` arg accepts a comma-separated subset
  ("exec,mcp" etc.) when only some are interesting. Pre-W4 rows have
  NULL trace_id and are returned alongside matched rows so the user
  doesn't lose context that pre-dates the trace propagation.

- **`trace_id TEXT` column on every event table.** Added to
  `mcp_calls`, `net_events`, `fs_events`,
  `tool_calls`, `tool_responses`, `audit_events` (model_calls and
  exec_events already had it). Indexes added on each. Fresh DBs get
  the column from `CREATE_SCHEMA`; existing DBs get it via
  idempotent `ALTER TABLE ADD COLUMN` on next open. Unblocks
  `capsem_timeline --trace_id <X>` to UNION across all event classes
  for one logical user action. Population through the writer API
  follows in a subsequent commit; pre-population rows are NULL and
  the timeline tool tolerates that gracefully.

- **W3C trace context propagated to every spawned capsem-* binary +
  per-stage timing on the suspend hot path.** capsem-service injects
  `CAPSEM_VM_ID`, `CAPSEM_TRACE_ID`, `TRACEPARENT`, `TRACESTATE` into
  capsem-process at spawn (cold-boot + resume paths); capsem-process
  forwards them when spawning capsem-mcp-aggregator. New helper
  `capsem_core::telemetry::child_trace_env(vm_id)` in one place; if
  this binary is itself a child of another capsem-* binary, the
  parent's traceparent is forwarded verbatim, so the whole tree shares
  one trace_id. Top-of-tree binaries synthesize a fresh
  `00-<32hex>-<16hex>-01` traceparent from blake3(vm_id + nanos).
  Suspend now emits `target=suspend op=apple_vz_pause`,
  `op=apple_vz_save_state`, `op=with_quiescence`, and
  `target=fs op=fsync path=rootfs.img` events with `duration_ms` --
  closes parent ISSUE.md pattern (6) and the today-2026-05-02
  "fsync timing was missing" debugging session.
- **Top-5 `_ => {}` enum arms now log instead of dropping.** vsock
  port dispatcher, lifecycle port, `handle_guest_msg`, and the MCP
  aggregator main match. An unknown variant now emits
  `tracing::warn!(target = "ipc", unhandled = ?other, "unknown
  variant; this binary may be older than its peer")` -- closes parent
  ISSUE.md pattern (3).

- **`capsem_panics`, `capsem_triage`, `capsem_host_logs` MCP tools.**
  AI agents (and developers via `capsem-mcp`) can now triage Capsem
  failures in one tool call without leaving the conversation:
  - `capsem_panics` -- structured panic + backtrace extractor across
    `~/.capsem/run/{service,mcp,gateway,tray}.log` and capsem-app's
    latest jsonl. Returns `[{ ts, binary, thread, location, message,
    frames }]` with `/Users/<x>/` paths redacted to `~/`. Run this
    FIRST when investigating an unexplained failure.
  - `capsem_triage` -- ranked summary of recent panics, dropped IPC
    frames (`target=ipc` warns from W1), 4xx/5xx server errors
    (`target=service` from W3.5), and slow operations (`target=fs
    op=fsync` etc., >500ms). Default lookback "30m"; accepts "5m",
    "1h", "24h", "7d", or RFC3339.
  - `capsem_host_logs` -- read any host log by symbolic name with
    grep + tail filtering. Hard-coded allowlist (no path traversal).
  Three new service HTTP endpoints (`/triage`, `/panics`,
  `/host-logs/{name}`) reuse the W2 JSON output shape, the W3 schema
  hash, and the W3.5 status field for deterministic ranking.

- **`capsem support-bundle` -- one command, one redacted tar.gz, ready
  to attach to a bug report.** Gathers `~/.capsem/run/*.log`,
  `~/.capsem/logs/*.jsonl`, the last N session directories
  (session.db + serial.log + process.log + metadata.json), assets
  manifest, redacted user.toml/corp.toml, version + OS info, dmesg
  (Linux), and a blake3 fingerprint of the MITM CA cert (the cert
  itself is NEVER bundled). Default output:
  `~/.capsem/support/capsem-support-<UTC-ts>-<host>.tar.gz`. Five
  redaction rules strip Bearer tokens, sk-/AIza/xoxb- API key prefixes,
  TOML/JSON keys named like a secret, and `/Users/<x>/` paths;
  `--no-redact` disables. `--include-rootfs` opt-in (off by default --
  rootfs.img is huge and rarely useful). Manifest schema v1 includes a
  ranked "next steps" list pointing at where to look in the bundle and
  which `target=` filters to grep for.

- **Every `AppError` returned by the capsem-service HTTP layer now
  emits a structured `tracing` event automatically.** Done in
  `IntoResponse` so all 104 `AppError(StatusCode, msg)` call sites are
  covered with zero codemod: 5xx → `error!`, 4xx → `warn!`, other →
  `info!` with `target = "service"` and the status code as a
  structured field. Pre-W3.5: the user got a 500 in the response with
  nothing in `service.log` to trace back from. Optional
  `app_error_logged!` macro lets a call site emit a SECOND event
  earlier (with the same status field) when an in-flight span is more
  informative than the late one fired at response-build time.

- **Versioned IPC handshake: cross-version mixes fail loudly in ~1s.**
  Every typed IPC connection between capsem-service and capsem-process
  now exchanges a `Hello { version, schema_hash, peer, traceparent }`
  frame on the raw UnixStream before the bincode channel takes over.
  `version` bumped to `1`. `schema_hash` is a build-script-emitted
  FNV-1a 64 hash of the protocol source bytes -- catches enum
  reordering / variant additions that don't bump version. On mismatch:
  `tracing::error!(target = "ipc", peer_id, ours_hash, peer_hash,
  "IPC handshake failed; refusing connection")` within 1 second instead
  of the pre-sprint 30-second silent timeout. Side-channel design
  (handshake on the raw stream before bincode) preserves the existing
  `Sender<ServiceToProcess>` / `Receiver<ProcessToService>` API; W1's
  `try_send!` codemod sites are unchanged. Pre-W3 binaries fail decode
  within 5 seconds (HELLO_TIMEOUT).

- **All host-side binaries now write JSON-per-line logs to
  `~/.capsem/run/{service,mcp,gateway,tray}.log`** -- consolidated
  through a single `capsem_core::telemetry::init()` entry point. Eight
  binaries (capsem-service, -process, -mcp, -mcp-aggregator,
  -mcp-builtin, -gateway, -tray, plus the macros consumer in capsem)
  now share one tracing-subscriber bootstrap. The four that previously
  emitted compact-format text (gateway, tray, mcp-builtin,
  mcp-aggregator) now emit structured JSON, so `capsem support-bundle`
  and the upcoming `capsem_panics` MCP tool can parse every host log
  with one decoder. Each binary's `service.start` line carries
  `protocol_version` + `schema_hash` so cross-version-mix can be
  detected from a single log read once W3 lands.
- **W3C `TRACEPARENT` env var captured at startup** and exposed via
  `capsem_core::telemetry::current_parent_traceparent()` /
  `ambient_capsem_trace_id()`. No OpenTelemetry runtime dep this
  sprint -- traceparent is a structured field in JSON for now;
  tracing-opentelemetry layer is a future-sprint addition. Adding it
  later is purely an additional `Layer` on the existing subscriber.

### Changed (observability)
- **Silent IPC drops in suspend/resume/exec/file paths now log at
  `target="ipc"`.** ~50 sites across `capsem-process/src/{vsock,ipc,
  main,terminal,job_store}.rs`, `capsem-service/src/main.rs`, and
  `capsem/src/main.rs` were `let _ = X.send(...)` -- a closed receiver
  silently swallowed the message with no trace. New `try_send!` macro
  in `capsem-core::macros` wraps every IPC/vsock send and emits a
  `tracing::warn!(target = "ipc", channel, error)` line on failure.
  Filter with `RUST_LOG=ipc=warn` to see only dropped-message events.
  Cleanup paths where a closed receiver is the documented design
  (e.g. broadcast publish into `TerminalOutputQueue`) keep the bare
  `let _ = ` and carry an inline `// channel-closed-ok: <reason>`
  marker so the audit grep can exclude them.

### Changed (persistent overlay)
- **EXT4 journal re-enabled on the persistent overlay-upper.** Previously
  formatted with `mke2fs -O ^has_journal`; switched to default
  `has_journal` and mount with `data=ordered`. Costs ~5-10% IOPS;
  enables metadata replay on resume so directory listings stay
  consistent after suspend/resume cycles where in-flight metadata
  writes hadn't been flushed. Verified via `tune2fs -l /dev/loop0`:
  `Filesystem features: has_journal ... metadata_csum`. Standard
  suspend/resume + heavy-churn directory listing now both work.
  (Heavy-churn DATA reads of a subset of files still hit
  `Input/output error` -- that's the loop-device-io-after-resume
  sprint's remaining work, fixable only by moving rootfs.img off
  VirtioFS to a real VZ block device.)

### Fixed (lifecycle)
- **Guest-initiated `shutdown` left persistent VMs marked Defunct
  instead of Stopped.** The lifecycle path (`capsem-sysutil shutdown`
  -> vsock:5004 -> `ProcessToService::ShutdownRequested`) had no
  service-side listener; the process just sent `Shutdown` to itself
  and exited cleanly. The cleanup task interpreted "instance still in
  the map at exit" as `unexpected_exit=true` and flipped the registry
  to `defunct`, so `capsem list` showed Defunct and the test
  `test_guest_shutdown_preserves_persistent_and_resume` failed.
  Distinguish: a clean `ExitStatus::success()` is graceful regardless
  of who initiated it; only non-zero exit / signal kill is a crash.

### Fixed (suspend/resume durability)
- **`cd /root && ls` after `capsem resume` failed with "cannot open
  directory '.': No such file or directory".** Apple VZ writes to the
  persistent overlay's `rootfs.img` were buffered in macOS's APFS page
  cache. After `save_state`, capsem-process exited before APFS flushed,
  so the next boot read a stale `rootfs.img` and the EXT4 overlay-upper
  served stale inodes -- the cwd handle in the resumed shell pointed at
  garbage. Three-stage flush now layered on suspend:
  1. Guest agent: `sync()` + `BLKFLSBUF` + `fsync(/dev/loop0)` (existed).
  2. Guest agent: `fsync(/mnt/shared/system/rootfs.img)` -- sends
     `FUSE_FSYNC` over VirtioFS so the host VirtioFS daemon flushes its
     own buffered writes against the real macOS file (NEW).
  3. Host capsem-process: `sync_all()` on `rootfs.img` after
     `save_state` returns -- catches APFS dirty pages (NEW).
  Confirmed end-to-end against the live service: simple suspend/resume
  + `cd /root && ls` works; suspend with churn across `/tmp /var /opt
  /etc /usr/local` survives; file *contents* on the EXT4 overlay are
  durable. Heavy directory churn (~50 new entries per dir then
  immediate suspend) can still leave EXT4 directory data blocks with
  stale checksums on resume -- file reads succeed but `readdir`
  returns I/O error. Tracked in
  `sprints/loop-device-io-after-resume/ISSUE.md`; the next step is
  forcing an `fsync` on each parent directory inside the guest before
  signalling SnapshotReady.
- **Failed suspend left VM marked "Suspended" with a corrupt checkpoint.**
  When `with_quiescence` failed (timeout, channel closed) the spawn task
  ignored the error, sent `StateChanged{Suspended}` anyway, and exited
  with code 0. The service then marked the VM as suspended; the next
  resume cold-booted against the half-written rootfs.img and kernel-
  panicked with `EXT4-fs error inode #N: iget: checksum invalid` ->
  `overlayfs failed`. Fix: only send the Suspended state and `exit(0)`
  when the operation actually succeeded; on failure, log the error and
  `exit(1)` so the service treats it as a crash and does not write the
  checkpoint marker.
- **Silent IPC connection close on protocol mismatch.** Two binaries
  built across an enum-variant addition (`StopTerminalStream`) talked
  past each other; the receive side closed the connection silently with
  the decoder error swallowed. Fix: log the rx error at `warn` level so
  the next protocol-skew bug surfaces in the first run instead of
  presenting as a "guest doesn't respond" timeout.

### Fixed (capsem shell)
- **Terminal garbage on shell exit.** Pressing Ctrl-C / typing `exit` in
  `capsem shell` could leave the user's parent terminal flooded with
  binary garbage (MessagePack frames -- `bootconfig`, `epoch_secs`,
  `Pong` repeated). Two compounding bugs:
  1. `output_task` (the spawned reader of `ProcessToService` IPC frames)
     was never aborted on exit. tokio `JoinHandle::drop` does NOT cancel
     -- the task lived on, kept holding `stdout`, and any in-flight
     `TerminalOutput` frame wrote to the user's now-cooked-mode shell.
  2. The host-side `capsem-process` kept queuing `TerminalOutput` for the
     dropped IPC connection because the client never told it to stop.
- Fix: `run_shell` now sends a new `ServiceToProcess::StopTerminalStream`
  before exit, aborts the local task, drops the IPC writer, and writes a
  minimal terminal reset (`\x1b[0m\x1b[?25h\r\n` -- SGR reset, show
  cursor, CRLF; deliberately no alt-screen toggle or screen clear so
  scrollback is preserved).
- Defenses: `capsem_proto::looks_like_ipc_frame` ships a detector for the
  `to_vec_named` adjacently-tagged enum prefix that produced the garbage;
  `capsem-process` calls it on every `TerminalOutput` payload and emits a
  loud `warn!` if a leak ever resurfaces. 15 unit tests in
  `crates/capsem/src/shell_exit/tests.rs` pin: the reset sequence shape,
  every variant of both `HostToGuest` and `GuestToHost` matching the
  detector, no false positives on ANSI/UTF-8/scrollback content, and
  the load-bearing tokio behavior (`JoinHandle::drop` does not cancel,
  `JoinHandle::abort` does).

### Changed (kernel)
- The backend image spec ships `kernel_branch = "auto"` instead of a
  hardcoded `"6.6"`. `resolve_kernel_version("auto")` queries
  kernel.org/releases.json and picks the newest non-EOL longterm branch's
  latest patch (today: `6.18.26`). Pin to a specific branch by setting
  `kernel_branch = "X.Y"` (e.g. `"6.6"`) for reproducibility / security
  freeze. Killed the duplicated `"6.6"` literal in `models.py` /
  the removed scaffold rail -- single source of truth is now the profile-derived
  backend image spec.

### Changed (bootstrap)
- `bootstrap.sh` moved to the repo root (was `scripts/bootstrap.sh`).
- Phase 1 now auto-installs `rustup` (sh.rustup.rs) and `just` (just.systems
  -> `~/.local/bin`) instead of printing hints and bailing.
- Phase 2 auto-installs `uv` (astral.sh), `pnpm` (brew on macOS,
  get.pnpm.io on Linux), and on macOS `colima` + `docker` + `docker-buildx`
  with Rosetta-enabled VM start (`colima start --vm-type vz --vz-rosetta
  --memory 8 --cpu 8`). Linux docker stays manual (distro-specific, sudo,
  group, daemon -- prints clear apt/dnf hints instead).
- Each install gates on a `[Y/n]` prompt; **Enter accepts** (Y is the
  default). `--yes` and non-tty input both auto-accept for CI.
- Stopped silencing every installer (`--quiet`, `>/dev/null`). Real errors
  were getting swallowed -- `uv sync` failures showed up as a mystery
  `exit 1` with no diagnostic.
- Closing message no longer tells you to run `just build-assets` (it
  already ran as part of doctor's auto-fix in Phase 3).

### Fixed (bootstrap)
- `cargo install cargo-tauri` was wrong -- the crate is `tauri-cli` (the
  binary it produces is `cargo-tauri`). Fixed in `scripts/doctor-common.sh`.

### Fixed
- **Asset download URL.** `download_missing_assets` built the URL from the
  asset version (`v2026.0424.1`) instead of the binary version (`v1.0.{ts}`),
  so every fresh install 404'd against the GitHub Release. Releases are tagged
  by binary version; the asset version lives only inside the manifest.
- **Manifest schema mismatch.** The CI release pipeline writes
  `binaries.releases.<v> = {version, files}`, but the Rust `BinaryRelease`
  struct required `{date, min_assets}`. Every published manifest was
  unparseable -- the binary couldn't even *get* to the URL builder before
  failing. Made `date` / `min_assets` / `min_binary` optional, added
  `version` / `files` to round-trip pkg/deb metadata. `pick_asset_version`
  treats empty `min_assets` as "no constraint" and falls back to
  `assets.current`.
- **Removed broken Makefile.** The legacy Makefile bypassed `_pack-initrd`,
  `gen_manifest`, and `create_hash_assets`, so `make` produced a binary that
  couldn't resolve any VM asset at boot. Use `just` for everything.

### Added (defenses)
- Pinned the asset URL contract in `asset_download_url()` with unit tests so
  future drift between the downloader and `release.yaml`'s upload step
  (`gh release upload "$f#${arch}-${base}"`) is caught at compile time.
- `verify-release-downloads` post-flight job: after every release, downloads
  the published manifest, curl-checks every `<base>/v<tag>/<arch>-<name>` URL
  is reachable, AND runs the just-released binary's `capsem update --assets`
  against real GitHub. Closes the gap that hid the URL bug for one release.
- Fixed `tests/capsem-install/test_asset_download.py`: fake release dir was
  at `v<asset_version>` (mirroring the same buggy mental model as the code).
  Now at `v<binary_version>` so it actually models GitHub.
- Dropped the `_build-host` dependency from `just test-install`. The recipe
  builds host crates inside the container that has the GTK/glib -dev libs;
  the duplicate runner-side build was failing on Ubuntu 24.04 arm64 (no
  libglib2.0-dev), which masked the asset-URL bug because the e2e never ran.

### Security (frontend deps)
- **`marked` 18.0.0 -> 18.0.3** (GHSA-6v9c-7cg6-27q7, HIGH): infinite recursion
  in tokenizer. Direct dev dep; bumped to `^18.0.2`, lockfile resolved 18.0.3.
- **`postcss` >=8.5.10 enforced via pnpm override** (GHSA-qx2v-qp2m-jg93,
  MODERATE): XSS via unescaped `</style>` in CSS stringify output. Pulled
  transitively through `@sveltejs/vite-plugin-svelte > vite > postcss`.
  Override forces every node in the lockfile to >=8.5.10.

### Added (CI)
- **`just audit` recipe.** Fast standalone gate (cargo audit + pnpm audit only,
  no test/build). `just test` Stage 1 already runs both audits; this is the
  pre-push check that doesn't require ~15 min of full-suite work first.
- **`test-linux` job no longer hard-fails when `/dev/kvm` is missing.** The
  "Enable KVM" step is now `continue-on-error: true`, and the verification
  step emits a workflow warning instead of `::error::` + `exit 1`. Hosted
  ARM runners do not always expose nested virt; the compile + non-KVM unit
  tests still run, and real-KVM coverage runs in the release pipeline.
  Workflow comments link future readers to `sprints/done/ci-green` so the
  hard-fail doesn't get reintroduced.

### Changed (Colima default)
- **Bumped Colima default RAM from 8 GB to 16 GB** across `bootstrap.sh`,
  `scripts/doctor-macos.sh`, three skills (`dev-setup`, `dev-start`,
  `build-images`), and four docs pages (architecture/build-system,
  architecture/custom-images, development/getting-started, development/stack).
  The Tauri install-test cold build (`just test-install`) blew past 8 GB
  during cargo compile of the capsem-mcp crates and SIGTERM'd at exit 143.
  16 GB is the recommended floor; 12 GB is the absolute minimum.
- Bumped `@tauri-apps/api` from `^2.10.1` to `^2.11.0` to match the Rust
  `tauri` v2.11.0 crate (`cargo tauri build` refuses mismatched majors/minors).

### Fixed (install-test fixture)
- `tests/capsem-install/test_asset_download.py` hardcoded `serve_dir/v1.0.1/`
  for the fake release dir, but the installed binary builds asset URLs from
  its own `CARGO_PKG_VERSION` (e.g. `v1.0.1777065213`). Every run inside the
  install-test container 404'd. Replaced with `f"v{_binary_version()}"` -- a
  helper that runs `capsem --version` once and uses the result -- so the
  fixture always matches the binary under test, regardless of release tag.

### Deferred
- **Orthogonal asset/binary release cadence** (separate tag scheme + workflow
  for asset-only bumps) is still postponed -- revisit after this URL fix
  ships. The defenses above are designed to also guard the future split.
