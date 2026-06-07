# Reconciled Settings/Profile/Corp Format

Status: target contract for snapshot restore. This document is for review before
implementation.

Hard guardrail: do not change the current security event object, plugin
contract, rule format, detection format, or plugin/rule/detection corp/profile
file locations. If implementation is blocked by that, stop and ask.

## Ownership

`settings.toml` is UI/application preferences only. It must not own VM behavior,
profiles, assets, rules, detections, AI, MCP, skills, credentials, or plugins.

`profile.toml` owns runtime behavior: profile identity, description, icon,
availability, assets, VM defaults, rule files, default rules, profile rules, AI
provider convenience declarations, MCP, skills, credential broker config, plugin
config, and tool config source records.

`corp.toml` owns constraints and reporting over profiles: corp rules, corp rule
files/endpoints, locks, refresh metadata, and integration endpoints. It may
constrain profile behavior, but it does not become UI settings.

## Settings

Settings are only app/appearance preferences. This is intentionally small.

```toml
# ~/.capsem/settings.toml

[app]
auto_update = true
notifications = true
start_service_at_login = true

[appearance]
theme = "system"
font_size = 14
reduced_motion = false
```

Not allowed in settings:

- `[profiles.*]`
- `[corp.*]`
- `[rule_files]`
- `[ai.*]`
- `[plugins.*]`
- `[mcp]`
- `[skills]`
- `[credentials]`
- `[assets]`
- VM/resource defaults

Current file targets:

- `config/settings.toml`
- `config/profiles/code.toml`
- `config/corp.toml`

`config/user.toml.default` was removed because it documented profile-owned AI,
repository, VM, guest-env, and plugin behavior as user settings.

## Profile

Profile identity is first-class. UI labels and icons come from this file; the UI
does not invent them.

```toml
# profiles/coding/profile.toml

id = "coding"
name = "Coding"
description = "Default coding VM with AI CLIs, MCP tools, and profile-owned security rules."
icon_svg = "<svg viewBox=\"0 0 16 16\" aria-hidden=\"true\"></svg>"
revision = "2026.06.07.1"

[availability]
web = true
shell = true
mobile = false

[catalog]
channel = "stable"
update_policy = "auto"
manifest_url = "https://releases.capsem.dev/profiles/coding/manifest.json"
manifest_pubkey = "minisign:..."

[vm]
cpu_count = 6
ram_gb = 8
scratch_disk_size_gb = 32

[assets]
format = "profile-assets.v1"
filesystem = "erofs"
compression = "lz4hc"
compression_level = 12

[assets.arch.arm64.kernel]
name = "vmlinuz"
url = "https://releases.capsem.dev/assets/arm64/vmlinuz"
hash = "blake3:..."
signature = "minisig:..."
size = 12345678
content_type = "application/octet-stream"

[assets.arch.arm64.initrd]
name = "initrd.img"
url = "https://releases.capsem.dev/assets/arm64/initrd.img"
hash = "blake3:..."
signature = "minisig:..."
size = 12345678
content_type = "application/octet-stream"

[assets.arch.arm64.rootfs]
name = "rootfs.erofs"
url = "https://releases.capsem.dev/assets/arm64/rootfs.erofs"
hash = "blake3:..."
signature = "minisig:..."
size = 12345678
content_type = "application/vnd.capsem.erofs"
filesystem = "erofs"
compression = "lz4hc"
compression_level = 12

[assets.arch.x86_64.kernel]
name = "vmlinuz"
url = "https://releases.capsem.dev/assets/x86_64/vmlinuz"
hash = "blake3:..."
signature = "minisig:..."
size = 12345678
content_type = "application/octet-stream"

[assets.arch.x86_64.initrd]
name = "initrd.img"
url = "https://releases.capsem.dev/assets/x86_64/initrd.img"
hash = "blake3:..."
signature = "minisig:..."
size = 12345678
content_type = "application/octet-stream"

[assets.arch.x86_64.rootfs]
name = "rootfs.erofs"
url = "https://releases.capsem.dev/assets/x86_64/rootfs.erofs"
hash = "blake3:..."
signature = "minisig:..."
size = 12345678
content_type = "application/vnd.capsem.erofs"
filesystem = "erofs"
compression = "lz4hc"
compression_level = 12
```

The current `ProfileAssetConfig` only has `channel/kernel/initrd/rootfs`
strings. That is not enough. Restore work must replace it with per-architecture
asset declarations while keeping EROFS/LZ4HC as the accepted runtime format on
all supported architectures.

## Rule Files

Rule file locations live in profile/corp, not settings. Detection can point at
Sigma YAML. Enforcement/rules use the current TOML rule format.

```toml
[rule_files]
enforcement = "rules/enforcement.toml"
sigma = "rules/detection.yaml"
```

## Default Rules

Default rules are visible rules. They are not a second engine.

```toml
[profiles.defaults.default_http_requests]
name = "default_http_requests"
action = "allow"
priority = "default"
reason = "Default allow for HTTP requests."
match = "has(http.host)"

[profiles.defaults.default_dns_queries]
name = "default_dns_queries"
action = "allow"
priority = "default"
reason = "Default allow for DNS queries."
match = "has(dns.qname)"

[profiles.defaults.default_mcp_activity]
name = "default_mcp_activity"
action = "allow"
priority = "default"
reason = "Default allow for MCP server activity and tool calls."
match = "has(mcp.method) || has(mcp.server.name) || has(mcp.tool_call.name) || has(mcp.tool_list)"

[profiles.defaults.default_model_calls]
name = "default_model_calls"
action = "allow"
priority = "default"
reason = "Default allow for model calls."
match = "has(model.provider) || has(model.name) || has(model.request.body) || has(model.response.body) || has(model.request.tool_calls)"

[profiles.defaults.default_file_activity]
name = "default_file_activity"
action = "allow"
priority = "default"
reason = "Default allow for file reads, writes, creates, deletes, imports, and exports."
match = "has(file.read.path) || has(file.write.path) || has(file.create.path) || has(file.delete.path) || has(file.import.path) || has(file.export.path) || has(file.content)"

[profiles.defaults.default_process_activity]
name = "default_process_activity"
action = "allow"
priority = "default"
reason = "Default allow for process execution and audit activity."
match = "has(process.exec.path) || has(process.command) || has(process.exec.id)"
```

## Profile Rules

This is the current rule format. Do not change it during restore.

```toml
[profiles.rules.skill_loaded]
name = "skill_loaded"
action = "allow"
detection_level = "informational"
reason = "Record when a skill file is loaded."
match = 'file.read.path.matches("(^|.*/)skills/.+\\.md$") && file.read.ext == "md"'

[profiles.rules.block_untrusted_dns]
name = "block_untrusted_dns"
action = "block"
detection_level = "high"
reason = "Block known untrusted DNS requests."
match = 'dns.qname.matches("(^|.*\\.)evil.example$")'
```

## AI Provider Convenience Rules

AI blocks live in profiles or corp as rules. Provider sections are authoring
convenience; they compile into the same `SecurityRuleSet`/CEL rail.

```toml
[ai.openai]
name = "OpenAI"
protocol = "openai"
url = "https://api.openai.com/v1"
aliases = ["api.openai.com", "chatgpt.com", "oaistatic.com", "oaiusercontent.com"]
listen_ports = [443]
allowed_remote_targets = ["api.openai.com:443"]
files = ["/root/.codex/config.toml"]

[ai.openai.rules.http_api]
name = "openai_http_api_observed"
action = "allow"
detection_level = "informational"
reason = "Observe OpenAI HTTP traffic."
match = 'http.host.matches("(^|.*\\.)(openai\\.com|chatgpt\\.com|oaistatic\\.com|oaiusercontent\\.com)$")'

[ai.openai.rules.dns_api]
name = "openai_dns_api_observed"
action = "allow"
detection_level = "informational"
reason = "Observe OpenAI DNS traffic."
match = 'dns.qname.matches("(^|.*\\.)(openai\\.com|chatgpt\\.com|oaistatic\\.com|oaiusercontent\\.com)$")'

[ai.openai.rules.config_credential_broker]
name = "openai_config_credential_broker"
plugin = "credential_broker"
action = "postprocess"
type = "api-key"
credential = "api_key"
reason = "Broker OpenAI credentials from tool config reads."
match = 'file.read.path == "/root/.codex/config.toml" && has(file.read.content)'
```

No raw credentials are exposed in rule matches. Credential broker logs/reporting
use BLAKE3 references.

## Plugins

Plugins live in profile/corp. Every non-dummy plugin must have a rule that
references it. The plugin contract is frozen for this sprint.

```toml
[plugins.credential_broker]
mode = "rewrite"
detection_level = "informational"

[profiles.rules.credential_broker_http]
name = "credential_broker_http"
plugin = "credential_broker"
action = "postprocess"
reason = "Broker credentials observed in approved HTTP provider flows."
match = 'has(http.host)'
```

## MCP

MCP config is profile-owned mechanics. MCP decisions are rules, not MCP policy.

```toml
[mcp]
health_check_interval_secs = 60

[[mcp.servers]]
id = "filesystem"
name = "filesystem"
url = "http://127.0.0.1:9000"
enabled = true

[[mcp.servers.tools]]
id = "read_file"
name = "read_file"
enabled = true
```

If the current MCP Rust type uses a different concrete shape, restore must
adapt the example to the real type without reintroducing MCP decision policy.
The invariant is profile -> server -> tools/resources/prompts, not global MCP
tools.

## Skills

Skills stay as a profile-owned placeholder for now. It is acceptable that the
runtime is not fully implemented yet, but the ownership stays profile.

```toml
[skills]
paths = ["/root/.codex/skills/security/SKILL.md"]
```

## Credentials

Credential broker is on by default and profile-owned.

```toml
[credentials]
broker_enabled = true
```

## Tool Config Sources

Tool config source records let the broker/profile rail explain where a tool
configuration was observed without exposing raw secrets.

```toml
[tool_config_sources.codex]
tool_id = "codex"
guest_path = "/root/.codex/config.toml"
format = "toml"
observed_hash = "blake3:2222222222222222222222222222222222222222222222222222222222222222"
inferred_endpoint_ref = "ai.openai"
credential_refs = ["credential:blake3:1111111111111111111111111111111111111111111111111111111111111111"]
allowed_overlays = ["mcp_injection", "broker_placeholders", "endpoint_selection"]
```

## Corp

Corp owns constraints and reporting endpoints. It can reference rule files and
Sigma files. Corp priorities may be negative; profile/user rules do not get
negative priorities.

```toml
# /etc/capsem/corp.toml

refresh_interval_hours = 24

[corp_rule_files]
enforcement = "corp/enforcement.toml"
sigma = "corp/detection.yaml"
sigma_output_endpoint = "https://siem.example.invalid/capsem/sigma"
open_telemetry = "https://otel.example.invalid/v1/traces"
remote_enforcement = "https://security.example.invalid/capsem/enforcement"

[corp.defaults.default_http_block_unknown]
name = "corp_default_http_block_unknown"
action = "block"
priority = -10
corp_locked = true
reason = "Corp baseline block for disallowed HTTP destinations."
match = 'has(http.host)'

[corp.rules.block_openai]
name = "block_openai"
action = "block"
priority = -100
corp_locked = true
detection_level = "high"
reason = "Corp policy blocks OpenAI."
match = 'http.host.matches("(^|.*\\.)(openai\\.com|chatgpt\\.com|oaistatic\\.com|oaiusercontent\\.com)$")'

[plugins.credential_broker]
mode = "rewrite"
detection_level = "informational"
```

Corp can also provide AI convenience sections if needed, but they must compile
into the same rule rail and must not create a second provider policy engine.
