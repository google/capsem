---
name: dev-mitm-proxy
description: MITM proxy development for Capsem -- the air-gapped network interception layer. Use when working on TLS termination, HTTP inspection, SecurityEvent/CEL enforcement, cert minting, SSE parsing, telemetry recording, or debugging network issues. Covers the full proxy pipeline, content-encoding handling, and lessons learned from past bugs.
---

# MITM Proxy

The MITM proxy is the most complex subsystem in Capsem. It intercepts all HTTPS traffic from the air-gapped guest VM, inspects it, applies policy, and records telemetry. Treat it as a system, not a collection of hacks -- every capability must be general-purpose.

## Pipeline

```
Guest curl -> iptables REDIRECT -> capsem-net-proxy (guest, port 10443)
  -> vsock port 5002 -> Host MITM proxy
  -> SNI parse -> network mechanics snapshot
  -> TLS terminate (rustls, per-domain cert minted from Capsem CA)
  -> HTTP request parse (hyper)
  -> build typed SecurityEvent (http/model roots)
  -> SecurityRuleSet/CEL evaluation
  -> configured plugin stages
  -> Forward to real upstream over TLS
  -> Record telemetry to session DB
  -> Stream response back to guest
```

## Key source files

| File | What |
|------|------|
| `crates/capsem-core/src/net/mitm_proxy/` | Async MITM proxy (rustls + hyper): TLS termination, HTTP inspection, upstream bridging, telemetry hooks |
| `crates/capsem-core/src/net/cert_authority.rs` | CA loader + on-demand domain cert minting with RwLock cache |
| `crates/capsem-core/src/net/policy.rs` | Network mechanics: ports, capture, decompression, routing, cache settings |
| `crates/capsem-core/src/net/sni.rs` | SNI parser for TLS ClientHello |
| `crates/capsem-core/src/net/policy_config/` | profile/corp parsing into network mechanics and `SecurityRuleSet` |
| `crates/capsem-core/src/security_engine/` | `SecurityEvent`, `SecurityRuleSet`/CEL evaluation, plugins, endpoint DTOs |
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

## Enforcement evaluation order

1. Profile/corp config materializes network mechanics and a `SecurityRuleSet`.
2. The network engine parses and normalizes HTTP/model evidence into one typed
   `SecurityEvent`.
3. `SecurityRuleSet` evaluates CEL once over that event. Default behavior is
   expressed as normal late-priority profile rules.
4. A block decision is absolute once effective. Ask and allow decisions remain
   auditable ledger rows.
5. Plugins run by typed stage from their descriptors; CEL rules do not call
   plugins and plugin-private fields do not become public rule roots.

## Certificate authority

- Static CA keypair: `security/keys/capsem-ca.key` + `security/keys/capsem-ca.crt` (ECDSA P-256)
- Certs minted on-demand per domain, cached in `RwLock<HashMap>`
- CA baked into guest rootfs via `update-ca-certificates` + certifi patch + env vars
- No security value from the CA itself -- the guest is already fully sandboxed

## Provider wire format references

Read these for the exact SSE format, request/response shapes, and telemetry extraction points:
- `references/anthropic-wire.md` -- Anthropic Messages API (event-typed SSE, gzip gotcha)
- `references/openai-wire.md` -- OpenAI Chat Completions + Responses API (data-only SSE, [DONE] sentinel)
- `references/google-wire.md` -- Google Gemini (complete JSON per event, no tool call IDs, camelCase)

## Testing the proxy

- Unit tests: `cargo test -p capsem-core net` (SecurityEvent evaluation, SNI parsing, cert minting)
- In-VM: `just run "capsem-doctor -k network"` (TLS trust chain, port blocking, domain filtering)
- Telemetry: `just run "curl -s https://api.anthropic.com/"` then `just inspect-session` (check net_events)
- Adversarial: test with blocked domains, overlapping wildcards, malformed SNI, huge request bodies
