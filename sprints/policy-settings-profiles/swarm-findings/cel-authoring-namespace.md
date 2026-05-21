# CEL Authoring Namespace Findings

Status: completed
Agent: 019e4c17-b86d-7403-8ff7-ec70bdbac487 / Herschel

## Scope

Investigate the right architecture for a canonical policy authoring namespace
that lets users write ergonomic rules such as:

```cel
http.request.host.contains("google")
http.request.url.contains("google")
mcp.request.tool_name == "filesystem.read_file"
model.request.provider == "gemini"
file.activity.path_class == "workspace"
```

The namespace must be the rule authoring ABI. Rule authors must not write
`event.*` at all. The internal normalized `SecurityEvent` may remain the audit,
telemetry, and sink envelope, but CEL evaluation should receive direct
canonical policy context roots/functions such as `http`, `dns`, `mcp`, `model`,
`file`, and `process`.

## Required Questions

- What code owns current CEL compilation/evaluation?
- What code owns Detection IR to CEL lowering?
- What canonical namespaces and fields should exist for S08b/S08c?
- Should the first slice be direct CEL context variables/functions, generated
  accessor schema, or a hybrid?
- Does the current Rust `cel` crate support method functions such as
  `http.request.header("authorization").exists()` directly, or do we need a
  slightly different first-slice shape?
- Which tests prove every exposed path/function is stable and safe?
- What sprint docs and downstream surfaces need updating?

## Findings

### P0: Current CEL Runtime Exposes The Wrong ABI

Impact: runtime rules currently see `event` as the only CEL variable, which
makes normalized `SecurityEvent` JSON the de facto authoring contract. This
conflicts with the corrected product contract: rule authors must write direct
domain roots such as `http.request.host.contains("google")`, never `event.*`.

Paths:
- `crates/capsem-security-engine/src/lib.rs:1471`
- `crates/capsem-security-engine/src/lib.rs:1541`
- `crates/capsem-security-engine/src/lib.rs:1584`

Required proof:
- `cel_context_rejects_event_root`
- `cel_context_allows_http_request_host_contains`
- `handle_enforcement_compile_rejects_event_root`
- `handle_enforcement_backtest_matches_canonical_http_rule`

Transfer status: captured; fold into S08b before runtime rule routes are called
stable.

### P1: Detection IR Currently Lowers To Internal Event Paths

Impact: S08c corpora would cement the wrong language if detection fixtures
continue lowering to `event.subject.*`.

Paths:
- `crates/capsem-core/src/security_packs.rs:230`
- `crates/capsem-core/src/security_packs.rs:318`
- `crates/capsem-core/src/security_packs.rs:343`

Required proof:
- `detection_ir_lowers_to_canonical_cel_roots`
- `detection_ir_lowered_rule_matches_real_cel_context`
- negative fixture proving `event.subject...` is invalid.

Transfer status: captured; S08b/S08c boundary task.

### P1: Profile Policy Validation Is Still A Separate CEL-Like Surface

Impact: profile/settings rules use a custom callback-local parser and field
allowlist. That needs migration to the same canonical context, or an explicit
compatibility boundary, otherwise we will have two policy languages.

Paths:
- `crates/capsem-core/src/net/policy/condition.rs:3`
- `crates/capsem-core/src/net/policy/condition.rs:362`
- `crates/capsem-core/src/settings_profiles/mod.rs:1928`
- `crates/capsem-core/src/settings_profiles/mod.rs:4051`

Required proof:
- `profile_rule_validation_compiles_canonical_cel`
- `profile_rule_validation_rejects_event_root`
- `legacy_policy_condition_fields_are_mapped_or_rejected_explicitly`

Transfer status: captured; S08b/S08c task depending on scope.

### P2: Header, Body, And Missing-Value Semantics Need Formal Typing

Impact: helpers such as `http.request.header("authorization").exists()` and
`http.request.body.text.contains("secret")` need deterministic behavior for
case-insensitive headers, missing bodies, redaction, and unavailable callback
surfaces.

Finding: the current `cel` crate supports custom functions and method-style
calls through `Context::add_function` and `This<T>`, while dotted field access
works naturally over maps. The first implementation should inject canonical
domain roots as CEL maps/views and register helper functions for operations
that maps cannot express cleanly.

Required proof:
- `cel_context_header_lookup_returns_optional_present`
- `cel_context_header_lookup_missing_is_absent`
- `cel_context_body_text_redacted_is_empty_or_absent`
- `cel_context_rejects_unknown_function`

Transfer status: captured; S08b task.

## Recommended Architecture

Use canonical CEL context injection. Do not translate public rule source to
`event.*`.

Implementation shape:
- Define a versioned, typed policy-context contract in `capsem-proto` or a
  similarly shared protocol crate.
- Build a CEL context in `capsem-security-engine` that injects only public
  roots: `http`, `dns`, `mcp`, `model`, `file`, `process`, `profile`, and
  `common`.
- Represent stable dotted fields as CEL maps/views.
- Register helper methods/functions for `header(name)`, optional presence, and
  other non-field accessors.
- Reject `event`, unknown roots, unknown functions, and unsupported paths at
  compile/install/backtest/hunt time using a shared allowlist/reference check.

## Initial Namespace Slice

- `http.request.host`, `method`, `scheme`, `path`, `query`, `url`,
  `path_class`, `bytes`
- `http.request.header(name)`
- `http.request.body.text`, `body.size`, `body.redaction_state`
- `http.response.status`, `header(name)`, `body.text`, `body.size`
- `dns.request.qname`, `qtype`, `domain_class`
- `mcp.request.server_id`, `server_name`, `tool_name`, `arguments`
- `model.request.provider`, `model`, `estimated_input_tokens`,
  `estimated_cost_micros`
- `model.response.estimated_output_tokens`, `stop_reason`
- `file.activity.operation`, `path_class`, `byte_count`
- `process.activity.operation`, `command_class`
- `common.profile_id`, `profile_revision`, `vm_id`, `session_id`,
  `event_type`, `enforceability`

## Tests Run

Static/no-edit investigation only.
