# H09 - Network And RPS Attribution

## Goal

Explain the Linux/macOS RPS and endpoint-latency gaps with the same discipline
as disk throughput: separate guest network path, vsock bridge, MITM/proxy
processing, security-engine evaluation, host service/gateway polling, TUI
status refresh, DNS, and any workspace/disk dependency before landing speedups.

## Why This Exists

The current benchmark baseline shows HTTP RPS at 0.83x macOS and proxy
throughput at 0.93x macOS, so network is not as far behind as the disk/rootfs
lanes. It is still user-visible and can become the next bottleneck once disk
attribution lands. Endpoint-latency artifacts also show service/global
control-plane reads in the low-millisecond range, which needs attribution
before optimizing status/TUI polling or proxy code.

## Scope

- Trace the request lifecycle first, then use benchmarks as proof. A network
  benchmark run without a named mechanism is not sufficient progress.
- Split RPS-facing paths into explicit lanes:
  - guest HTTP through net-proxy and host MITM;
  - guest DNS through dns-proxy and host resolver bridge;
  - host service and gateway endpoint latency;
  - TUI/status polling overhead;
  - security-engine request evaluation;
  - workspace/disk interactions in any file-serving or policy-context path.
- Add low-cardinality counters where missing:
  - guest-to-host vsock request counts, bytes, latency, and errors;
  - MITM request counts, body bytes, policy-evaluation latency, upstream time,
    and response-write latency;
  - DNS request counts, cache/resolver latency, and failures;
  - gateway/service status endpoint request counts and latency;
  - TUI polling interval and request volume.
- Compare relevant pieces against Firecracker/crosvm only where they share the
  same VM/device transport shape. MITM, gateway, and policy-engine comparisons
  are Capsem-specific and should use host-native/control benchmarks instead.
- Refresh the canonical Linux/macOS/host-native benchmark comparison after the
  trace has identified the lanes and counters that need proof.

## Out Of Scope

- Redesigning the MITM proxy before the attribution counters identify a
  dominant bottleneck.
- Treating internet latency as a VM performance problem. Benchmarks must keep
  local/control paths separate from upstream network variance.
- Apple VZ implementation changes. Shared benchmark/counter additions should be
  suitable for macOS reruns.

## Acceptance Gates

- Every RPS claim identifies the lane: guest network, vsock bridge, MITM,
  DNS, security engine, service/gateway endpoint, TUI/status polling, or
  workspace/disk dependency.
- `just benchmark` records refreshed HTTP, throughput, endpoint-latency,
  security-engine, and host-native artifacts.
- New counters are visible through status/session telemetry or the
  OTel-ready metric contract.
- A real VM run proves the counters move during `capsem-bench http`,
  `capsem-bench throughput`, and at least one endpoint-latency path.

## Source Trace

- Guest HTTP(S) traffic does not use virtio-net/tap today. `capsem-agent`
  redirects guest localhost TCP through `capsem-net-proxy`, then opens a host
  vsock connection per client connection.
- `capsem-process` receives those host vsock fds and dispatches SNI proxy
  connections into the host MITM handler. DNS, audit, exec, lifecycle, terminal,
  and control traffic use sibling vsock ports.
- Gateway/status attribution already has low-cardinality metrics for `/status`
  cache/refresh/service fan-out and catch-all service proxy endpoints.
- Process-side vsock attribution now has low-cardinality metrics for accepted
  connections, closed connections by result, active handlers, and handler
  duration by port kind.
- Guest-side process attribution was a concrete per-connection cost:
  `capsem-net-proxy` read `/proc/net/tcp*` and walked `/proc/<pid>/fd` for
  every accepted TCP connection before opening vsock. The first code-path
  improvement replaces the per-connection fd walk with a shared throttled
  socket-owner index. This may return `unknown` for some very short burst
  connections between refresh windows, so the next VM proof must measure both
  RPS and attribution quality.
- Remaining proof: run guest HTTP/proxy throughput in a real VM and confirm
  process-side vsock metrics move alongside existing MITM/DNS metrics, then
  expose the useful subset through status/session telemetry before making an
  RPS performance claim.
- First post-change diagnostic: `CAPSEM_BENCH_MITM_DURATION=3 capsem-bench
  mitm-load` completed after the dev profile fix, but it is not accepted
  performance evidence. It reported request exceptions at every concurrency
  level and 5-10s tail latencies, while the committed reference baseline has
  zero request exceptions and ~1k-3k RPS. This matches the same host DNS/network
  failure that forced remote asset downloads to fail earlier, so the next RPS
  proof needs either restored DNS/network or a local deterministic upstream.
- Deterministic MCP echo proof: `just exec "capsem-bench mcp-load && cat
  /tmp/capsem-benchmark.json"` completed with zero errors through
  `local__echo`. This avoids DNS/upstream variance and isolates guest stdio
  relay -> framed vsock:5002 -> MITM MCP endpoint -> aggregator ->
  `capsem-mcp-builtin` -> response. Current Linux numbers versus
  `benchmarks/mcp-load/baseline.json`:

| concurrency | current RPS | baseline RPS | RPS ratio | current p99 | baseline p99 | p99 ratio |
| ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 1 | 309.8 | 2162.5 | 0.143x (-85.7%) | 3.6 ms | 1.1 ms | 3.21x |
| 10 | 761.5 | 3792.0 | 0.201x (-79.9%) | 15.4 ms | 4.4 ms | 3.48x |
| 50 | 786.1 | 4061.4 | 0.194x (-80.6%) | 82.2 ms | 17.4 ms | 4.72x |
| 200 | 782.7 | 3965.0 | 0.197x (-80.3%) | 296.6 ms | 70.8 ms | 4.19x |

This fails the documented `mcp-load` p99 regression gate (>2x) at every
concurrency level. Because all rows have zero errors, this is a real MCP
transport/dispatch bottleneck to trace next, not an upstream network failure.
- Source trace of the current `local__echo` path:
  1. guest `capsem-bench mcp-load` holds one FastMCP stdio session to
     `/run/capsem-mcp-server`;
  2. guest `capsem-mcp-server` frames every JSON-RPC request over the single
     vsock:5002 connection and has a separate response reader thread, so the
     relay can pipeline by stream id;
  3. host `mcp_frame::serve_io` splits framed vsock read/write, validates
     request/stream shape, applies MCP policy/runtime security, acquires the
     bounded `CAPSEM_MCP_INFLIGHT` semaphore, and spawns a request handler;
  4. `McpEndpointState::handle_request` dispatches `tools/call` through
     `AggregatorClient::request`;
  5. `capsem-process` writes length-prefixed msgpack to
     `capsem-mcp-aggregator`, whose reader loop spawns one handler per request
     and whose writer can return responses out of order by request id;
  6. `McpServerManager::dispatch_call_tool` resolves the peer under a sync
     read lock, drops the lock before awaiting rmcp, and routes `local__echo`
     through the pool-safe local builtin stdio pool;
  7. after the builtin response, the MITM framed handler awaits
     `log_mcp_call_with_policy`, which enqueues both `McpCall` and
     `ResolvedSecurityEvent` through the bounded durable `DbWriter` channel
     before the framed response is queued back to the guest.
- Historical comparison matters: retired framed-MITM runs hit
  ~9k-10k RPS at concurrency 10/50 and ~8.3k-9.2k RPS at concurrency 200 with
  zero errors on the same logical path, so the current ~780 RPS ceiling is a
  regression to locate, not an expected Linux/KVM/vsock limit.
- New low-cardinality MCP timing histograms added for the next proof:
  `mitm.mcp_stage_duration_ms{stage,method_kind,tool_kind,result}` for framed
  parse, endpoint dispatch, telemetry enqueue, response enqueue, and response
  write; `mitm.mcp_endpoint_dispatch_ms{method_kind,tool_kind,result}` for the
  MITM endpoint; and
  `mitm.mcp_aggregator_request_ms{method_kind,tool_kind,result}` for
  process-to-aggregator round trips. Tool labels are bounded (`local_echo`,
  `local_http`, `local_snapshot`, `local_other`, `external`, `none`,
  `unknown`) so the metrics can flow to OTel without high-cardinality tool
  names.
- Live stage proof now has an opt-in runtime path:
  `CAPSEM_METRICS_DEBUG_INTERVAL_SECS=<seconds>` installs a capsem-process
  debug metrics recorder and emits compact `mcp_metric_snapshot` log lines for
  the MCP histograms above. This is a diagnostic bridge until the real OTLP
  exporter is configured; the expected next artifact is a same-run `mcp-load`
  result plus `process.log` stage summaries showing which stage owns p95/p99.
- Live proof on 2026-06-02, isolated branch service, 5s per level:
  no-recorder `mcp-load` was c=1 312.0 RPS p99 4.2ms, c=10 770.8 RPS p99
  17.1ms, c=50 752.6 RPS p99 77.3ms, c=200 771.4 RPS p99 300.7ms, all zero
  errors. With `CAPSEM_METRICS_DEBUG_INTERVAL_SECS=2`, the same branch shape
  was c=1 296.6 RPS, c=10 737.8 RPS, c=50 740.6 RPS, c=200 807.0 RPS, all
  zero errors, so the diagnostic recorder is not the root regression.
- Stage snapshots during the attributed run show `tools/call local_echo`
  `endpoint_dispatch` / `mitm.mcp_aggregator_request_ms` dominating at roughly
  1.13-1.20ms average, 1.24-1.35ms p95, and 1.31-1.55ms p99 across steady
  snapshots. `parse_json_rpc` stayed below ~0.09ms p99,
  `telemetry_enqueue` below ~0.12ms p99, and response enqueue/write below
  ~0.06ms p99. The c=200 ~270-300ms client p99 is therefore queueing behind a
  roughly 770-800 RPS single hot path, not slow parse, response write, or
  telemetry enqueue.
- Next decomposition instrumentation is in place. Process-side snapshots now
  include `mcp.aggregator_client_stage_duration_ms` for channel send, driver
  queue wait, request MessagePack encode, request frame write, response frame
  read, response MessagePack decode, and response route. Aggregator stderr
  snapshots include `mcp.aggregator_stage_duration_ms` for request frame read,
  request MessagePack decode, handler queue wait, manager lookup, server RPC,
  response channel send, response MessagePack encode, and response frame
  write. Builtin stderr snapshots include `mcp.builtin_tool_duration_ms` for
  local builtin tool execution. `CAPSEM_METRICS_DEBUG_INTERVAL_SECS` now flows
  through process -> aggregator -> builtin so the next `mcp-load` run can
  separate rmcp stdio transport/funnel cost from actual builtin tool work.
- Decomposition proof on 2026-06-02, same isolated service with
  `CAPSEM_METRICS_DEBUG_INTERVAL_SECS=2`, completed `CAPSEM_BENCH_MCP_DURATION=5
  capsem-bench mcp-load` with zero errors: c=1 265.2 RPS p99 4.1ms, c=10
  590.8 RPS p99 19.2ms, c=50 586.0 RPS p99 93.9ms, c=200 636.4 RPS p99
  377.4ms. This run carries heavier multi-process stage recorders, so use it
  as attribution proof rather than a clean speed baseline.
- The decomposition shows the actual `local__echo` builtin is not the
  bottleneck: `mcp.builtin_tool_duration_ms{tool_kind=local_echo}` stayed
  around 0.015ms average with p99 mostly 0.02-0.03ms. Aggregator
  `server_rpc` to the builtin stdio peer owned the largest non-idle stage at
  roughly 0.68-0.69ms average, 0.76-0.79ms p95, and 0.86-0.89ms p99.
  Aggregator `response_frame_write` added roughly 0.19-0.20ms average and
  0.25-0.27ms p99; request decode, handler queue, manager lookup, response
  encode, and process-side route/encode/write stages were all sub-0.1ms p99
  in steady snapshots. The next code-path bet should therefore remove or
  collapse local builtin stdio/RMCP round trips for safe builtin tools, then
  rerun the same attribution to prove the dispatch ceiling moved.
- Endpoint-level `local__echo` collapse landed as the first code-path fix:
  the MITM MCP endpoint now returns the safe echo diagnostic result directly
  after framed MCP policy evaluation, while external tools and networked or
  stateful local builtins still use the isolated aggregator/builtin
  subprocesses. Fresh-initrd Linux VM proof: canonical `mcp-load` reached
  c=1 407.2 RPS p99 2.9ms, c=10 608.4 RPS p99 19.2ms, c=50 601.0 RPS p99
  139.6ms, and c=200 616.8 RPS p99 755.0ms, all zero errors. Compared with
  the earlier no-recorder local-echo branch run (c=1 312.0 RPS p99 4.2ms,
  c=10 770.8 RPS p99 17.1ms, c=50 752.6 RPS p99 77.3ms, c=200 771.4 RPS
  p99 300.7ms), the fix improves the single-request path by +30.5% RPS and
  -31.0% p99 latency but does not solve the high-concurrency ceiling.
- Post-fix attributed raw proof shows local echo no longer spends time in the
  process-to-aggregator pipe during current windows: `mitm.mcp_endpoint_dispatch_ms`
  for `tools/call local_echo` is roughly 0.04-0.05ms average and p99 below
  0.09ms, while `parse_json_rpc`, `telemetry_enqueue`, `response_enqueue`, and
  `response_write` remain individually small. Raw pipelined probes still land
  around 618 RPS on one relay connection and 620 RPS across four relay
  connections, so the remaining regression is not local builtin dispatch. The
  next trace should focus on framed guest/host transport, VM/vsock scheduling,
  and session telemetry/DB side effects that are not represented by the
  existing per-stage enqueue timers.
- DB-writer backpressure collapse landed as the next code-path fix: successful
  framed MCP responses now enqueue the already policy-checked response and
  release the MCP in-flight permit before awaiting the two session DB audit
  writes (`mcp_calls` and resolved security event). This preserves request and
  response policy enforcement before bytes return to the guest, but removes
  session DB backpressure from the response critical path. Regression proof:
  `framed_mcp_response_is_not_held_behind_db_writer_backpressure` saturates a
  real `DbWriter` and still observes the framed `local__echo` response within
  200ms. Live Linux proof via `just exec "capsem-bench mcp-load"` reached c=1
  489.8 RPS p99 3.0ms, c=10 772.0 RPS p99 15.0ms, c=50 775.2 RPS p99
  108.9ms, and c=200 787.9 RPS p99 519.7ms, all zero errors. Against the
  previous fresh-initrd endpoint-fast-path run (407.2/608.4/601.0/616.8 RPS),
  RPS improved +22.6%, +28.4%, +29.0%, and +29.6% at c=1/10/50/200.
- MCP load ablation added to the canonical `capsem-bench mcp-load` path. The
  same run now reports FastMCP, raw JSON-RPC through one guest relay, and raw
  JSON-RPC through four guest relay processes. Linux proof:
  FastMCP 486.7/756.7/785.6/795.9 RPS at c=1/10/50/200; raw-single
  571.6/795.1/800.4/806.5 RPS; raw-multiprocess 550.4/780.8/782.2/816.7 RPS.
  Interpretation: FastMCP costs about 17% at c=1 but is not the high-concurrency
  ceiling. Four relay/vsock connections do not lift the cap and worsen c=200
  tail latency, so the next source trace should focus on shared host/vsock,
  framed MCP scheduling, security/telemetry CPU, or KVM virtio-vsock delivery
  rather than guest Python/FastMCP.

## First Questions

- Is the Linux RPS gap actually in KVM/vsock, or in host-side MITM/security
  processing?
- Why did `local__echo` regress to ~0.2x baseline throughput with >3x p99
  latency while producing zero errors? Trace guest stdio relay, framed vsock
  single-stream behavior, host MCP endpoint parsing, aggregator dispatch, and
  builtin stdio server round trips before changing code. The current leading
  suspect is now the process-to-aggregator/local builtin dispatch path rather
  than post-dispatch telemetry enqueue.
- Does TUI/status polling add measurable endpoint contention when sessions are
  active?
- Are weak RPS results correlated with VirtioFS workspace reads, policy-context
  file access, or session database writes?
- Are DNS and HTTP regressions separate, or both symptoms of the same
  guest-to-host bridge path?
