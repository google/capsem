# Reconciled Settings/Profile/Corp Format

Status: target contract for snapshot restore. This document is for review before
implementation.

Hard guardrail: do not change the current security event object, plugin
contract, rule format, detection format, or plugin/rule/detection corp/profile
file locations. If implementation is blocked by that, stop and ask.

## Ownership

`settings.toml` is UI/application preferences only. It must not own VM behavior,
profiles, assets, rules, detections, AI, MCP, credentials, or plugins.

`profile.toml` owns runtime behavior: profile identity, description, icon,
availability, assets, VM defaults, rule files, default rules, profile rules,
provider control rules, plugin config, and MCP server configuration. Observed
tool config sources, credential references, and provider configured state are
runtime evidence/status, not static profile content. The built-in local MCP
server is real:
`mcp.local` runs `/run/capsem-mcp-server`/`capsem-mcp-builtin` and exposes
HTTP fetch plus workspace snapshot tools. The canonical `code` profile must
represent that real built-in server, not fake in-VM filesystem tools.

`corp.toml` owns constraints and reporting over profiles: corp rules, corp rule
files/endpoints, locks, `refresh_policy`, and integration endpoints. It may
constrain profile behavior, but it does not become UI settings.

## Trust Chain

Runtime asset trust is deliberately small:

- corp/profile configuration chooses the asset URL;
- the profile asset descriptor carries the expected BLAKE3 hash and size;
- runtime download/ensure verifies the actual bytes against that hash;
- release evidence is SBOM and build provenance, not a second manifest
  authority rail.

Do not put fake signing keys, content types, filesystem formats, compression
levels, kernel flags, or build knobs in profile/corp payloads. Those belong to
build, benchmark, release, or SBOM artifacts. Profiles select assets; they do
not describe how those assets were manufactured.

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
- `[assets]`
- VM/resource defaults

Current source file targets:

- `config/settings.toml`
- `config/profiles/code.toml`
- `config/corp.toml`

`config/user.toml.default` was removed because it documented profile-owned AI,
repository, VM, guest-env, and plugin behavior as user settings.

Generated runtime config target:

- `target/config/`

`config/` is checked-in source material and support files. It may contain
templates, sample/default source profiles, corp/settings source files, and rule
files. It must not be hand-mutated to match a local repacked initrd, rootfs, or
kernel.

`target/config/` is the instantiated runtime config for the current build. It
is where the current asset manifest hashes, materialized profile files, copied
rule files, and generated runtime manifests belong. VM smoke, doctor, install,
and benchmark proof for the current build must validate and boot from
`target/config`, not from a manually edited checked-in profile.

Generation rule: `target/config` must be produced by the same `capsem-admin`
and `just` rail used by CI/release. Do not add a local-only patcher. The
accepted rail is the profile-derived admin path behind `just build-kernel`,
`just build-rootfs`, `just build-assets`, `_pack-initrd`, `smoke`, and `test`.

## Profile

Profile identity is first-class. UI labels and icons come from this file; the UI
does not invent them.

```toml
# profiles/coding/profile.toml

id = "coding"
name = "Coding"
description = "Optimized for coding and long-running agents."
icon_svg = "<svg viewBox=\"0 0 16 16\" aria-hidden=\"true\"></svg>"
revision = "2026.06.07.1"
refresh_policy = "24h"

[availability]
web = true
shell = true
mobile = false

[vm]
cpu_count = 6
ram_gb = 8
scratch_disk_size_gb = 32

[assets]
format = "profile-assets.v1"
refresh_policy = "on_profile_refresh"

[assets.arch.arm64.kernel]
name = "vmlinuz"
url = "https://releases.capsem.dev/assets/arm64/vmlinuz"
hash = "blake3:..."
size = 12345678

[assets.arch.arm64.initrd]
name = "initrd.img"
url = "https://releases.capsem.dev/assets/arm64/initrd.img"
hash = "blake3:..."
size = 12345678

[assets.arch.arm64.rootfs]
name = "rootfs.erofs"
url = "https://releases.capsem.dev/assets/arm64/rootfs.erofs"
hash = "blake3:..."
size = 12345678

[assets.arch.x86_64.kernel]
name = "vmlinuz"
url = "https://releases.capsem.dev/assets/x86_64/vmlinuz"
hash = "blake3:..."
size = 12345678

[assets.arch.x86_64.initrd]
name = "initrd.img"
url = "https://releases.capsem.dev/assets/x86_64/initrd.img"
hash = "blake3:..."
size = 12345678

[assets.arch.x86_64.rootfs]
name = "rootfs.erofs"
url = "https://releases.capsem.dev/assets/x86_64/rootfs.erofs"
hash = "blake3:..."
size = 12345678
```

Implementation note: `ProfileAssetConfig` now parses this per-architecture
shape, including only URL/hash/size asset metadata for kernel, initrd, and
rootfs artifacts. `refresh_policy` is a top-level profile field, and asset
refresh is owned by `[assets].refresh_policy`. Build format, compression,
content-type, and signing claims stay out of the profile contract.

## Rule Files

Rule file locations live in profile/corp, not settings. Detection can point at
Sigma YAML. Enforcement/rules use the current TOML rule format.

```toml
[rule_files]
enforcement = "rules/enforcement.toml"
sigma = "rules/detection.yaml"
```

## Default Rules

Default rules are visible rules. They are not a second engine, and they do not
need a `profiles.defaults.default_*` namespace. They are defaults because their
priority is `default`.

```toml
[default.http]
name = "http"
action = "allow"
priority = "default"
reason = "Default allow for HTTP requests."
match = "has(http.host)"

[default.dns]
name = "dns"
action = "allow"
priority = "default"
reason = "Default allow for DNS queries."
match = "has(dns.qname)"

[default.mcp]
name = "mcp"
action = "allow"
priority = "default"
reason = "Default allow for MCP server activity and tool calls."
match = "has(mcp.method) || has(mcp.server.name) || has(mcp.tool_call.name) || has(mcp.tool_list)"

[default.model]
name = "model"
action = "allow"
priority = "default"
reason = "Default allow for model calls."
match = "has(model.provider) || has(model.name) || has(model.request.body) || has(model.response.body) || has(model.request.tool_calls)"

[default.file]
name = "file"
action = "allow"
priority = "default"
reason = "Default allow for file reads, writes, creates, deletes, imports, and exports."
match = "has(file.read.path) || has(file.write.path) || has(file.create.path) || has(file.delete.path) || has(file.import.path) || has(file.export.path) || has(file.content)"

[default.process]
name = "process"
action = "allow"
priority = "default"
reason = "Default allow for process execution and audit activity."
match = "has(process.exec.path) || has(process.command) || has(process.exec.id)"

```

## Profile Rules

Enforcement rules live in the referenced enforcement file, not inline in the
profile. This is the current rule format. Do not change it during restore.

```toml
[profiles.rules.block_untrusted_dns]
name = "block_untrusted_dns"
action = "block"
detection_level = "high"
reason = "Block known untrusted DNS requests."
match = 'dns.qname.matches("(^|.*\\.)evil.example$")'
```

Detection rules live in the referenced Sigma YAML file. Do not add detection
rules just to observe ordinary AI traffic.

```yaml
title: skill_loaded
level: informational
logsource:
  product: capsem
  service: security_event
detection:
  selection:
    file.read.name: SKILL.md
    file.read.ext: md
  condition: selection
capsem:
  action: allow
  reason: Record when an agent skill file is loaded.
```

## AI Provider Status

Do not add static `[ai.*]` provider metadata to the canonical profile. A bare
block that says OpenAI, Anthropic, Gemini, or Ollama exists does not say whether
that provider is allowed, blocked, configured, credentialed, routed, or actually
observed. That is theater.

Provider-scoped rules are valid only as a single rule for that provider. Do not
split provider behavior into a bag of small rules that must be reconciled later.

```toml
[ai.openai.rule]
name = "openai_api_requests"
action = "allow"
priority = 10
reason = "Allow OpenAI API requests for this profile."
match = 'http.host.matches("(^|.*\\.)openai\\.com$")'
```

That rule is the control plane for the provider. It says whether matching
provider activity is allowed, blocked, or asked, and how detection is recorded.
It does not mean credentials exist or the provider is configured.

Provider state must be computed from first-party truth:

- enforcement rules say whether traffic is allowed, blocked, or asked;
- detection/Sigma rules say what should be reported;
- credential broker plugin runtime status says which opaque brokered credential
  references exist;
- runtime security events say what actually happened.

If Ollama or a custom OpenAI-compatible endpoint needs host routing, that is a
profile-owned network route once the routing rail exists. It is not
`listen_ports` inside an AI metadata block.

No raw credentials are exposed in rule matches. Credential broker logs/reporting
use BLAKE3 references.

## Plugins

Plugins live in profile/corp. Plugin config governs whether the plugin is
enabled, how it behaves, and what event/filter scope it owns. Do not also add a
CEL rule just to invoke the same plugin. Rules remain for enforcement/detection
policy; plugins own their own filtering and materialization hooks. For the
credential broker, the plugin owns its HTTP-boundary materialization hook
internally. The plugin contract is frozen for this sprint.

Reasoning: if the behavior can be expressed as a CEL/Sigma rule, it should be a
rule. A plugin exists only for work a rule cannot do by itself: mutation,
materialization, external scanning, credential substitution, protocol-specific
rewrites, or other side effects with their own audited contract.

```toml
[plugins.credential_broker]
mode = "rewrite"
detection_level = "informational"
```

Profile/corp config tracks plugin policy and plugin-specific config. The plugin
object/registry owns display, lifecycle, benchmark, status, and capability
metadata so the UI reflects the plugin, not duplicated profile copy.

Plugin object contract:

| Field | Contract |
|---|---|
| `id` | Stable lowercase plugin id, used as the config key. |
| `version` | Semver plugin implementation version. It is emitted in profile plugin lists, VM plugin status, logs, and benchmark output. |
| `name` | Human-readable plugin name supplied by the plugin registry, not profile config or UI. |
| `description` | Plugin-owned description supplied by the plugin registry. |
| `info` | Plugin-owned details for UI/help/status surfaces. |
| `stages` | Ordered execution stages, using typed enum values such as `pre_decision`, `post_decision`, and `runtime_status`. This tells operators whether the plugin can mutate before CEL enforcement, after CEL enforcement, or only report status. |
| `mode` | `disable`, `allow`, `ask`, `block`, or `rewrite`. |
| `detection_level` | Default plugin detection level when enabled. |
| `scope` | Plugin-owned filter/scope config. CEL rules do not invoke plugins. |
| `status_schema` | Plugin-owned VM status shape for UI rendering. |
| `stats_schema` | Plugin-owned counters shape for UI rendering. |
| `performance_counters` | Required plugin runtime counters: invocation count, match/skip count, mutation count, allow/ask/block/rewrite count, error count, total latency, p50/p95/p99 latency, max latency, and per-stage latency. Counters live in memory for VM status and can be exported to benchmark/debug sinks. |
| `benchmark` | Plugin-owned benchmark spec: stable benchmark id, fixture/event corpus, measured metrics, and budgets. `capsem-bench` must be able to discover and run these specs without the UI inventing benchmark behavior. |
| `supports` | Declared capabilities such as `add`, `edit`, `delete`, `reload`, `status`, and `stats`. |

### Plugin Runtime Routes

Profile routes expose intended plugin configuration. VM routes expose runtime
truth and stats. The UI must not infer credential/provider state from AI config
or rule files; it must query the plugin runtime routes for the VM it is showing.
VM status/info surfaces must include the active plugin list and plugin health
from an in-memory runtime snapshot. They must not perform session DB reads on
the hot path. DB-backed latest/forensic routes remain separate ledger queries.
Plugin status/stats must include enough performance counters to identify
whether latency came from plugin filtering, plugin mutation/materialization,
CEL evaluation, logging enqueue, or downstream runtime work.

| Endpoint | Method | Contract |
|---|---|---|
| `/profiles/{profile_id}/plugins/list` | `GET` | List profile plugin config plus registry-owned name, description, info, schema, and capabilities. No runtime counters. |
| `/profiles/{profile_id}/plugins/add` | `POST` | Add one profile plugin config object after validating the plugin id and object schema. |
| `/profiles/{profile_id}/plugins/{plugin_id}/info` | `GET` | Inspect one profile plugin config object. |
| `/profiles/{profile_id}/plugins/{plugin_id}/edit` | `PATCH` | Edit profile plugin config where user-owned policy allows it. |
| `/profiles/{profile_id}/plugins/{plugin_id}/delete` | `DELETE` | Remove one profile plugin config object where user-owned policy allows it. |
| `/profiles/{profile_id}/plugins/reload` | `POST` | Reload profile plugin config and publish it to affected VM runtimes. |
| `/vms/{vm_id}/info` | `GET` | Return VM configuration/runtime info, including active plugin descriptors, versions, modes, stages, health, and last in-memory status snapshot. No DB reads. |
| `/vms/{vm_id}/status` | `GET` | Return hot-path VM liveness/readiness counters from memory, including active plugin health summaries. No DB reads. |
| `/vms/{vm_id}/plugins/list` | `GET` | List plugins active in one VM with descriptor metadata, version, stages, runtime health, and aggregate in-memory performance counters. |
| `/vms/{vm_id}/plugins/{plugin_id}/status` | `GET` | Return one plugin's VM-scoped in-memory runtime status, performance counters, last error, last security event id, version, and stage health. No DB reads. |
| `/vms/{vm_id}/plugins/{plugin_id}/stats` | `GET` | Return plugin-owned performance counters for one VM, including per-stage latency and error counts. |
| `/vms/{vm_id}/plugins/{plugin_id}/reload` | `POST` | Ask one VM runtime to reload one plugin's runtime state when the plugin supports reload. |

Credential broker status is intentionally opaque. It may report counts,
brokered BLAKE3 references, last use timestamps, last event ids, and health. It
must not expose raw credentials or pretend there is an AI-provider broker.

### Security Engine Performance Counters

The security engine must expose in-memory counters alongside plugin counters so
latency attribution is possible:

- CEL compile count, compile error count, total/percentile compile latency, and
  rule count per profile generation.
- CEL evaluation count, matched-rule count, no-match count, error count,
  total/p50/p95/p99/max evaluation latency, and latency by event family/type.
- Security engine stage counters for pre-plugin time, CEL evaluation time,
  post-plugin time, decision selection time, detection append time, logging
  enqueue time, and total boundary time.
- Rule hot counters: per-rule match count, detection count, block/ask/allow
  count, and latency contribution when measurable.

These counters are debug/benchmark local truth. They must be available from
in-memory status/stats surfaces without reading `session.db`. Ledger rows remain
for forensic truth after the fact.

## MCP

MCP is profile-owned. The current code has a real built-in local server, but it
is partly injected outside the profile:

- `guest/config/mcp/local.toml`
- `config/defaults.toml` `[mcp.local]`
- `crates/capsem-mcp-builtin/src/main.rs`
- `crates/capsem-core/src/mcp/builtin_tools.rs`

The built-in server is `local`, transport `stdio`, command
`/run/capsem-mcp-server`, and it exposes:

- `echo`
- `fetch_http`
- `grep_http`
- `http_headers`
- `snapshots_changes`
- `snapshots_list`
- `snapshots_revert`
- `snapshots_create`
- `snapshots_delete`
- `snapshots_history`
- `snapshots_compact`

Target profile shape:

```toml
[mcp]
health_check_interval_secs = 60

[mcp.servers.local]
name = "Local"
description = "Built-in local tools: HTTP fetch and workspace snapshots."
transport = "stdio"
command = "/run/capsem-mcp-server"
builtin = true
enabled = true
```

Do not model the built-in server as `http://127.0.0.1:9000`, and do not add
fake `read_file`/`write_file` tool definitions. Tool discovery comes from the
server catalog/cache. Per-tool enable/disable/edit is addressed by
profile-scoped MCP endpoints under the real server id.

Restore invariant:

- profile owns real MCP server configuration, including `mcp.local`;
- server-owned tools/resources/prompts live under that server;
- decisions are ordinary security rules over MCP security events;
- no `McpPolicy`/decision provider rail exists;
- no hidden `build_server_list_with_builtin()` injection that bypasses profile
  ownership remains.

## Tool Config Sources

Do not put `tool_config_sources` in the static profile. They are observed
runtime evidence: a tool config file was seen at a guest path, parsed, hashed,
and linked to brokered credential references. The values cannot be known before
the VM runs, and fake BLAKE3 placeholders are worse than empty config.

Expose observed tool config sources through profile/session status and the
security ledger, backed by real hashes and credential references emitted by the
broker/runtime path.

## Corp

Corp owns constraints and reporting endpoints. It can reference rule files and
Sigma files. Corp priorities may be negative; profile/user rules do not get
negative priorities.

Corp source implies corporate ownership/lock. Do not add `corp_locked = true`
inside corp rules. Do not use `priority = "default"` for corp rules; that string
means the profile/built-in fallback priority. The current contract reserves corp
priorities as `-1000..=-10` and profile/user priorities as `10..=1000`.

```toml
# /etc/capsem/corp.toml

refresh_policy = "24h"

[corp_rule_files]
enforcement = "corp/enforcement.toml"
sigma = "corp/detection.yaml"
sigma_output_endpoint = "https://siem.example.invalid/capsem/sigma"
open_telemetry = "https://otel.example.invalid/v1/traces"
remote_enforcement = "https://security.example.invalid/capsem/enforcement"

[plugins.credential_broker]
mode = "rewrite"
detection_level = "informational"
```

```toml
# /etc/capsem/corp/enforcement.toml

[corp.rules.block_evil_example]
name = "block_evil_example"
action = "block"
priority = -100
detection_level = "high"
reason = "Example corp rule proving negative-priority enforcement from corp source."
match = 'http.host.matches("(^|.*\\.)evil\\.example$")'
```

Keep the sample corp rule set intentionally small. We only need one rule to
prove corp-file loading, negative priority, and source ownership.
