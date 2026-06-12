---
name: dev-mitm-proxy
description: MITM/network intercept development for Capsem -- the air-gapped network interception layer. Use when working on TLS termination, HTTP inspection, cert minting, SSE parsing, telemetry recording, or debugging network issues. Covers the full proxy pipeline, content-encoding handling, and lessons learned from past bugs.
---

# MITM Proxy

The MITM proxy is the network engine's HTTPS interception boundary. It
intercepts traffic from the air-gapped guest VM, normalizes it into typed facts,
hands a `SecurityEvent` to the security engine, and preserves allowed runtime
bytes for upstream. Treat it as a system, not a collection of hacks -- every
capability must be general-purpose.

## Security boundary

Network code parses transport bytes, routes traffic, and emits typed
`SecurityEvent` facts. It must not broker credentials, create credential refs,
run CEL/security decisions, or sanitize ledger projections. Those belong to the
security engine plugin rail. Every security plugin has the same data contract:
it receives a `SecurityEvent` and returns a `SecurityEvent`; the plugin stage
only controls ordering (`preprocess`, `postprocess`, or `logging`).

There are two materialization paths and they must never be collapsed:

- **Runtime materialization** prepares bytes for the real upstream. It may
  resolve a broker ref back to a real credential because the protocol needs it.
- **Ledger materialization** prepares the event stored in `session.db`,
  structured logs, route JSON, and UI stats. It must contain only broker refs,
  hashes, bounded previews, typed detections, and plugin execution evidence.

No credential logic belongs in HTTP header formatters, DB readers, frontend
transforms, debug harnesses, or route adapters. If a future change needs
capture, injection, redaction, threat intel, or PII handling, implement it as a
security plugin stage over `SecurityEvent -> SecurityEvent`.

## Pipeline

```
Guest curl -> iptables REDIRECT -> capsem-net-proxy (guest, port 10443)
  -> vsock port 5002 -> Host MITM proxy
  -> SNI parse -> network metadata capture
  -> TLS terminate (rustls, per-domain cert minted from Capsem CA)
  -> HTTP request parse (hyper)
  -> SecurityEvent -> SecurityRuleSet + plugin rail
  -> Forward to real upstream over TLS
  -> Record telemetry to session DB
  -> Stream response back to guest
```

## Key source files

| File | What |
|------|------|
| `crates/capsem-core/src/net/mitm_proxy.rs` | Async MITM proxy (rustls + hyper): TLS termination, HTTP inspection, upstream bridging |
| `crates/capsem-core/src/net/cert_authority.rs` | CA loader + on-demand domain cert minting with RwLock cache |
| `crates/capsem-core/src/security_engine/` | Rule/plugin/decision rail over `SecurityEvent` |
| `crates/capsem-core/src/net/sni.rs` | SNI parser for TLS ClientHello |
| `crates/capsem-agent/src/net_proxy.rs` | Guest-side TCP-to-vsock relay |

## Content-Encoding: the systemic rule

The proxy MUST handle response decompression as a general capability. This is not optional, not per-feature.

1. Normalize `Accept-Encoding` in outgoing requests to only allow encodings we can decompress (gzip at minimum)
2. Transparently decompress response bodies before any parsing (SSE, body preview, telemetry)
3. Never strip encoding headers as a workaround -- that breaks upstream behavior

**Why this matters**: Failing to handle gzip on Anthropic SSE responses caused all model/token/cost metadata to be NULL. The SSE parser received compressed garbage. This went undetected because Google's API happened to not compress SSE in testing. The fix was general-purpose decompression, not an Anthropic-specific hack.

## Serde optimization for ai_traffic parsers

The ai_traffic parsers (`openai.rs`, `google.rs`, `request_parser.rs`) deserialize LLM request/response bodies that can be megabytes. Never use `serde_json::Value` for struct fields that hold large unconstrained JSON (tool call args, function responses, model outputs). Use `Box<serde_json::value::RawValue>` for fields that are only stringified, and remove unused fields entirely. See `/dev-rust-patterns` for the full pattern and examples.

## SSE parsing

AI provider APIs (Anthropic, OpenAI, Google) use Server-Sent Events for streaming responses. The proxy parses SSE to extract model names, token counts, and cost data for telemetry.

SSE parsing happens AFTER decompression. The body must be plaintext UTF-8 by the time the SSE parser sees it.

## model_calls filtering

Only emit `model_calls` telemetry for actual LLM API paths (e.g., `/v1/messages`, `/v1/chat/completions`), not every request to an AI provider domain. Health checks, auth endpoints, and static assets should not create model_call rows.

## Security evaluation order

1. Network mechanics parse and normalize SNI, HTTP, DNS, model, and process
   facts into a `SecurityEvent`.
2. Profile and corp rules compile into one `SecurityRuleSet`; profile defaults
   are normal late-priority rules.
3. Security plugins run by stage over the same `SecurityEvent` object:
   `preprocess`, rule evaluation, `postprocess`, then `logging` before ledger
   handoff.
4. Runtime materialization forwards allowed bytes upstream. Ledger
   materialization writes the sanitized/enriched event to `session.db`, logs,
   routes, and UI stats.

## Certificate authority

- Static CA keypair: `config/capsem-ca.key` + `config/capsem-ca.crt` (ECDSA P-256)
- Certs minted on-demand per domain, cached in `RwLock<HashMap>`
- CA baked into guest rootfs via `update-ca-certificates` + certifi patch + env vars
- No security value from the CA itself -- the guest is already fully sandboxed

## Provider wire format references

Read these for the exact SSE format, request/response shapes, and telemetry extraction points:
- `references/anthropic-wire.md` -- Anthropic Messages API (event-typed SSE, gzip gotcha)
- `references/openai-wire.md` -- OpenAI Chat Completions + Responses API (data-only SSE, [DONE] sentinel)
- `references/google-wire.md` -- Google Gemini (complete JSON per event, no tool call IDs, camelCase)

## Testing the proxy

- Unit tests: `cargo test -p capsem-core net` (policy evaluation, SNI parsing, cert minting)
- In-VM: `just run "capsem-doctor -k network"` (TLS trust chain, port blocking, domain filtering)
- Telemetry: `just run "curl -s https://api.anthropic.com/"` then `just inspect-session` (check net_events)
- Adversarial: test with blocked domains, overlapping wildcards, malformed SNI, huge request bodies
