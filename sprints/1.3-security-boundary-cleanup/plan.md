# 1.3 Security Boundary Cleanup Plan

## Why

The current credential/debug loop exposed an architecture smell: credential
handling was drifting toward transport formatting. That breaks the single-rail
security model. The network engine must parse, classify, route, and preserve
runtime bytes; the security engine must own decisions and plugin mutation; the
logger must only receive a sanitized ledger projection.

This sprint burns the ambiguous boundary. It splits credential handling into
two explicit phases and renames code/docs so future work cannot confuse network
mechanics with security decisions.

## End Posture

- **Network engine** owns transport mechanics only: capture bytes, parse facts,
  route requests, preserve client/upstream behavior, and emit a `SecurityEvent`.
- **Security engine** owns rules, plugin execution, decisions, detections, and
  event mutation.
- **Credential broker pre-plugin** owns credential capture and runtime
  substitution from VM-origin traffic to host-side broker references. It may
  attach opaque broker refs to the in-memory event and prepare safe runtime
  injection metadata, but it must not be implemented as network formatter
  heuristics.
- **Log sanitizer final plugin** owns the final ledger projection. It is the
  last mutation step before any `SecurityEvent` is materialized into logger
  rows, structured logs, UI JSON, or route responses.
- **Runtime materialization and ledger materialization are separate.** The
  upstream request may carry the real header/token when needed; the ledger must
  carry only broker refs, hashes, bounded previews, and typed redaction facts.
- No credential classification, hashing, or provider-sensitive redaction lives
  in HTTP header formatting, MITM/network intercept utility code, DB readers, or
  frontend transforms.

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
   - RED: disabling/removing the sanitizer fails closed before logger write.
   - RED: network header formatter has no credential/provider-specific
     behavior and cannot produce `credential_ref` by itself.
   - RED: UI/stats route payloads expose only sanitized fields.

2. Plugin split.
   - Introduce explicit plugin stages for pre-decision/pre-runtime mutation and
     final ledger sanitization if the existing enum cannot express the
     ordering safely.
   - Move credential capture/substitution behavior into the credential broker
     pre-plugin.
   - Add `log_sanitizer` as a mandatory final plugin for logger materialization.
   - Make missing final sanitizer a fail-closed condition for all security-event
     logger writes.

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
     broker handles runtime capture/injection, log sanitizer owns durable
     projection.
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
  - Security engine plugin ordering and fail-closed sanitizer contract.
  - Credential broker pre-plugin captures/stores/attaches refs without logging
    raw values.
  - Log sanitizer removes raw values from ledger projection.
- Functional:
  - HTTP request reaches hermetic upstream with expected auth behavior.
  - Logger/session DB contains only sanitized credential refs/hashes.
  - Service/gateway stats routes return sanitized JSON.
- Adversarial:
  - Raw secret in header/query/body/response never reaches durable logs.
  - Missing sanitizer fails closed.
  - Network formatter cannot independently credential-classify.
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
- Missing/broken sanitizer fails closed.
- Network engine code has no credential-sensitive formatter heuristics.
- Docs and skills state the boundary in plain language.
- Focused tests pass, Ironbank test is green, changelog updated, commit pushed.
- Architecture docs and relevant skills describe the boundary and forbid the
  old drift.
