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
- Add a reusable local Streamable HTTP MCP server with a real rmcp tool.
- Replace remote MCP manager tests with local proofs.
- Prove broker-owned MCP auth resolves to real bearer material before dispatch.
- Prove unresolved broker refs fail before any MCP network request.

## Proof Matrix

- Unit/contract:
  - HTTP recorder captures method, URI, lower-cased headers, and body.
- Functional:
  - MCP manager connects to the local rmcp server, discovers `echo`, and calls
    it through the production manager dispatch path.
- Adversarial:
  - Missing broker credential reference fails closed before the local MCP
    server receives any request.
- E2E/integration:
  - Local in-process TCP server exercises real HTTP and rmcp transport without
    remote services.
- Telemetry/observability:
  - Fixture records outbound HTTP headers and MCP tool arguments for assertions.
- Performance:
  - Local HTTP recorder is available for the follow-up debug/benchmark sprint.

## Done

- Normal MCP manager tests do not contact remote public services.
- The local fixtures live in shared test support, not as one-off inline mocks.
- Tracker and route gate name the local proof as the MCP route/mechanics test
  foundation.
