---
title: CEL syntax
description: Which CEL expressions and event fields can I use in Capsem rules?
---

Capsem uses a subset of [Common Expression Language (CEL)](https://cel.dev/)
in a TOML rule's `match` field. This reference covers the complete supported
expression syntax. For a procedure, see [Enforce security policy](/guides/enterprise/enforce-security-policy/).

## Operators

| Syntax | Meaning | Example |
| --- | --- | --- |
| `field == "value"` | Equal to a value. | `http.method == "GET"` |
| `field != "value"` | Different from a value. | `http.method != "GET"` |
| `a && b` | Both expressions match. | `http.host == "example.com" && http.method == "GET"` |
| `a \|\| b` | Either expression matches. | `http.method == "GET" \|\| http.method == "HEAD"` |
| `(expression)` | Group expressions. | `http.host == "example.com" && (http.method == "GET" \|\| http.method == "HEAD")` |
| `has(field)` | The field is present. | `has(model.name)` |

`&&` binds more tightly than `||`. Expressions short-circuit: an AND stops at
its first non-match; an OR stops at its first match.

## String functions

| Function | Meaning | Example |
| --- | --- | --- |
| `field.contains("text")` | Contains a substring. | `http.path.contains("/admin/")` |
| `field.startsWith("text")` | Starts with a prefix. | `http.path.startsWith("/v1/")` |
| `field.endsWith("text")` | Ends with a suffix. | `file.export.name.endsWith(".csv")` |
| `field.matches("regex")` | Contains a regular-expression match. | `http.host.matches("(^\|.*\\.)example\\.com$")` |
| `field.contains_pii()` | Capsem extension: contains `@` or an SSN-shaped `123-45-6789` value. | `model.request.body.contains_pii()` |

String matching is case-sensitive. `matches()` uses the Rust regex engine:
anchor with `^` and `$` when the whole value must match. Look-around and
backreferences are unsupported. `contains_pii()` is a simple heuristic, not a
complete personal-data classifier.

## Values and quoting

Comparison values and string-function arguments must be quoted strings.
Single and double quotes are accepted. Boolean fields compare to `"true"` or
`"false"`; number-like fields compare to strings such as `"443"`.

```toml
match = 'http.host == "example.com" && tcp.port == "443"'
```

TOML literal strings (`'…'` or multiline `'''…'''`) preserve CEL backslashes.
With TOML basic strings (`"…"`), TOML decodes its escapes before CEL sees them.

CEL strings decode `\\`, `\"`, `\'`, `\/`, `\n`, `\t`, `\r`, `\xHH`,
`\uHHHH`, and `\UHHHHHHHH`. Other escapes remain literal, including regex
escapes such as `\.` and `\d`. Unterminated strings, malformed Unicode escapes,
and trailing content are rejected.

## Missing fields

Equality, inequality, and string functions return false when their field is
absent or has no comparable value. In particular, a missing field does not
match `field != "value"`. `has(field)` tests presence; it does not test whether
the value is true. Use `field == "true"` to test a `.valid` Boolean.

## Security-event fields

Only the following fields are accepted. A field may be absent for a particular
event. Field names are case-sensitive; unknown names are rejected.

| Group | Fields |
| --- | --- |
| HTTP | `http.host`, `http.method`, `http.path`, `http.query`, `http.status`, `http.body`, `http.valid` |
| DNS | `dns.qname`, `dns.qtype`, `dns.valid` |
| Model | `model.provider`, `model.name`, `model.request.body`, `model.request.tool_calls`, `model.request.valid`, `model.response.body`, `model.response.valid`, `model.tool_call.valid`, `model.valid` |
| MCP | `mcp.method`, `mcp.server.name`, `mcp.server.valid`, `mcp.tool_call.name`, `mcp.tool_call.valid`, `mcp.tool_list`, `mcp.tool_list.valid`, `mcp.request.id`, `mcp.request.method`, `mcp.request.arguments`, `mcp.request.valid`, `mcp.response.content`, `mcp.response.valid`, `mcp.event.valid`, `mcp.valid` |
| Container | `container.image`, `container.registry`, `container.digest`, `container.valid` |
| IP / transport | `ip.value`, `ip.version`, `ip.valid`, `tcp.port`, `tcp.valid`, `udp.port`, `udp.valid` |
| Process | `process.name`, `process.command`, `process.valid`, `process.audit.valid`, `process.exec.id`, `process.exec.path`, `process.exec.stdout`, `process.exec.stderr`, `process.exec.exit_code`, `process.exec.valid` |
| File | `file.kind`, `file.content`, `file.valid`; for each operation `create`, `delete`, `export`, `import`, `read`, `write`: `file.<operation>.content`, `.ext`, `.mime_type`, `.name`, `.path`, `.valid` |
| Network | `network.action`, `network.id`, `network.mode`, `network.name`, `network.protocol`, `network.publication.id`, `network.side`, `network.target`, `network.valid`; for each endpoint `source`, `destination`: `network.<endpoint>.generation`, `.ip`, `.port`, `.vm_id`, `.vm_name` |

The abbreviated file and network rows denote exact field-name expansions,
not wildcard expressions. For example, use `file.export.path`, not
`file.<operation>.path`. See [security events](/architecture/security-engine/#what-a-security-event-contains)
for how these facts are collected.

## Unsupported syntax

Capsem does not accept general CEL arithmetic, numeric ordering (`<`, `>`,
`<=`, `>=`), `!`, `in`, lists or map literals, indexing, ternary expressions,
comprehensions, `size()`, or arbitrary functions. Bare Boolean/number literals
and field-to-field comparisons are also unsupported. An empty expression,
unknown field, unsupported term, or invalid regex fails validation.

## Expression limits

Compilation expands Boolean expressions into clauses. A condition may produce
at most 1,024 clauses and 16,384 atoms in total. Expressions exceeding either
limit are refused during validation; shorten or split the rule instead of
assuming a syntactically valid expression will compile.
