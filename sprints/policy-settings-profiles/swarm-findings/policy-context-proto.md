# Policy Context Proto Contract Findings

Status: completed
Agent: 019e4c1e-cedd-76b1-b843-0c83d1aad218 / Mencius

## Scope

Implement the shared typed policy context contract in `capsem-proto`. This is
the schema that the high-level DSL and public CEL roots mirror. It must be
testable, fully defined, and free of evaluator/business logic.

Constraints:
- Do not name current public Rust structs `*V1`.
- Use `POLICY_CONTEXT_SCHEMA_VERSION` plus a `schema_version` field.
- Keep CEL and rule-evaluation logic out of `capsem-proto`.
- Use typed structs/enums with `serde(deny_unknown_fields)` where appropriate.
- Prefer deterministic maps/collections for stable JSON and tests.

Expected first-slice roots include:
- `common`
- `http.request`, `http.response`
- `dns.request`
- `mcp.request`, `mcp.response`
- `model.request`, `model.response`
- `file.activity`
- `process.activity`
- `profile.activity`

## Required Questions

- Which exact structs/enums belong in `capsem-proto` versus
  `capsem-security-engine`?
- What missing/absent/redacted semantics should be represented in the schema?
- What tests pin the schema, roundtrips, header lookup, and versioning?
- Does the schema provide enough structure for the future high-level DSL mirror?

## Findings

### Completed: Shared Policy Context Schema Landed

Changed files:
- `crates/capsem-proto/src/lib.rs`
- `crates/capsem-proto/src/policy_context.rs`
- `crates/capsem-proto/src/policy_context/tests.rs`

Implemented:
- `POLICY_CONTEXT_SCHEMA_VERSION: u16 = 1`
- root `PolicyContext` with `schema_version`
- typed serde roots for `common`, `http`, `dns`, `mcp`, `model`, `file`,
  `process`, and `profile`
- canonical first-slice fields such as `http.request.host`,
  `http.request.url`, `http.request.path`, `http.request.header(name)`,
  `dns.request.qname`, `mcp.request.server_id`, `mcp.request.tool_name`,
  `model.request.provider`, `model.request.estimated_cost_micros`,
  `file.activity.path_class`, `file.activity.byte_count`, and
  `process.activity.command_class`
- deterministic `BTreeMap` headers and labels
- case-insensitive HTTP header lookup helpers
- explicit body states: `missing`, `redacted`, `text`, `binary`
- `#[serde(deny_unknown_fields)]` on schema structs
- no CEL dependency and no policy evaluation logic
- no current public `*V1` policy-context type names

Transfer status: captured. Next owner is S08b engine injection:
`capsem-security-engine` must build CEL context roots from this schema and
reject authored `event.*` rules.

## Tests Run

- `cargo fmt --check` passed.
- `cargo test -p capsem-proto policy_context` passed: 7 tests.
- `cargo test -p capsem-proto` passed: 165 tests, 1 doc-test ignored.
