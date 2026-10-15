---
title: Telemetry reference
description: What activity does Capsem record and how can I query or export it?
---

Each VM has a ledger for commands, model and MCP activity, network observations,
file activity, security decisions, and detections. The log writer owns that
ledger and its body archive; clients query through the gateway.

For a procedure, see [Investigate a VM](/guides/enterprise/investigate-a-vm/).

## Records and correlation

| Record family | Information | Limit |
| --- | --- | --- |
| Structured exec | Command, output, exit status, timing. | Commands sent through the execution interface. |
| Guest process/audit | Observed process activity and attribution. | Observation can arrive after execution; it cannot prevent it retroactively. |
| HTTP / DNS | Parsed destinations, requests/responses, timing, decisions. | Supported inspected traffic only. |
| Model | Parsed provider/model messages, usage, related requests and tool activity. | Protocol parsing and retained body completeness apply. |
| MCP | Server/tool identity, arguments/results, decision and duration. | Tool policy and transport bounds apply. |
| Files | Guest filesystem observations and related process/trace context. | Retained observations do not prove every possible filesystem operation. |
| Security | Event, matched rules, effective decision, plugin evidence, detections. | Events that cannot be admitted to required audit storage are refused at enforceable boundaries. |
| Private network | Membership, connections, and network decisions. | Separate network audit stream; not HTTP/model inspection of member bytes. |

Trace IDs connect related records. A record without a trace ID is not proof
that it belongs to the selected request. Timeline trace filtering also retains
legacy records without trace IDs.

## Queries

| Interface | Contract |
| --- | --- |
| VM timeline | Layers `exec`, `tool`, `net`, `fs`, `model`; default 200 events, capped at 2000. `since` accepts lookback duration or RFC3339 timestamp. |
| VM history | Structured exec and guest audit records; layer/search/pagination filters. |
| VM statistics | Summary and detailed typed counters; not a replacement for underlying records. |
| Triage | Bounded recent evidence and model totals; not a full ledger export. |
| Host logs | Symbolic source: service, mcp, gateway, tray, app; clients cannot request arbitrary host paths. |
| Network logs | Cursor-based pages with VM, connection, event type, decision, and time filters. |

Log queries filter by a literal `grep` substring before applying `tail`.
`tail=0` returns no lines. `max_bytes` defaults to 100 KiB per host stream
and 5 MiB per VM stream, capped at 5 MiB.

The exact request/response types are generated in the
[Python](/sdk/python/), [TypeScript](/sdk/typescript/), and [Rust](/sdk/rust/) references.

## Archived bodies

`GET /vms/{id}/bodies/{event_id}` returns an event's archived bodies.
Request and response have their own event identities; use a model record's
`response_event_id` to retrieve its full response.

| Field | Meaning |
| --- | --- |
| `event_id`, `source_table`, `direction` | Record and body identity. |
| `content_type` | Observed media type when available. |
| `original_bytes` / `stored_bytes` | Observed and retained sizes. |
| `truncated` | Storage retained less than the original body. |
| `truncated_for_transport` | This response returns less than the retained body. |
| `body_hash` | Recorded body hash. |
| `encoding`, `content` | UTF-8 or base64-encoded body. |

`max_bytes` bounds decoded bytes per returned body and is clamped by the
service. A preview is not the complete body. Check both truncation flags before
using content as complete evidence.

## Export and reporting

`GET /vms/{id}/bodies/export.warc.gz` exports retained bodies as compressed
WARC. This is a body archive export, not a universal export of all ledger tables.

Corporate configuration has Sigma output and OpenTelemetry endpoints.
Their presence does not imply a generic OCSF or SIEM export schema.
Use the configured reporting contract; retain the VM ID, trace/event IDs,
policy revision, and completeness flags needed for your investigation.

## Integrity and retention

The service anchors producer commitments outside the confined ledger directory.
Verification detects alteration, substitution, omission, reordering, and stale
generation reuse in checkpointed records. A compromised producer can still lie
before committing an observation; a crash can lose an unanchored tail.

Accepted writes and durable checkpoints are different states. The logger owns
batching, flush barriers, SQLite WAL, archives, and query freshness.
Stopped retained VMs remain inspectable through granted reader channels.
Deleting a VM removes its retained state and retires its commitments.
