# Capsem Ironbank

Ironbank is the black-box release ledger suite. These tests exercise Capsem
through the VM, `capsem-doctor`, hermetic local protocol services, the session
DB, structured logs, UDS routes, HTTP routes, and UI-facing JSON. They do not
look at Rust internals to decide expected behavior.

Rules:

- No `skip`, `skipif`, `slow`, optional marker, or public-network fixture.
- No status-code-only replay.
- No row-exists proof.
- No parser-only proof.
- One deterministic stimulus must assert the full chain.
- Every DB/log/route field must be asserted exactly, covered by a typed
  invariant, or explicitly marked not applicable.
- Package-manager tests must prove the package works, not merely that it was
  installed.
- MCP tests must drive the installed `capsem mcp` CLI through the real service
  socket and then assert the full ledger: CLI output, UDS route, HTTP gateway
  route, session DB rows, security ledger rows, MCP protocol rows, structured
  logs, counters, and UI-facing JSON.

If a public contract is missing, write the RED test against the missing
contract and fix the product contract before relying on implementation details.
