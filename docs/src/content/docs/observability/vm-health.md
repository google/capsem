---
title: VM Health
description: Live VM status, Security Engine counters, model/provider/cost fields, and OTel boundaries.
sidebar:
  order: 1
---

VM health is a live typed summary, not a raw SQL view. `capsem-process`
maintains in-memory counters from accepted resolved security events. Persistent
VMs seed/recompute from `session.db` once at load time; hot status reads do not
scan SQLite.

## Fields

| Category | Examples |
|---|---|
| Profile | `profile_id`, `profile_revision`, `profile_status`, package/asset pin state. |
| HTTP/DNS/MCP | request counts, denied counts, MCP calls, DNS queries. |
| Model | provider/model, model call count, input/output tokens, estimated cost. |
| File/process | file event count, process event count, exec count. |
| Security | total security events, enforcement decisions, blocks, detection findings, latest block, latest detection. |

Host-owned AI calls can correlate with a VM/session/profile for explanation,
but they charge host/service counters, not VM counters.

## Surfaces

- `capsem status --json`
- `capsem list` and `capsem info`
- gateway `/status`
- service `/info/{id}`
- Settings -> Policy and Sessions UI panels
- future `/metrics` and OpenTelemetry exporters

## OTel Rules

Metrics use bounded labels: profile id, profile revision, event family,
decision, provider, model, rule id where cardinality is controlled. Full local
evidence stays in timeline/backtest/hunt/session APIs, not in metric labels.

Rate-limit and budget enforcement is reserved for S22. The bedrock release
exposes the quota dimensions and counters needed for that later sprint; it does
not claim budget enforcement.

