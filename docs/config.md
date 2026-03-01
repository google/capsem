# Settings Registry Format (`config/defaults.toml`)

The settings registry defines all built-in settings for Capsem. It is embedded at compile time via `include_str!()` and parsed by `policy_config.rs`.

## Node Types

The TOML table tree has two node types, distinguished by the presence of a `type` key:

### Group Node (no `type`)

Groups organize settings into categories and propagate metadata to children.

```toml
[settings.ai.anthropic]
name = "Anthropic"                      # Display name (inherited by children as `category`)
description = "Claude Code AI agent"    # Group description (not used by children)
enabled_by = "ai.anthropic.allow"       # Parent toggle ID (inherited by all children except the toggle itself)
collapsed = false                       # Whether the group starts collapsed in the UI
```

Group metadata fields:
- `name` -- category display name, inherited by all descendant settings
- `description` -- group description (informational only)
- `enabled_by` -- parent toggle ID, propagated to child settings (except the toggle itself)
- `collapsed` -- UI collapse state, inherited by children

### Setting Leaf (has `type`)

A leaf defines an actual setting with a default value.

```toml
[settings.ai.anthropic.api_key]
name = "API Key"
description = "Anthropic API key for Claude Code."
type = "apikey"
default = ""
collapsed = false

[settings.ai.anthropic.api_key.meta]
env_vars = ["ANTHROPIC_API_KEY"]
```

Required fields:
- `name` -- display name
- `description` -- help text
- `type` -- one of: `text`, `number`, `password`, `url`, `email`, `apikey`, `bool`, `file`
- `default` -- default value (must match the type)

Optional fields:
- `collapsed` -- whether the setting starts collapsed in the UI (default: `false`)

### Meta Sub-table

Extra metadata lives under a `meta` sub-table on the leaf:

```toml
[settings.registry.github.allow.meta]
domains = ["github.com", "api.github.com", "*.githubusercontent.com"]
rules.repos = { path = "/repos/*", get = true, post = true, put = true, delete = false }
```

Meta fields:
- `domains` -- domain patterns for network policy (array of strings)
- `choices` -- valid values for choice settings (array of strings)
- `min` / `max` -- bounds for number settings
- `rules` -- HTTP method permissions keyed by rule name
- `guest_path` -- guest filesystem path for `file`-type settings
- `env_vars` -- environment variable names to inject in the guest when the value is non-empty

## ID Construction

Setting IDs are dot-separated paths derived from the TOML table nesting. For example, `[settings.ai.anthropic.allow]` produces the ID `ai.anthropic.allow` (the `settings` prefix is stripped).

## Inheritance Rules

1. **Category**: the nearest ancestor group with a `name` key determines the category for all descendant settings.
2. **enabled_by**: propagated from the nearest ancestor group with `enabled_by`, except the setting whose ID matches the `enabled_by` value (the toggle itself gets `enabled_by = None`).
3. **collapsed**: inherited from the nearest ancestor group, can be overridden per-setting.

## User / Corp Files

Settings files (`~/.capsem/user.toml` and `/etc/capsem/corp.toml`) store overrides as flat key-value pairs:

```toml
[settings]
"ai.anthropic.allow" = { value = true, modified = "2026-01-15T10:30:00Z" }
"ai.anthropic.api_key" = { value = "sk-ant-...", modified = "2026-01-15T10:30:00Z" }
```

Keys must be quoted (dotted keys in TOML create nested tables, not flat keys). Corp settings override user settings per-key.
