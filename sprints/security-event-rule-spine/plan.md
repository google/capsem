# Plan: Security Event Rule Spine

## Problem

Capsem's intended architecture is:

```text
parse/normalize
  -> SecurityEvent
  -> preprocess plugins
  -> detect/enforce CEL rules
  -> postprocess plugins
  -> materialize/log/emit
```

The current implementation is mixed. Security action plugins already receive
and return `SecurityEvent`, but CEL matching still uses per-callback subjects:

```text
PolicyCallback::HttpRequest  + HttpRequestPolicySubject
PolicyCallback::DnsQuery     + DnsQueryPolicySubject
PolicyCallback::McpRequest   + McpDecisionRequest
PolicyCallback::ModelRequest + ModelRequestPolicySubject
```

That demux creates drift. It lets a rule API be correct in one family while
missing model responses, file boundaries, process events, or parts of MCP.

## Target Contract

Rules are one authored object evaluated over the full canonical
`SecurityEvent`. Top-level rules live in `corp.rules` or `profiles.rules`;
provider blocks are profile/corp policy semantically. Provider-scoped rules are
only convenience/default namespaces that compile into the same rule rail.

```toml
[profiles.rules.openai_http_observed]
name = "openai_http_observed"
detection_level = "informational"
action = "allow"
match = 'http.host.matches("(^|.*\.)(openai\.com|chatgpt\.com|oaistatic\.com|oaiusercontent\.com)$")'
```

```toml
[profiles.rules.openai_model_observed]
name = "openai_model_observed"
detection_level = "informational"
action = "allow"
match = 'model.provider == "openai"'
```

The engine evaluates that expression against the current event. If the current
event has no referenced root, those fields simply do not match. The rule is
not split.

## Authoring Schema

Rule homes:

```toml
[corp.rules.<rule_id>]
name = "<stable_rule_name>"
action = "allow|ask|block|preprocess|postprocess"
match = "<CEL over SecurityEvent>"
```

```toml
[profiles.rules.<rule_id>]
name = "<stable_rule_name>"
action = "allow|ask|block|preprocess|postprocess"
match = "<CEL over SecurityEvent>"
```

Provider convenience rules:

```toml
[ai.<provider>.rules.<rule_id>]
name = "<stable_rule_name>"
action = "allow|ask|block|preprocess|postprocess"
match = "<CEL over SecurityEvent>"
```

MCP-specific convenience namespaces may be added later, but they must compile
into the same `SecurityRule` list. No HTTP/DNS/MCP/model verb buckets.

Optional fields:

```toml
detection_level = "informational|low|medium|high|critical"
priority = -1000..1000
corp_locked = true
reason = "human-readable context"
```

Plugin configuration:

```toml
[plugins.credential_broker]
mode = "rewrite"
detection_level = "informational"
```

Plugins own their own filtering. Rules must not use `plugin = ...`. Raw
authorization headers, raw API keys, and raw credential file contents are
inspected inside the broker plugin and are logged only through BLAKE3
substitution references.

PII example:

```toml
[profiles.rules.redact_pii]
name = "openai_prompt_pii_redact"
action = "preprocess"
match = 'has(model.request.body)'
```

PII detection is plugin work. The plugin inspects/redacts privately and returns
the mutated `SecurityEvent`; profile rules remain normal CEL rules.

File scanner example:

```toml
[profiles.rules.scan_import]
name = "file_import_vt_scan"
action = "postprocess"
match = 'file.import.path.matches(".*")'
```

File event fields are verb-shaped. The first-class verbs are:

- `file.import.*`: copied/imported into the VM/session.
- `file.export.*`: exported out of the VM/session.
- `file.read.*`
- `file.create.*`
- `file.write.*`
- `file.delete.*`

Each verb exposes the same minimum fields: `path`, `name`, `ext`,
`mime_type`, and `content`.

Corp block example:

```toml
[corp.rules.block_openai]
name = "openai_api_block"
action = "block"
corp_locked = true
match = 'http.host.matches("(^|.*\.)(openai\.com|chatgpt\.com|oaistatic\.com|oaiusercontent\.com)$")'
```

## Implementation Slices

### T0: Contract Freeze

- Define `RuleAction`.
- Define `DetectionLevel`.
- Define `SecurityRuleProfile`.
- Validate rule ids and names.
- Validate action-specific required fields.
- Validate priority source defaults and constraints.
- Add fixture TOML for detection metadata, broker, PII, VT, allow, ask, block.

### T1: SecurityEvent CEL Subject

- Expand `SecurityEvent` to carry typed optional roots:
  `http`, `dns`, `mcp`, `model`, `file`, `process`, `credential`, `snapshot`.
- Implement `PolicySubject` for `SecurityEvent`.
- Preserve existing helper verbs such as `contains`, `matches`, `has`, and
  root-safe missing-field behavior.
- Add tests where cross-root `OR` evaluates correctly without splitting.
- Add tests where cross-root missing fields evaluate false.

### T2: Rule Compiler

- Compile `corp.rules`, `profiles.rules`, and convenience provider rules into
  one internal `SecurityRule` list.
- Keep stable `rule_id` and mandatory `rule_name`.
- Compile CEL once per authored rule.
- Remove `on` and `if` from provider authoring.
- Delete the old provider-rule compiler path instead of preserving it as a
  compatibility layer.

Implementation note: `ProviderRuleProfile` remains only as the settings-file
adapter for `[ai.<provider>]` metadata and rules. It delegates to
`SecurityRuleProfile`/`SecurityRuleSet`, and its old callback enum/compiler
has been removed. Provider defaults no longer generate old `policy.http`,
`policy.dns`, `policy.mcp`, or `policy.model` callback rules.

### T3: Plugin Actions

- Done: `SecurityEventEngine` evaluates one `SecurityRuleSet` against one
  canonical `SecurityEvent`.
- Revised: enabled plugins run by their own declared stage through
  `plugin(SecurityEvent) -> SecurityEvent`. Rules no longer dispatch plugins.
- Done: the emitter sees exactly one final post-action event.
- Done: configured missing plugins fail closed before emission.
- Done: `credential_broker` is registered as a built-in postprocess-capable
  plugin and uses credential observations on the event, not matched rule
  metadata.
- Revised: `credential.*` is not a first-party CEL root in 1.3; broker refs
  stay on the event/ledger as forensic evidence.
- Deferred: PII and VirusTotal plugin implementations are future plugins on
  this same contract.

### T4: Detection, Enforcement, Ask

- Every matched rule emits one `security_rule_events` ledger row through the
  logger sink. This row is the forensic source of truth for both detection and
  enforcement:
  `event_id`, `event_type`, `rule_id`, `rule_action`, `detection_level`,
  `rule_json`, `event_json`, and `trace_id`.
- `event_id` is exactly 12 lowercase hex characters. It is generated as a
  UUIDv4-derived id for primary runtime event rows, then reused by any matched
  rule rows for that same event.
- Runtime producers call `emit_security_write` / `emit_security_write_blocking`
  for primary logger events; those functions assign and return the event id.
  Any matched rule ledger row for the same normalized `SecurityEvent` must use
  that returned id.
- Runtime producers then call `emit_matching_security_rules` /
  `emit_matching_security_rules_blocking` with the returned event id, the
  runtime event type, the active `SecurityRuleSet`, and the canonical
  `SecurityEvent`. The helper emits every matched rule row and returns the
  count.
- `emit_matching_security_rules_with_decision` /
  `emit_matching_security_rules_with_decision_blocking` are the authoritative
  enforcement helpers. They evaluate once, emit every matched ledger row, and
  return the typed `SecurityEnforcementDecision` derived from those same
  matches.
- `detection_level` is never nullable; use `none` when the rule has no
  detection metadata.
- `rule_action` defaults to `allow` and is stored as the enum-backed canonical
  value.
- `rule_json` is the rule snapshot at match time. `event_json` is the
  normalized `SecurityEvent` payload that matched. Together they must be enough
  to investigate from `session.db` after the active ruleset changes.
- `GET /security/{id}/latest` returns the full stored `SecurityRuleEvent`
  rows. `GET /security/{id}/info` returns DB-regenerated counters from
  `security_rule_events`.
- `allow`, `ask`, `block`, `preprocess`, and `postprocess` all use this same
  ledger row; there is no separate detection/enforcement output path.
- Done: HTTP upstream materialization now has a typed guard that only
  materializes after an `allow` decision. `ask` and `block` fail before
  materialization.
- Done: `ask` creates an append-only pending row in `security_ask_events`
  through the logger sink. The row carries `ask_id`, triggering `event_id`,
  `event_type`, `rule_id`, `rule_name`, strict status, rule snapshot, matched
  event payload, trace id, and optional resolver/reason.
- Done: ask resolution is append-only: `approved` and `denied` rows share the
  same `ask_id`. Approval resolves into `allow`; denial resolves into `block`;
  pending continues to block materialization.
- Done: `security.ask` is a typed internal runtime security event.
- Done: detection and enforcement share ordering and matching.
- Done: rule-match tracing carries safe low-cardinality labels from the rule
  snapshot: `rule_id`, `rule_name`, `rule_action`, `rule_detection_level`,
  and `provider`.

### T5: Runtime Hook Burn-In

Every producer must create/pass a `SecurityEvent` into the same engine:

- HTTP request.
- HTTP response.
- DNS query and response if emitted.
- MCP tool call, tool list, resource/prompt methods, initialize, notifications,
  unknown MCP methods.
- Model request.
- Model response.
- Model tool call.
- Model tool response.
- File import.
- File export.
- File read.
- File create.
- File write.
- File delete.
- Process exec request.
- Process exec complete.
- Process audit.
- Credential observed/substitution.
- Snapshot.

Implementation note: the current slices wire MITM HTTP/model telemetry, DNS
telemetry, MCP telemetry, file monitor/tool telemetry, explicit service/process
file boundaries, process exec/audit/complete, credential substitution, and
snapshots. `MergedPolicies.security_rules` carries the compiled provider/default
`SecurityRuleSet` into capsem-process, reloads it with policy reloads, and the
runtime emitters write primary rows plus matched `security_rule_events` rows with
the same 12-lower-hex primary `event_id`. Service host-workspace import/export
requests do not open a second DB writer; they send a typed `LogFileBoundary` IPC
job to capsem-process, which owns the session DB writer and active ruleset.

### T6: Sigma And Refactor

- Done: Sigma-derived detections import through
  `SecurityRuleProfile::parse_sigma_yaml`.
- Done: Sigma import produces typed `SecurityRuleAction` plus typed
  `DetectionLevel`, not string callbacks/actions.
- Done: Sigma import derives a valid rule id/name from the title, requires
  level metadata, and compiles the Sigma condition into one `match` expression.
- Done: Sigma import validates against the same first-party `SecurityEvent`
  CEL roots.
- Done: referenced `rule_files.sigma` files merge into
  `MergedPolicies.security_rules` through the same runtime rule rail as
  TOML-authored enforcement rules.
- Done: separate callback-shaped Sigma tests were removed from
  `validate_imported_policy_rule_json`; stale callback fields now fail through
  the `SecurityEvent` root validator.
- Done: tests prove parser-compatible Sigma YAML imports, evaluates against
  `SecurityEvent`, rejects stale fields, resolves relative rule files, and
  stays loadable by the Python Sigma parser gate.

### T7: Burn Old API

- Remove provider-owned `on`.
- Remove provider-owned `if`.
- Remove provider-owned `decision`.
- Remove provider-owned `actions`.
- Remove old credential block proposals.
- Remove old authoring shape from settings parsing/validation.
- Keep migration error messages clear.
- Add burn guards so new provider rules cannot require central callback edits.

### T8: Verification

- Unit tests for schema validation.
- CEL tests for cross-root single rules.
- Plugin tests for event mutation.
- Ask resolution tests.
- Sigma import/refactor tests.
- Runtime integration tests for HTTP, DNS, MCP, model request/response,
  file/process, and credential events.
- Session DB tests for rule labels and detection/enforcement rows.
- OTEL label tests proving no raw secret/log explosion.
- Adversarial tests for malformed CEL, missing roots, unknown plugin, bad
  names, bad levels, and invalid priority.

## Done Means

- The public authoring format is `corp.rules` and `profiles.rules`, with
  `ai.<provider>.rules` as convenience/default authoring only.
- One cross-root rule remains one rule through parse, compile, match, log, and
  OTEL.
- All first-party runtime security event types are covered by the unified
  engine or fail a drift test.
- The old callback-demux authoring path is gone from provider rules.
