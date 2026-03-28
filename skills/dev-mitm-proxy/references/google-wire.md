# Google Gemini API Wire Format

Source: `crates/capsem-core/src/net/ai_traffic/google.rs` (300+ lines)

## Endpoints

- `POST /v1beta/models/{model}:generateContent` -- sync
- `POST /v1beta/models/{model}:streamGenerateContent` -- streaming

Model name extracted from URL path (unique to Google -- other providers put it in the request body).

## SSE format

Each SSE event is a **complete JSON object** (not deltas like Anthropic/OpenAI). Parts contain full text, function calls, or thoughts.

```
data: {"candidates":[{"content":{"parts":[{"text":"Hello"}],"role":"model"}}],"usageMetadata":{"promptTokenCount":10,"candidatesTokenCount":5}}

data: {"candidates":[{"content":{"parts":[{"text":" world!"}],"role":"model"},"finishReason":"STOP"}],"usageMetadata":{"promptTokenCount":10,"candidatesTokenCount":12}}
```

### Function calls (complete, not streamed)

```
data: {"candidates":[{"content":{"parts":[{"functionCall":{"name":"search","args":{"q":"rust"}}}],"role":"model"},"finishReason":"STOP"}]}
```

No tool call IDs provided by Google. Capsem generates **synthetic IDs** from the function name.

### Thinking content

```
data: {"candidates":[{"content":{"parts":[{"text":"Let me think...","thought":true}],"role":"model"}}]}
```

Parts with `thought: true` are thinking content, routed to `ThinkingDelta` events.

### Parsed types (from source)

```rust
#[serde(rename_all = "camelCase")]
struct StreamChunk {
    candidates: Option<Vec<Candidate>>,
    usage_metadata: Option<UsageMetadata>,
    model_version: Option<String>,
}

struct Candidate {
    content: Option<Content>,
    finish_reason: Option<String>,
}

struct Content {
    parts: Option<Vec<Part>>,
}

struct Part {
    text: Option<String>,
    function_call: Option<FunctionCall>,
    thought: Option<bool>,
}

struct FunctionCall {
    name: Option<String>,
    args: Option<Box<serde_json::value::RawValue>>,  // RawValue -- not Value
}

struct UsageMetadata {
    prompt_token_count: Option<u64>,
    candidates_token_count: Option<u64>,
    cached_content_token_count: Option<u64>,
    thoughts_token_count: Option<u64>,
}
```

Note: all fields use `camelCase` on the wire (serde `rename_all`).

### Telemetry extraction
- Model from `model_version` field or URL path (`/models/{model}:action`)
- Input tokens: `prompt_token_count`
- Output tokens: `candidates_token_count`
- Cached tokens: `cached_content_token_count`
- Thinking tokens: `thoughts_token_count`
- Finish reasons: `STOP`, `MAX_TOKENS`, `SAFETY`, `RECITATION`

## Request parsing

- `system_instruction.parts` -- system prompt (array of parts)
- `contents` -- messages array
- `tools[].functionDeclarations` -- tool definitions
- Function responses from trailing `role: "function"` messages

## Key differences from Anthropic/OpenAI

1. Complete JSON objects per event (not deltas)
2. No tool call IDs (synthetic IDs generated)
3. Model name in URL path, not request body
4. `camelCase` field naming throughout
5. Function calls are complete in a single part (not streamed incrementally)
