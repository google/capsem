# S08 Side Sprint - Canonical AI Interaction Evidence

## Status

Proposed as a pre-S08b foundation side sprint. This is intentionally a side
document rather than another numbered board item: S08b remains the active engine
implementation sprint, but S08b must not harden model/MCP enforcement,
detection, telemetry, quotas, timeline, or plugin contracts against thin
provider-specific parser summaries.

## Purpose

Before S08 policy, CEL, Sigma, backtest, telemetry, and future quota behavior
lock in, Capsem needs a canonical internal evidence model for AI traffic. The
gap is not basic model parsing; the current code already extracts useful model,
stream, usage, text, tool-call, and tool-result summaries. The gap is that the
Security Engine needs a rich, durable, provider-neutral representation of:

- model requests;
- model responses;
- model-emitted tool calls;
- tool results returned to the model;
- MCP tool executions linked to the model/tool lifecycle.

This is policy substrate, not product UI and not generic gateway work.

## Decision

Capsem will add a canonical AI interaction evidence layer and project model/MCP
security-event subjects from it. Provider wire parsers remain provider-specific
and rich; policy, detection, telemetry, timeline, backtest, and plugins consume
the canonical evidence projection.

Minimum provider families for the first slice:

- OpenAI Chat Completions;
- OpenAI Responses;
- Anthropic Messages;
- Google/Gemini content parts and function-call/function-response traffic;
- MCP tool calls/results through Capsem's aggregator path.

Bedrock and Vertex-specific variants are not first-slice requirements. They may
be added later as provider adapter coverage without changing the canonical
evidence contract.

## Canonical Evidence Model

Initial internal structs:

```text
ModelInteractionEvidence
  interaction_id
  trace_id
  profile_id
  vm_id
  session_id
  user_id
  provider
  api_family
  model
  request
  response
  tool_calls[]
  tool_results[]
  mcp_executions[]
  usage
  parse_status
  evidence_status
```

```text
ModelRequestEvidence
  request_id
  provider
  api_family
  model
  stream
  system_prompt_preview
  message_count
  tools_declared_count
  raw_shape_version
  unknown_fields_present
```

```text
ModelResponseEvidence
  response_id
  provider_response_id
  stop_reason
  text_preview
  thinking_preview
  content_blocks[]
  usage
  raw_shape_version
```

```text
ModelToolCallEvidence
  tool_call_id
  index
  provider_call_id
  raw_name
  normalized_name
  arguments_raw
  arguments_json
  arguments_status
  origin
  linked_mcp_call_id
  status
  parse_confidence
```

```text
ModelToolResultEvidence
  tool_call_id
  linked_mcp_call_id
  content_kind
  content_preview
  content_json
  is_error
  result_status
  returned_to_model
  parse_confidence
```

```text
McpToolExecutionEvidence
  mcp_call_id
  server_id
  tool_name
  namespaced_tool_name
  transport
  request_arguments_raw
  request_arguments_json
  result_kind
  result_preview
  result_json
  is_error
  latency_ms
  linked_model_interaction_id
  linked_model_tool_call_id
  link_status
```

Typed content blocks are first-class:

```text
AiContentBlock
  Text
  Json
  Image
  File
  ToolUse
  ToolResult
  Reasoning
  CacheMarker
  Redacted
  Unknown
```

## Required Enums

The first implementation must avoid stringly-typed status fields. Required
enums:

- `AiProvider`: `openai`, `anthropic`, `google_gemini`, `unknown`.
- `AiApiFamily`: `openai_chat_completions`, `openai_responses`,
  `anthropic_messages`, `google_gemini_content`, `mcp`, `unknown`.
- `ArgumentsStatus`: `valid_json`, `partial_json`, `malformed_json`,
  `not_json`, `redacted`, `absent`.
- `ParseStatus`: `complete`, `partial`, `malformed`, `unsupported`,
  `redacted`.
- `EvidenceStatus`: `complete`, `partial`, `ambiguous`, `orphaned`,
  `untrusted`.
- `ToolOrigin`: `native_provider_tool`, `mcp_tool`, `local_builtin_tool`,
  `unknown`.
- `LinkStatus`: `linked`, `unlinked_pending`, `orphan_model_tool_call`,
  `orphan_mcp_execution`, `ambiguous`, `not_applicable`.
- `ToolCallStatus`: `proposed`, `executed`, `blocked`, `returned_to_model`,
  `error`, `unknown`.

## Linking Rules

Origin and linkage must prefer real Capsem records over name heuristics.

Primary linkage source:

```text
model/tool evidence <-> MCP aggregator execution records
```

Fallback heuristics such as namespaced tool-name parsing are allowed only when
the real aggregator linkage is absent, and they must set explicit confidence
and `LinkStatus` values.

The evidence must answer:

- which model requested the tool call;
- which provider/API shape produced it;
- what provider call id/index/name/arguments were emitted;
- whether arguments were valid, partial, malformed, absent, or redacted;
- whether the call was executed, blocked, orphaned, or pending;
- which MCP server/tool handled it when known;
- what result came back and whether it was returned to the model;
- how confident Capsem is about parsing and linkage.

## Security Event Projection

The Security Engine keeps stable policy-facing subjects, but model/MCP subjects
are projected from canonical evidence, not raw provider JSON and not today's
thin parser summaries.

Policy-facing fields include:

```text
model.provider
model.api_family
model.name
model.stream
model.tool_calls[].name
model.tool_calls[].origin
model.tool_calls[].arguments
model.tool_calls[].arguments_status
model.tool_calls[].linked_mcp_call_id
model.tool_results[].is_error
model.usage.input_tokens
model.usage.output_tokens
mcp.server_id
mcp.tool_name
mcp.arguments
mcp.result_status
mcp.linked_model_tool_call_id
evidence.parse_confidence
evidence.link_status
```

CEL, Sigma-derived detection predicates, backtest, telemetry, and timeline
queries should target those stable fields. Raw provider payload access is an
explicit escape hatch, not the default rule vocabulary.

## Host AI Client Compatibility

This evidence layer should also support future service-owned AI calls such as:

```text
await model.prompt(model, "summarize this session")
await model.prompt(model, "name this VM")
```

The right split is:

- **Share the evidence model.** Host-originated prompts, summaries, VM naming,
  support-bundle summaries, and future local-model tasks emit
  `ModelInteractionEvidence` and project into the same resolved-event,
  telemetry, cost, provider/model, and timeline vocabulary.
- **Keep execution separate from VM network transport.** A future Host AI
  Client or Inference Engine should own service-side provider adapters,
  credentials, retry/timeout behavior, and response parsing. It should not be
  modeled as guest HTTP traffic or as a Network Engine transport event.
- **Annotate source and scope explicitly.** Host AI events carry
  `source_engine = host_ai` or equivalent plus optional `vm_id`, `profile_id`,
  `session_id`, `conversation_id`, and `purpose` when the call is tied to a VM
  or timeline.
- **Run through the same Security Engine boundary when governance matters.**
  The host client can submit model events for enforcement/detection/telemetry so
  profile or service policy can govern provider/model/cost/tool behavior. The
  final action semantics stay service-owned rather than transport-owned.

This keeps one audit/provenance vocabulary for all AI activity while avoiding a
fake coupling between host summarization and guest network proxy mechanics.

## Acceptance Criteria

1. Canonical evidence structs exist for model interaction, request, response,
   tool call, tool result, MCP execution, usage, content blocks, and link status.
2. Existing AI parsing populates canonical evidence for OpenAI Chat
   Completions, OpenAI Responses, Anthropic Messages, and Google/Gemini content
   parts/function traffic.
3. MCP aggregator execution records can link to model tool calls when known.
4. Unknown, pending, ambiguous, and orphan linkage is represented explicitly.
5. Tool arguments preserve raw and parsed forms plus argument status.
6. Tool result content preserves kind, preview, parsed JSON when applicable,
   error status, returned-to-model status, and parse confidence.
7. Security events project canonical evidence into CEL/Sigma-addressable
   model/MCP fields.
8. Golden fixtures cover streaming tool-call deltas, completed tool calls,
   malformed/partial arguments, tool results returned to the model, linked MCP
   execution, orphan model tool calls, orphan MCP executions, and provider
   unknown-field drift.
9. Existing model/MCP behavior continues to work while the new evidence becomes
   the policy-facing substrate.
10. S08b/S08c/S08d can consume the canonical evidence without depending on
   provider-specific request/response JSON paths.

## Testing Matrix

- Unit/contract: serde roundtrip, strict enum parsing, argument-status
  classification, content-block extraction, stable ids, provider-family
  adapters.
- Functional: OpenAI Chat, OpenAI Responses, Anthropic, Google/Gemini, and MCP
  aggregator fixtures project into expected security-event fields.
- Adversarial: malformed JSON, partial streaming arguments, duplicated call ids,
  missing provider ids, ambiguous MCP linkage, orphan model/MCP events,
  redacted arguments, unknown provider fields.
- E2E/VM: VM-originated model tool call linked to an MCP execution and returned
  tool result after S08b engine wiring.
- Telemetry/session DB: resolved event records contain evidence, linkage,
  usage, cost, provider/model, and full local evidence unless an export path
  explicitly redacts.
- Performance: baseline parser/projection overhead captured in S08d; this side
  sprint only defines the data path and focused unit/fixture proof.

## Downstream References

- S08b must build model/MCP `SecurityEvent` subjects from this evidence layer.
- S08c uses this layer for shared event/rule corpora and backtest parity.
- S08d benchmarks enforcement/detection over evidence-backed model/MCP events.
- S11/S12 status and OTel consume provider/model/tool/cost/linkage fields.
- S14/S15/S16/S16a consume evidence-backed rule UI, confirm context, profile
  visibility, and timeline blocks.
- S19/S19a document and market AI/MCP policy, detection, forensics, and
  performance only after this substrate is proven.
