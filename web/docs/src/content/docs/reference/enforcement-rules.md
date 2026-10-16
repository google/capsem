---
title: Enforcement rules reference
description: What is the enforcement rule format and how does Capsem choose a decision?
---

This reference defines Capsem's enforcement rule format and how matching rules,
priorities, and plugins determine an operation's effective decision. It covers
rule fields, processing actions, and validation, including the distinction
between recording a detection and refusing access.

Enforcement files are TOML. Each enabled rule matches a
[CEL expression](/reference/cel/) against a security event and specifies
`allow`, `ask`, or `block`.
For a procedure, see [Enforce security policy](/guides/enterprise/enforce-security-policy/).

## Rule groups

| Group | Owner |
| --- | --- |
| `[profiles.rules.KEY]` | User/general profile rules. |
| Provider rule groups | Rules scoped to configured provider identities. |
| `[corp.rules.KEY]` | Administrator-owned rules compiled from corporate sources. |

```toml
[profiles.rules.block_api_writes]
name = "block_api_writes"
action = "block"
priority = 10
match = 'http.host == "api.example.com" && http.method != "GET" && http.method != "HEAD"'
reason = "Only GET and HEAD are permitted."
```

## Fields

| Field | Type | Default / contract |
| --- | --- | --- |
| `name` | String | Required display name. |
| `action` | `allow\|ask\|block` | Required. |
| `match` | String | Required CEL condition; see the complete [syntax](/reference/cel/). |
| `enabled` | Boolean | `true`. Disabled rules do not enforce. |
| `priority` | Integer or `"default"` | Source-dependent; lower numbers evaluate first. |
| `detection_level` | Severity | Optional: informational, low, medium, high, critical. |
| `reason` | String | Optional explanation recorded with the decision. |
| `corp_locked` | Boolean | `false`; corporate authority is validated from the source, not trusted from this flag alone. |
| `managed` | Tagged object | Managed MCP/plugin target; generated/validated by policy tooling. |
| Plugin-specific fields | Plugin-defined values | Validated for that plugin; not arbitrary extensions to the CEL field set. |

## Processing actions

`preprocess`, `rewrite`, and `postprocess` identify processing rules; they
do not select an allow/ask/block decision. `rewrite` also accepts the legacy
aliases `redact`, `mutate`, and `neutralize`. Plugin handling determines
the transformation. Use `postprocess` for an observation-only Sigma detection.

## Priority and decisions

| Source | Permitted numeric band | Default |
| --- | --- | --- |
| Corporate or corporate-locked | -1000 through -10 | -10 |
| User/plugin | 10 through 1000 | 10 |
| Built-in default | Named `"default"` | 1001 |

Rules sort by ascending priority, then rule ID. The first matching enforcement
rule selects the rule decision. Matching detections are still recorded.
A user/plugin cannot claim a negative corporate priority.

Preprocess plugins prepare event facts. Postprocess plugins can tighten
`allow` to `ask` or `block`; they cannot loosen a refusal.
An `ask` is pending approval, not permission to forward.

Network connection and probe admission require an explicit allow. Other event
defaults and provider rules still matter; the absence of your own match is
not a universal allow or block rule.

## Detection versus enforcement

A severity records evidence; an action controls admission where the operation
has an enforcement boundary. Observation events, including guest audit reports,
cannot stop an operation that has already happened.

[Sigma rules](/reference/sigma/) compile into the same rule model.
A detection-only postprocess rule can record a severity without changing access.

## Load and validate

User files are selected by `[rule_files].enforcement`; administrator files
by `[corp_rule_files].enforcement`. Relative paths resolve from their containing
configuration file. Validate with:

```sh
capsem-admin enforcement validate ~/.capsem/access-rules.toml --source user --json
```

Validation checks syntax, known event fields, priorities, and rule structure.
Test an allowed and a refused operation to verify the effective policy.
Required audit admission must also succeed before an enforced operation proceeds.
