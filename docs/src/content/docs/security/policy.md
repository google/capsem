---
title: Rule Authoring
description: Canonical rule roots, decisions, rewrites, and the enforcement/detection split.
sidebar:
  order: 25
---

Capsem rules are profile-owned and evaluated by the Security Engine over typed
Security Events. The old `policy.<type>.<rule_name>` runtime and raw
`request.*` authoring path are gone.

Use this page for the shared authoring vocabulary. Use
[Enforcement](/security/enforcement/) for synchronous allow/ask/block/rewrite
behavior and [Detection Format](/security/detection/) for Sigma-compatible
finding rules.

## Two Rule Families

| Family | Runtime effect | API group | Admin workflow |
|---|---|---|---|
| Enforcement | `allow`, `ask`, `block`, or `rewrite` at a synchronous boundary | `/enforcement/*` | `capsem-admin enforcement ...` |
| Detection | Attach findings to the resolved event; never blocks by itself | `/detection/*` | `capsem-admin detection ...` |

Detection and enforcement may use similar canonical fields, but they are not
the same semantic surface. Detection is evidence and hunting. Enforcement is a
transport decision.

## Canonical Roots

Authored rules target high-level typed roots. Do not author rules against
internal `event.*`, raw `subject.*`, or provider-specific JSON paths.

| Event family | Example roots |
|---|---|
| HTTP | `http.request.host`, `http.request.url`, `http.request.path`, `http.request.method`, `http.request.header("authorization")`, `http.request.body.text`, `http.response.status`, `http.response.body.text` |
| DNS | `dns.request.qname`, `dns.request.qtype`, `dns.response.rcode`, `dns.response.answers` |
| MCP | `mcp.request.server_name`, `mcp.request.tool_name`, `mcp.request.arguments`, `mcp.response.result_status`, `mcp.response.content` |
| Model | `model.request.provider`, `model.request.name`, `model.request.messages`, `model.request.tool_calls`, `model.response.output_text`, `model.response.tool_calls` |
| File | `file.activity.path`, `file.activity.path_class`, `file.activity.operation`, `file.activity.snapshot_id` |
| Process | `process.exec.argv`, `process.exec.cwd`, `process.exec.env_keys`, `process.exec.exit_code` |

Examples:

```text
http.request.host.contains("google")
http.request.url.contains("admin")
http.request.path.startsWith("/admin")
http.request.header("authorization").exists()
http.request.body.text.contains("secret")
mcp.request.tool_name == "github__get_file_contents"
model.request.provider == "google" && model.request.name.contains("gemini")
```

## Enforcement Shape

```toml
[security.rules.http.block_metadata]
on = "http.request"
if = 'http.request.host == "169.254.169.254"'
decision = "block"
priority = 10
reason = "metadata endpoints are not reachable from corp VMs"
```

| Field | Required | Description |
|---|---:|---|
| `on` | yes | Synchronous boundary, such as `http.request` or `mcp.request`. |
| `if` | yes | CEL expression over canonical roots. |
| `decision` | yes | `allow`, `ask`, `block`, or `rewrite`. |
| `priority` | yes | Lower numbers run first. |
| `reason` | no | Short audit string stored with the resolved event. |

## Decisions

| Decision | Behavior |
|---|---|
| `allow` | Continue through the boundary. |
| `ask` | Create an approval challenge and fail closed unless approved. |
| `block` | Stop at the boundary and return a denial response. |
| `rewrite` | Apply validated declarative mutations, then continue. |

`warn` is not an enforcement decision.

## Rewrites

Plugins and rules declare mutations; Rust validates and applies them to the
real request, response, model payload, MCP payload, or file/process event.

```json
{
  "op": "strip_header",
  "path": "http.request.headers.authorization"
}
```

Each event type has an allowlist of legal rewrite targets. Rewrites outside the
allowlist fail closed before the transport body is changed.

## Priority Tiers

| Range | Owner | Notes |
|---|---|---|
| `-1000` to `-1` | Corp-exclusive | Only valid in corp profiles or corp directives. |
| `0` | System/toggle-derived | Used by generated provider/MCP capability rules. |
| `1` to `999` | User-authored | Recommended interactive range. |
| `1000` | Catch-all | System-emitted only. |

Rules are evaluated in ascending priority. Lower number means earlier decision.

Corp directives that add or replace rule values must use the corp window
`[-1000, 0]`. Catch-all priority `1000` is reserved for system-emitted defaults
and is rejected for hand-authored rules.

## Rule Ownership

Resolved rules carry ownership metadata so UI, CLI, and audit logs can explain
why a rule exists and whether it is editable:

| Field | Meaning |
|---|---|
| `owner_setting_path` | Dotted setting path that produced the rule, such as `ai.providers.google.enabled`. |
| `owner_setting_label` | Human-readable label for "managed by" UI copy. |
| `editable` | `false` for setting-derived rules; direct mutations must target the owning setting. |

Ownership classes:

| Class | Editable | Example |
|---|---:|---|
| Hand-authored rule | yes | `security.rules.http.allow_corp` |
| Capability-derived rule | no | `security.capabilities.network_egress` |
| Toggle-derived rule | no | `ai.providers.google.enabled` |
| Corp-directive replacement | yes | `corp_directives[0]` |

If a caller edits a non-editable rule directly, the mutation gate returns
`Forbidden { owner_setting_path }`. The fix is to edit the owning setting or
profile directive.

## Rules Under Settings

Rules can live at top level or under the setting that owns them. Nesting keeps
provenance close to the control it describes:

```toml
[ai.providers.google]
enabled = true

[ai.providers.google.rules.http.allow_gemini]
on = "http.request"
if = 'http.request.host == "generativelanguage.googleapis.com"'
decision = "allow"
priority = 0
```

The resolver tags the emitted rule with
`owner_setting_path = "ai.providers.google"`.

## HTTP Callback Split

HTTP request rules can use broad `http.request` callbacks or the read/write
split used by catch-all generation:

| Callback | Methods |
|---|---|
| `http.read` | `GET`, `HEAD`, `OPTIONS` |
| `http.write` | `POST`, `PUT`, `PATCH`, `DELETE` |

For example, a read-only profile can emit an allow catch-all for `http.read`
and a block catch-all for `http.write`.

## Catch-All Rules

The resolver emits one catch-all per rule type at priority `1000`. Catch-alls
run only when no earlier rule matched.

| Capability | Generated catch-alls |
|---|---|
| `security.capabilities.network_egress` | `dns.default`, `http.default_read`, `http.default_write`, `model.default` |
| `security.capabilities.mcp_tools` | `mcp.default` |

## Non-Migrations

The old hardcoded default allow/block lists are not migrated into profile
rules. Hosts that should be reachable must be represented by explicit corp or
user rules. The old `http_upstream_ports` allowlist also exits with the removed
NetworkPolicy runtime.

## Backtest And Evidence

Both enforcement and detection support backtests. Backtests return aggregate
counts plus up to 100 diverse matched evidence rows by default. Local evidence
is not redacted for a user with access to the session; exported telemetry keeps
bounded/redacted summaries.

## Telemetry

The Security Engine emits a resolved event before telemetry, audit logging, and
detection export projections. The resolved event carries the final decision,
findings, matched rules, mutations, trace/profile/VM/user attribution, and
evidence refs. VM status and OpenTelemetry summaries are derived from those
typed events, not from ad hoc policy tables.
