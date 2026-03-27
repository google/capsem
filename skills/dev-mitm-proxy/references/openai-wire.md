# OpenAI API Wire Format

Source: `crates/capsem-core/src/net/ai_traffic/openai.rs` (500+ lines)

Covers OpenAI and OpenAI-compatible APIs (Codex, local models). Two API variants supported.

## Endpoints

- `POST /v1/chat/completions` -- Chat Completions API
- `POST /v1/responses` -- Responses API (newer)

Both emit `model_calls` telemetry.

## SSE format

No `event:` lines -- all events are `data:` only. Ends with `data: [DONE]` (filtered by SseParser).

### Chat Completions streaming

```
data: {"id":"chatcmpl-...","model":"gpt-4o","choices":[{"index":0,"delta":{"role":"assistant"},"finish_reason":null}]}

data: {"id":"chatcmpl-...","model":"gpt-4o","choices":[{"index":0,"delta":{"content":"Hello"},"finish_reason":null}]}

data: {"id":"chatcmpl-...","model":"gpt-4o","choices":[{"index":0,"delta":{},"finish_reason":"stop"}],"usage":{"prompt_tokens":10,"completion_tokens":25,"prompt_tokens_details":{"cached_tokens":0},"completion_tokens_details":{"reasoning_tokens":0}}}

data: [DONE]
```

### Tool calls in Chat Completions

```json
{"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_...","type":"function","function":{"name":"tool_name","arguments":""}}]}}]}
{"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"{\"q\":"}}]}}]}
{"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"\"rust\"}"}}]}}]}
```

Tool call arguments stream incrementally via `function.arguments` deltas.

### Responses API streaming

Different event structure with typed events:
- `response.output_item.added` -- new output item (text, function_call)
- `response.output_text.delta` -- text content delta
- `response.function_call_arguments.delta` -- tool call argument delta
- `response.reasoning_summary_text.delta` -- reasoning content
- `response.completed` -- final event with usage

### Parsed types (from source)

```rust
struct ChatCompletionChunk {
    id: Option<String>,
    model: Option<String>,
    choices: Option<Vec<Choice>>,
    usage: Option<Usage>,
}

struct Choice {
    index: Option<u32>,
    delta: Option<ChoiceDelta>,
    finish_reason: Option<String>,
}

struct ChoiceDelta {
    content: Option<String>,
    tool_calls: Option<Vec<ToolCallDelta>>,
}

struct Usage {
    prompt_tokens: Option<u64>,
    completion_tokens: Option<u64>,
    prompt_tokens_details: Option<PromptTokensDetails>,
    completion_tokens_details: Option<CompletionTokensDetails>,
}

struct PromptTokensDetails {
    cached_tokens: Option<u64>,
}

struct CompletionTokensDetails {
    reasoning_tokens: Option<u64>,
}
```

### Telemetry extraction
- Model from first chunk or usage chunk
- Input tokens: `prompt_tokens`
- Output tokens: `completion_tokens`
- Cached tokens: `prompt_tokens_details.cached_tokens`
- Reasoning tokens: `completion_tokens_details.reasoning_tokens` (o1/o3 models)
- Finish reasons: `stop`, `tool_calls`, `length`, `content_filter`

## Request parsing

### Chat Completions request
- `model`, `stream`, `messages` array, `tools` array
- System prompt from first `role: "system"` message
- Tool results from trailing `role: "tool"` messages with `tool_call_id`

### Responses API request
- `model`, `stream`, `input` (messages), `instructions` (system)
- Tool results from trailing `role: "tool"` in `input` array
