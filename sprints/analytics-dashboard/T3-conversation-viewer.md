# T3: Conversation Viewer

New "Conversation" tab in the per-session view that renders the AI model conversation as a rich, scrollable chat timeline with user messages, assistant responses, thinking blocks, and tool calls.

## Why

Our MITM proxy captures the full API request AND response for every model call. We have:
- The user's messages (in `request_body_preview` -- the full JSON request body including the messages array)
- The assistant's response (in `text_content`)
- Extended thinking (in `thinking_content`)
- Tool calls with full arguments and response previews
- Conversation threading via `trace_id`

The history viewer's conversation browser is its most compelling feature. We can build an equivalent using our existing data and SQL queries (already defined in sql.ts).

## Dependencies

- T0 (5MB field cap for full request_body_preview content)

## Data available per model call

Each `model_call` record in session.db contains:

**Request side (user's turn):**
- `request_body_preview` (up to 5MB after T0) -- full JSON request body including `messages` array with user messages. For multi-turn conversations, each request re-sends all prior messages, so this grows. The latest user message is the last `role: "user"` entry.
- `system_prompt_preview` -- the system prompt
- `messages_count` -- number of messages in the request

**Response side (assistant's turn):**
- `text_content` (up to 5MB) -- assistant's text output
- `thinking_content` (up to 5MB) -- extended thinking/reasoning
- `stop_reason` -- `end_turn`, `tool_use`, `max_tokens`, etc.

**Tool interaction (linked tables):**
- `tool_calls` -- `tool_name`, `arguments` (full JSON), `origin` (native/mcp), `call_id`
- `tool_responses` -- `content_preview`, `is_error`, `call_id`

**Metadata:**
- `provider`, `model`, `input_tokens`, `output_tokens`, `cache_creation_tokens`, `cache_read_tokens`
- `estimated_cost_usd`, `duration_ms`, `trace_id`, `timestamp`

## Existing SQL queries (in sql.ts)

All already defined and ready to use:

- `TRACES_SQL` -- list traces with aggregated stats (provider, model, call_count, total_input_tokens, total_output_tokens, total_duration_ms, total_cost, total_tool_calls, stop_reason, system_prompt_preview)
- `TRACE_DETAIL_SQL` -- all model_calls in a trace ordered by id (timestamp, provider, model, thinking_content, text_content, input_tokens, output_tokens, duration_ms, estimated_cost_usd, stop_reason, request_body_preview, system_prompt_preview, messages_count, tools_count)
- `TRACE_TOOL_CALLS_SQL` -- all tool_calls for a trace (model_call_id, call_index, call_id, tool_name, arguments, origin)
- `TRACE_TOOL_RESPONSES_SQL` -- all tool_responses for a trace (model_call_id, call_id, content_preview, is_error)

---

## Task 3.1: Trace list sidebar

Left panel listing conversation traces grouped by `trace_id` (using `TRACES_SQL`):

- Vertical list of trace cards
- Each card shows:
  - Provider icon (use existing provider color tokens)
  - Model name (truncated if long)
  - Call count badge
  - Total tokens (formatted)
  - Total cost (formatted)
  - Timestamp (relative, e.g., "2m ago")
  - Stop reason indicator (small icon or text: "completed" for end_turn, "tool_use" for mid-conversation)
- Click to select and load that trace's messages in the main panel
- Auto-select most recent trace on mount
- Scrollable if many traces

**Component**: `frontend/src/lib/components/conversation/TraceList.svelte`
**Props**: `vmId: string`, event callback for trace selection

---

## Task 3.2: User message extraction

Parse `request_body_preview` JSON to extract the latest user message for display.

**Logic:**
1. Parse `request_body_preview` as JSON
2. Navigate to `messages` array (Anthropic format: `body.messages`, OpenAI: `body.messages`)
3. Find the last entry where `role === "user"`
4. Extract text content:
   - If `content` is a string: use directly
   - If `content` is an array (Anthropic content blocks): find blocks with `type === "text"`, concatenate their `.text` fields
   - Skip `tool_result` blocks (those are tool responses being fed back)
5. If JSON parsing fails (truncated): show "Message content unavailable" gracefully

**Component**: `frontend/src/lib/components/conversation/UserMessage.svelte`
**Props**: `requestBodyPreview: string | null`

Styled differently from assistant messages: left-aligned, lighter background, user icon.

---

## Task 3.3: Message timeline (main panel)

For each `model_call` in the selected trace (ordered by id, using `TRACE_DETAIL_SQL`):

```
┌─ User Message ──────────────────────────────────┐
│ (extracted from request_body_preview)            │
│ Rendered as markdown                             │
└──────────────────────────────────────────────────┘

┌─ Assistant Response ────────────────────────────┐
│ [provider] [model]  [tokens: in/out] [$cost] [duration]
│                                                  │
│ ┌─ Thinking (collapsible) ────────────────────┐ │
│ │ thinking_content as monospace text          │ │
│ └─────────────────────────────────────────────┘ │
│                                                  │
│ text_content rendered as markdown with shiki     │
│ code blocks                                      │
│                                                  │
│ ┌─ Tool: Read ────────────────────────────────┐ │
│ │ arguments: {"file_path": "/src/main.rs"}   │ │
│ │ Result (collapsible): content_preview...    │ │
│ └─────────────────────────────────────────────┘ │
│                                                  │
│ ┌─ Tool: Bash ────────────────────────────────┐ │
│ │ arguments: {"command": "cargo build"}      │ │
│ │ Result: (terminal-style output)             │ │
│ └─────────────────────────────────────────────┘ │
└──────────────────────────────────────────────────┘
```

**Component**: `frontend/src/lib/components/conversation/MessageCard.svelte`
**Props**: model_call data + associated tool_calls + tool_responses

**Metadata bar**: small chips showing `[provider]`, `[model]`, `[in: X / out: Y tokens]`, `[$cost]`, `[duration]`
Uses `text-muted-foreground-1 text-xs` styling, positioned at top of card.

---

## Task 3.4: Tool call rendering

**Component**: `frontend/src/lib/components/conversation/ToolCallCard.svelte`
**Props**: `toolName: string`, `arguments: string`, `response: string | null`, `isError: boolean`, `origin: string`

Rendering varies by tool type:

| Tool type | Arguments display | Response display |
|---|---|---|
| **Bash** | Command extracted from `{"command": "..."}` shown as monospace code | Terminal-style: dark bg (`bg-gray-900`), monospace, white text |
| **Read** | File path extracted from `{"file_path": "..."}` | Code block with shiki syntax detection based on file extension |
| **Edit/Write** | File path + `old_string`/`new_string` shown | Highlighted diff (old in red bg, new in green bg) |
| **Grep/Glob** | Pattern + path extracted from arguments | File list or match results as plain text |
| **MCP tools** | Server name badge (from `origin`) + full JSON args (highlighted) | JSON response preview (highlighted) |
| **Other** | Raw JSON arguments (shiki-highlighted as JSON) | Raw content_preview |

Error styling: if `is_error === true`, card gets `border-destructive` border color.

Collapsible: tool response content is collapsible (start collapsed if >10 lines, expanded if <=10).

---

## Task 3.5: Thinking block

**Component**: `frontend/src/lib/components/conversation/ThinkingBlock.svelte`
**Props**: `content: string`

- Collapsible section (collapsed by default)
- Header: "Thinking" with chevron toggle
- Content: monospace text (`font-mono text-sm`), preserving whitespace
- Styled with muted background (`bg-muted`) and subtle left border
- Only rendered when `thinking_content` is non-null and non-empty

---

## Task 3.6: Summary header

At the top of the conversation view (above the timeline) for the selected trace:

```
┌───────────────────────────────────────────────────┐
│ [provider icon] Model Name                        │
│ 12 calls  ·  45.2K tokens  ·  $0.23  ·  8.4s    │
└───────────────────────────────────────────────────┘
```

- Total model calls in trace
- Total input + output tokens (formatted)
- Total cost (sum of estimated_cost_usd)
- Total duration (sum of duration_ms, formatted)
- Provider + model name

---

## Task 3.7: Navigation integration

- Add "Conversation" tab to StatsView tab bar
- New `StatsTab` value: `'conversation'`
- Placed between AI and Tools tabs
- Icon: `ChatCircle` from `phosphor-svelte`
- When selected, renders `ConversationView` instead of the other tab content

**Main view component**: `frontend/src/lib/components/views/ConversationView.svelte`

Layout:
```
┌────────────────┬──────────────────────────────────┐
│ Trace List     │ Summary Header                   │
│ (sidebar)      ├──────────────────────────────────┤
│                │ Message Timeline (scrollable)     │
│ [trace 1] <-   │                                  │
│ [trace 2]      │ UserMessage                      │
│ [trace 3]      │ MessageCard (assistant + tools)  │
│                │ UserMessage                      │
│                │ MessageCard (assistant + tools)  │
│                │ ...                              │
└────────────────┴──────────────────────────────────┘
```

Sidebar width: `w-64` (256px), main panel fills remaining space.

---

## Files to create/modify

| File | Action |
|---|---|
| `frontend/src/lib/components/views/ConversationView.svelte` | Create -- main layout (sidebar + summary + timeline) |
| `frontend/src/lib/components/conversation/TraceList.svelte` | Create -- trace sidebar with selection |
| `frontend/src/lib/components/conversation/UserMessage.svelte` | Create -- user message extracted from request_body_preview |
| `frontend/src/lib/components/conversation/MessageCard.svelte` | Create -- assistant response card with metadata bar |
| `frontend/src/lib/components/conversation/ToolCallCard.svelte` | Create -- tool call + response with per-tool rendering |
| `frontend/src/lib/components/conversation/ThinkingBlock.svelte` | Create -- collapsible thinking section |
| `frontend/src/lib/components/views/StatsView.svelte` | Modify -- add Conversation tab to tab bar |
| `frontend/src/lib/sql.ts` | Already has TRACES_SQL, TRACE_DETAIL_SQL, TRACE_TOOL_CALLS_SQL, TRACE_TOOL_RESPONSES_SQL |

## Verification

1. Boot a VM, run an AI agent doing a multi-turn conversation with tool use (e.g., Claude Code with file reads, edits, bash)
2. Open Stats > Conversation tab
3. Verify trace list sidebar shows conversations with correct metadata (provider, model, tokens, cost, timestamp)
4. Click a trace:
   - User messages render with markdown
   - Assistant responses render with markdown + shiki code highlighting
   - Tool calls appear inline with correct per-tool rendering (Bash = terminal, Read = code, etc.)
   - Thinking blocks are collapsible (collapsed by default)
   - Metadata bar shows tokens, cost, duration per message
5. Verify summary header shows correct aggregate stats for selected trace
6. Verify error tool results have `border-destructive` styling
7. Verify collapsible tool responses work (>10 lines start collapsed)
8. Test with session that has no model_calls -- show empty state ("No conversations recorded")
9. Test trace selection -- clicking different traces loads different conversations
10. `pnpm run check` passes with no warnings
