---
title: Configuration reference
description: Which configuration files control Capsem and how are they combined?
---

Capsem combines built-in defaults, user settings, and corporate constraints.
The service owns the effective policy supplied to each VM's workers.

## Files

| Input | Format | Purpose |
| --- | --- | --- |
| `~/.capsem/settings.toml` | TOML | User preferences, provider/MCP settings, plugins, and rule-file selections. |
| `/etc/capsem/corp.toml`, then `~/.capsem/corp.toml` | TOML | Administrator constraints and reporting configuration. System corporate entries win conflicts. |
| `[rule_files].enforcement` | Path to TOML | User enforcement rules. |
| `[rule_files].sigma` | Path to YAML | User detection rules. |
| `[corp_rule_files].enforcement` | Path to TOML | Administrator enforcement rules. |
| `[corp_rule_files].sigma` | Path to YAML | Administrator detection rules. |
| `vm/active_policy.toml` | Generated TOML | Effective policy prepared by the service; do not edit it. |

`CAPSEM_CORP_CONFIG` selects one exclusive corporate file instead of the two
standard corporate paths.

Relative rule-file paths are resolved from their containing configuration file.
`CAPSEM_HOME` and `CAPSEM_RUN_DIR` relocate installation/runtime paths for
isolated installations; they do not select a remote SDK endpoint.

## Application preferences

| Key | Type | Default |
| --- | --- | --- |
| `app.auto_update` | Boolean | `true` |
| `app.notifications` | Boolean | `true` |
| `app.start_service_at_login` | Boolean | `true` |
| `appearance.theme` | String | `"system"` |
| `appearance.font_size` | Number | `14` |
| `appearance.reduced_motion` | Boolean | `false` |

These preferences do not grant worker permissions or override security rules.

## Runtime policy

| Section | Contract |
| --- | --- |
| `rule_files` | User enforcement and Sigma file references. |
| `corp_rule_files` | Corporate enforcement/Sigma file references and reporting endpoints. |
| `providers` / `mcp` | Configured provider/server identities and scoped rules. |
| `profiles.rules` | User rule group; see [Enforcement rules](/reference/enforcement-rules/). |
| `corp.rules` | Corporate rule group; cannot be minted by a user rule file. |
| `images` | Catalog selection, permitted fetch sources, and admitted images. Corporate image policy replaces the user's section. |
| `network` | Router, DNS, supported upstream ports, and capture bounds. |
| `managed_sessions` | Service-owned temporary VM lease/cleanup limits. |
| `plugins` | Plugin mode and detection level; plugin decisions may tighten enforcement. |

Built-in defaults load first, user settings overlay them, and corporate
constraints take precedence. A user cannot create corporate priority by setting
a negative number or marking a rule locked. Invalid corporate policy refuses
security admission; the settings UI remains available for repair.

Each runtime receives an acknowledged policy revision. Proxy reloads install
one immutable revision for routing, DNS, plugins, provider identity, and
response processing. Credential admission checks both the acknowledged runtime
policy and current policy.

## Validation and schema

```sh
capsem-admin settings validate ~/.capsem/settings.toml --json
capsem-admin enforcement validate ~/.capsem/access-rules.toml --source user --json
capsem-admin detection validate ~/.capsem/detection.yaml --source user --json
```

Use `--source corp` when validating administrator rules.
Shape validation does not prove that a rule will allow the intended operation:
test its effective decision in a VM.

The checked-in [settings JSON Schema](https://github.com/google/capsem/blob/0.7/config/settings/schema.generated.json)
defines the settings shape. The
[configuration types](https://github.com/google/capsem/blob/0.7/crates/capsem-config/src/types.rs)
and [rule types](https://github.com/google/capsem/blob/0.7/crates/capsem-config/src/security_rule_profile.rs)
define runtime policy contracts. UI metadata describes controls; it does not
decide runtime behavior.
