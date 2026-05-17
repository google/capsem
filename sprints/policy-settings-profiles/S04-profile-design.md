# S04 - Profile Design

## Goal

Lock the v1 profile TOML contract so rule format, inheritance intent, and
runtime policy semantics are unambiguous before implementation completion.

## Design Decisions (Locked)

- A **profile** is the only user-facing session/VM security selector.
- v1 profile type surface is `everyday-work` and `coding`.
- `skills.groups` stays schema-visible, but has no v1 behavioral semantics.
- Security stays two-layered:
  - `[security.capabilities]` for high-level controls.
  - `[security.rules.<type>.<rule_name>]` for advanced policy rules.
- Canonical profile rule default priority is `1`.
- Parent inheritance is required by contract via `extends_profile_id` with
  merge/lock semantics enforced by S05/S06.

## Canonical Profile TOML Contract

- Required identity: `id`, `name`, `description`, `best_for`, `profile_type`.
- Optional: `icon_svg`, `extends_profile_id`, and section-specific optional
  fields.
- Profile sections: `general`, `appearance`, `ai`, `mcp`, `skills`, `vm`,
  `security`.
- Appearance inheritance behavior for v1:
  - if a child profile omits appearance fields, it inherits from parent profile
    first, then service-level appearance defaults.

### Security Rule Table Contract

- Canonical rule path is `security.rules.<type>.<rule_name>`.
- `<type>` is `mcp|http|dns|model|hook`.
- `<rule_name>` is `[A-Za-z0-9_-]+`.
- Required fields: `on`, `if`, `decision`.
- Optional: `priority` (default `1`), `reason`.
- `decision` enum: `allow|ask|block|rewrite`.
- Callback contract reuses policy-v2 callback space:
  - `mcp.request`, `mcp.response`
  - `http.request`, `http.response`
  - `dns.request`, `dns.response`
  - `model.request`, `model.response`, `model.tool_call`, `model.tool_response`
  - `hook.decision`
- Condition grammar reuses existing supported subset:
  - `&&`, `==`, `!=`, `has()`, `matches()`, `contains()`,
    `startsWith()`, `endsWith()`
  - callback field allowlists are enforced.
- Rewrite rules:
  - only valid when `decision = "rewrite"`.
  - require valid `rewrite_target`/`rewrite_value` pairing (or valid HTTP
    header-strip rewrite mode where supported).
  - capture references in `rewrite_value` must exist in `rewrite_target`.

### MCP Argument Path Contract

- `arguments.<path>` is a dotted JSON path inside the MCP `arguments` object,
  not just one segment.
- Examples:
  - `arguments.text`
  - `arguments.issue.title`
  - `arguments.payload.token`

### Canonical Example (Engine-Aligned v1)

```toml
version = 1
id = "everyday-work"
name = "Everyday Work"
description = "Balanced defaults for day-to-day work."
best_for = "Balanced defaults for day-to-day work."
profile_type = "everyday-work"
icon_svg = "<svg ...>...</svg>"
extends_profile_id = "base-everyday"

[security.capabilities]
credential_brokerage = "ask"
pii_detection = "ask"
mcp_rag = "allow"
mcp_tools = "allow"
network_egress = "ask"
file_boundaries = "ask"
audit = "audit"

[security.rules.http.block_openai_repo]
on = "http.request"
if = 'request.host == "github.com" && request.path.startsWith("/openai/")'
decision = "block"
priority = 1
reason = "Block direct OpenAI repo fetches"

[security.rules.mcp.redact_prod_token]
on = "mcp.request"
if = 'method == "tools/call" && tool.name == "github__create_issue" && arguments.text.contains("prod-token-")'
decision = "rewrite"
priority = 1
rewrite_target = 'arguments.text =~ "prod-token-[A-Za-z0-9]+"'
rewrite_value = "[redacted-token]"
reason = "Redact production tokens before MCP calls"

[security.rules.model.block_secret_prompt]
on = "model.request"
if = 'request.body.contains("AWS_SECRET_ACCESS_KEY")'
decision = "block"
priority = 1
reason = "Block model requests containing secret material"
```

## Validation Rules

- Unknown fields are rejected.
- Duplicate profile IDs and duplicate rule names under the same
  `security.rules.<type>` table are rejected.
- Bad callback/type combinations, malformed conditions, or invalid rewrite
  configs are rejected with provenance-friendly paths.
- Missing `best_for` for base/corp/user profiles is rejected.
- Locked base/corp roots remain immutable for user-level writes.

## Revisit/Implementation Tasks

- [x] Implement `extends_profile_id` parse/validate wiring in S05.
- [x] Migrate profile parser/model from `security.raw_rules` to canonical
      `security.rules.<type>.<rule_name>` tables.
- [x] Enforce default profile rule priority = `1`.
- [x] Add parser tests for callback contract (`dns.request` and
      `dns.response`, reject `dns.query`) in canonical profile rule tables.
- [x] Add parser/runtime tests for `arguments.<path>` dotted JSON path behavior.
- [x] Keep capability-derived rules in effective output with locked provenance.

## Status

- S04 design authority is now `security.rules.<type>.<rule_name>`.
- Profile parsing now uses canonical
  `security.rules.<type>.<rule_name>` tables; remaining policy callback/field
  normalization and rewrite parity gaps are tracked in S06-pre and S06a.
- Code-gap audit refresh on 2026-05-14:
  - resolved in `settings_profiles`: `extends_profile_id` field,
    self-reference rejection, v1 profile-type scope
    (`everyday-work|coding`), and default rule priority `1`.
  - resolved in `settings_profiles`: canonical
    `security.rules.<type>.<rule_name>` parser/model migration and callback/type
    validation (`dns.query` rejected at profile parser boundary).
  - still pending outside this slice: DNS callback normalization away from
    `dns.query` in runtime policy enums/parsers and model request rewrite
    parity.
- S04 closure refresh on 2026-05-14:
  - parser coverage now includes canonical MCP dotted argument-path conditions
    (`arguments.issue.title`) in profile rules.
  - effective settings tests now assert derived capability rules remain in
    output with locked provenance for locked profiles.
  - remaining runtime callback/field normalization stays tracked in S06-pre and
    rewrite parity in S06a; those no longer block S04 design closure.

## Coverage Ledger

- Unit/contract: parser and validator coverage must move to canonical
  `security.rules` shape and priority defaults.
- Functional: profile load/discovery and effective settings assembly must
  surface canonical rules with provenance.
- Adversarial: malformed callback names, bad rewrite captures, and type/field
  mismatches must fail closed with clear paths.
- E2E/VM: validated in S06/S18 once resolver/runtime cutover is complete.
- Telemetry: provenance and matched rule IDs must remain visible in debug/status.
- Performance: not applicable to design closure.
