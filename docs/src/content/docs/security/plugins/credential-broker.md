---
title: Credential Broker Plugin
description: Built-in Capsem security plugin for brokered credential capture.
---

Plugin id: `credential_broker`

Version: supplied by the plugin registry descriptor and emitted in profile
plugin lists, VM plugin status, logs, and benchmark output.

Stage: plugin-owned HTTP-boundary materialization. CEL rules do not invoke the
credential broker.

Stages:

- `pre_decision`: capture and substitute brokered references before CEL
  enforcement sees the materialized boundary.
- `runtime_status`: report opaque broker state and health from memory.

Config:

```toml
[plugins.credential_broker]
mode = "rewrite"
detection_level = "informational"
```

Inputs: outbound HTTP boundaries, remote MCP auth boundaries, plus
plugin-owned broker state. Raw credentials remain private to the broker and are
not exposed as CEL fields.

Mutation: stores observed credentials through the broker and writes the brokered `credential:blake3:*` reference back onto the event.

MCP contract: remote MCP server config may carry only brokered auth metadata:

```toml
[mcp.servers.remote.auth]
kind = "oauth" # or "bearer"
credential_ref = "credential:blake3:..."
```

The broker owns OAuth/API-key material and resolution. MCP TOML must not store
raw `bearer_token`, `bearerToken`, `Authorization`, `X-Api-Key`, refresh tokens,
or access tokens.

Decision: plugin policy can request `allow`, `ask`, `block`, or `rewrite`; `rewrite` keeps the effective decision at `allow` while recording mutation intent.

Status contract: credential state is opaque and VM-scoped. The UI must query
`/vms/{vm_id}/plugins/credential_broker/status` or
`/vms/{vm_id}/plugins/credential_broker/stats`; it must not infer credential
state from AI/provider config. VM `info` and `status` include the active
credential broker descriptor, version, stage health, and last in-memory status
snapshot without reading `session.db`.

Benchmark contract: the plugin descriptor owns a stable benchmark spec for
capture, substitution, failed materialization, and status snapshot overhead.
Benchmarks must report plugin id, version, stage, event count, latency, and
mutation count.

Detection contract: enabled executions append one `SecurityDetectionEvent` to `SecurityEvent.detections` with `source = "plugin"`, the configured `detection_level`, plugin id, plugin mode, and reason.

Failure: broker storage errors abort broker materialization and the event is not
emitted by the security engine.

Tests must prove capture, BLAKE3 reference logging, rewrite mutation, VM-scoped
status/stats, and failure without raw credential leakage.
