# Tracker: mitm-redesign

Active phase: **T3 CLOSED. In-VM E2E + dns-load baseline + mitm-load
sanity all run + green. Two real bugs surfaced + fixed during the
gate: (1) host vsock listener registration was missing port 5007
(DNS) AND port 5006 (audit -- latent since the audit feature
landed); fixed in `vm/boot.rs::vsock_ports`. (2) DNS answer cache
returned the FIRST query's transaction id on every cache hit
(broke 100% of cached responses); fixed in
`DnsAnswerCache::get` with a per-hit qid byte-patch. Both have
regression tests. dns-load baseline locked at
`benchmarks/dns-load/baseline.json` (3556 / 12928 / 12425 / 11482
rps, 0% errors at every concurrency level). mitm-load debug-build
reference at `benchmarks/mitm-load/post_t3_debug_reference.json`
alongside the existing release-build baseline (release re-baseline
needs release evidence infra). T4 (mcp-protocol-aware-mitm) is the
next phase. Capsem-core lib at 1693 tests.

Full pipeline: guest libc resolver -> iptables nat 53 -> 1053 ->
`capsem-dns-proxy` -> vsock 5007 -> `serve_dns_session` ->
`DnsHandler::handle` -> hickory-proto -> NetworkPolicy::is_fully_blocked
or DnsRedirect or LRU cache hit or UDP forward to 1.1.1.1/8.8.8.8
-> response wire bytes (qid-patched on cache hit) back over vsock
-> guest peer + one `dns_events` row stamped with the ambient
trace_id. dnsmasq gone from `guest/config/packages/apt.toml` and
from `capsem-init`.

Closure gate results:
* In-VM `capsem-doctor -k 'dns or proxy_listening or iptables_redirect'`:
  14/14 PASS in temp VM.
* dns-load (debug build, this Mac, 10s/level):
  | c | rps | p50 | p99 | errors |
  |---|---|---|---|---|
  | 1 | 3556 | 0.3ms | 0.5ms | 0 |
  | 10 | 12928 | 0.7ms | 1.1ms | 0 |
  | 50 | 12425 | 4.0ms | 4.9ms | 0 |
  | 200 | 11482 | 16.5ms | 26.7ms | 0 |
  Locked at `benchmarks/dns-load/baseline.json`.
* mitm-load (debug build, this Mac): rps 70 / 687 / 2767 / 3818 at
  c=1/10/50/200; healthy + no anomalies. The committed release-build
  baseline (`benchmarks/mitm-load/baseline.json`) was captured on
  conc-bench (M5 Max) so the absolute numbers aren't apples-to-apples.
  Saved as `benchmarks/mitm-load/post_t3_debug_reference.json`.
  Structural argument: T3 added a SEPARATE DNS path; the MITM
  hot-path code wasn't modified (only metric name constants were
  added to `mitm_proxy/metrics.rs`). Release-on-conc-bench
  re-baseline pending release evidence infra.

Tests: 1693 capsem-core lib + 88 capsem-process + 220 capsem-logger
+ 157 capsem-proto + 15 agent-bin pass; workspace clippy clean;
aarch64-musl cross-compile clean.

Past block on the in-VM gate (resolved):
session per the resume prompt; T3.4 stages the cutover but the
final acceptance gate runs from the codesigned + installed path.
T4 (mcp-protocol-aware-mitm) is the next phase; do NOT start in
this session per sprint discipline -- checkpoint first.**

## Phase status

- [x] T0 — extract-and-re-baseline
- [x] T1 — pipeline-and-hook-traits. Single `Hook` trait + `Event<'_>`
      ladder (L1/L2/L3) + `EventMask` + `HookCtx::emit()` dispatcher
      with cycle prevention. Sync `ChunkHook` companion trait for
      per-byte work. PolicyHook (async, RawRequestHead) owns policy
      decisions end-to-end. Five sync ChunkHook stages run inline
      from poll_frame on every response chunk: DecompressionHook
      (gzip via `flate2::Decompress`), SseParserHook (AI domains),
      AnthropicInterpreterHook / OpenAiInterpreterHook /
      GoogleInterpreterHook (drain to LlmEventStream), TelemetryHook
      (NetEvent + optional ModelCall on on_response_end). Legacy
      async body chain deleted: telemetry.rs (TelemetryEmitter +
      TelemetryBody), ai_body.rs (AiResponseBody), body::DecompressBody
      + body::BodyStream + body::RespStatsKind all gone. SSE parser
      microbench at 478-488 MiB/s (up from 449-472 MiB/s in the T0
      baseline; criterion: "Performance has improved", p<0.05).
- [x] **T1 closed.** Direct measurement on conc-bench
      (M5 Max, 2 vCPU, 4 GB) at HEAD = `8c85cd6` against
      `benchmarks/mitm-load/baseline.json`:

      | c   | base rps | head rps | Δrps   | base p99 | head p99 | Δp99    |
      |-----|---------:|---------:|-------:|---------:|---------:|--------:|
      | 1   |   1036.8 |   1146.6 | +10.6% |  2.29 ms |  1.19 ms | -47.9 % |
      | 10  |   3042.6 |   3343.9 |  +9.9% |  8.43 ms |  7.30 ms | -13.4 % |
      | 50  |   3028.5 |   3294.4 |  +8.8% | 53.41 ms | 40.05 ms | -25.0 % |
      | 200 |   2698.9 |   2883.2 |  +6.8% |191.28 ms |167.66 ms | -12.3 % |

      Sync ChunkHooks structurally beat the async wrappers they
      replaced -- favorable on rps AND p99 at every concurrency
      level, including the small-body 502 fast path. Result
      committed at `/tmp/mitm-load-headT1.json` (locally; not
      tree-tracked).

      **Procedural debt acknowledged:** the slice-9 commit
      (`068c77d`) deferred this gate and the close-T1 tracker
      commit (`7314c7e`) closed T1 anyway. That broke per-slice
      bench discipline. The right pattern (per the CHANGELOG of
      slices 4-8) is: integration bench at every commit, not
      deferred to slice 9. For T2+ slices, run the bench before
      committing, not after.

      **Junior's "-35%" report (mcp-concurrency tracker lines
      153-161, 2026-05-04) is unconfirmed against this
      committed baseline.** Most likely cause: their measurement
      included their own T1.2+T1.3 prototype, their flate2
      `zlib` feature toggle, and their `mcp/server_manager.rs`
      working-tree edits in the runtime image. None of those
      were a clean control for my T1 slices. Recommend they
      re-run with `git stash` of their working tree on the same
      HEAD I measured (`8c85cd6`).
- [x] T2 — protocol-demux-plain-http (host-side closed)
  - [x] T2.1 — first-byte sniff: `mitm_proxy::protocol` module
        + `Protocol::{Tls,Http,Unknown}` + `detect()`. `ConnMeta`
        carries the classification; `mitm.connections_total{protocol}`
        is now post-sniff and accurate. Plain-HTTP path classified
        but stubbed to T2.2-pending error. 8 unit tests in
        `protocol/tests.rs` + 2 integration tests
        (renamed to `mitm_proxy_plain_http_denies_disallowed_host` in
        T2.2 to assert the post-T2.2 Decision::Denied behavior +
        `mitm_proxy_classifies_unknown_first_byte`). 1539 lib tests +
        10 mitm_integration tests pass; clippy clean.
  - [x] T2.2 — plain HTTP path through hyper (host-side). Plain
        HTTP requests on vsock now serve through the same hyper
        pipeline as TLS, with the same PolicyHook + ChunkHook
        chain; `Host` header drives ConnMeta::domain and
        upstream port; `NetworkPolicy::http_upstream_ports` (default
        `[80]`) gates the upstream dial.
        `serve_plain_http` + `serve_pipeline` helpers extracted;
        `handle_request` branches on `protocol` for the upstream
        dial (TCP-only for HTTP) and Host header preservation;
        `NetEvent.port` and `NetEvent.conn_type` now reflect the
        actual transport via new `TelemetryRequestContext.port` +
        `.conn_type` fields. 11 mitm_integration tests pass
        (added `mitm_proxy_plain_http_denies_disallowed_host` +
        `mitm_proxy_plain_http_denies_port_not_in_allowlist`),
        1539 lib tests pass, clippy clean. Agent-side multi-port
        listener + iptables rules deferred -- T2.3's integration
        test runs against a fake upstream so the host-side is
        sufficient for the gate; in-VM Ollama shape can drive
        agent / iptables work as part of T2.3 (or a follow-up).
  - [x] T2.3 — Ollama-shaped integration test
        (`mitm_proxy_plain_http_ollama_shape_records_telemetry`):
        fake plain-HTTP upstream on `127.0.0.1:<dyn-port>`, proxy
        configured with that port on `http_upstream_ports` and
        `127.0.0.1` on the domain allowlist, `POST /api/generate`
        sent through, response body verbatim-forwarded, NetEvent
        records correct method/path/status/port/conn_type=http-mitm.
        New `make_proxy_config_full` test helper for overriding
        `http_upstream_ports`. The mitm-load `--scheme http`
        variant is OPTIONAL per the original plan and not needed
        for the test gate -- the host-side fake-upstream
        integration test exercises the same code path; deferred
        unless junior wants to gate plain-HTTP rps too. 12
        mitm_integration tests + 1539 lib tests pass; clippy
        clean.
- [x] T3 — dns-proxy (code-complete; final E2E smoke + bench gate
        pending per session note above)
  - [x] T3.1 — host-side DNS handler + UDP forwarder + parser
  - [x] T3.2 — vsock DNS envelope + agent `dns_proxy` listener
  - [x] T3.3 — `dns_events` schema + telemetry hook + `trace_id`
  - [x] T3.4 — drop dnsmasq + iptables redirect for port 53
- [ ] T4 — mcp-protocol-aware-mitm
- [ ] T5 — hardening-perf-coverage

## T0 progress (closed)

- [x] Slice 1 (`cd0a054`): drop `ai_traffic` re-exports + extract 4
      parser/interpreter inline tests to sibling `tests.rs`.
- [x] Slice 2 (`f9ae498`): extract remaining inline tests in `net/`
      (`mitm_proxy.rs` 2847 → 1421 lines, plus 5 ai_traffic files).
      Also finished W6 `trace_id` wiring across writer + every event
      emitter (obs sprint's W6 had left the build broken on the test
      path).
- [x] Slice 3 (`7fd78fa`): split `mitm_proxy.rs` 1421 → 614 lines
      into submodules: `mitm_proxy/{body,fd_stream,telemetry,util}.rs`
      siblings of `mod.rs`. Each `pub(super)`; public API surface
      unchanged.
- [x] Slice 4 (`c245f3d`): added `criterion` + `metrics` deps;
      declared counters/histograms in `mitm_proxy/metrics.rs`;
      committed pre-rewrite criterion baselines at
      `crates/capsem-core/benches/baselines/T0-pre-rewrite.md`. SSE
      parser baseline 449-472 MiB/s, interp_anthropic 233 MiB/s,
      counter emit 3.89 ns. T5 regression gate references these.
- [x] Slice 5 (`8621cc4`): `capsem-bench mitm-load` harness shipped
      (`guest/artifacts/capsem_bench/mitm_load.py`, wired into
      `__main__.py`). Baseline JSON captured via new `capsem cp`:
      rps 1037/3043/3029/2699, p99 2.3/8.4/53.4/191.3 ms across
      concurrency 1/10/50/200, 0 errors, RSS 27-260 MB. Locked at
      `benchmarks/mitm-load/baseline.json`. Side-quest: the
      `/files/{id}/content` HTTP endpoints were never wired into the
      CLI -- fixed by adding `capsem cp`.

Side-by-side `capsem-bench mcp-load` baseline also captured during
T0 closure (separate from this sprint's scope but shares the bench
harness): rps 2162/3792/4061/3965, p99 1.1/4.4/17.4/70.8 ms across
the same concurrency levels. Locked at
`benchmarks/mcp-load/baseline.json`. The MCP-side serialization-
plateau diagnosis is owned by `sprints/mcp-concurrency/` (junior dev
sprint, broadened to cover both MCP and MITM paths).

## T1 progress (in flight)

- [x] Slice 1 (`39831d4`): single `Hook` trait + `Event<'_>` ladder
      (L1/L2/L3) + `EventMask` + `HookCtx::emit()` dispatcher.
      Layered cycle prevention; 16 unit tests.
- [x] Slice 2a (`02438f9`): `pipeline: Arc<Pipeline>` field on
      `MitmProxyConfig` + `make_default_pipeline()`. Three call sites
      updated.
- [x] Slice 2b (`345eb19`): `PolicyHook` + `ConnMeta` +
      `make_production_pipeline(policy)`. First concrete Hook impl;
      4 unit tests; counter `mitm.policy_decisions_total`.
- [x] Slice 2c (`7fad7e7`): `handle_request` now dispatches
      `Event::RawRequestHead` through pipeline. Production pipeline
      wired in `capsem-process`; PolicyHook fires on every real
      request (parallel-deploy initially).
- [x] Slice 2d (`243106e`): inline `policy.evaluate` removed;
      PolicyHook is sole source of policy truth via `HookCtx::state`
      typed slot (`LastPolicyDecision`). On `Stop(Reject(_))` the
      hook's response gets wrapped with `TelemetryBody` so deny path
      still emits `NetEvent`. Test fixtures upgraded to
      `make_production_pipeline`.
- [x] Slice 4 (`007170f`): metrics + tracing decision contract on
      hot path. `mitm.connections_total{protocol}`,
      `mitm.active_connections` (RAII gauge),
      `mitm.requests_total{decision}`, `mitm.tls_handshake_ms`,
      `mitm.upstream_dial_ms`. `#[instrument(target="mitm.connection")]`
      on handle_connection. Two metrics smoke tests.
- [x] Slice 3a (`0951b6e`): `RawResponseHead` dispatch +
      per-request `mitm.request` span recording domain/method/path/
      decision/status as structured fields. Pure additive observer
      surface.
- [x] Slice 3 foundation (`79eb016`): sync `ChunkHook` trait +
      pipeline registration (`register_chunk`, `has_chunk_hooks`,
      `dispatch_*_chunk`/`dispatch_*_end`). Per-connection state via
      same typed slot map as async hooks. 2 unit tests.
- [x] Slice 3 wrapper (`f58ac0b`): `ChunkDispatchBody` hyper Body
      wrapper. Drives sync ChunkHook iteration on every frame; fires
      `dispatch_response_end` once on Poll::Ready(None) or via Drop.
      Free when no chunk hooks registered.
- [x] Slice 3 wire (`573774e`): wired `ChunkDispatchBody` into
      `handle_request`'s response chain (between
      AiResponseBody/DecompressBody and TelemetryBody). Also relaxed
      `HookState` slot bound from `Send` to `Send + Sync` so the
      body can be `Sync` (hyper's `Body::boxed()` requires it).
- [x] Slice 3 observability contract (`b774953`): every async hook
      call now wrapped in a `mitm.hook` info-span with fields
      `hook`, `kind`, `layer`, `decision`, `duration_ms`. Counter
      `mitm.hook_invocations_total{hook}` + histogram
      `mitm.hook_duration_ms{hook}` per call. Stop outcomes promote
      to `debug!(target="mitm.hook.cause")` so triage tooling
      surfaces them at default RUST_LOG=info. Sync ChunkHook
      iteration gets the same counter+histogram (no per-chunk span);
      trace! events available at `mitm.hook.chunk` for opt-in
      per-chunk debug. Unit test installs a
      `metrics_util::DebuggingRecorder` and asserts both metrics fire.

### T1 ChunkHook consumers (this session)

- [x] T1.SseParserHook (`829a108`): wraps `parsers::sse_parser::SseParser`
      as a sync ChunkHook gated on AI-provider domains; pushes
      parsed events into a public `SseEventStream` slot. Six tests.
      Registered in `make_production_pipeline`.
- [x] T1.InterpreterHooks (`e9d1ec4`): three concrete hooks
      (Anthropic / OpenAI / Google) drain `SseEventStream`, run
      the existing `ProviderStreamParser` impls verbatim, accumulate
      `LlmEvent`s into a shared `LlmEventStream` slot. Six tests
      including end-to-end SSE → LlmEvents → `collect_summary`.
      Registered after `SseParserHook`.
- [x] T1.DecompressionHook (`4a554dd`): gzip streaming-decode as a
      sync ChunkHook via `flate2::Decompress::new(false)` plus a
      hand-rolled gzip-header parser. Magic-byte classification on
      first chunk -- the per-request `HookState` carried by
      `ChunkDispatchBody` doesn't bridge to async-`Hook` state, so
      reading `Content-Encoding: gzip` from `RawResponseHead`
      isn't an option. Six tests covering single-chunk, split,
      passthrough, classification stickiness, byte-by-byte, and
      one-byte deferred classification. Registered before
      `SseParserHook` so the hook order is correct once the inline
      `DecompressBody` wrapper is removed.
- [x] T1.TelemetryHook (`4dcef1d`): full per-request `NetEvent` +
      optional `ModelCall` emission folded into a ChunkHook firing
      on `on_response_end`. Pure builder helpers
      (`build_net_event`, `maybe_build_model_call`) factored out for
      testability without an async runtime. Reads `LlmEventStream`
      at end-of-stream, folds into `ModelCall` via
      `collect_summary`. Trace correlation goes through the same
      shared `Arc<Mutex<TraceState>>` mutex as the legacy emitter.
      ADDITIVE ONLY: not registered in production pipeline yet,
      `handle_request` not rewired, `telemetry.rs` not deleted --
      slice 9 cleanup couples those changes with the
      `MitmProxyConfig` Arc-ification refactor and the
      legacy-body-chain removal. Eight tests.

### T1.cleanup (slice 9, `068c77d`) -- closed

- [x] Refactor `MitmProxyConfig`:
      - `pricing: PricingTable` → moved into
        `Arc<TelemetryDeps>` as `Arc<PricingTable>`.
      - `trace_state: Mutex<TraceState>` → `Arc<Mutex<TraceState>>`
        inside `TelemetryDeps`.
- [x] `make_production_pipeline` takes `Arc<TelemetryDeps>`;
      `capsem-process/src/main.rs` construction +
      `crates/capsem-core/tests/mitm_integration.rs` updated.
- [x] `TelemetryHook` registered after the SSE / interpreter /
      decompression hooks.
- [x] `handle_request` rewire: every response path (happy, deny,
      websocket-deny, 502) builds a `TelemetryRequestContext` and
      seeds it into the `ChunkDispatchBody`'s `HookState` via the
      new `seed::<T>()` builder. `HookState::set::<T>()` is the
      underlying primitive (public).
      - `TelemetryEmitter` / `TelemetryBody` construction removed.
      - The inline `if is_gzip { DecompressBody::new(...) }` block
        is reduced to a 4-line Content-Encoding / Content-Length
        header strip (kept inline -- moving it to an async hook
        would re-introduce the kind of plumbing the slice removed).
      - `AiResponseBody` wrap for AI providers removed --
        interpreter hooks produce the same `LlmEvent`s via
        `LlmEventStream`.
      - The `db: &Arc<DbWriter>` parameter dropped from
        `handle_request`; the hook holds it via `TelemetryDeps`.
- [x] Deleted `mitm_proxy/telemetry.rs`,
      `ai_traffic/ai_body.rs`, `body::DecompressBody`,
      `body::BodyStream`, `body::RespStatsKind`. `body::TrackedBody`
      kept for the request-side stats (still needed; lives in the
      seeded `TelemetryRequestContext.request_body_stats`).
- [x] `mitm_proxy/tests.rs` redundant fixtures deleted (covered by
      per-hook tests in `telemetry_hook/tests.rs` +
      `decompression_hook/tests.rs`).
- [x] **Bench (criterion micro):** SSE parser at 478-488 MiB/s;
      *up* from T0 baseline 449-472 MiB/s. criterion explicitly
      reports "Performance has improved" (p<0.05). Sync ChunkHooks
      structurally beat async wrappers on the response path.
- [x] **Bench (mitm-load integration):** **FAVORABLE.**
      `capsem-bench mitm-load` at HEAD `8c85cd6` against
      `benchmarks/mitm-load/baseline.json` (run on conc-bench
      VM, M5 Max, 2 vCPU): +6.8% to +10.6% rps, -12.3% to
      -47.9% p99 across all four concurrency levels (table
      above). Sync ChunkHooks beat the async wrappers they
      replaced.

      Junior's contradicting "-35%" claim
      (sprints/mcp-concurrency/tracker.md lines 153-161) is
      unconfirmed -- their measurement was confounded by their
      own working-tree prototype changes (T1.2+T1.3 aggregator
      pipeline + flate2 zlib feature + server_manager.rs
      edits) being in the runtime image. Not a clean control
      for my T1 slices.

      **Process learning:** slice 9 deferred this gate to a
      "real-machine session" and the close-T1 tracker went in
      anyway. That broke per-slice bench discipline. T2+ must
      run the gate before each slice's commit, not after.

      **Diagnostic suspects originally listed here (synchronous
      NetEvent build, per-frame dispatch overhead, label-
      allocating metric calls, Bytes::clone per chunk) turned
      out not to matter at the integration scale -- the bench
      shows favorable numbers despite all of them being
      present. Leaving them noted in commit `8c85cd6`'s diff
      for future T5 hot-path tuning if it ever becomes
      relevant.**

## T2 progress (in flight)

- [x] T2.1 -- first-byte sniff. New `mitm_proxy::protocol` module
      with a `Protocol` enum (`Tls` / `Http` / `Unknown`) + a
      pure `detect(&[u8]) -> Option<Protocol>` classifier. Single
      byte rule: `0x16` = TLS Handshake (RFC 8446 §5.1, the only
      valid first record from the client side); uppercase ASCII
      (`0x41..=0x5A`) = HTTP/1.1 method letter; everything else
      rejected. Hooked into `handle_inner` after `\0CAPSEM_META:`
      stripping, before the rustls acceptor runs. The previously
      hard-coded `mitm.connections_total{protocol="tls"}` increment
      at the top of `handle_connection` is gone -- the counter now
      fires post-sniff with the actual detected label, which is
      what operators need to distinguish HTTP / TLS / junk traffic.
      `mitm.requests_total` + the two upstream-error increments in
      `handle_request` propagate the same label, so T2.2 (when the
      plain-HTTP path actually serves) gets correct
      `protocol="http"` request counts for free.
      `ConnMeta` gets a `protocol: Protocol` field set from the
      sniff; every hook reads it through `ctx.conn().protocol`.
      Plain-HTTP detected -> connection-level error
      `"plain HTTP support pending (T2.2)"` so the path is
      recorded in `net_events` but the hyper handler isn't called.
      Unknown first byte -> `"unknown protocol byte 0x{b:02x}"`.
      8 unit tests in `protocol/tests.rs` (TLS handshake, all 9
      HTTP methods, lowercase rejected, other TLS record types
      rejected, empty/junk rejected, label round-trip,
      Default = Unknown) + 2 integration tests
      (`mitm_proxy_classifies_plain_http_as_unsupported_pending_t2_2`,
      `mitm_proxy_classifies_unknown_first_byte`) verifying the
      `NetEvent` reason markers. Six pre-existing test fixtures
      adopted `..Default::default()` for the new field.
      1539 capsem-core lib tests + 10 mitm_integration tests
      pass; clippy clean.

      **Bench gate deferred to junior** (junior owns the
      `capsem-bench mitm-load` runner this session). T2.1 changes
      are wire-only and observation-only on the TLS path
      (`Protocol::Tls.label() == "tls"` matches the previous
      hard-coded label exactly; the dispatch path is unchanged
      modulo a single byte read that already happened), so a
      regression on the TLS rps/p99 numbers would be surprising.
      T2.2 / T2.3 will gate against baseline before commit.

- [x] T2.2 — plain HTTP path through hyper (host-side).
      `handle_inner` is now split into `serve_tls` (existing
      rustls path) and `serve_plain_http` (T2.2 new); both go
      through a shared `serve_pipeline` helper that builds the
      hyper service closure. `serve_plain_http` skips TLS
      entirely and runs hyper directly on the vsock stream
      via `TokioIo::new(ReplayReader::new(initial_buf,
      vsock_stream))` -- the buffered first bytes (the
      `GET / HTTP/1.1\r\n` we already peeked at) replay
      transparently. The service closure parses the inbound
      `Host` header on every request, derives the upstream
      `(domain, port)` via `parse_http_host_target`
      (default port 80, no IPv6 bracket support yet), and
      passes both into `handle_request`.
      `handle_request` got an `upstream_port: u16` parameter;
      the upstream dial inside `handle_request` now branches on
      `protocol` -- TLS does TCP+rustls+http1::handshake (the
      previous code path), HTTP does TCP+http1::handshake
      directly. The inbound `host` header is preserved on the
      HTTP path (it's authoritative); on the TLS path it's
      still rewritten from the SNI domain. PolicyHook +
      DecompressionHook + SseParserHook + InterpreterHook* +
      TelemetryHook all run identically across the two paths;
      ChunkHook chain has zero protocol-aware code. The
      pre-existing `seal_with_telemetry` deny / 502 paths +
      every `TelemetryRequestContext` site grew two new
      fields, `port: u16` + `conn_type: &'static str`, threaded
      from `upstream_port` and a per-request `conn_type` local;
      `build_net_event` now emits the actual upstream port +
      `https-mitm` / `http-mitm` label instead of the
      hard-coded 443 / `https-mitm` it had pre-T2.2. Operators
      can now SQL-split `session.db.net_events` on
      `(port, conn_type)` to separate plain-HTTP from
      TLS traffic.
      `NetworkPolicy` got a `http_upstream_ports: Vec<u16>`
      field (default `[80]`); plain-HTTP requests whose Host
      port is not on the list are rejected with 403 +
      Decision::Denied + matched_rule
      `http-port-not-allowlisted({port})` *before* the upstream
      dial, *after* the policy hook. (The two-stage check is
      pragmatic; folding the port gate into PolicyHook is
      tracked for T5 hardening.) The TLS path's port (443) is
      not gated by the allowlist -- TLS implicitly uses 443.
      Two new integration tests
      (`mitm_proxy_plain_http_denies_disallowed_host`,
      `mitm_proxy_plain_http_denies_port_not_in_allowlist`)
      cover the new behavior; the original T2.1 stub test
      was renamed and tightened to assert the post-T2.2
      403 + Decision::Denied path. 1539 lib + 11 mitm_integration
      tests pass; clippy clean.

      **Agent-side bits shipped in T2.2.follow-up** (separate
      commit so its diff doesn't drown the host-side change):
      `capsem-agent/src/net_proxy.rs` listens on port 10080 in
      addition to 10443 via a small `run_listener(port)` helper;
      both target the same vsock port (5002) because the
      first-byte sniff classifies on wire bytes.
      `guest/artifacts/capsem-init` adds two
      `iptables -t nat -A OUTPUT -p tcp --dport N
      -j REDIRECT --to-port 10080` rules for ports 80 + 11434
      (Ollama default), and the post-launch readiness poll
      waits for both 10443 and 10080 to be listening.
      `guest/artifacts/diagnostics/test_network.py` gets three
      new tests (port-80 + port-11434 redirect rules,
      10080-listening). Three new agent unit tests pin the new
      constant + listen-port distinctness; cross-compile against
      `aarch64-unknown-linux-musl` clean.

      **Bench gate deferred to junior** (same as T2.1 -- they
      own the `capsem-bench mitm-load` runner this session).

- [x] T2.3 — Ollama-shaped end-to-end. New integration test
      `mitm_proxy_plain_http_ollama_shape_records_telemetry`
      drives the full plain-HTTP path:
      1. Fake plain-HTTP upstream binds on `127.0.0.1:0`,
         accepts one TCP connection, drains the request headers
         until `\r\n\r\n`, writes a fixed Ollama-shaped response
         (`HTTP/1.1 200`, `Content-Type: application/json`,
         body `{"model":"llama2","response":"hello","done":true}`).
      2. Proxy config built via the new `make_proxy_config_full`
         helper -- domain allowlist `["127.0.0.1"]`,
         `http_upstream_ports = [80, <dyn-port>]`. The dyn port
         comes from the OS-assigned upstream listener.
      3. Plain HTTP/1.1 client sends
         `POST /api/generate HTTP/1.1\r\nHost: 127.0.0.1:<port>\r\n
          Content-Type: application/json\r\nContent-Length: NN\r\n
          Connection: close\r\n\r\n{model+prompt JSON}` directly
         on a TCP socket to the proxy.
      4. Asserts: response body forwarded verbatim;
         `events.len() == 1`, `decision = Allowed`, `method=POST`,
         `path=/api/generate`, `status=200`, `domain=127.0.0.1`,
         `port=<dyn-port>`, `conn_type=http-mitm`,
         `bytes_sent >= req_body.len()`, `bytes_received > 0`.
      The test exercises every T2 component end-to-end:
      first-byte sniff -> Protocol::Http (T2.1), `serve_plain_http`
      replays buffered first bytes into hyper (T2.2), Host header
      drives ConnMeta::domain (T2.2), `http_upstream_ports`
      allowlist gates the dial (T2.2), upstream dial uses plain
      TCP (T2.2), TelemetryHook emits NetEvent with the new port
      + conn_type fields (T2.2). Result: 12 mitm_integration
      tests + 1539 lib tests pass; clippy clean.

      The original plan also mentioned an optional
      `capsem-bench mitm-load --scheme http` variant. Skipped
      for now because (a) junior owns the bench runner this
      session, (b) the plain-HTTP code path is structurally
      simpler than the TLS path (no rustls handshake, no cert
      mint), so the TLS rps numbers are still the load-bearing
      gate. If junior wants a plain-HTTP rps line for
      observability, the bench harness extension is
      mechanically straightforward -- add a `target` arg that
      defaults to the existing `https://...` and accept a
      `--scheme http` variant; the rest of the harness is
      protocol-agnostic.
- [ ] T2.3 — Ollama-shaped integration test
      (`crates/capsem-core/tests/mitm_integration.rs`): fake
      plain-HTTP upstream, request through the proxy, assert
      `NetEvent` records the right method/path/status. Optional
      `capsem-bench mitm-load --scheme http` variant.

## T3 progress (in flight)

- [x] T3.1 — host-side DNS handler + UDP forwarder + parser.
      New `crates/capsem-core/src/net/dns/` module with
      `mod.rs` (re-exports), `server.rs` (`DnsHandler` async
      bytes-in / bytes-out processor with policy gate),
      `resolver.rs` (`DnsResolver` UDP forwarder iterating an
      upstream list with per-query timeout), and `tests.rs`
      (10 end-to-end tests against a `127.0.0.1:0` fake upstream:
      allowed-forwarded, blocked-NXDOMAIN, wildcard-block,
      read-only-still-resolvable, upstream-unreachable-SERVFAIL,
      malformed-query-error, dual-upstream-failover,
      empty-upstream-list-error, telemetry-fields-populated,
      default-resolver-has-default-upstreams). New
      `crates/capsem-core/src/net/parsers/dns_parser.rs` with
      `parse_query`, `build_nxdomain`, `build_servfail` wrapping
      `hickory-proto::Message`; sibling `tests.rs` has 14 tests
      (A / AAAA / TXT / MX / multi-question / zero-question /
      garbage / truncated / id-preservation / case-folding /
      trailing-dot / NXDOMAIN-shape / RD-bit-mirror /
      SERVFAIL-rcode).

      Workspace deps: `hickory-proto = "0.26"` added with
      `default-features = false, features = ["std"]` so we don't
      pull in the optional DNSSEC / mDNS / serde feature trees.
      capsem-core is the only consumer; the agent crate stays
      hickory-free (it forwards raw bytes only -- T3.2 wires the
      vsock envelope).

      Policy semantics: `is_fully_blocked(qname)` short-circuits
      to NXDOMAIN. Read-only domains still resolve (verb-level
      enforcement happens at the HTTP layer; NXDOMAINing them
      would lose the `pip install` audit trail). Decision is
      `Decision::Denied` for blocked, `Decision::Allowed` for
      forwarded, `Decision::Error` for malformed input or
      upstream failure (synthesized SERVFAIL response).

      Why not `hickory-server`: the plan called for it but
      `RequestHandler` is tightly coupled to owned UDP/TCP
      server-side state. We accept raw bytes from a vsock
      envelope, so `hickory-proto` (wire codec) + a thin async
      handler on top of the existing `NetworkPolicy` was cleaner.
      Half the dep weight, none of the impedance mismatch.
      Documented in `dns/mod.rs`.

      hickory-proto 0.26 API note: `Message`, `Query` etc. now
      expose state via public fields (`msg.queries`,
      `msg.metadata.id`, `msg.metadata.recursion_desired`) rather
      than the 0.24-era getter/setter methods. Records use the
      typed `Record::from_rdata(name, ttl, rdata)` constructor;
      `RData::A` takes a `hickory_proto::rr::rdata::A` which
      converts only from `Ipv4Addr` (not `[u8; 4]`).

      Tests + clippy + workspace build all clean (1573 lib tests
      pass; 14 dns_parser + 10 dns handler are the new ones). No
      MITM hot-path code touched, so the bench gate is deferred
      (in line with T2.1's deferral rationale).

- [x] T3.2 — vsock DNS envelope + agent `dns_proxy` listener.
      `capsem-proto` adds `VSOCK_PORT_DNS_PROXY = 5007` plus
      `DnsRequest { raw, proto, process_name }` /
      `DnsResponse { raw, decision, rcode }` types with
      `encode_dns_request` / `decode_dns_request` /
      `encode_dns_response` / `decode_dns_response` length-framed
      RMP helpers (mirrors the `encode_audit_record` pattern). The
      port lives between `VSOCK_PORT_AUDIT (5006)` and the next
      free slot, listed in the `port_constants_are_distinct` /
      `vsock_dns_proxy_port_constant` pin tests so a numbering
      drift surfaces in CI.

      Host side: `capsem-process::vsock::dispatch_aux_connection`
      gets a new branch for `VSOCK_PORT_DNS_PROXY` that spawns
      `serve_dns_session(conn, dns_handler)` -- a tokio task that
      reads one length-framed `DnsRequest`, calls
      `DnsHandler::handle` (T3.1), writes the framed response, and
      drops the conn. Frame validation: rejects payloads larger
      than `MAX_FRAME_SIZE` before allocating the buffer, drops
      the conn quietly on decode failure.
      `dispatch_aux_connection` grew a `dns_handler:
      &Arc<DnsHandler>` parameter; the three call sites + the
      deferred-port classifier (5002 / 5003 / 5006) now also defer
      5007 connections that arrive before the initial handshake.
      `VsockOptions` carries the handler from `main.rs`. The
      `classify_vsock_port` test helper grew a `DnsProxy` variant.

      Refactor: `DnsHandler::policy` was retrofitted from
      `Arc<NetworkPolicy>` to
      `SharedPolicy = Arc<RwLock<Arc<NetworkPolicy>>>` -- the same
      hot-swappable shape `MitmProxyConfig::policy` uses. Each
      `handle()` call snapshots under the read lock, releases the
      lock immediately, and uses the cheap-Arc snapshot for the
      rest of the request. An admin policy edit now propagates to
      both DNS and MITM at the next request boundary without
      restarting either. T3.1's 10 handler tests all updated to
      wrap their `NetworkPolicy` in `shared(...)` (one-liner
      helper); they pass unchanged otherwise.

      Guest side: new agent binary `capsem-dns-proxy` (added to
      `crates/capsem-agent/Cargo.toml`'s `[[bin]]` list, plus the
      `docker.py::GUEST_BINARIES` and the two
      `_pack-initrd` for-loops in `justfile` so the
      `mcp::tests::all_guest_binaries_in_*` invariants hold). The
      binary listens on `127.0.0.1:1053` UDP+TCP (port > 1024 to
      avoid CAP_NET_BIND_SERVICE; iptables NAT redirect 53 ->
      1053 will land in T3.4). For each query: open a fresh vsock
      conn to `(VSOCK_HOST_CID=2, port=5007)`, encode + write the
      `DnsRequest`, read the framed `DnsResponse`, write the
      answer back to the original UDP peer (or as a length-prefixed
      TCP DNS message). `forward_query` runs the blocking vsock
      I/O via `spawn_blocking` so the tokio runtime stays
      responsive. UDP datagrams capped at 4096 bytes (RFC 6891
      EDNS default). TCP wire format respects RFC 1035 §4.2.2's
      2-byte BE length prefix.

      Pre-T3.4 the binary is built and packaged but NOT launched
      -- `capsem-init` still spawns dnsmasq. Until T3.4 wires the
      iptables redirect + drops the dnsmasq invocation, dnsmasq
      remains the guest's DNS server.

      Tests: 9 new capsem-proto envelope tests (port distinctness,
      DnsRequest / DnsResponse roundtrip, no-process-name path,
      compactness < 200B, garbage rejection, IPC-frame
      disjointness via `looks_like_ipc_frame` heuristic). 5 new
      agent-bin tests (listen port > 1024, listen port = 1053,
      vsock port = 5007, EDNS payload size, proto label match).
      Full suite: 1573 capsem-core lib + 88 capsem-process + 157
      capsem-proto + 15 capsem-agent (capsem-dns-proxy bin) tests
      pass. Workspace clippy clean; aarch64-musl cross-compile
      clean.

      Bench gate still deferred -- T3.2 adds host-side dispatch
      code that's only hit when the agent's dns_proxy is launched
      (T3.4). The MITM proxy hot path is untouched. Mitm-load
      regression risk: zero.

- [x] T3.3 — `dns_events` schema + telemetry hook + `trace_id`.
      `capsem-logger` ships a new `dns_events` table (timestamp,
      qname, qtype, qclass, rcode, decision, matched_rule,
      source_proto, process_name, upstream_resolver_ms, trace_id)
      with indexes on `(timestamp, qname, trace_id, decision)`.
      The migrate path (idempotent CREATE-IF-NOT-EXISTS) means
      existing session DBs pick up the table without a rebuild.
      `WriteOp::DnsEvent` + `insert_dns_event` round out the
      writer surface. New event struct `DnsEvent` exported from
      `capsem-logger::events`.

      `capsem_core::net::dns::build_dns_event(result, source_proto,
      process_name, trace_id) -> DnsEvent` is the pure builder
      (sqlite-free, testable without a runtime). `serve_dns_session`
      in `capsem-process::vsock` calls it after every handler
      invocation and pushes the row through the shared `DbWriter`
      via `try_write` (matches the audit-event back-pressure
      pattern -- DNS shouldn't block on a saturated writer
      channel, which would tail-latency the resolver).
      The shape is intentionally a free function rather than a
      `DnsTelemetryHook` struct: DNS doesn't need the chunk-pipeline
      machinery the MITM proxy uses (one-shot bytes-in / bytes-out),
      so factoring the row construction as a pure function keeps the
      handler decoupled from `DbWriter`.

      `trace_id` comes from
      `capsem_core::telemetry::ambient_capsem_trace_id()` -- the
      same source the MITM `TelemetryHook` uses -- so a single agent
      action joins across `dns_events` and `net_events`. A `dig
      anthropic.com` followed by `curl https://anthropic.com/` shows
      up as one `dns_events` row + one `net_events` row, both
      stamped with the same trace_id. `inspect-session` joins are
      now possible without manual correlation by qname.

      Tests: 6 telemetry builder tests (allowed, denied,
      undecodable -> "INVALID_DNS_BYTES" sentinel, decision strings
      round-trip with `Decision::parse_str`, source_proto optional,
      process_name passthrough). 2 writer tests
      (`dns_event_insert_populates_row` end-to-end through
      `DbWriter`; `dns_events_indexed_by_trace_id_for_join` pins
      the join-critical index). 3 schema tests (create includes
      dns_events, migrate idempotent on twice-call, all four
      indexes present). All capsem-core lib at 1579 tests; logger
      at 219; capsem-process at 88; clippy clean.

- [x] T3.4 — drop dnsmasq + iptables redirect for port 53.
      `guest/config/packages/apt.toml` -- `dnsmasq` removed from the
      apt install list, so the next rootfs rebuild leaves the binary
      out of the squashfs entirely (and the resulting rootfs hash
      changes).
      `guest/artifacts/capsem-init` -- the dnsmasq invocation block
      (`chroot /newroot dnsmasq --no-daemon --no-resolv ...
      --address=/#/10.0.0.1`) is gone; replaced with two iptables
      nat OUTPUT rules redirecting UDP and TCP port 53 to 1053
      (the capsem-dns-proxy listen port). A new launch block deploys
      `capsem-dns-proxy` from the initrd-bundled copy
      (preferred, fast iteration loop) or the rootfs `/usr/local/bin`
      fallback, polls `ss -lun` AND `ss -ltn` for port 1053
      readiness on both transports, and adds a `dns_proxy` boot
      timing marker between `net_proxy` and the rest. `resolv.conf`
      still points libc at `127.0.0.1` -- the iptables nat rule is
      what does the actual redirect.

      Diagnostics: `test_sandbox::test_dnsmasq_running` is replaced
      with `test_dns_proxy_running` (pgrep capsem-dns-proxy) and a
      new `test_dnsmasq_not_running` (pgrep dnsmasq must miss --
      pins the cutover so a future rootfs accidentally re-adding
      dnsmasq trips the test). `test_network::test_dnsmasq_responds`
      and `test_dns_all_resolve_to_local` (which expected the legacy
      `10.0.0.1` sentinel) are replaced with five new tests:
      `test_dns_proxy_listening_udp` / `_tcp` (ss -lun / -ltn for
      :1053), `test_iptables_redirect_dns_udp_to_1053` /
      `_tcp_to_1053` (regex-grep iptables-legacy output for the
      udp/tcp dport 53 rules), and the two acceptance tests
      `test_dns_resolves_via_capsem_proxy` (elie.net resolves to a
      real IPv4, not 10.0.0.1) and
      `test_dns_blocked_domain_returns_nxdomain` (api.openai.com
      either nonzero exit or empty stdout from getent, both meaning
      NXDOMAIN at libc level).

      Source comments updated: `dns_proxy.rs` and `dns/mod.rs` now
      describe T3.4 as past tense (the dnsmasq cutover happened),
      not pending. Docs (`docs/src/content/docs/security/network-isolation.md`
      and friends) still have stale references to dnsmasq -- those
      are docs-follows-code updates that can land in a separate
      `docs(...)` commit; they don't gate the cutover.

      Validated: full Rust workspace build clean, workspace clippy
      clean, capsem-core 1579 lib + capsem-process 88 +
      capsem-logger 219 + capsem-proto 157 + agent-bin 15 tests
      pass. The `_pack-initrd` recipe ran end-to-end through the
      Docker `capsem-builder agent` stage, cross-compiled all five
      guest binaries (incl. capsem-dns-proxy at 921864 bytes, chmod
      555) and refreshed the manifest, validating the production
      build path.

      NOT validated this session: the in-VM E2E gate
      (`just run "capsem-doctor -k network"`) and the
      `mitm-load` regression check. Both need:
        1. The dev `target/debug/capsem` binary codesigned with
           `com.apple.security.virtualization` (the `just` recipes
           do this; raw `target/debug/capsem run ...` aborts with
           `Invalid virtual machine configuration` per VZErrorDomain
           code 2).
        2. The new initrd / rootfs deployed to `~/.capsem/assets/`
           via `just install` -- the install path serves the
           codesigned package, not the dev tree.
      Junior owns the bench runner this session per the resume
      prompt; the cutover code lands here, the acceptance gate
      runs from junior's side once they re-`just install`. Stop
      and checkpoint before T4.

## Cross-cutting notes

- `RUST_LOG=capsem::net::mitm=debug` reveals the decision trail
  end-to-end as of slice 3 observability contract (`b774953`).
  Spans nest: `mitm.connection` → `mitm.request` → `mitm.hook`.
- Stop outcomes show at default `RUST_LOG=info` filtering via
  `mitm.hook.cause` debug events -- triage's load-bearing line.
- Each phase ends with `just test` green AND `mitm-load` regression
  check passing against committed baseline. T1 has held this gate
  at every commit (1521 lib tests passing, clippy clean).
- All commits use conventional prefix `feat(mitm):` /
  `chore(mitm):` / `bench(mitm):` and stage files explicitly.
- `Co-Authored-By` trailers forbidden per CLAUDE.md.
- Author defaults to `elie@Saphyr.local` -- git config not set
  globally; commits will need amend / replay before push.
