# Pre-rewrite baseline

Captured before T1 (pipeline-and-hook-traits) lands. Numbers from
`cargo bench -p capsem-core --bench <name> -- --warm-up-time 2
--measurement-time 5 --sample-size 30` on Apple Silicon (Darwin
25.3.0). T5's regression gate compares against this file via
`critcmp`; any bench >5% slower than these numbers fails CI.

Refresh procedure: rerun the same command, copy the median number
from the criterion output into the right column, commit with a
CHANGELOG entry explaining why the baseline moved.

## sse_parser

Bytes/sec through `SseParser::feed` on a 1MB Anthropic-shaped
event-stream corpus, varying chunk size.

| Variant | Median time | Median throughput |
|---------|------------:|------------------:|
| 1MB_in_4KB_chunks | 2.10 ms | 465 MiB/s |
| 1MB_in_64KB_chunks | 2.12 ms | 472 MiB/s |
| 1MB_in_1MB_chunk | 2.23 ms | 449 MiB/s |

Plan budget: ≥500 MiB/s on a single core. Currently at 449-472 MiB/s
across chunk sizes. T1's hook dispatch must not push this lower; T5's
hot-path fixes (bounded accumulator, atomic stats) should push it
above the budget.

## parser_jsonrpc (placeholder)

Real JSON-RPC parser ships in T4. For now the bench measures
`serde_json::from_str` on representative MCP envelopes -- the floor
the T4 parser must meet (or beat).

| Envelope | Median time | Median throughput |
|----------|------------:|------------------:|
| tools_list_req | 178 ns | 311 MiB/s |
| tools_call_req | 369 ns | 347 MiB/s |
| tools_list_resp | 1.01 µs | 333 MiB/s |

Plan budget: ≥200 MiB/s on a single core. serde_json is ~330 MiB/s,
so T4's parser has headroom over the budget but the budget remains
intact even if T4 chooses a slower-but-streaming approach.

## interp_anthropic

Full SSE-parse + AnthropicStreamParserWithState pipeline on a
representative tool-use response (text + tool_use + input_json_delta
accumulation -- the hottest interpreter path).

| Variant | Median time | Median throughput |
|---------|------------:|------------------:|
| tool_use_full_pipeline | 4.76 µs | 233 MiB/s |

No plan budget yet; locking in this number so T1's wrapper hook
(SseParserHook -> AnthropicInterpreterHook) doesn't silently regress
the parser+interpreter chain.

## mitm_pipeline (placeholder)

Hook trait + dispatch ships in T1. For now the bench measures the
metrics facade overhead (every counter/histogram emission per request
once T1 wires them).

| Variant | Median time |
|---------|------------:|
| metrics_describe_all | 6.31 ns |
| counter_emit_no_recorder | 3.89 ns |

`counter_emit_no_recorder` is the per-request floor: each `counter!()`
call hooks pay until T5 wires an exporter. ~4 ns is well inside the
plan's <100 µs empty-hook overhead budget; counters are effectively
free until a real recorder is installed.
