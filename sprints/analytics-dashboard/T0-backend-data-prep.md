# T0: Backend Data Preparation

Two schema/backend changes that unblock richer frontend analytics.

## Why

1. **Field truncation at 256KB** -- The conversation viewer needs full API request bodies (which contain the user's messages array -- grows each turn) and full response content. 256KB truncates long conversations. Bumping to 5MB covers even large multi-turn sessions.

2. **Cache tokens buried in JSON** -- `model_calls.usage_details` stores cache info as `{"cache_read": 150}`. Querying requires `json_extract()`, can't be indexed, and isn't rolled up to main.db. History viewer has input/output/cache-create/cache-read as a 4-segment donut -- we need first-class columns for this.

## Gap Analysis: History Viewer vs Capsem Data

| History Viewer Feature | Their Data Source | Our Equivalent | Gap? |
|---|---|---|---|
| **Token distribution donut** (input/output/cache-create/cache-read) | JSONL `usage.cache_creation_input_tokens`, `cache_read_input_tokens` | `model_calls.input_tokens`, `output_tokens` + `cache_creation_tokens`, `cache_read_tokens` (promoted to first-class columns in this sprint). Also rolled up to `sessions` table in main.db. | **Ready** (after this sprint) |
| **Activity heatmap** (daily calendar grid) | `DailyStats[]` with date, message_count, session_count, total_tokens | `sessions.created_at` in main.db | **Derivable** -- GROUP BY date(created_at) gets session_count per day. No precomputed daily table. |
| **Tool usage bar chart** (top tools by frequency) | Parsed from JSONL tool_use blocks | `top_tools` + `top_mcp_tools` from GET /stats (main.db aggregates) | **Ready** -- data already returned by API, just not rendered. |
| **Provider usage chart** | Parsed from JSONL model field | `top_providers` from GET /stats | **Ready** -- already returned. |
| **Cost breakdown by model** (per-model pricing) | Hardcoded per-model rates, parsed from JSONL | `ai_usage` in main.db groups by `provider` only, not model. Per-session `model_calls` has `model` + `estimated_cost_usd` | **Gap** -- main.db per-provider only. Recommendation: per-model only in per-session view. |
| **Daily/weekly trend charts** | DailyStats array, 7-day window | `sessions` with `created_at`, `stopped_at`, per-session rollup totals | **Derivable** -- GROUP BY date(created_at) on recent sessions. |
| **Session duration** (total, average) | `first_message_time - last_message_time` | `sessions.created_at` and `stopped_at` in main.db | **Ready** -- `stopped_at - created_at`. |
| **Message count** (per session, global) | JSONL line count | Per-session: `COUNT(*) FROM model_calls`. Global: `SUM(call_count)` from `ai_usage`. | **Derivable** -- not in sessions table but queryable from ai_usage. |
| **Billing vs conversation tokens** | Tracks both separately | We capture all API tokens uniformly via MITM | **Skip** -- not applicable to our model. |
| **Avg tokens per message** | `total_tokens / message_count` | `SUM(input+output) / COUNT(*)` from model_calls | **Derivable** per session. |
| **Tool success rate** | `is_error` flag on tool results | `tool_responses.is_error` in session.db | **Derivable** per session. |
| **Conversation viewer** (messages, thinking, tool calls) | Full JSONL message content | `request_body_preview` (user messages), `text_content` (assistant), `thinking_content`, `tool_calls.arguments`, `tool_responses.content_preview` | **Ready** (after this sprint) -- 5MB field cap sufficient for long conversations. |

### What we have that they DON'T

| Capsem Exclusive | Description |
|---|---|
| **Network telemetry** | Full HTTP request/response capture via MITM (domains, status codes, bytes, headers, body previews, policy decisions) |
| **MCP call details** | Per-server, per-tool call tracking with decision (allowed/denied/warned) |
| **File system events** | Real-time workspace modifications (create/modify/delete with sizes) |
| **Snapshot events** | APFS snapshot lifecycle with per-snapshot file change attribution |
| **Process attribution** | Every network/model/tool call attributed to a specific process name + PID |
| **Network policy** | Allowed vs denied decisions with matched_rule for each request |

### Key decisions

- **Per-model global aggregation**: Show per-model only in per-session view (option c). main.db stays per-provider.
- **Global message count**: Derive from `SUM(call_count)` in `ai_usage`. No schema change needed.
- **Billing vs conversation tokens**: Skip. Not applicable to MITM model.

---

## Task 0.1: Bump MAX_FIELD_BYTES from 256KB to 5MB

The `MAX_FIELD_BYTES` constant in `crates/capsem-logger/src/writer.rs:12` caps all TEXT fields via the `cap_field()` helper. This applies to: `request_body_preview`, `text_content`, `thinking_content`, `system_prompt_preview`, `arguments`, `content_preview`, `request_body_preview`/`response_body_preview` (net_events), `request_preview`/`response_preview` (mcp_calls).

**File**: `crates/capsem-logger/src/writer.rs`
**Change**: `MAX_FIELD_BYTES` from `262_144` (256KB) to `5_242_880` (5MB)
**Impact**: Larger session.db files for sessions with very long conversations. Most fields are well under 256KB anyway -- only `request_body_preview` in model_calls grows significantly (it contains the full messages array which accumulates each turn).

---

## Task 0.2: Promote cache tokens to first-class columns

### session.db schema

**File**: `crates/capsem-logger/src/schema.rs`

Add to `model_calls` CREATE TABLE (after `output_tokens`):
```sql
cache_creation_tokens INTEGER DEFAULT 0,
cache_read_tokens INTEGER DEFAULT 0,
```

### ModelCallEvent struct

**File**: `crates/capsem-logger/src/events.rs`

Add to `ModelCallEvent`:
```rust
pub cache_creation_tokens: u64,
pub cache_read_tokens: u64,
```

Update the `Default` / construction sites to initialize these to 0.

### Logger writer

**File**: `crates/capsem-logger/src/writer.rs`

Add the two new columns to the INSERT INTO model_calls statement and bind the values from `call.cache_creation_tokens` and `call.cache_read_tokens`.

### Logger reader

**File**: `crates/capsem-logger/src/reader.rs`

Update the SELECT in `read_model_call` / `list_model_calls` to include the new columns. Map them in the row deserialization.

### MITM parser -- extract cache tokens from API responses

**File**: `crates/capsem-core/src/net/ai_traffic/` (likely `response_parser.rs` or `events.rs`)

When building the `ModelCallEvent` from the API response:

**Anthropic** (`/v1/messages`): The response `usage` object has:
- `cache_creation_input_tokens` (optional)
- `cache_read_input_tokens` (optional)

These are already partially captured into `usage_details` JSON. Now also populate the new struct fields.

**OpenAI** (`/v1/chat/completions`): The response `usage` object has:
- `prompt_tokens_details.cached_tokens` (cache read)
- No cache creation equivalent

**Google** (`/v1beta/models/*/generateContent`): Check `usageMetadata` for cache fields.

### main.db rollup

**File**: `crates/capsem-core/src/session/index.rs`

**sessions table** -- add columns:
```sql
total_cache_creation_tokens INTEGER DEFAULT 0,
total_cache_read_tokens INTEGER DEFAULT 0,
```

**ai_usage table** -- add columns:
```sql
cache_creation_tokens INTEGER DEFAULT 0,
cache_read_tokens INTEGER DEFAULT 0,
```

**Rollup logic**: When rolling up a session (on stop/crash), SUM the new columns from model_calls and write to both tables.

### Type structs

**File**: `crates/capsem-core/src/session/types.rs`

Add to `GlobalStats`:
```rust
pub total_cache_creation_tokens: u64,
pub total_cache_read_tokens: u64,
```

Add to `ProviderSummary`:
```rust
pub cache_creation_tokens: u64,
pub cache_read_tokens: u64,
```

Add to `SessionRecord`:
```rust
pub total_cache_creation_tokens: u64,
pub total_cache_read_tokens: u64,
```

### Frontend types

**File**: `frontend/src/lib/types/gateway.ts`

Add matching fields to `GlobalStats`, `ProviderSummary`, and `SessionRecord` TypeScript interfaces.

---

## Files to modify (complete list)

| File | Change |
|---|---|
| `crates/capsem-logger/src/writer.rs` | Bump MAX_FIELD_BYTES to 5MB, add cache columns to INSERT |
| `crates/capsem-logger/src/schema.rs` | Add cache_creation_tokens, cache_read_tokens to model_calls CREATE TABLE |
| `crates/capsem-logger/src/events.rs` | Add cache fields to ModelCallEvent struct |
| `crates/capsem-logger/src/reader.rs` | Read new columns in SELECT + row mapping |
| `crates/capsem-core/src/net/ai_traffic/` | Extract cache tokens from Anthropic/OpenAI/Google API response usage blocks |
| `crates/capsem-core/src/session/index.rs` | Add cache columns to sessions + ai_usage tables, populate in rollup |
| `crates/capsem-core/src/session/types.rs` | Add cache fields to GlobalStats, ProviderSummary, SessionRecord |
| `frontend/src/lib/types/gateway.ts` | Add cache fields to TS types |

## Verification

1. `cargo test` passes for capsem-logger and capsem-core
2. Boot a VM, run an AI agent that uses Claude (Anthropic API with cache)
3. Query session.db: `SELECT cache_creation_tokens, cache_read_tokens FROM model_calls` -- values populated
4. Stop session, check main.db rollup: `SELECT total_cache_creation_tokens, total_cache_read_tokens FROM sessions` -- populated
5. Verify request_body_preview contains full request (check a model_call with long conversation)
6. Frontend `pnpm run check` passes (updated gateway.ts types)
