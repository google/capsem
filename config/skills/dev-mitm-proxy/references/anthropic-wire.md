# Anthropic API Wire Format

Source: `crates/capsem-core/src/net/ai_traffic/anthropic.rs` (619 lines)

## Endpoints

- `POST /v1/messages` -- create message (streaming or sync). Only this path emits `model_calls`.
- `POST /v1/messages/batches` -- batch API (not streamed, no telemetry)

## Request

```http
POST /v1/messages HTTP/1.1
Host: api.anthropic.com
Content-Type: application/json
x-api-key: sk-ant-...
anthropic-version: 2023-06-01
```

Key fields extracted by `request_parser.rs`:
- `model` (string)
- `stream` (bool)
- `system` (string or content blocks array)
- `messages` (array, count tracked)
- `tools` (array, count tracked)
- Tool results: trailing user messages with `block_type: "tool_result"`, has `tool_use_id`

## Streaming SSE format

Uses `event:` lines to distinguish types. Events:

```
event: message_start
data: {"type":"message_start","message":{"id":"msg_...","model":"claude-sonnet-4-20250514","usage":{"input_tokens":10,"output_tokens":0}}}

event: content_block_start
data: {"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}

event: content_block_delta
data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Hello"}}

event: content_block_stop
data: {"type":"content_block_stop","index":0}

event: message_delta
data: {"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":25}}

event: message_stop
data: {"type":"message_stop"}
```

### Content block types
- `text` -- text content, deltas are `text_delta`
- `tool_use` -- tool call, deltas are `input_json_delta` (streaming JSON arguments)
- `thinking` -- thinking content, deltas are `thinking_delta`

### Parsed types (from source)

```rust
struct MessageInfo {
    id: Option<String>,
    model: Option<String>,
    usage: Option<Usage>,  // input_tokens, output_tokens, cache_read_input_tokens
}

struct ContentBlock {
    r#type: String,  // "text", "tool_use", "thinking"
    id: Option<String>,  // tool_use id: "toolu_..."
    name: Option<String>,  // tool name
}

struct Delta {
    r#type: String,  // "text_delta", "input_json_delta", "thinking_delta"
    text: Option<String>,
}
```

### Telemetry extraction
- `message_start` -> model name, input_tokens, cache_read_input_tokens
- `message_delta` -> output_tokens, stop_reason
- Stop reasons: `end_turn`, `tool_use`, `max_tokens`, `content_filter`

## Content-Encoding

Anthropic compresses SSE with gzip when `Accept-Encoding: gzip` is present. The proxy MUST decompress before SSE parsing. This caused the NULL telemetry bug -- compressed SSE is binary garbage to the text parser.

## Non-streaming response

Usage in top-level JSON:
```json
{
  "usage": {"input_tokens": 10, "output_tokens": 25, "cache_read_input_tokens": 0}
}
```
