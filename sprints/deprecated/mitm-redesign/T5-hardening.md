# T5: hardening-perf-coverage

**Status:** Not Started
**Depends on:** T2, T3, T4
**Blocks:** none (final phase)

## Goal

Ship the new MITM with adversarial coverage, fuzzing, performance regression gates, and the named hot-path fixes from the plan's § Performance contract proven by bench numbers. No phase ships before T5; T5 is the gate that lets the meta-sprint close.

## Deliverables

### Adversarial test suite

- Malformed SNI (truncated, oversized, invalid UTF-8 in name).
- Oversized request bodies (memory bound enforcement).
- Slowloris (per-connection deadline; idle reaper).
- Header smuggling (CL/TE conflict, duplicate Host, ambiguous folded headers).
- Host/SNI mismatch (TLS SNI says X, HTTP Host says Y).
- DNS poisoning attempts (response with wrong qname; truncated answers).
- JSON-RPC adversarial: missing `jsonrpc` field, integer overflow ids, batch-of-1000.
- SSE adversarial: 10MB single event (must emit `Stop(Reject 413)`); event with no terminating blank line.

### cargo-fuzz harnesses

- `fuzz/fuzz_targets/sse_parser.rs`
- `fuzz/fuzz_targets/jsonrpc_parser.rs`
- `fuzz/fuzz_targets/dns_parser.rs`
- CI runs each for 60s per merge.

### Coverage gate

- `cargo tarpaulin -p capsem-core` shows ≥80% line on `net/mitm/`, ≥90% on `net/parsers/` and `net/interpreters/`.
- CI job fails below the floor.

### Performance regression CI

- `cargo bench -p capsem-core` + `critcmp` against `crates/capsem-core/benches/baselines/` — any bench >5% slower fails CI.
- `capsem-bench mitm-load --concurrency 1,10,50,200` against `benchmarks/mitm-load/baseline.json` — any p99 >2x baseline fails CI.
- `tokio-metrics` runtime monitor reports `busy_ratio < 0.7` at 200 concurrent connections in CI smoke (gauge: `mitm.runtime_busy_ratio`).

### Performance correctness tests

- `test_no_blocking_on_async`: custom waker fails if any single hook poll exceeds 1ms wall time.
- `test_lock_contention`: 200 concurrent connections to the same fresh domain — TLS handshake p99 < 50ms (proves cert-cache fix).
- `test_telemetry_backpressure`: stall the logger writer, drive 1000 requests — connection task latency unaffected; `mitm.telemetry_dropped_total` increments; `warn!` fires.
- `test_memory_bound`: 1GB SSE response — host RSS growth bounded.

### Hot-path fixes (delivered + proven)

Each named fix from the plan's § Performance contract lands with a bench number in CHANGELOG:
- Pre-TLS metadata read (`mitm_proxy.rs:151-186` original) — handshake setup serialization removed.
- Cert cache `OnceCell`/`moka` — concurrent first-time mint waiters share one operation.
- Upstream connection pool — pipelined requests no longer serialize on a per-connection mutex.
- `TrackedBody` atomic counters — lock churn removed from poll path.
- Telemetry off connection task — emit_model_call no longer blocks response cleanup.
- Bounded SSE accumulator — `EventTooLarge` → `Stop(Reject 413)`.
- `tokio-metrics` runtime gauges wired (under `tokio-unstable` feature flag).
- Logger writer used for MITM telemetry (synchronous SQLite writes removed from connection path).

## Acceptance

- Every test in this phase passes.
- CI gates active and exercised on a deliberately-regressing branch (proof they actually fail when they should).
- CHANGELOG shows before/after bench numbers for each hot-path fix.
- Coverage report committed.
- `just test` passes end-to-end.

## Commit shape

Five+ expected commits:
1. `test(mitm): adversarial suite (SNI/body/slowloris/smuggling/DNS/JSON-RPC/SSE)`.
2. `test(mitm): cargo-fuzz harnesses for sse/jsonrpc/dns parsers`.
3. `ci(mitm): coverage gate + critcmp regression gate + mitm-load p99 gate`.
4. `perf(mitm): hot-path fixes — cert cache, conn pool, atomic body stats, off-task telemetry`.
5. `test(mitm): no-blocking-on-async + telemetry backpressure + memory-bound`.

## Notes

- Hot-path fixes are delivered IN T5 (not earlier) so the bench numbers in CHANGELOG show clean before/after deltas relative to the T0 baseline. Tempting to fix them as we encounter them in T1-T4; resist.
- `tokio-metrics` is behind `tokio-unstable` cfg; release build must compile both with and without it.
- The CI gates are load-bearing for sustainability — without them, every later sprint will silently re-introduce the issues we just removed.

## T2 follow-up items (deferred here intentionally)

These were noticed during T2 but are correctness/UX polish, not phase-blocking. Each has the load-bearing test in place already so the regression net survives until the fix lands.

### `http_upstream_ports` is hardcoded ([80, 11434])

**Where:** `crates/capsem-core/src/net/policy.rs::DEFAULT_HTTP_UPSTREAM_PORTS`. The value lives as a `&'static [u16]` baked into `NetworkPolicy::new`, with no path through `policy_config` to override it from `user.toml` / `corp.toml`.

**Why it's hardcoded today:**
1. The plan called for "Configurable upstream-port allowlist (default 80)" but the host-side gate was the load-bearing piece for the test gate. The plumbing through `policy_config` (loader entry, settings_meta, builder wiring, env-var fallback, corp-override merge) is ~50 lines spread across `policy_config/loader.rs` + `policy_config/builder.rs` + `policy_config/settings_meta.rs` and would have drowned the T2.2 commit's signal-to-noise.
2. The list also has to mirror `guest/artifacts/capsem-init`'s iptables redirects exactly — a pure host-side config plumb without the matching guest-side knob is half a feature. Doing both at once is a bigger ticket.
3. Default `[80, 11434]` covers the canonical "Ollama from inside the VM" workflow T2 was designed to enable, so the hardcoded default is functionally complete for the immediate use case.

**What lands in T5 (or earlier follow-up):**
- New setting `security.web.http_upstream_ports` (comma-separated u16 list). Loader entry + corp-override + env-var fallback (`CAPSEM_WEB_HTTP_UPSTREAM_PORTS`).
- Builder reads the resolved value into `NetworkPolicy.http_upstream_ports`, with the existing `DEFAULT_HTTP_UPSTREAM_PORTS` as the fallback when unset.
- Guest-side iptables list driven from the same config (probably via an env var or `--http-redirect-ports` flag passed to capsem-init by capsem-process at boot, since the guest doesn't have direct read access to user.toml).
- Test: a settings-overridden allowlist is reflected in `NetworkPolicy.http_upstream_ports` AND in the iptables rules visible inside the guest.

### Corrupted-gzip path produces empty client response

**Where:** `crates/capsem-core/src/net/mitm_proxy/mod.rs::handle_request` — the gzip-classified path strips `Content-Encoding` + `Content-Length` from the response head, then hands the body to `ChunkDispatchBody`. When every emitted chunk decodes to empty bytes (which is exactly what `flate2::Decompress` produces on a fully-corrupt deflate body), hyper's chunked encoder appears to buffer the response head waiting for the first non-empty body chunk that never arrives, so the client sees zero bytes.

**Locked down by:** `mitm_proxy_plain_http_corrupted_gzip_response_doesnt_crash`. The current contract the test gates on is "no panic, exactly one NetEvent, `bytes_received == 0`" -- it explicitly does NOT gate on the client receiving the response head, because that's the buggy behavior we're documenting here.

**Why it's not a panic / data corruption:**
- Telemetry still fires correctly (NetEvent has `status=200, bytes_received=0`, the response was logged).
- The connection eventually closes (FIN propagates from upstream-close → proxy-close), so the client's `read_to_end` returns rather than hanging forever (test has a 5s deadline that never trips).
- No memory unsafety.

**What lands in T5 (or earlier follow-up):**
- Either explicitly flush the response head into the writer before the first `poll_frame` on `ChunkDispatchBody`, OR avoid emitting `Bytes::new()` from `DecompressionHook` and instead pass through the raw input on decode failure (with a `mitm.decompression_errors_total` counter increment). Probably option B since "do not silently swap a 200-OK upstream response for an empty client response" is the more user-visible invariant.
- Test should then tighten to assert the client receives `HTTP/1.1 200` plus the chunked terminator, not just "no crash + NetEvent".
