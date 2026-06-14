# 1.3 Security Boundary Cleanup Plan

## Why

The current credential/debug loop exposed an architecture smell: credential
handling was drifting toward transport formatting. That breaks the single-rail
security model. The network engine must parse, classify, route, and preserve
runtime bytes; the security engine must own decisions and plugin mutation; the
logger must only receive a sanitized ledger projection.

This sprint burns the ambiguous boundary. It establishes explicit plugin object
contracts and renames code/docs so future work cannot confuse network
mechanics with security decisions or logging projection.

## End Posture

- **Network engine** owns transport mechanics only: capture bytes, parse facts,
  route requests, preserve client/upstream behavior, and emit a `SecurityEvent`.
- **Security engine** owns rules, plugin execution, decisions, detections, and
  event mutation.
- **Plugin object contract** is one shape everywhere: plugin receives a
  `SecurityEvent` and emits/returns a `SecurityEvent`. That is it. No plugin
  gets network formatter state, DB writer state, route state, or a logger
  side-channel object. Stage-specific traits/objects may define when the plugin
  runs, but the data contract remains `SecurityEvent -> SecurityEvent`.
- **Credential broker plugin** owns credential capture, storage, and runtime
  substitution from VM-origin traffic to host-side broker references. It does
  not care about logging and must not be implemented as network formatter
  heuristics.
- **Log sanitizer logging-plugin** owns durable ledger projection inside the
  security engine pipeline. It runs after pre plugins, rules, post plugins, and
  emission-time decision work, but before the event is handed to logger/storage
  materializers. It does not care whether brokering happened; it transforms the
  `SecurityEvent` it receives into another `SecurityEvent`.
- **Runtime materialization and ledger materialization are separate.** The
  upstream request may carry the real header/token when needed; the ledger must
  carry only broker refs, hashes, bounded previews, and typed redaction facts.
- No credential classification, broker-reference creation, or
  provider-sensitive redaction lives in HTTP header formatting, MITM/network
  intercept utility code, DB readers, frontend transforms, or a logger-specific
  fallback branch.
- The logger remains a ledger writer. It writes the event it is handed; logging
  plugins produce the already-sanitized/enriched projection before handoff.

## Naming Cleanup

Names must describe the boundary:

- Use `network engine` / `network intercept` for transport capture and routing.
- Use `security engine` for rule/plugin/decision execution.
- Use `credential broker` for capture/store/inject behavior.
- Use `log sanitizer` for final ledger-safe projection.
- Keep legacy `mitm_proxy` paths only where immediate module renames would be a
  mechanical follow-up; user-facing text, docs, tracker language, and new code
  must not teach that credential/security logic belongs to "MITM".

## Tasks

1. Contract tests first.
   - RED: a request with `Authorization: Bearer raw-secret` sent through the
     security engine with broker + sanitizer enabled keeps upstream/runtime
     materialization valid but ledger materialization contains no raw secret.
   - RED: the security engine logging-plugin sanitizes raw credential-bearing
     events before logger/storage materialization.
   - RED: network header formatter has no credential/provider-specific
     behavior and cannot produce `credential_ref` by itself.
   - RED: UI/stats route payloads expose only sanitized fields.

2. Plugin split.
   - Introduce explicit plugin stages for pre-rule mutation, post-rule
     mutation, and logging-time materialization if the existing enum cannot
     express the ordering safely.
   - Define explicit plugin object contracts: base plugin metadata plus pre,
     post, and logging stages. Every stage must be `SecurityEvent ->
     SecurityEvent`; any different input/output contract is a second rail and
     is rejected.
   - Move credential capture/substitution behavior into the credential broker
     plugin at the appropriate pre-rule/runtime stage.
   - Add `log_sanitizer` as the logging plugin that produces ledger-safe event
     projection before logger materialization.
   - Do not add a logger fallback/special case. Sanitization is a plugin stage,
     not DB-writer behavior.

3. Materialization split.
   - Define one function/type for upstream/runtime HTTP materialization.
   - Define one function/type for ledger/log materialization.
   - Ensure logger writes and frontend stats read from the ledger projection,
     never from raw runtime bytes.
   - Preserve client-visible bytes and upstream headers where protocol requires
     real credentials.

4. Boundary cleanup.
   - Remove credential/provider redaction from network formatter/utilities.
   - Rename newly touched user-facing logs/docs from MITM-centric wording to
     network-engine/security-engine wording.
   - Add code comments only at the boundary where they prevent future drift.
   - Update architecture docs so admins/developers see the same rail:
     network engine parses/routes, security engine decides/mutates, credential
     broker handles runtime capture/injection, logging plugins own durable
     projection/enrichment.
   - Update developer skills so future agents do not put credential logic back
     into network formatters, DB readers, frontend transforms, or ad hoc test
     harnesses.

5. Ironbank proof.
   - Add/extend `tests/ironbank/` coverage for HTTP credential header capture,
     broker ref ledger output, route/UI JSON output, and no raw secret in
     session DB/logs.
   - Add model SDK/OpenAI-compatible replay proof using the hermetic mock server
     so model requests still work while logs stay sanitized.
   - Add an adversarial test for raw secret in headers, query, JSON body, form
     body, and response token body.

## Files Likely Touched

- `crates/capsem-core/src/security_engine/*`
- `crates/capsem-core/src/security_engine/plugins/*`
- `crates/capsem-core/src/credential_broker.rs`
- `crates/capsem-core/src/net/mitm_proxy/*` only to remove security logic and
  route materialized events correctly
- `crates/capsem-logger/*`
- `crates/capsem-service/*` route payload contracts if they currently expose raw
  network rows
- `tests/ironbank/*`
- `sprints/1.3-release-correction/*`
- `docs/src/content/docs/architecture/*`
- `skills/*` if boundary rules need developer reinforcement

## Proof Matrix

- Unit/contract:
  - Security engine plugin object contracts and ordering.
  - Every plugin object receives a `SecurityEvent` and emits/returns a
    `SecurityEvent`.
  - Credential broker plugin captures/stores/attaches refs without owning
    logging projection.
  - Log sanitizer removes raw values from ledger projection.
- Functional:
  - HTTP request reaches hermetic upstream with expected auth behavior.
  - Logger/session DB contains only sanitized credential refs/hashes.
  - Service/gateway stats routes return sanitized JSON.
- Adversarial:
  - Raw secret in header/query/body/response never reaches durable logs.
  - Missing sanitizer fails closed.
  - Network formatter cannot independently credential-classify or produce
    broker references.
- E2E/VM:
  - Ironbank VM/protocol test drives a real client-style request through
    Capsem and checks client bytes, DB rows, logs, UDS/HTTP route payloads.
- Telemetry:
  - Structured logs identify broker capture, broker injection, sanitizer
    redaction, plugin latency, and security decision without raw secrets.
- Performance:
  - Plugin counters record latency. Benchmarks must show sanitizer work is
    bounded by preview caps and does not reparse large bodies unnecessarily.

## Done

- No raw credential can appear in session DB, route JSON, structured logs, or
  frontend stats when broker + sanitizer are enabled.
- Real upstream/runtime credential behavior still works.
- Logging plugins emit sanitized events for logger/materializer paths without
  adding logger-specific fallback logic.
- Network engine code has no credential-sensitive formatter heuristics.
- Docs and skills state the boundary in plain language.
- Plugin object contracts are explicit in code/docs: plugins get a
  `SecurityEvent`, emit a `SecurityEvent`, and no other object is accepted.
- Focused tests pass, Ironbank test is green, changelog updated, commit pushed.
- Architecture docs and relevant skills describe the boundary and forbid the
  old drift.
