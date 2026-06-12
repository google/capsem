# Ironbank Ledger Tests

Status: release-blocking contract.

Ironbank is Capsem's black-box ledger suite for 1.3. It sits beside
Winterfell and lives under `tests/ironbank/`. Its rule is simple: what goes
into Capsem must come out through the same public truth everywhere: client
result, parsed security facts, decision, detection/enforcement ledger,
protocol tables, structured logs, status counters, UDS routes, HTTP routes,
and UI-facing JSON.

## Authoring Rule

Ironbank tests are written from the outside. Test authors may read public
contracts, CLI help, docs, route responses, generated schemas, hermetic
fixture definitions, logs, DB rows, and installed package metadata. They must
not read Rust/product internals to decide expected behavior. If behavior has
no public contract, the RED test is that the contract is missing.

## No Escape Hatches

- No Rust parser/unit test can close an Ironbank gate.
- No public-network dependency.
- No mocks of the Capsem path.
- No fallback route.
- No status-code-only replay.
- No row-exists proof.
- No `skip`, `skipif`, `slow`, optional marker, or manual OAuth/client dance
  as release proof.

## One Stimulus, Full Ledger

Each protocol case sends one deterministic stimulus and asserts, at minimum:

1. Client-visible result.
2. Parser family/type classification.
3. Parsed request fields.
4. Parsed response fields.
5. Protocol-specific SQLite row.
6. Unified security ledger row.
7. Detection level/rule row when expected.
8. Structured service/gateway log evidence.
9. In-memory status/stats counters.
10. UDS route output.
11. HTTP gateway route output.
12. UI-facing serialization shape when the route backs the UI.

Every emitted field is covered to the penny: exact value when deterministic,
typed invariant/range/shape/provenance when nondeterministic, or explicit
not-applicable entry. Unknown DB, log, or route fields fail the test until the
field coverage ledger is updated.

## Required Families

- HTTP: plain JSON, denied, ask, preprocess rewrite, postprocess rewrite,
  HTTPS/MITM, gzip, chunked, SSE, WebSocket, truncated upstream, large
  body/header cap with no secret leak.
- DNS: A/AAAA, TXT, denied, malformed/truncated, long-label exfil,
  local/private answer using IP/TCP/UDP/default ask facts.
- Model: OpenAI-compatible, Anthropic streaming, Gemini/AGY streaming,
  unknown-compatible-provider, non-stream JSON, SSE, tool declarations,
  executed tool calls, tool responses, usage/tokens, thinking/reasoning,
  truncation/error, denied and accepted cases.
- MCP: every configured MCP server/tool path must work black-box and be
  faithfully accounted for. Ironbank must exercise server list, tool list,
  refresh, tool call, resources/prompts, accepted/denied/ask, request args,
  response body, no phantom executed calls, duplicate suppression,
  route-visible server/tool evidence, session DB rows, security ledger rows,
  structured logs, UDS output, HTTP gateway output, and UI-facing JSON. A
  command existing in `--help` is not proof.
- Credential broker/plugins: OAuth token capture, header/query/cookie/body
  capture, stored-ref injection, brokered substitution/rewrite, disabled,
  ask, block, and error modes with no raw-secret leak.
- File/process/snapshot: file create/read/write/delete/import/export,
  symlink escape, preview caps, process observation/exec/failure, snapshot as
  route-only hermetic subsystem.

## Package Managers

Package-manager proof is functional. For apt, npm, uv, pip, node, or profile
package rails, installing is not enough. The test must assert binary presence,
version/hash where relevant, and then run the package in a way that proves it
does its job. Example: installing `zstd` must compress and decompress known
bytes and compare the output.
