# T0: Observability and Local Lab Contract

## Principle

This sprint uses standard observability technology. We do not add a bespoke timing ledger.

Implementation should use:

- `tracing` spans for structured execution boundaries.
- OpenTelemetry-compatible metrics/spans for debug and benchmark collection.
- Local-only export by default.

## Debug-Only Export Contract

Allowed:

- local JSON benchmark artifacts.
- local process logs.
- local debug endpoint gated behind dev/test mode.
- in-memory collector used by tests.

Forbidden by default:

- exporting benchmark/debug spans to an upstream OTEL collector.
- including raw secrets in any span field, metric label, log field, or benchmark artifact.
- adding high-cardinality labels such as full URL, raw path with secrets, request body, response body, or token values.

## Span Contract

Span names are stable public debug contract. Do not rename without updating
tests, docs, benchmark parsers, and saved artifacts.

### MITM and Network Spans

| Span | Parent | Required labels | Purpose |
| --- | --- | --- | --- |
| `capsem.mitm.request` | request root | `protocol`, `body_kind`, `status`, `decision` | One guest request from first readable bytes through final guest response/error. |
| `capsem.mitm.vsock_classify` | `capsem.mitm.request` | `protocol`, `status` | Initial protocol classification and first-buffer handling. |
| `capsem.mitm.tls_guest_handshake` | `capsem.mitm.request` | `protocol`, `status`, `error_kind` | Guest-facing TLS termination. |
| `capsem.mitm.policy.request` | `capsem.mitm.request` | `protocol`, `rule_count`, `decision`, `status` | Request-side policy/security-rule evaluation. |
| `capsem.mitm.security_actions` | `capsem.mitm.request` | `rule_count`, `action_count`, `status`, `error_kind` | Matched preprocess/postprocess action plugin execution. |
| `capsem.mitm.model.request_policy` | `capsem.mitm.request` | `provider`, `body_kind`, `rule_count`, `decision`, `status` | Model request body parse and security-rule evaluation. |
| `capsem.mitm.upstream.prepare` | `capsem.mitm.request` | `protocol`, `provider`, `status`, `error_kind` | Upstream request materialization after security actions. |
| `capsem.mitm.upstream.send` | `capsem.mitm.request` | `protocol`, `provider`, `status`, `status_class`, `error_kind` | Upstream connect/send/receive, including dial and TLS where applicable. |
| `capsem.mitm.policy.response` | `capsem.mitm.request` | `protocol`, `rule_count`, `decision`, `status` | Response-head policy/security-rule evaluation. |
| `capsem.mitm.model.response_policy` | `capsem.mitm.request` | `provider`, `body_kind`, `rule_count`, `decision`, `status` | Model response body parse and security-rule evaluation. |
| `capsem.mitm.body.chunk_hooks` | `capsem.mitm.request` | `protocol`, `body_kind`, `hook_count`, `status`, `error_kind` | Chunk hook aggregate timing for decompression/SSE/model hooks. |
| `capsem.mitm.websocket` | `capsem.mitm.request` | `protocol`, `status`, `close_kind`, `error_kind` | WebSocket upgrade and frame relay once T1/T4 make it first-class. |
| `capsem.mitm.telemetry.emit` | `capsem.mitm.request` | `event_type`, `event_family`, `status` | Primary telemetry/security-event handoff. |

### Security Event and DB Spans

| Span | Parent | Required labels | Purpose |
| --- | --- | --- | --- |
| `capsem.security_event.emit` | producer span or request root | `event_type`, `event_family`, `status`, `error_kind` | Canonical security-event emission boundary. |
| `capsem.security_rule.evaluate` | `capsem.security_event.emit` | `event_type`, `event_family`, `rule_count`, `decision`, `status` | Rule matching over one `SecurityEvent`; no callback fan-out labels. |
| `capsem.db.enqueue` | producer span | `event_type`, `event_family`, `status`, `queue_result` | Queue send/wait before the single DB writer. |
| `capsem.db.write_batch` | DB writer task | `batch_size_bucket`, `status`, `error_kind` | SQL batch execution for writer-owned rows. |
| `capsem.db.shutdown_flush` | DB writer shutdown | `status`, `batch_size_bucket`, `error_kind` | Final flush/checkpoint before process exit. |

### Launch Spans

| Span | Parent | Required labels | Purpose |
| --- | --- | --- | --- |
| `capsem.launch.service` | launch root | `status`, `error_kind` | Service process startup and service socket readiness. |
| `capsem.launch.gateway` | launch root | `status`, `error_kind` | Gateway startup and gateway readiness. |
| `capsem.launch.process_spawn` | launch root | `status`, `error_kind` | Per-VM capsem-process spawn. |
| `capsem.launch.vm_boot` | `capsem.launch.process_spawn` | `status`, `boot_mode`, `rootfs_kind`, `error_kind` | VM provision/boot/restore timing. |
| `capsem.launch.vsock_ready` | `capsem.launch.vm_boot` | `status`, `boot_mode`, `error_kind` | First control/terminal vsock readiness. |
| `capsem.launch.first_network_ready` | `capsem.launch.vm_boot` | `status`, `protocol`, `error_kind` | First usable network request inside the VM. |

### Allowed Label Values

Labels must stay low-cardinality. Use enums and buckets, not user data.

| Label | Allowed values |
| --- | --- |
| `protocol` | `http`, `https`, `websocket`, `mcp`, `dns`, `file`, `process`, `model`, `unknown` |
| `event_type` | `RuntimeSecurityEventType::as_str()` only |
| `event_family` | `RuntimeSecurityEventFamily::as_str()` only |
| `decision` | `none`, `allow`, `ask`, `block`, `preprocess`, `postprocess`, `error` |
| `status` | `ok`, `error`, `denied`, `cancelled`, `timeout` |
| `status_class` | `none`, `1xx`, `2xx`, `3xx`, `4xx`, `5xx`, `error` |
| `provider` | settings-defined provider id or `none`; never endpoint URL |
| `body_kind` | `empty`, `tiny`, `10kb`, `1mb`, `10mb`, `gzip`, `sse`, `websocket`, `stream`, `unknown` |
| `rule_count` | exact integer when cheap; bucket as `0`, `1`, `2-5`, `6-20`, `20+` in metrics |
| `action_count` | exact integer when cheap; bucket as `0`, `1`, `2-5`, `6+` in metrics |
| `hook_count` | exact integer when cheap; bucket as `0`, `1`, `2-5`, `6+` in metrics |
| `queue_result` | `enqueued`, `full`, `closed`, `timeout`, `error` |
| `batch_size_bucket` | `0`, `1`, `2-10`, `11-100`, `101-1000`, `1000+` |
| `error_kind` | `none`, `parse`, `policy`, `action`, `upstream`, `tls`, `dns`, `db`, `timeout`, `io`, `cancelled`, `unknown` |
| `boot_mode` | `cold`, `resume`, `fork`, `unknown` |
| `rootfs_kind` | `erofs`, `squashfs`, `unknown` |
| `close_kind` | `normal`, `policy`, `peer`, `timeout`, `error`, `unknown` |

Forbidden labels/fields also include raw hostnames and paths when those values
can come from users, prompts, tools, credentials, query strings, or file names.
Use `provider`, `event_type`, fixed endpoint case names, or bounded buckets
instead.

## Local Lab Contract

The local lab must support deterministic tests for:

- small HTTP response.
- fixed-size body response.
- gzip response.
- SSE/model-like stream.
- slow chunked response.
- credential-looking response.
- deny target.
- WebSocket echo.
- WebSocket ping/pong.
- WebSocket close/error.

Each endpoint must be deterministic enough to benchmark across release branches.

## Replacement Contract

Default tests should not rely on public network services. Public network checks should be explicit smoke tests only.

Replace:

- default HTTP throughput benchmark target.
- default proxy throughput benchmark target.
- release-gated diagnostics that hit public websites.

Keep:

- explicit public smoke tests, skipped unless requested.
- provider smoke tests, skipped unless credentials are configured and the user explicitly requests them.

## Boundary Foundation Contract

This is the course-correction contract for model endpoints and VM byte
import/export. Dynamic plugins are intentionally deferred until after 1.3, but
the rails they need are not deferred.

### Model Endpoint Contract

Providers/endpoints are settings-defined data, not hardcoded Rust enum variants.

This is a replacement contract. The old closed provider classification path must
not remain as a parallel production engine.

The settings layer owns:

- endpoint id.
- provider/policy identity.
- protocol parser id.
- guest-facing aliases.
- guest-facing listen/intercept ports.
- upstream URL.
- credential reference or credential slot.
- allowed remote target constraints.
- discovered tool config source indexes.
- provider-owned detection, capture, replace, allow, and block rules.
- compiled rule ids, priorities, actions, and corp-lock metadata.

Provider and credential detection:

- Detection is not a separate matcher schema.
- The profile authoring format places rules under the provider/profile object.
- The target authoring format uses `rule = '<CEL expression>'` everywhere.
  `if = ...` is a legacy/temporary spelling and should be burned.
- The target authoring format does not use explicit `on`. The compiler infers
  runtime callback families from first-party field roots used in `rule`, such
  as `http.*`, `dns.*`, `model.*`, `credential.*`, and `file.*`.
- Credential materialization is also a rule, but it lives under
  `[ai.<provider>.credentials.<slot>]` because the block defines how the
  matched credential is rendered. It does not need `actions`, `storage`, `key`,
  `aliases`, or `surfaces`.
- Corporate profiles may author their own provider-owned rules and mark them
  locked. They do not need to drop down to internal `policy.generated.*`
  stanzas for normal provider control.
- Detection/capture/replace rules compile to action rules over first-party
  security events.
- Enforcement rules compile to `allow`, `ask`, `block`, or `rewrite` rules.
- The engine can normalize provider-owned rules into deterministic internal
  rule ids, but users should not have to author top-level `policy.*` stanzas
  for common provider behavior.
- Rules have deterministic ids, explicit priority, and corp-lock metadata.

Target examples:

```toml
[ai.openai.credentials.primary]
type = "api-key"
header = "Authorization"
prefix = "Bearer "
rule = 'http.host.contains("api.openai.com") || model.provider == "openai"'

[ai.openai.rules.detect_config]
decision = "detect"
rule = 'file.path == "/root/.codex/config.toml"'
priority = 10

[ai.openai.rules.corp_block]
decision = "block"
rule = 'http.host.contains("api.openai.com") || model.provider == "openai"'
priority = -10
corp_locked = true
```

Compiler requirements:

- Infer event families from `rule` and register the rule against each supported
  runtime callback family.
- Reject rules whose field roots cannot be mapped to a first-party security
  event family.
- Keep explicit callbacks as an internal compiled representation only.

Corporate policy may:

- disable a provider entirely.
- disable an endpoint alias, listen port, remote host, model family, or tool.
- lock provider-owned detection/action rules so user discovery cannot mutate
  them.
- lock generated deny/allow/action rules so user settings cannot override them.

Provider discovery owns settings patches:

- Credential observations, OAuth/token exchanges, and recognized VM tool config
  files may create or update endpoint records, credential refs, and config
  source index metadata.
- Discovery must check corp-locked detection/policy records before auto-adding
  or enabling a provider.
- Discovery writes through the broker/settings path only.
- Discovery emits security/substitution events so auto-added settings can be
  audited.
- Discovery must not become a second setup state machine.

The UI owns presentation only:

- configured providers.
- detected provider observations.
- brokered credential references and status.
- endpoint routing and discovered tool config sources.

The UI must not own provider truth, raw credential collection, or an onboarding
provider wizard.

Tool config source index records:

- id.
- tool id, such as `codex`, `claude`, `gemini`, or future local tools.
- guest path.
- format, such as TOML, JSON, env, or opaque text.
- owning endpoint id when applicable.
- credential references discovered in or associated with the config.
- observed hash/version.
- allowed Capsem overlays, such as MCP injection, telemetry/update disablement,
  or broker-reference placeholder patching.

The tool/user config file is the source of truth for tool-specific config.
Settings must not contain a second full rendered copy of the file. Config source
indexes support discovery, audit, and narrow Capsem overlays; they do not
classify provider identity by themselves, and settings must not contain raw
credential material.

The adapter registry owns protocol parsing:

- OpenAI wire format.
- Anthropic wire format.
- Google wire format.
- Ollama wire format.
- future compatible/custom wire formats.

The security-event layer owns routing/materialization:

- classify request against model endpoint settings.
- attach endpoint/provider/protocol identity to the security event.
- run CEL/Sigma rules and registered actions.
- materialize the upstream request from the final post-action security event.
- emit canonical `model.call` rows and related security events.

Litmus cases:

- Ollama on `local.ollama` or guest `:11434` routes to configured host/remote upstream without core provider enum edits.
- A custom OpenAI-compatible endpoint uses `protocol = "openai"` and a custom provider identity from settings.

Burn requirements:

- No production MITM/policy/telemetry path should match provider enum variants
  to decide endpoint identity.
- No new provider should require editing a central enum.
- Existing built-in providers should be represented as default endpoint
  settings plus built-in protocol adapters.
- Built-in providers should also ship default detection/action rules and
  enforcement rules, not hardcoded MITM checks.
- Corp-disabled providers must remain disabled even when auto-discovery sees
  credentials or config files for them.
- Old setup/onboarding provider screens must not be required for a provider to
  become usable.
- Direct provider API-key settings must not be the only path; discovered
  brokered credentials/configs can create or update provider settings.
- Config materialization must emit first-party security/file events and must
  not store raw credentials or rendered config content in settings.

### VM Byte Boundary Contract

All external bytes in or out of the VM/workspace boundary must become
first-party security events before they cross the boundary when enforcement is
possible.

Canonical boundary roots:

- `file.import`
- `file.export`
- `http.request`
- `http.response`
- `dns.query`
- `dns.response`
- `model.call`
- `mcp.tool_call`
- `process.exec`

Security rail:

```text
parser/materializer builds SecurityEvent
rules match SecurityEvent
actions may enrich/mutate SecurityEvent
enforcement decides
boundary operation happens only after allow
logger records event/action/decision
```

Required fields for byte-carrying boundary events:

- source or destination class, such as `service.upload`, `api.write_file`,
  `mcp.restore`, `mitm.http`, `model.endpoint`, or `download.export`.
- normalized path or endpoint identity when applicable.
- size when known.
- content hash when bytes are available.
- body/content reference when bytes must not be copied into logs.
- process/session/trace context when available.
- action results, such as future private VT or PII scanner verdicts, as
  reference-safe structured metadata.

Burn requirements:

- `fs_monitor` is audit/reconciliation only. It must not be the sole
  enforcement proof for incoming bytes.
- Service upload, API/CLI `write_file`, restore/snapshot, download/export, and
  MITM materialization paths must not bypass the security-event gate.
- Future private VT scanning and PII scanning must attach as actions over these
  events instead of creating a second scanner engine.
