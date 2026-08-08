# Policy Hook Spec0

## Goal

Policy Hook Spec0 is the first external compatibility contract for Capsem
policy decisions. It lets Capsem forward a normalized policy decision
request to a local or HTTPS hook server and receive a typed decision back.
The same Rust wire types must drive local hook dispatch, remote HTTPS
forwarding, and OpenAPI export.

OpenAPI 3.1 is the default export format because third-party plugin authors
can generate receiving servers from it. If implementation later chooses a
different format, it must be equally generator-friendly and committed as a
versioned artifact.

## Required Export

- Generated artifact: `config/policy-hook-openapi.json` or
  `docs/api/policy-hook-openapi.yaml`.
- Source of truth: Rust `serde` wire types, not hand-written JSON/YAML.
- Compatibility fields: `spec_version`, `schema_hash`, and stable enum
  strings.
- Tests:
  - generated artifact matches the Rust types
  - every policy callback is represented
  - every decision is represented
  - rewrite target/value fields are represented
  - unknown enum values fail closed in the runtime parser
  - sample request/response fixtures validate against the exported spec

## Endpoints

- `POST /v1/policy/decision`: evaluate one policy subject.
- `POST /v1/policy/batch-decision`: evaluate multiple independent
  subjects with the same response shape.
- `GET /v1/policy/spec`: return the exact OpenAPI document or a hash and
  URL for the document the server implements.
- `GET /v1/health`: liveness and supported `spec_version` list.

## Request Shape

Each request has shared envelope fields:

- `spec_version`: `"policy-hook/v0"`.
- `decision_id`: caller-generated idempotency/correlation id.
- `trace_id`: Capsem trace id when available.
- `session_id`: session id when safe to expose.
- `on`: the normalized callback being evaluated.
- `subject`: discriminated union payload for the selected callback.
- `preview`: optional bounded preview fields.
- `hashes`: stable hashes for payloads that are too large or too sensitive.
- `audit_context`: process name, pid if known, provider/server/domain,
  and config source.

Policy callbacks:

- `mcp.request`
- `mcp.response`
- `http.request`
- `http.response`
- `dns.query`
- `dns.response`
- `model.request`
- `model.response`
- `model.tool_call`
- `model.tool_response`

Local TOML rules use the same callback names in their `on` field and CEL
conditions evaluate against the same normalized `subject` model. Remote
hooks receive already-normalized subjects; they do not define a second
policy language.

## Response Shape

Every response returns:

- `decision`: `allow`, `ask`, `block`, or `rewrite`.
- `decision_id`: echoed or server-generated id.
- `rule_id`: stable rule identifier.
- `priority`: local rule priority when the decision is tied to a configured
  rule; lower numbers evaluate first.
- `reason`: human-readable but audit-safe text.
- `ttl_ms`: optional cache duration for identical requests.
- `rewrite_target`: required for `rewrite`, absent otherwise.
- `rewrite_value`: required for `rewrite`, absent otherwise.
- `redactions`: fields Capsem must avoid logging.
- `audit_tags`: optional low-cardinality labels.

`rewrite_target` is targeted, not a free-form patch. It uses the same
validated selector/regex grammar as local policy and may expose captures
that `rewrite_value` can reference:

- MCP: argument value or response field.
- HTTP: URL, query value, request header, response header, or strip header.
- DNS: A/AAAA answer value.
- Model: request field, response field, tool-call argument, or tool-response
  content.

## Security Rules

- HTTPS hooks require configured endpoint allowlist, auth, timeout, body cap,
  and retry budget.
- HTTPS only, except explicit localhost/dev mode.
- Bearer token or mTLS authentication is required outside localhost/dev mode.
- Corp `block` decisions cannot be weakened by a hook.
- Enforcement hooks fail closed on timeout, auth failure, schema mismatch, or
  malformed response unless a rule explicitly permits local fallback.
- Hook request and response payloads are not logged by default. Only fields
  marked audit-safe by the spec can enter `session.db` or process logs.
- Rewrite payloads from hooks must be redacted by default. Credential broker
  hooks should prefer opaque references over literal secret values.

## MVP Acceptance

- A generated OpenAPI 3.1 artifact exists in the repo.
- A tiny test hook server can be generated from the artifact or validated
  against it and used in an E2E decision test.
- Local hook dispatch and HTTPS hook dispatch share the same request and
  response structs.
- Session telemetry records hook endpoint id, spec version/hash, decision id,
  latency, timeout/error status, and fallback status without leaking payloads.
