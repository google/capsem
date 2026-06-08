# Local Test Harness Slice

## Why

Release proof cannot depend on public MCP servers, AI providers, GitHub, or any
other remote service. The next-generation testing rail starts with small local
external services that record exactly what Capsem sends while keeping the
Capsem path itself real.

The discipline is:

- Mock only the outside world.
- Do not mock the security engine, credential broker, MCP manager, rule
  compiler, or runtime dispatch path.
- Keep local fixtures reusable for E2E, benchmarks, and debugging.
- Replace internet-backed tests with local adversarial proofs instead of
  demoting them to skipped folklore.

## Scope

- Add a reusable local HTTP recorder for request/header/body capture.
- Add reusable static HTTP fixture responses so builtin HTTP tools can fetch,
  grep, paginate, and inspect headers without remote services.
- Extend `capsem-debug-upstream` with deterministic text, HTML, large HTML,
  bytes, gzip, SSE, credential-shaped, deny-target, and WebSocket fixtures.
- Add a reusable local Streamable HTTP MCP server with a real rmcp tool.
- Replace remote MCP manager tests with local proofs.
- Replace builtin HTTP fetch/grep/header tests with local fixture proofs.
- Make `capsem doctor` start a host-side local debug upstream on
  `127.0.0.1:11434` and inject only `CAPSEM_BENCH_MITM_LOCAL_BASE_URL`; guest
  HTTP/WebSocket clients must reach it through normal iptables-nft redirection,
  not direct proxy environment variables or socket overrides.
- Replace integration-test Google/CDN traffic with the local debug upstream
  `/tiny`, `/bytes/10mb`, and corp-blocked `/deny-target` fixtures.
- Replace session DB row-generation curls with deterministic denied-domain
  probes so logging tests do not need public reachability.
- Prove broker-owned MCP auth resolves to real bearer material before dispatch.
- Prove unresolved broker refs fail before any MCP network request.

## Proof Matrix

- Unit/contract:
  - HTTP recorder captures method, URI, lower-cased headers, and body.
  - Static HTTP fixture responses preserve headers, status, and body.
- Functional:
  - MCP manager connects to the local rmcp server, discovers `echo`, and calls
    it through the production manager dispatch path.
  - Builtin `fetch_http`, `grep_http`, and `http_headers` call a local HTTP
    fixture through the production reqwest path.
  - `capsem doctor` provisions its VM with a local debug upstream base URL so
    doctor MCP and network diagnostics exercise the real iptables-nft/MITM spine
    locally.
- Adversarial:
  - Missing broker credential reference fails closed before the local MCP
    server receives any request.
  - Integration corp enforcement blocks local `/deny-target` through the
    SecurityRuleSet/CEL rail and the session DB must contain the denied row.
- E2E/integration:
  - Local in-process TCP server exercises real HTTP and rmcp transport without
    remote services.
  - `scripts/integration_test.py` starts `capsem-debug-upstream` on
    `127.0.0.1:11434` and no longer curls Google or a public CDN for release
    proof.
- Telemetry/observability:
  - Fixture records outbound HTTP headers and MCP tool arguments for assertions.
  - Integration/session tests assert local allowed, local denied, and local
    throughput rows directly from `session.db`.
- Performance:
  - `capsem-bench http` and `throughput` consume
    `CAPSEM_BENCH_MITM_LOCAL_BASE_URL` when present; public benchmarking remains
    explicit opt-in only.

## Done

- Normal MCP manager tests do not contact remote public services.
- Normal builtin HTTP tests do not contact remote public services.
- `capsem doctor` normal execution starts/uses a deterministic local debug
  upstream and does not require public internet.
- Integration and session DB tests no longer use public Google/CDN/`elie.net`
  requests as release proof.
- The local fixtures live in shared test support, not as one-off inline mocks.
- Tracker and route gate name the local proof as the MCP route/mechanics test
  foundation.
