# 1.3 Current Release Hotlist

> Execution moved to `sprints/1.3-release-correction/`.
> Keep this file as manual-loop evidence only. Do not implement from this list
> directly without reconciling into the release-correction tracker first.

This is the active debug list for the 1.3 release loop. Older captured bugs in
`tracker.md` are historical evidence; this file is the working queue.

## P0 Release Blockers

- [ ] Security boundary cleanup blocks credential/model release readiness
  - Execution tracker:
    `sprints/1.3-security-boundary-cleanup/tracker.md`.
  - Network engine parses/routes only; it must not decide, broker, redact, or
    credential-classify.
  - Security engine owns rules/plugins/decisions.
  - Every plugin gets a `SecurityEvent` and emits/returns a `SecurityEvent`.
    No plugin gets network, logger, DB, route, or formatter side-channel
    objects.
  - Credential broker is a plugin for runtime capture/store/injection; it does
    not own logging projection.
  - Log sanitizer is a security-engine logging plugin before DB/log/route/UI
    materialization; it does not care whether brokering happened.
  - Runtime bytes and ledger bytes are separate materializations: upstream may
    need the real header/token, but session DB, structured logs, route JSON, and
    frontend stats must only see sanitized broker refs/hashes/bounded previews.
  - No logger-specific sanitizer fallback and no network formatter side-channel.
  - Architecture docs and developer skills must be updated as part of the same
    fix so future agents keep credential handling in the broker/sanitizer rail.

- [ ] No more manual credential/client runs until due-diligence gate passes
  - Do not ask for another Claude/Codex/AGY/OAuth manual run until the local
    hermetic/Ollama/protocol lab proves the core rails without user
    credentials.
  - The gate must prove `user.toml` is burned, not merely ignored: no supported
    config path, broker path, MCP path, service path, runtime policy path, test
    helper, benchmark helper, or profile route may read or write it.
  - The gate must prove profile routes are complete and correct for every
    materialized profile before the UI/TUI uses them: no 404/501, no missing
    overview/enforcement/detection/plugin/MCP/assets route, no mutation route
    that claims success without profile persistence.
  - The gate must prove profile-owned rules/config drive Ollama/local-network
    access, MCP defaults/overrides, plugin modes, detection levels, assets, and
    bootstrap files. No settings/global/user fallback may decide profile
    behavior.
  - The gate must run doctor/e2e/bench against local hermetic services and
    inspect the session DB/logs before any user credential is involved.
  - Manual real-client auth is a final capture/compatibility confirmation, not
    the debugging strategy.
  - Proof slice closed 2026-06-11: `tests/capsem-serial/test_mitm_local_benchmark.py`
    no longer writes a `user.toml`/`settings.toml` policy side channel for
    local MITM ports; focused contract test covers this so benchmark helpers
    cannot rename the old rail.
  - Proof slice closed 2026-06-11: `capsem-process` startup, policy reload,
    and MCP refresh now load runtime rules/plugins/MCP/model endpoints from
    the selected profile directory passed by the service. A build-chain guard
    fails if `capsem-process/src` reintroduces settings/corp runtime loaders or
    old MCP server builders; process CLI tests require `--profile-dir`.

- [ ] Profile/config format linter
  - Add a fast always-on config linter, ruff-style: boring, quick, clear
    diagnostics, and run 100% of the time.
  - It must use the existing `capsem-admin` contract rails instead of adding a
    new `capsem-admin config lint` command.
  - It must cover corp, settings, profile catalog, profile files, rules,
    detection YAML, MCP config, plugins, assets, manifest, and OBOM pins.
  - Profile/admin creation paths must not be able to create an invalid profile.
  - Burn `~/.capsem/user.toml` completely: it is a legacy settings rail that
    must not exist as a supported contract.
  - Remove the APIs and call sites that keep that rail alive, including
    `user_config_path`, `load_settings_files`, `CAPSEM_USER_CONFIG`, and any
    runtime/broker/MCP/service path that reads user runtime policy from
    `user.toml`.
  - Current evidence: a stale retired `ai.anthropic.api_key` entry in
    `~/.capsem/user.toml` prevented credential broker saves during AGY OAuth.
    A dead config file must never block the credential broker or runtime
    security path.

- [ ] Multi-profile materialization bug
  - `just _materialize-config` must materialize every checked-in profile.
  - Materializing `code` must not clobber the generated `co-work` profile back
    to stale source asset hashes.
  - Proof must show both `target/config/profiles/code/profile.toml` and
    `target/config/profiles/co-work/profile.toml` point at current
    `file://` EROFS/LZ4HC assets with matching BLAKE3 hashes.

- [ ] Profile VM storage resources are not proven/applied
  - Manual evidence from Ollama session `code-mq9ymjb2`: installing Ollama
    downloaded `ollama-linux-arm64.tar.zst` and failed extracting under
    `/usr/local/lib/ollama` with repeated `No space left on device`.
  - This should not happen if the `code` profile's
    `vm.scratch_disk_size_gb = 64` is actually applied to the system overlay.
    In VirtioFS mode `/usr/local` writes go through overlayfs to `/dev/vdb`,
    backed by host `guest/system/rootfs.img`. The invariant is:
    `profile.vm.scratch_disk_size_gb == session rootfs.img logical size ==
    guest /dev/vdb size == guest overlay available size` within filesystem
    overhead.
  - Add due-diligence tests and doctor/status evidence proving profile VM
    resources materialize into every new session, stale sessions cannot lie
    about current profile resources, and incompatible/old sessions are clearly
    marked instead of silently running with undersized disks.
  - Doctor/status/debug must report guest `df -h`, `df -i`, `/dev/vdb` size,
    overlay mount source/options, host `rootfs.img` logical size, host physical
    allocated blocks, and free space on the host filesystem backing sparse
    images. ENOSPC must identify whether the limit is guest filesystem,
    rootfs.img logical size, inode exhaustion, or host backing-store pressure.
  - Package/toolchain smoke must include a bounded large-write/install probe
    that proves `/usr/local`, `/var/cache/apt`, `/tmp`, `/var/tmp`, and `/root`
    have expected capacity and fail with actionable diagnostics before any
    partial package extraction corrupts the session.
  - Proof slice closed 2026-06-11: live Ollama test session
    `code-mq9ymjb2` had a 2GiB logical `guest/system/rootfs.img` under a
    profile that now requires 64GiB. Service list/info/status now mark this
    class as `Incompatible` with a rootfs logical-size mismatch reason and
    delete-only actions instead of offering resume/start.

- [ ] Package payload closed contract
  - `.pkg` and `.deb` must contain the app/binaries, runtime config, selected
    manifest, and manifest provenance only.
  - VM asset blobs must not be embedded in installer payloads.
  - Package tests must fail if rootfs/initrd/kernel blobs enter the package.

- [ ] Credential broker Keychain namespace/prompt storm
  - Manual evidence: macOS prompts repeatedly for credential items named
    `com.capsem.credential`, `com.capsem.credentials`, and
    `org.capsem.credentials` during release testing.
  - Canonical production Keychain service namespace is
    `org.capsem.credentials` because the product identity is `capsem.org`.
    `com.capsem.credential` and `com.capsem.credentials` are legacy/wrong and
    must not be used by new broker writes. If migration is needed, it must be
    explicit, one-shot, tested, and silent after completion.
  - The broker must not ask Keychain on every security event. Capture/injection
    needs per-process caching/singleflight/batching so one AGY/Claude/Codex
    session does not trigger a prompt storm.
  - Keychain access must be memory-first and out of status/UI hot paths.
    Once a credential is captured, the broker must keep enough material in
    process memory to keep active agents authenticating without touching
    Keychain on every event. Keychain is durable backing for startup/reload or
    real substitution cache misses, not a per-request dependency. It is
    acceptable for macOS to ask for Keychain access when the user deliberately
    loads credentials; it is not acceptable for stats/status refreshes to
    hammer Keychain or prompt repeatedly after "Always Allow".
  - Linux currently uses a restricted disk-backed durable credential store
    behind the same opaque `CredentialStore` object until we add a real Linux
    secret backend. Release debt: replace the Linux disk backend with native
    protected storage while preserving the `CredentialStore` API.
  - Proof must include a macOS-keychain contract test around service/account
    naming, a test-store equivalent proving repeated broker resolution does
    not hit the backing store per event, and route/plugin runtime counters that
    expose cache hits/misses without raw secrets.
  - 2026-06-13 proof slice: credential storage now goes through the opaque
    `CredentialStore` object. Runtime capture writes memory first, durable
    storage second; replay/status checks are memory-only; real substitution
    can hydrate on cache miss; service `/status` only reports store
    ready/degraded, while `/profiles/{id}/plugins/credential_broker/credentials/info`
    owns backend/cache/hydration details. Added
    `/profiles/{id}/plugins/credential_broker/credentials/reload` as the
    explicit user retry route. Focused proof:
    `cargo test -p capsem-core credential_broker -- --nocapture`; `cargo test
    -p capsem-service credential_broker -- --nocapture`; `cargo test -p
    capsem-service service_status_reports_ready_empty_credential_store_without_inventory_counters
    -- --nocapture`; `cargo check -p capsem-core -p capsem-service -p
    capsem-process -p capsem-proto`; `npm test -- --run
    src/lib/__tests__/api.test.ts`.

- [ ] File boundary ask/rewrite IPC contract is incomplete
  - Manual/code evidence from the S5 Ironbank plugin matrix: file boundary IPC
    originally returned only `success/error`, so plugin rewrite had no channel
    to return mutated bytes to the service. The fix must return rewritten data
    for import/export/read/write boundaries that can be safely transformed.
  - Ask has the same shape problem for decisions: it must not collapse into a
    generic 500/error. File import/export ask must return a typed ask response
    carrying `ask_id`/rule evidence, and the service must not write imported
    bytes or return exported bytes until the ask is resolved.
  - Proof must cover allow, block, rewrite-with-mutated-bytes, disable, and
    ask-pending across UDS result, HTTP status/body, `fs_events`,
    `security_rule_events`, and route-visible latest/status payloads.
  - 2026-06-13 proof slice: file boundary IPC now carries rewritten bytes from
    `capsem-process` back to the service, and the service writes/returns those
    bytes only after the plugin-aware security event rail allows them. Focused
    proof covers UDS data propagation, import/export fail-closed behavior, and
    Ironbank rewrite evidence:
    `cargo test -p capsem-service upload_logs_file_import_before_writing_workspace_file
    -- --nocapture`; `cargo test -p capsem-service
    mounted_file_import_export_routes_log_boundary_events -- --nocapture`;
    `cargo test -p capsem-service
    upload_does_not_write_workspace_file_when_import_ledger_fails --
    --nocapture`; `CAPSEM_TEST_PRESERVE_ALWAYS=1 uv run python -m pytest
    tests/ironbank/test_doctor_ledger.py::test_runtime_plugin_action_matrix_pays_file_import_ledger_debt
    -q -s --tb=short`. Ask remains open and must return a typed ask response,
    not a generic 500.

- [ ] Hermetic integration matrix for all security/event rails
  - Add a release-blocking local integration suite that drives real requests
    through the same Capsem network/MITM/security/logging path used by VMs.
    Parser-only fixtures and DB-row-only tests are not enough.
  - Use local hermetic upstream servers/fixtures, not public APIs, for all
    paths. The tests must prove both byte delivery to the client and ledger
    emission to the session DB/logs.
  - The local test server must use a real OAuth/OIDC library for OAuth flows
    rather than hand-rolled token strings. Tests should exercise auth-code,
    token exchange, refresh, and failure paths through the same broker rail.
  - Model protocol fixtures should be real captured/sanitized records for
    Claude/Anthropic, OpenAI/Codex-compatible, and Gemini/AGY-compatible
    traffic. Store requests/responses as reusable JSON/SSE fixture files so new
    provider cases can be added by recording and sanitizing another exchange.
    The hermetic upstream can then replay those fixtures while Capsem proves
    forwarding, parsing, policy, and logging.
  - Build a recorder that dumps sanitized model exchanges into the fixture
    corpus, then use the same corpus for replay tests. Recording must cover
    model variants beyond simple chat: thinking/reasoning traces, streaming
    deltas, tool declarations, executed tool calls/tool results, large prompts,
    empty/error responses, provider-specific metadata, and any other fields the
    model APIs emit. Adding a new model/provider case should mean recording,
    sanitizing, and replaying a fixture, not writing a bespoke fake.
  - The recorder/replay harness must cover every protocol rail, not just model
    APIs: plain HTTP, HTTPS/MITM, DNS queries/responses, MCP stdio/HTTP JSON-RPC
    initialize/list/tools/call/resources, credential broker capture/rewrite
    cases, and file/process security events where fixture replay is practical.
    The release suite should grow by adding sanitized recorded fixtures for
    each real protocol shape.
  - Ollama should be a first-class live-local backend for recorder and smoke
    tests because it can exercise OpenAI-compatible and Anthropic-compatible
    local traffic and is documented as usable by Codex/Claude integrations.
    Add profile-owned bootstrap/config coverage for clients that can target
    Ollama, including Antigravity-style config such as
    `.antigravity/config.json` with provider `ollama`, `baseUrl`, `model`, and
    `contextLength`.
  - The recorder must support recording against the developer's current local
    Ollama service when available. It should drive real local Ollama requests
    through Capsem's routed/MITM path, sanitize the captured exchanges, and add
    them to the reusable fixture corpus. Replay tests then use those fixtures
    so we can debug protocol parsing without depending on a live Ollama daemon
    for every CI run.
  - The recorder must have explicit client lanes for Claude Code, Codex, AGY,
    and direct protocol probes. Each lane should record the client's real
    startup/config/auth/model/tool traffic through Capsem, sanitize it, and
    store it as replayable fixtures. The fixture metadata must include client
    name/version, config file paths used, protocol family, streaming mode,
    auth mode, expected ledger rows, and expected client-visible bytes.
  - OAuth recording must use a real local OAuth/OIDC provider in the hermetic
    protocol lab for automated tests, plus a manual capture/import path for
    real client OAuth dances such as AGY/Google and Claude login. The recorder
    must classify auth URL, callback/code exchange, token exchange, refresh,
    and failure paths, then sanitize raw codes/tokens while preserving enough
    shape for replay and broker assertions.
  - Client-specific recorder probes must cover at least:
    Claude Code with MCP permissions/dangerous-mode bootstrap and Anthropic or
    Ollama/Anthropic-compatible traffic; Codex with official provider/profile
    config and Ollama/OpenAI-compatible traffic; AGY with `.antigravity` /
    `.gemini` bootstrap, Google OAuth, Gemini/Google streaming traffic, and
    Ollama-compatible local config where supported; direct protocol probes for
    OpenAI-compatible, Anthropic-compatible, Gemini-compatible, MCP JSON-RPC,
    SSE/WebSocket, and credential broker cases.
  - Ollama smoke must also prove the guest package/runtime image can install
    ordinary tooling needed by local backend tests. Manual evidence from
    session `code-mq9ymjb2`: `apt install zstd` completed package processing
    but triggered `/usr/bin/mandb: error while loading shared libraries:
    libmandb-2.11.2.so: cannot open shared object file: Permission denied`,
    plus apt warned that download ran unsandboxed as root because
    `/var/cache/apt/archives/partial/...` was not accessible to `_apt`. This is
    a guest image/package permission or readonly-overlay contract bug, not an
    Ollama protocol bug. Add a smoke test that installs a small package and
    verifies maintainer triggers/shared libraries work under the profile rootfs
    before claiming Ollama/local backend setup works.
  - Ollama itself must not be installed inside the normal guest profile as the
    release test strategy. Manual evidence from an Ollama VM: the upstream
    installer downloaded `ollama-linux-arm64.tar.zst` and failed extracting
    CUDA/llama libraries under `/usr/local/lib/ollama` with repeated `No space
    left on device` errors. The correct release path is host/local-protocol-lab
    Ollama routed through Capsem, not burning guest disk on a local model
    server. Doctor should still report guest disk/free-space and package
    install health clearly so oversized tool installs fail with actionable
    evidence rather than partial corruption.
  - Be explicit about address ownership in Ollama tests: `localhost:11434`
    means guest-local Ollama. If Ollama runs on the host or test harness, the
    profile must use a Capsem-routed host alias/port and the security ledger
    must show that traffic through the normal network/MITM path.
  - HTTP coverage: normal request/response, large bodies, gzip/decompression,
    chunked/streaming body, keep-alive, headers, and bounded previews.
  - DNS coverage: allowed query, blocked query, TXT/long-name exfil shape, and
    rule/detection logging.
  - Model coverage: Anthropic, OpenAI, and Google/AGY protocol shapes;
    streaming SSE and non-streaming JSON; request and response parsing;
    provider/model/token extraction; tool declarations vs executed tool calls;
    exact client-visible stream bytes; and no `hyper serve error`.
  - MCP coverage: initialize, list, tools/call, resources, remote MCP-over-HTTP
    JSON-RPC shape, local built-in MCP, and separation of list/protocol noise
    from real executed tool-call counters.
  - Credential broker coverage: `captured`, `brokered`, `injected`, and error
    events; capture/rewrite must not break HTTP headers, SSE framing, or
    client-visible bytes.
  - Credential broker coverage must include all supported credential material
    types, not only HTTP `Authorization` headers: bearer/basic headers, API
    keys in headers and query params, OAuth auth codes/access tokens/refresh
    tokens/id tokens, JSON/form response bodies, cookies/session cookies,
    file-backed CLI config credentials, environment-style key files, and
    MCP/tool configuration credentials. Each type needs capture, broker,
    inject/replay, failure logging, and no raw durable guest-secret proof.
  - Security engine coverage: allow/pass, ask, block/deny, rewrite/mutate,
    preprocess, postprocess, detection levels, default rules, profile/corp
    priority, and ledger rows for every decision.
  - Security event/CEL contract is missing routed/resolved IP semantics. We
    must expose first-party IP fields for HTTP/DNS/network routes, including
    destination/routed IPs and DNS answers where available, and provide real CEL
    helpers/quantification over those IP values. Do not fake private-network
    policy with host regexes.
  - The same contract must include TCP/UDP route semantics, not only HTTP/DNS
    names: transport protocol, source/destination ports, resolved endpoint,
    routed endpoint, loopback/private/link-local/multicast classification, and
    enough tuple identity for policy and forensic logging. CEL should operate
    on these typed route facts directly so network rules can cover TCP, UDP,
    DNS, HTTP, HTTPS, SSE/WebSocket, and local forwarded services consistently.
  - Add explicit `valid` booleans to parsed first-party CEL objects so rules can
    test object presence/parse success without provider/name/string hacks. Do
    this consistently at both family level and meaningful sub-object level:
    `http.valid`, `dns.valid`, `mcp.valid`, `model.valid`, `file.valid`,
    `process.valid`, `ip.valid`, `tcp.valid`, `udp.valid`,
    `mcp.tool_call.valid`, `mcp.tool_list.valid`, `mcp.event.valid`,
    `model.request.valid`, `model.response.valid`, `model.tool_call.valid`,
    `file.read.valid`, `file.write.valid`, `file.create.valid`,
    `file.delete.valid`, `file.import.valid`, `file.export.valid`,
    `process.exec.valid`, and `process.audit.valid`. Tests must prove these are
    real CEL booleans and not nullable/string conventions.
  - Rule match inputs must be parsed event facts only: `http`, `dns`, `mcp`,
    `model`, `file`, `process`, `ip`, `tcp`, and `udp`. `security.*` is output
    decision/ledger state produced by rules/plugins and must not be a rule
    predicate root; otherwise rules can depend on their own decisions.
  - Add default IP/network guard rules using the real IP abstraction:
    direct localhost/private/non-routable network access is `ask` by default.
    Profile-approved local backends such as Ollama are controlled by explicit
    profile-owned rules using the existing enforcement action enum, not a
    boolean: `allow`, `ask`, `block`, or `disable`. `disable` means the
    route-backed rule exists but does not run; it is not a UI-only state. The
    UI/TUI/API must reflect the rule contract directly, and tests must prove
    Ollama/local-network access changes behavior only through the profile rule
    while other private network access remains at the default `ask` policy.
  - UI/API route coverage: every declared profile/session/stats/settings route
    used by the UI must return the expected contract for every materialized
    profile, with no 404/501.

- [ ] Capsem Doctor / in-VM diagnostic contract is too weak
  - `capsem-doctor` is the canonical in-VM truth probe and must be upgraded
    instead of adding scattered package-manager smoke hacks. It should prove
    the profile image is actually usable for agent work.
  - The current local hermetic/debug server is only partially used by
    benchmark paths (`capsem-bench mitm-local`) and is not wired as the shared
    doctor/integration/benchmark substrate. That is a split rail. Replace it
    with one Capsem local protocol lab used by doctor, integration tests,
    recorder/replay, and benchmarks.
  - `capsem-bench mitm-local` as a separate escape-hatch mode must be removed
    or folded into the normal `capsem-bench` contract. There is one benchmark
    tool. Local hermetic MITM/protocol checks are part of the standard benchmark
    suite and release gate, not a hidden opt-in.
  - Doctor must run against the same hermetic local server/recorder harness used
    by the release integration matrix where practical, so HTTP/HTTPS, DNS,
    MCP, model-shaped traffic, OAuth/broker flows, SSE/streaming, and local
    backend/Ollama paths are verified from inside the VM through the real
    Capsem network/MITM/security/logging path.
  - Doctor must exercise every supported protocol rail and representative edge
    case, not just happy-path connectivity: plain HTTP, HTTPS/MITM, gzip/body
    handling, chunked/streaming, SSE, WebSocket, DNS query/response/TXT-exfil
    shape, MCP stdio/HTTP JSON-RPC initialize/list/tools/call/resources, model
    request/response/tool-declaration/tool-call fixtures, OAuth/broker capture
    and injection, file events, process events, import/export, local backend
    routing, snapshot operations, built-in local MCP tools that call the
    hermetic local server, and blocked/error paths.
  - Doctor must prove the security rail immediately, end to end. It must load
    test rules, trigger every rule action (`allow`, `ask`, `block`, `disable`,
    `preprocess`, `rewrite`, `postprocess`), trigger each detection level,
    exercise the detection facade/Sigma-derived path as well as native
    enforcement rules,
    verify immediate enforcement/detection behavior at the request boundary,
    and then verify the corresponding security-event ledger rows, detection
    vectors, rule ids, actions, decisions, plugin evidence, trace ids, and
    event-specific tables in `session.db`.
  - Remove `--fast` from `just smoke`, `just test`, and every release proof.
    The default doctor path must be fast enough because the protocol lab is
    local and hermetic. If a narrow developer subset exists, it must be
    explicitly named as a targeted subset and cannot be called or treated as
    smoke/release proof.
  - `just smoke` must run the real doctor and the release-critical E2E tests.
    `just test` must run the real doctor, all standard E2E suites, the
    benchmark suite, package/install gates, and Winterfell-style package/fork
    proof. No e2e suite may be silently skipped, hidden behind an environment
    variable, or demoted to a manual-only gate without being marked as an
    explicit release blocker.
  - Doctor must include a guest toolchain health matrix: `apt`, dpkg
    maintainer triggers/shared-library loading, Python, `pip`, `uv`, Node,
    `npm`, `npx`, packaged agent CLIs, shell aliases/wrappers, MCP bootstrap,
    DNS, TLS, filesystem writes, and workspace/profile root assumptions.
  - Doctor must capture full stdout/stderr and fail on dangerous patterns such
    as `Permission denied`, missing shared libraries, `_apt` unsandboxed
    downloads, broken symlinks, hidden package-manager failures, protocol EOFs,
    `hyper serve error`, and truncated/non-JSON tool responses. Do not pipe
    diagnostics through `tail` or otherwise discard the evidence needed to fix
    the image.
  - Doctor output should be structured enough for `capsem status/debug` and bug
    reports: each probe has id, category, command/route, duration, result,
    evidence path, and remediation hint where known. The smoke/release doctor
    must cover the complete release-critical contract.
  - Benchmarks must use the same hermetic protocol lab as doctor and
    integration tests. The release benchmark output must include scaled
    concurrency and request counts for HTTP/SSE/WebSocket, DNS, MCP, credential
    broker, model replay, storage/rootfs, startup, lifecycle, and fork. Tiny
    request counts such as 10 requests are not valid release proof for high-rps
    paths.
  - The Justfile must express this contract plainly: no benchmark-only local
    server, no `user.toml` policy side channel, no hidden public-network
    fallback, no skipped hermetic load path, and no discarded diagnostic output.
  - Add tests proving doctor catches the `code-mq9ymjb2` failure class: package
    install appears mostly successful but maintainer trigger/shared-library
    execution reports `Permission denied`.

- [ ] Installed UI profile readiness
  - UI profile cards must reflect `/profiles/list` and per-profile asset
    status.
  - Missing profile assets must show a download action; ready profile assets
    must enable start.
  - The `co-work` profile must not appear broken because of stale generated
    asset pins.
  - Asset status should render a checklist/list with checkmarks or errors.

- [ ] Profile selection and multi-profile UI
  - Profile selection must be route-backed and multi-profile aware everywhere.
  - Use select controls for profile enum/list choices.
  - The real `co-work` profile must remain a fixture so single-profile
    assumptions cannot creep back in.
  - UI settings must not invent profile data or collapse profile-backed state
    into app settings.
  - Current manual evidence: selecting `Code` in the profile settings/detail UI
    renders `API error 404`. This proves the UI is calling a missing or wrong
    profile route. Add a route-contract test that enumerates every UI-declared
    profile surface route for every materialized profile and fails on 404/501.
  - The route-contract test must cover Overview, Enforcement, Detection,
    Plugins, MCP, Assets, and profile selector transitions for both `code` and
    `co-work`; no human click should be needed to discover missing routes.
  - Dashboard profile cards should follow the Preline card component family
    from `https://preline.co/docs/components/card.html`: card shell, content
    body, and action buttons should match the documented pattern. Keep all
    names/descriptions/icons/readiness from the profile/routes; the UI only
    chooses layout and button affordances.
  - Remove the global `Customize VM...` dashboard action. Each profile card
    should expose `New` as the primary/accent action and `Customize` as the
    secondary/grey action.

- [ ] VM state contract
  - User-facing product language must say `Sessions` and `Profiles`, not `VMs`.
    VM can remain an internal implementation term where appropriate.
  - Session names regressed to raw ids such as `code-mq9ye61s`. Keep raw ids as
    internal stable identifiers, but default user-facing session names should be
    friendly generated names, and create flows should offer a user-provided
    name. The UI/TUI/CLI should display friendly names while retaining raw ids
    for debug/support details.
  - Do not show the build/version string in the top bar of the session detail
    screen; version/build evidence belongs in status/debug/support surfaces.
  - CLI, TUI, and UI must use one backend state enum.
  - Incompatible/defunct VMs must not offer resume/start.
  - Purge must remove defunct VM state.
  - Incompatible/defunct VM rows must be visually disabled/greyed out and
    expose only valid actions, at minimum delete/purge.
  - Optional future affordance: if technically possible and safe, expose
    read-only disk/file inspection for incompatible/defunct VMs; do not show
    it as an active VM action unless it is real.

## P1 Manual-Loop Blockers

- [ ] AGY model/tool observability
  - The code profile should provide the `agy` alias/wrapper that launches with
    the required dangerous-permission flag.
  - AGY traffic must parse into model activity. Tool-call activity must only be
    shown when AGY actually performs a tool call; do not infer or fabricate tool
    calls from model streaming, snapshot internals, HTTP polling, or process
    noise.
  - Stats must show AGY model/tool activity through the unified session DB and
    security-event path.
  - Manual generation evidence from `code-mq9x5edq`: AGY accepted
    `write me a poem to poem.md`, but then reported `model unreachable` /
    network issue. AGY's own log shows repeated EOF failures on
    `POST https://daily-cloudcode-pa.googleapis.com/v1internal:streamGenerateContent?alt=sse`.
    Capsem `net_events` recorded those requests as allowed HTTP 200 rows with
    empty response previews, while `process.log` emitted `hyper serve error:
    user sent unexpected header` immediately after the streaming requests.
    Root cause to investigate: MITM forwarding of SSE/streaming model
    responses is breaking the client-visible stream despite policy allowing the
    request.
  - The same manual evidence created 10 `model_calls` rows for
    `/v1internal:streamGenerateContent`, provider `google`, but model identity,
    token counts, and response body are missing/empty. The model parser must
    preserve the streaming response and still emit complete first-party model
    telemetry.
  - No `poem.md` file events and no MCP rows were recorded, so the failure
    happened before file/tool execution. Any UI/stat surface that reported tool
    calls for this attempt is showing phantom activity and must be corrected.
  - Empty or malformed tool-call detections must be warnings, not counted tool
    calls. A tool call requires a real source row with non-empty provider/name
    identity and stable call id; empty parsed artifacts should produce
    structured warning telemetry and zero user-facing count.
  - Pasted AGY request evidence shows the request advertises 19
    `functionDeclarations` (`write_to_file`, `run_command`, `view_file`, etc.).
    Those are available tool declarations in the model request, not executed
    tool calls. `tools_count` may count declared/available tools only if the UI
    labels it as such; executed tool calls must come from response-side
    `functionCall`/tool-use events or real MCP `tools/call` rows.
  - SSE support is not proven end-to-end. Code has `SseParserHook` and provider
    interpreter hooks, but the AGY manual loop proves the stream forwarding
    contract is broken or incomplete: AGY receives EOF while Capsem logs HTTP
    200 and `hyper serve error: user sent unexpected header`. Add hermetic SSE
    gateway/MITM tests that prove streaming bytes are forwarded intact while
    telemetry is parsed.

- [ ] Credential broker observability and reuse
  - Broker capture/rewrite/replay evidence must be first-class in stats and
    plugin info, not buried under process activity.
  - Credential broker rows are a log, not an inventory object. Do not invent a
    generic `status` field or extra UI-only fields. The event verb must be the
    contract: `captured`, `injected`, or `brokered` (with explicit error
    evidence when the verb failed). The UI should group/filter by verb and show
    the log facts directly.
  - Do not expose a standalone BLAKE3 field as product vocabulary. If a broker
    event includes an opaque credential reference, display it only as the
    event's existing reference value; the important user-facing fact is the
    broker verb and what source/sink it applied to.
  - AGY OAuth capture events must be visible without exposing raw secrets.
  - Brokered credentials must be reusable across future VMs through the broker
    contract, not guest raw-secret config files.
  - Manual evidence: AGY still presents the Google OAuth login flow in a fresh
    session instead of reusing/replaying a previously brokered credential.
    The OAuth URL uses `accounts.google.com/o/oauth2/auth` with
    `redirect_uri=https://antigravity.google/oauth-callback`; do not treat the
    copied authorization code as loggable data.
  - Session DB evidence from `code-mq9x5edq`: `net_events` has AGY startup HTTP
    rows for `antigravity-unleash.goog`,
    `antigravity-cli-auto-updater-974169037036.us-central1.run.app`, and
    `play.googleapis.com`, but no `accounts.google.com` or
    `oauth2.googleapis.com` token-exchange row yet; `model_calls = 0`,
    `mcp_calls = 0`, and `substitution_events = 0`.
  - Updated manual evidence after completing AGY OAuth: session
    `code-mq9x5edq` logged `oauth2.googleapis.com POST /token` with
    credential refs, and the broker detected `client_secret`, auth `code`,
    `access_token`, `id_token`, and `refresh_token`. All five broker save
    attempts failed because `~/.capsem/user.toml` still contains retired
    `ai.anthropic.api_key` settings. Fix must burn/repair that stale settings
    path so broker storage is not blocked by dead AI-provider config.
  - AGY created `.gemini/antigravity-cli/antigravity-oauth-token` in the guest
    workspace. The replay plan should be expressed as broker `injected` events
    through the plugin contract, not raw token material and not an invented
    UI-only hash field.
  - Secret-safe inspection of the AGY token file shows JSON shape:
    top-level `auth_method = consumer` and nested `token` object with
    `access_token`, `refresh_token`, `expiry`, and `token_type`. This is the
    concrete credential shape the broker must capture/replay without exposing
    raw token material.
  - The raw AGY OAuth token should not reach durable guest workspace state.
    This is a boundary failure, not merely a reuse failure: the broker/plugin
    path must capture before guest persistence or immediately rewrite/neutralize
    the guest-visible material, then emit `captured` and `brokered`/`injected`
    events.
  - Inventory AGY/Gemini files from `code-mq9x5edq` and move only bootstrap
    files into the profile root packaging contract. Candidate bootstrap files:
    `.gemini/settings.json`, `.gemini/trustedFolders.json`,
    `.gemini/projects.json`, `.gemini/config/mcp_config.json` if AGY requires
    the file, `.gemini/config/projects/<id>.json`,
    `.gemini/antigravity-cli/settings.json`,
    `.gemini/antigravity-cli/keybindings.json`,
    `.gemini/antigravity-cli/cache/onboarding.json`, and
    `.gemini/antigravity-cli/cache/projects.json`.
  - Do not bake AGY runtime/generated state into profile root:
    `.gemini/antigravity-cli/antigravity-oauth-token`, logs, conversation DBs,
    history, installation IDs, updater locks, knowledge locks, downloaded cache
    binaries such as `bin/webm_encoder`, or Playwright cache.
  - Observed split to preserve in the design: `.gemini/settings.json` declares
    Gemini settings/auth preference, while AGY consumer OAuth state lives under
    `.gemini/antigravity-cli/antigravity-oauth-token`. Bootstrap profile files
    and broker-owned credential runtime state must stay separate.

- [ ] Claude bootstrap prompts
  - Fresh Claude run still prompts `New MCP server found in this project:
    capsem`. Live evidence shows Claude writes
    `/root/.claude/settings.local.json` with
    `enabledMcpjsonServers: ["capsem"]` after the user accepts. The profile
    root currently packages `/root/.mcp.json` and `/root/.claude/settings.json`
    but not that non-secret local approval file.
  - Fresh Claude run also prompts `WARNING: Claude Code running in Bypass
    Permissions mode` with `No, exit` / `Yes, I accept`. The current profile
    sets `permissions.defaultMode = "bypassPermissions"` and
    `skipDangerousModePermissionPrompt = true`, but that is insufficient for
    the installed Claude version/path. The Claude wrapper/bootstrap must use the
    proper supported flag/config state so `claude` starts without a manual
    first-run dangerous-mode prompt inside Capsem.
  - Fresh Claude also reports `claude command at /root/.local/bin/claude missing
    or broken · run claude install to repair`. Manual `claude install` repaired
    to Claude Code `2.1.173`, created `.local/bin/claude` as a symlink to
    `/root/.local/share/claude/versions/2.1.173`, and downloaded a 237 MB native
    binary into `.local/share/claude/versions/2.1.173`. The baked image/profile
    is therefore missing Claude's expected per-user native command layout or is
    shipping an incoherent wrapper/native install.
  - Repair also updated `/root/.claude.json` with native-install state and
    `lastReleaseNotesSeen = "2.1.170"`/runtime metrics. Do not bake volatile
    session metrics or user IDs; extract only the non-secret first-run/install
    contract needed to prevent prompts and broken-command warnings.
  - Add tests later that prove profile root/bootstrap contains every non-secret
    Claude first-run acknowledgement needed for Capsem's sandboxed profile,
    while never baking credentials.

- [ ] Claude LLM streaming / response path broken
  - Manual evidence from `code-mq9ye61s` after Claude OAuth login: Claude sends
    Anthropic `/v1/messages` requests and Capsem creates `model_calls` rows
    with provider `anthropic` and models such as `claude-sonnet-4-6` and
    `claude-haiku-4-5-20251001`.
  - The response side is broken: corresponding `net_events` rows show HTTP 200
    but `bytes_received = 0` and `response_body_preview = null`, followed by
    repeated `hyper serve error: user sent unexpected header`. This mirrors the
    AGY EOF failure and points at the MITM streaming/forwarding boundary, not
    at model classification.
  - Tool execution is not proven for this run: `tool_calls = 0` and
    `tool_responses = 0`; do not infer working tools from model request rows.
  - Claude also emits remote MCP-over-HTTP JSON-RPC traffic to
    `mcp-proxy.anthropic.com`; Capsem promotes it as unknown MCP by bounded
    JSON-RPC shape, but spans still show `provider = none` and the user-facing
    MCP count must not blur this with executed local Capsem tools.
  - Credential broker repeatedly observes Anthropic `Authorization` headers but
    save attempts fail because the dead `~/.capsem/user.toml` validation rail is
    still in the broker path. This blocks reusable credential proof and
    pollutes broker stats with `outcome = error`.
  - After AGY auth, `daily-cloudcode-pa.googleapis.com` traffic is logged as
    HTTP and generation attempts create partial `model_calls` rows, but AGY
    model parsing/telemetry is still incomplete and does not expose enough
    first-party evidence for stats or enforcement confidence.
  - Broker/provider hardening must be validated as one lane: provider
    detection, profile enforcement, broker capture/replay, and plugin/broker
    runtime evidence must agree on the same security-event ledger.

- [ ] Unknown AI/MCP detection
  - Unknown-domain OpenAI/Gemini/Claude-compatible model traffic must be
    detected from bounded protocol shape and include both `http.host` and
    `model.provider`.
  - MCP servers/tools discovered from VM activity must become visible and
    enforceable through profile-scoped MCP/rule surfaces.

- [ ] Security summary UI contract
  - Security stats must be generated from the security ledger rows, not
    invented summary vocabulary.
  - Remove/rename ambiguous `Rules hit` as a primary card. If needed, expose it
    as `Unique rules matched` in a secondary breakdown, because it is not an
    enforcement outcome.
  - Do not privilege `Blocks` as the only action card. The action summary must
    include every rule action bucket with explicit zeroes: allow/pass, ask,
    block/deny, disable, rewrite/mutate, preprocess, postprocess, and any other
    closed enum action we support. The `By Action` table/card is the canonical
    place for action counts.
  - Add detection-level summary from the ledger detection contract:
    `none`, `informational`, `low`, `medium`, `high`, `critical`, with zero
    buckets visible. Detection summary is distinct from enforcement action.
  - Future work: add graphs for action and detection trends, but do not block
    the release UI cleanup on charting.

- [ ] MCP UI/rule editing
  - Rename vague `Policy` UI to explicit Enforcement, Detection, and Plugins
    surfaces.
  - MCP builtin/local naming must be clear.
  - Builtin MCP must not show a misleading `stopped` lifecycle state.
  - Default MCP policy and per-server/per-tool overrides must be editable
    through profile-owned rules.
  - UI must not expose mutation controls that return 501.
  - Disabled MCP/rule/plugin rows should be greyed out with the right
    enum-backed policy/mode icon.

- [ ] MCP stats and pagination signal
  - User-facing MCP totals must not count internal noise such as snapshot or
    protocol maintenance as meaningful tool activity.
  - Session summaries/status must not use raw `COUNT(*) FROM mcp_calls` for
    user-facing MCP/tool-call counters. `raw_mcp_call_count()` may remain only
    as forensic/debug evidence; product stats must use the filtered user-call
    contract and make protocol/snapshot/system activity separate.
  - Large MCP tool responses must stay machine-readable JSON and must not break
    `snapshots` or `capsem-doctor` parsers with a textual pagination prefix.

- [ ] Plugin UI and route contract
  - Plugins must expose backend-owned name, description, version, stage,
    mode, detection level, counters, and status.
  - Disabled plugins must be greyed out and dummy plugins disabled by default.
  - Enum fields use selects; booleans use toggles.

- [ ] Profile overview contract
  - Overview should show profile capability/readiness: available surfaces,
    enabled plugins, credential broker status, credential reference list, and
    blockers that prevent using a surface.
  - It must not duplicate the asset/plugin tabs, but it must summarize their
    route-backed readiness clearly.

- [ ] Process/stats clarity
  - Process observations must be clearly distinguished from command execution
    and security events.
  - Statistics views must show plugin/broker/model/MCP activity through the
    right first-party tabs instead of burying evidence under Process.
  - Stats detail panels currently render the same row twice for all event
    types: first as a JSON object and then again as a raw key/value dump. This
    affects HTTP, DNS, and likely model/MCP/file/process details. Replace the
    generic duplicated drawer with one canonical presentation per event type:
    metadata once, then type-specific sections such as headers/previews for
    HTTP or resolver fields for DNS. Raw full-row JSON may exist only behind an
    explicit debug affordance, not as the default view.
  - HTTP detail specifically must not invent parsed backend fields. If
    `request_body_preview` or `response_body_preview` parses as JSON,
    pretty-print that bounded preview in-place and label it as a bounded
    preview.
  - Payload/content rendering must use the content metadata we already have:
    content-type/mimetype, file extension, parser result, and Shiki/code
    highlighting. JSON should render as formatted JSON, text as text, code as
    highlighted code, binary as a compact metadata/download view, and truncated
    previews must say they are truncated. No escaped JSON strings as the primary
    user-facing rendering when the payload can be parsed and highlighted.
  - File stats cards currently show `Imports` / `Exports` / `Brokered Refs`,
    but the live `fs_events` action vocabulary for the AGY session is
    `created`, `modified`, and `deleted` (`modified=92`, `created=52`,
    `deleted=2`, `credential_ref=0`). The top cards must summarize the same
    action vocabulary shown in the table, or explicitly separate import/export
    surfaces when real import/export events exist. Do not show zero-valued
    unrelated concepts as the primary file summary.

## P2 Hardening Follow-Ups

- [ ] Snapshot boundary
  - Snapshot state must stay route-backed and hermetic.
  - Snapshot internals must not bleed into user-facing MCP/file/process stats
    unless an AI explicitly calls the snapshot MCP tool.
  - Workspace symlink escape/restore protections must stay tested.
  - Snapshot restore must not follow symlinks out of the workspace.
  - Snapshot/file provenance bugs must be traceable before deleting any files
    created during manual loops.

- [ ] DNS exfiltration hardening
  - DNS tunneling control belongs in the security rail as a real rule/rate
    limiting/cost-control system, not a one-off DNS hack.

- [ ] Raw VSOCK hardening
  - Host VSOCK listener inventory and fail-closed registry must remain visible
    in debug/status output.

- [ ] Support/debug report quality
  - `capsem debug` and `capsem status` must include enough service/profile/VM/
    plugin/manifest evidence for useful bug reports.

## Current Execution Order

1. Fix multi-profile materialization with a failing test first.
2. Re-run focused profile/package tests.
3. Re-run `just test`.
4. Only then run install/manual UI validation again.
5. Implement the always-on config linter before release sign-off.
