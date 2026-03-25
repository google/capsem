# Settings Grammar (`config/defaults.toml`)

The settings registry defines all built-in settings, actions, and MCP servers for Capsem. It is embedded at compile time via `include_str!()` and parsed by `policy_config/registry.rs` and `policy_config/tree.rs`.

## File Structure

```
defaults.toml ::= [settings] Node*  [mcp] McpServerDef*
user.toml     ::= [settings] OverrideEntry*  [mcp] McpServerDef*
corp.toml     ::= [settings] OverrideEntry*  [mcp] McpServerDef*
```

## Node Types

Three node types in the `[settings]` section, distinguished by key presence:

| Discriminant | Node type | Purpose |
|---|---|---|
| has `type` | **Leaf** | Setting with stored value |
| has `action` | **Action** | UI button/widget, no stored value |
| neither | **Group** | Container with children |

## GroupNode

Groups organize settings into categories and propagate metadata to children.

```toml
[settings.ai.anthropic]
name = "Anthropic"
description = "Claude Code AI agent"
enabled_by = "ai.anthropic.allow"
enabled = true
collapsed = false
hidden = false
```

| Field | Type | Required | Default | Description |
|---|---|---|---|---|
| `name` | String | yes | -- | Display name (inherited by children as `category`) |
| `description` | String | no | -- | Group description |
| `enabled_by` | SettingId | no | -- | Parent toggle ID (propagated to children except the toggle itself) |
| `enabled` | Bool | no | `true` | Explicit enable/disable |
| `collapsed` | Bool | no | `false` | Starts collapsed in UI |
| `hidden` | Bool | no | `false` | Not shown in UI |

## LeafNode (Setting)

A leaf defines an actual setting with a default value.

```toml
[settings.ai.anthropic.api_key]
name = "Anthropic API Key"
description = "API key for Anthropic. Injected as ANTHROPIC_API_KEY env var."
type = "apikey"
default = ""
```

| Field | Type | Required | Default | Description |
|---|---|---|---|---|
| `name` | String | yes | -- | Display name |
| `description` | String | no | -- | Help text |
| `type` | SettingType | yes | -- | Data type (drives default widget) |
| `default` | Value | yes | -- | Default value (must match type) |
| `enabled_by` | SettingId | no | -- | Parent toggle ID |
| `enabled` | Bool | no | `true` | Explicit enable/disable |
| `collapsed` | Bool | no | `false` | Starts collapsed in UI |
| `hidden` | Bool | no | `false` | Not shown in UI |

### SettingType enum

| Type | Value format | Default widget |
|---|---|---|
| `text` | String | text input (`select` if `choices` set) |
| `number` | Integer | number input |
| `bool` | Boolean | toggle |
| `password` | String | masked input + reveal |
| `apikey` | String | masked input + reveal + prefix hint |
| `url` | String | text input |
| `email` | String | text input |
| `file` | `{ path, content }` | file editor + syntax highlighting |
| `string_list` | `["a", "b"]` | string chips |
| `int_list` | `[1, 2, 3]` | number input |
| `float_list` | `[1.0, 2.5]` | number input |

## Meta Sub-table

Extra metadata lives under a `.meta` sub-table on the leaf:

```toml
[settings.ai.anthropic.api_key.meta]
env_vars = ["ANTHROPIC_API_KEY"]
docs_url = "https://console.anthropic.com/settings/keys"
prefix = "sk-ant-"
```

| Field | Applies to | Type | Description |
|---|---|---|---|
| `widget` | any | Widget enum | Override default UI widget |
| `side_effect` | any | SideEffect enum | Frontend action on value change |
| `docs_url` | any | String | Documentation/help URL |
| `hidden` | any | Bool | Not shown in UI |
| `builtin` | any | Bool | Non-removable by user |
| `choices` | text | [String] | Select dropdown options |
| `min` | number | Integer | Minimum bound |
| `max` | number | Integer | Maximum bound |
| `prefix` | apikey | String | Expected key prefix hint |
| `filetype` | file | FileType | Syntax highlighting |
| `env_vars` | any | [String] | Guest env vars to inject |
| `domains` | bool | [String] | Domains to allow/block |
| `rules` | bool | RulesMap | HTTP method permissions |
| `format` | text | String | DEPRECATED: use `widget` |

### Widget enum

`toggle`, `text_input`, `number_input`, `password_input`, `select`, `file_editor`, `domain_chips`, `string_chips`, `slider`

### SideEffect enum

`toggle_theme`

### FileType enum

`json`, `bash`, `conf`, `toml`, `yaml`, `text`

## ActionNode

Actions are UI elements with no stored value. They trigger frontend behavior.

```toml
[settings.app.check_update]
name = "Check for updates"
description = "Manually check if a new version is available"
action = "check_update"
```

| Field | Type | Required | Default | Description |
|---|---|---|---|---|
| `name` | String | yes | -- | Display name |
| `description` | String | no | -- | Help text |
| `action` | ActionKind | yes | -- | Action identifier |
| `hidden` | Bool | no | `false` | Not shown in UI |

### ActionKind enum

`check_update`, `preset_select`, `rerun_wizard`

## MCP Server Definitions

MCP servers are a separate `[mcp]` section (not under `[settings]`). They are auto-injected into AI agent config files at boot.

```toml
[mcp.capsem]
name = "Capsem"
description = "Built-in Capsem MCP server for file and snapshot tools"
transport = "stdio"
command = "/run/capsem-mcp-server"
builtin = true
```

| Field | Type | Required | Default | Description |
|---|---|---|---|---|
| `name` | String | yes | -- | Display name |
| `description` | String | no | -- | Help text |
| `transport` | `stdio` or `sse` | yes | -- | Protocol |
| `command` | String | stdio only | -- | Command to run |
| `url` | String | sse only | -- | URL to connect to |
| `args` | [String] | no | `[]` | Command arguments |
| `env` | {Str: Str} | no | `{}` | Environment variables |
| `headers` | {Str: Str} | no | `{}` | HTTP headers (sse only) |
| `builtin` | Bool | no | `false` | Non-removable |
| `enabled` | Bool | no | `true` | Explicit enable/disable |

Resolution: `corp > user > defaults` (per key). Corp entries are corp-locked.

## ID Construction

Setting IDs are dot-separated paths from TOML nesting. `[settings.ai.anthropic.allow]` produces ID `ai.anthropic.allow` (the `settings` prefix is stripped).

## Inheritance Rules

1. **Category**: nearest ancestor group with a `name` key determines the category.
2. **enabled_by**: propagated from nearest ancestor, except the toggle itself.
3. **collapsed**: inherited from nearest ancestor, can be overridden.

## Value Resolution

```
corp.toml > user.toml > defaults.toml
```

Per-key merge: each setting resolved independently. Corp wins, then user, then default.

## Enabled Resolution

```
effective_enabled = explicit_enabled AND enabled_by_result
```

- `explicit_enabled`: corp > user > defaults > true
- `enabled_by_result`: if no `enabled_by` -> true; else look up parent toggle's effective value

## User / Corp Files

Override entries in `user.toml` and `corp.toml`:

```toml
[settings]
"ai.anthropic.allow" = { value = true, modified = "2026-01-15T10:30:00Z" }
"ai.anthropic.api_key" = { value = "sk-ant-...", modified = "2026-01-15T10:30:00Z" }
```

Keys must be quoted. Corp settings override user settings per-key. Corp can also set `enabled` and `hidden` on override entries.

MCP servers in user/corp files follow the same `[mcp]` format as defaults.toml.
