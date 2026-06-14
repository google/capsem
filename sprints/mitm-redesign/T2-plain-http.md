# T2: protocol-demux-plain-http

**Status:** Not Started
**Depends on:** T1
**Blocks:** T5

## Goal

Add plain-HTTP support to the MITM. Peek-based TLS vs. HTTP demux at the listener. iptables redirects port 80 + a configurable allowlist (e.g., 11434 for Ollama). Host-header policy on plain HTTP. Brotli + zstd decompression added to the response body hook. End-to-end Ollama smoke from a guest VM.

## Deliverables

- `crates/capsem-core/src/net/mitm/protocol.rs` — `ProtocolDetector` peeks first N bytes, routes to TLS path or HTTP path. DNS detection deferred to T3.
- `crates/capsem-core/src/net/mitm/http.rs` — plain HTTP parse via hyper without TLS termination.
- `crates/capsem-agent/src/net_proxy.rs` — listen on additional ports (default 80; configurable list in `policy_config`).
- `guest/artifacts/capsem-init` — iptables rules for port 80 and the configured allowlist.
- `crates/capsem-core/src/net/policy/` — host-header → domain extraction for plain HTTP; same policy engine.
- `crates/capsem-core/src/net/mitm/body.rs::DecompressionHook` — gzip + brotli + zstd via `async-compression`.

## Acceptance

- A guest can `curl http://host.capsem.internal:11434/api/generate` (Ollama-style) through the proxy. Request is policy-checked, logged in `net_events` with `trace_id`, and counted in `mitm.requests_total{protocol="http"}`.
- A guest can hit a brotli-compressed and a zstd-compressed upstream response; body is decompressed transparently.
- `mitm.requests_total{protocol="tls"}` continues to fire for HTTPS traffic — no regression.
- New unit tests for the protocol detector edge cases: short input, ambiguous bytes, non-HTTP/non-TLS junk.
- New chunk-boundary fuzz test against the HTTP parser.
- `mitm-load` baseline regression check passes.

## Commit shape

Three expected commits:
1. `feat(mitm): peek-based TLS vs HTTP protocol demux` — `protocol.rs` + dispatch.
2. `feat(mitm): plain HTTP path + Host-header policy + agent multi-port listener` — full plain-HTTP pipeline + iptables rules + policy.
3. `feat(mitm): brotli + zstd decompression hook` — DecompressionHook expansion + tests.
