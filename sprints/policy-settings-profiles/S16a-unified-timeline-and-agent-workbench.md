# S16a - Unified Timeline And Agent Workbench

## Status

Not started. Inserted after S08b because the UI must consume the canonical
structured timeline/security event contracts rather than direct SQL over legacy
telemetry tables.

## Placement

Runs after:

- S08a decides real CEL, Sigma-compatible detection packs, detection IR,
  finding shape, and profile-owned policy/detection pack pins.
- S08b defines the Security Engine and canonical resolved-event store with
  enough event ids, links, and journal facts for this sprint to build on.
- S16a owns the Conversation Engine, structured `/timeline/{id}` API, SDK
  adapters, timeline tables, and agent workbench UI.
- S15 lands production ask/confirm semantics.

Runs before final docs/release gate. It may run near S16/S17, but it is not
the same sprint as Profile UI.

## Goal

Make Capsem usable for everyday agent work, not just forensic inspection, using
one structured timeline endpoint.

This sprint absorbs the still-useful deep-dive ideas from the retired
`analytics-dashboard`, `better_stats`, and `forensics` boards: conversation
review, session evidence, raw-event drilldown, and timeline search. It must
rebuild them on the S08b canonical resolved-event journal instead of reviving
old direct-SQL dashboard endpoints.

The UI should expose:

- an agent workbench for Codex/Claude SDK-backed sessions when those adapters
  are enabled;
- terminal fallback for existing CLI-based workflows;
- a searchable timeline that can render conversation-grouped, chronological,
  findings, and artifact views from the same JSON response model.

## Product Model

The user should be able to open a VM/session and see:

- live agent conversation;
- security/detection annotations inline with the relevant turn;
- generated/modified files and snapshots linked to the turn that caused them;
- tool calls and MCP calls linked to model messages;
- network/process/file events linked as expandable evidence;
- search across prompts, responses, commands, files, tools, detections, and
  findings;
- timeline modes for forensic ordering and conversation grouping, driven by
  client-side filters/renderers over paginated `/timeline/{id}` blocks.

## Architecture Dependencies

This UI consumes the single structured timeline API:

```text
GET /timeline/{id}?cursor=<cursor>&limit=<n>&direction=<dir>&...
```

The API must provide stable pagination over typed timeline blocks:

- `cursor`, `limit`, and `direction=forward|backward`;
- optional `anchor_event_id`, `since`, and `until` for bounded reads;
- response metadata: `next_cursor`, `prev_cursor`, `has_more`, and a read
  watermark;
- stable block ids so the UI can merge streaming updates and paged history.

Client-side filtering/rendering modes should include:

- chronological evidence;
- conversation or turn review;
- process, activity, and trace views;
- findings and artifacts views;
- layer filters for security, conversation, file, network, process, model/MCP,
  snapshot, ask/confirm, and profile provenance.

The API returns a paginated JSON envelope containing typed timeline block
elements backed by:

- `security_events`;
- `security_event_steps`;
- `security_event_links`;
- `detection_findings`;
- `policy_results`;
- `confirm_results`;
- `timeline_threads`;
- `timeline_elements`;
- `timeline_artifacts`;
- domain projections only as compatibility/read-model helpers.

The UI must not run arbitrary direct SQL against legacy domain tables as its
primary data model. S16a owns the structured `/timeline/{id}` endpoint and
builds it from the canonical resolved-event journal produced by S08b.

Each timeline element family needs a dedicated block renderer. Rendering must
consume typed fields from the JSON block, not parse prose labels. The client may
filter and format the loaded page window locally, while asking the server only
for the next/previous page or a bounded read window.

## SDK Adapter Requirement

The UI can use Codex and Claude SDKs when available, but SDK integration is an
adapter contract, not the storage source of truth.

Expected shape:

```text
Codex/Claude SDK event stream
-> Conversation Engine adapter
-> ConversationSecurityEvent / structured timeline element
-> Security Engine / emitter
-> session.db resolved-event store + /timeline JSON
```

Terminal-only workflows still work:

```text
PTY transcript + model/tool/process telemetry
-> Conversation Engine fallback normalizer
-> structured timeline elements
```

SDK adapters must support:

- stable conversation/thread ids;
- user/assistant/tool/system message roles;
- tool proposal/result events;
- artifacts and file patches;
- streaming deltas without duplicating final messages;
- redaction before durable storage;
- graceful fallback when the SDK is missing or changes shape.

## UI Requirements

- Add a `Timeline` VM view next to terminal/stats/logs/files.
- Provide client-side filters for conversation, findings, files, tools,
  network, processes, snapshots, asks/confirms, and profile/rule provenance.
- Add block renderers for user message, assistant message, tool call/result,
  file change, process event, network/DNS event, MCP/model event, detection
  finding, ask/confirm, snapshot, artifact, profile/rule provenance, and
  unresolved/corrupted event blocks.
- Search should cover content plus metadata without leaking redacted payloads.
- Event detail panels must show the resolved-event journal: preprocessors,
  enforcement, ask/confirm, detection, postprocessors, emitter delivery.
- Findings appear inline and in a findings list.
- User can jump from conversation entry -> file artifact -> snapshot -> event
  evidence -> rule/finding.
- Export should produce a support-bundle-safe timeline with redaction state.

## Testing Matrix

- Unit/contract: conversation API types, SDK adapter event mapping, redaction,
  search query serialization, and UI state reducers.
- Functional: mock SDK stream creates ordered paginated timeline blocks with
  artifacts and event links; client filters and block renderers produce each
  required view from the same loaded page data.
- Adversarial: duplicated streaming deltas, missing SDK fields, malformed tool
  calls, redacted secrets, very large messages, and stale event links.
- E2E/VM: run a real terminal fallback workflow and at least one SDK-backed
  workflow when SDK support lands; verify `/timeline/{id}` pagination, client
  filtering, block rendering, search, and event linkage.
- Telemetry: timeline elements and security events agree on ids, timestamps,
  profile id, VM id, user id, and finding links.
- Performance: search latency, cursor pagination, and large-session rendering;
  no direct SQLite scans on hot UI refresh paths.

## Done Means

- Users have a friendly review/search UI for the full session narrative.
- The UI is backed by the S16a paginated structured `/timeline/{id}` API built
  from the S08b canonical resolved-event journal, not ad hoc direct SQL or a
  separate conversation endpoint.
- Filtering and formatting are client-side over typed timeline blocks, with one
  renderer per block family.
- Codex/Claude SDK adapters are either implemented or explicitly gated with a
  terminal fallback.
- Security findings, policy decisions, asks/confirms, and artifacts are
  explainable from the conversation view.
