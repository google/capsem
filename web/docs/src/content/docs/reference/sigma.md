---
title: Sigma syntax
description: Which Sigma fields, conditions, and actions does Capsem support?
---

Capsem loads a subset of [Sigma's YAML rule format](https://sigmahq.io/docs/basics/rules.html)
and compiles it into its shared security rule set. This reference covers the
complete supported format. For a procedure, see
[Writing detection rules](/guides/writing-detection-rules/).

## Rule fields

| Field | Required | Meaning |
| --- | --- | --- |
| `title` | Yes | Rule title; converted to a lowercase identifier with separators replaced by underscores, trimmed and truncated to 64 characters. Derived identifiers must be unique in the file. |
| `level` | Yes | `informational` (`info` alias), `low`, `medium`, `high`, or `critical`. |
| `logsource.product` | Yes | Must be `capsem`. |
| `logsource.service` | Yes | Must be `security_event`. |
| `detection` | Yes | Named selections and a `condition` string. |
| `description` | No | Describes the rule; used as the reason if no `capsem.reason` is set. |
| `id` | No | String identifier; used as a fallback reason. |
| `status`, `author`, `date` | No | Accepted string metadata; does not change evaluation. |
| `falsepositives` | No | Accepted list of strings; does not change evaluation. |
| `capsem` | No | Capsem action, reason, priority, and ownership settings below. |

Separate rules with YAML document separators (`---`). Unknown rule fields,
including `tags` and `references`, are rejected. `logsource` accepts only
`product` and `service`.

## Selections

Each selection is a nonempty mapping from a supported
[security-event field](/reference/cel/#security-event-fields) to a scalar or
nonempty list of scalars. Scalars may be strings, numbers, or Booleans and are
compared as strings. Values match exactly and case-sensitively.

```yaml
detection:
  selection:
    http.method: GET
    http.host:
      - a.example.com
      - b.example.com
  condition: selection
```

Fields within a selection are combined with AND. Values in a field's list are
combined with OR: this example requires GET **and** either hostname. Missing
fields do not match, including in negated selections.

Selection names use lowercase letters, digits, and underscores, with a maximum
of 64 characters. Nested mappings, nulls, empty selections/lists, field modifiers
such as `|contains` and `|re`, and wildcard matching are unsupported. A `*` in
a scalar value is a literal character.

## Conditions

| Syntax | Meaning |
| --- | --- |
| `selection` | Match a named selection. |
| `a and b` | Match both selections. |
| `a or b` | Match either selection. |
| `a and not b` | Match `a` and the negation of `b`. |

Operators are lowercase. `and` binds more tightly than `or`; each selection
stays grouped when compiled into CEL. `not` negates the next named selection.
Parentheses, repeated `not`, selection wildcards, `1 of`, `all of`, aggregation,
and correlation conditions are unsupported. Unknown selections are rejected.

## Capsem settings

| Field | Default | Meaning |
| --- | --- | --- |
| `capsem.action` | `allow` | `allow`, `ask`, `block`, `preprocess`, `rewrite`, or `postprocess`. Use `postprocess` to record a detection without selecting an enforcement decision. |
| `capsem.reason` | Description, then ID fallback | Reason recorded for the rule. |
| `capsem.priority` | User: `10`; corp: `-10` | User priorities `10`–`1000`; corp priorities `-1000`–`-10`. Lower numbers come first. User rules may also use `default` for priority `1001`. |
| `capsem.corp_locked` | `false` | Only corp configuration can create locked rules. |

A detection level does not determine an action. The default `allow` action can
participate in access decisions; do not use it to mean “observe only.” All matched
detections are recorded, while the first matching enforcement rule selects the
rule decision. Plugins can make that decision stricter. See the
[security engine](/architecture/security-engine/) for enforcement and observation boundaries.

## File configuration

```toml
[rule_files]
sigma = "detection.yaml"
```

The path is relative to the configuration file's directory unless absolute.
The file is parsed and validated when policy is loaded. Sigma selections use
the same event fields and missing-value behavior as the
[CEL syntax reference](/reference/cel/).
