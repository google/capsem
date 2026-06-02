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
- Framed MCP hot-path cleanup landed as a structural cross-architecture slice:
  `capsem-proto` now has a borrowed MCP frame decoder, the guest relay writes
  response payloads from borrowed frame buffers, the host parses inbound
  payloads without copying them into an owned `McpFrame`, MCP telemetry reuses
  JSON preview strings for byte counts, related `mcp_calls` and
  `security_events` rows share one `DbWriter` sender clone, and ready response
  frames are batched into one write/flush per connection. Scoped Linux
  `raw-single` proofs:
  - after borrowed decode only: 565.8/751.2/782.8/806.2 RPS at c=1/10/50/200;
  - after telemetry enqueue cleanup: 571.6/767.8/783.6/830.6 RPS;
  - after response batching: 576.4/805.4/788.4/819.6 RPS, with c=200 p99
    improving from roughly 498.7ms to 358.2ms in the same 5s scoped shape.
  Interpretation: payload copies and response flush granularity were not the
  primary throughput cap, but batching reduces high-concurrency tails. The
  remaining RPS ceiling is still shared host/vsock/framed scheduling or
  security/telemetry CPU, not FastMCP, raw guest relay, or response syscalls
  alone.
- Host-only framed MCP diagnostic added as an ignored Rust test:
  `cargo test -p capsem-core framed_mcp_host_duplex_throughput_diagnostic --
  --ignored --nocapture`. It drives the production `serve_io` framed parser,
  MCP policy, endpoint-local `local__echo`, response writer, and session DB
  telemetry over `tokio::io::duplex`, without guest stdio relay or vsock. First
  Linux proof processed 10,000 requests in 395.4ms, or 25,290.2 RPS. This
  strongly isolates the ~800 RPS VM `raw-single` ceiling away from host
  framed-MCP/security/telemetry CPU and toward guest relay, KVM/vsock delivery,
  or host-vsock socket integration.
- Direct-vsock lane added to `capsem-bench mcp-load` to bypass the guest
  `/run/capsem-mcp-server` stdio relay and speak framed MCP directly from the
  guest benchmark process to host vsock:5002. Scoped Linux proof:
  raw-single 574.0/784.8/784.8/822.0 RPS and direct-vsock
  572.2/806.4/811.0/842.8 RPS at c=1/10/50/200, all zero errors. Direct
  vsock is only +0%/+2.7%/+3.4%/+2.5% over the raw relay lane, so the guest
  stdio relay is not the main cap. Combined with the host-only 25k RPS proof,
  the next target is KVM/vsock delivery or host-vsock socket integration.
- KVM vhost-vsock queue notifications now expose the backend RX/TX kick
  eventfds and register them through `KVM_IOEVENTFD` on the virtio-mmio
  `QUEUE_NOTIFY` register, matching the virtio-blk shape and avoiding a
  userspace MMIO queue-notify bounce for normal guest vsock writes. Unit proof:
  `cargo test -p capsem-core hypervisor::kvm --lib` passed 350 KVM tests.
  Live proof via `just exec "capsem-bench mcp-load"` booted and completed, but
  throughput did not materially move: raw-single 573.4/765.2/785.4/815.0 RPS
  and direct-vsock 590.0/812.7/813.6/825.4 RPS at c=1/10/50/200, all zero
  errors. Against the prior 5s scoped direct-vsock proof
  572.2/806.4/811.0/842.8 RPS, that is +3.2%/+0.8%/+0.3%/-2.1%; queue-notify
  trapping alone is therefore not the ~800 RPS ceiling.
- KVM vhost-vsock now advertises `VIRTIO_RING_F_EVENT_IDX`; the live backend
  accepted it with `enabled_features=0x120000000`, confirming
  `VERSION_1|EVENT_IDX` rather than an unsupported guest-visible bit. Unit
  proof: `cargo test -p capsem-core hypervisor::kvm --lib` passed 350 KVM
  tests. Scoped live proof:
  `just exec "CAPSEM_BENCH_MCP_LANES=raw-single,direct-vsock
  CAPSEM_BENCH_MCP_DURATION=5 capsem-bench mcp-load"` completed with
  raw-single 591.6/767.8/773.6/818.0 RPS and direct-vsock
  589.6/782.0/789.8/834.2 RPS at c=1/10/50/200, all zero errors. Compared
  with the prior scoped direct-vsock proof 572.2/806.4/811.0/842.8 RPS, the
  direct-vsock deltas are +4.9%/-2.9%/-2.6%/-1.0%; event-index is correct
  virtio hygiene, but not the remaining throughput limiter.
- Direct-vsock transport-only attribution landed next. The benchmark now has a
  `direct-vsock-transport` lane that uses the same guest AF_VSOCK connection,
  host KVM/vhost-vsock path, `AsyncFdStream`, framed MCP parser, stream
  tracker, and writer batch path, but handles a reserved diagnostic echo before
  MCP policy, endpoint dispatch, aggregator, or session DB writes. Unit proof:
  `cargo test -p capsem-core net::mitm_proxy::mcp_frame --lib` passed 12 tests
  with one ignored diagnostic, `cargo test -p capsem-core hypervisor::kvm
  --lib` passed 350 KVM tests, and `uv run python -m pytest
  tests/test_capsem_bench_mcp_load.py -q` passed 7 tests. Scoped live proof:
  `just exec "CAPSEM_BENCH_MCP_LANES=direct-vsock,direct-vsock-transport
  CAPSEM_BENCH_MCP_DURATION=5 capsem-bench mcp-load"` measured same-run
  direct-vsock 588.0/812.8/806.0/822.8 RPS and transport-only
  3,086.6/13,632.2/22,003.0/37,027.6 RPS at c=1/10/50/200, all zero errors.
  The transport lane is 5.25x/16.77x/27.30x/45.00x faster with p99
  0.5/1.4/2.8/12.6ms versus direct-vsock 2.6/13.6/105.2/341.1ms. Conclusion:
  raw KVM/vhost-vsock transport and the host frame codec are not the current
  ~800 RPS ceiling; the next code target is the real MCP policy/dispatch/
  telemetry path after frame parsing.
- Rejected experiment: a guarded default-policy `local__echo` fast path that
  bypassed the general MCP inflight/task/policy path was tested but not
  accepted. It changed same-run direct-vsock RPS from
  588.0/812.8/806.0/822.8 to 596.8/813.0/811.8/828.2 at c=1/10/50/200
  (+1.5%/+0.0%/+0.7%/+0.7%). It improved some tail latency, especially c=200
  p99 341.1ms -> 304.8ms, but it is too local to the diagnostic benchmark and
  does not attack real external MCP tools or policy-heavy paths. Do not pursue
  local-echo-only bypasses as the next H09 implementation target.
- Rejected experiment: a per-connection MCP audit worker that moved successful
  response audit construction/writes off the request task was tested but not
  accepted. Same-run direct-vsock RPS changed from 588.0/812.8/806.0/822.8 to
  562.6/781.4/811.8/840.6 at c=1/10/50/200 (-4.6%/-3.9%/+0.7%/+2.3%), and
  c=200 p99 worsened 341.1ms -> 363.3ms. The shape suggests the bottleneck is
  not simply "request tasks keep doing audit work after response enqueue."
  Leave audit-worker fanout alone until a stronger trace points there.
- Accepted writer-path cleanup: `DbWriter` MCP execution evidence now derives
  request `arguments` and response JSON/text classification with borrowed
  `serde_json::RawValue` parsing instead of allocating a full `serde_json::Value`
  DOM on the writer thread. This preserves the same security behavior and audit
  rows: `mcp_calls` still insert, `ai_mcp_execution_evidence` still inserts,
  malformed previews keep the audit row/evidence row with text classification,
  and framed MCP blocked-request security-event logging still passes. Focused
  proof: `cargo test -p capsem-logger mcp_` passed 21 tests across logger unit
  and roundtrip suites, and `cargo test -p capsem-core log_mcp_call_writes_`
  passed the canonical and blocked MCP security-event logging tests. Scoped
  live proof after the change measured direct-vsock
  593.6/773.8/792.4/836.2 RPS at c=1/10/50/200, all zero errors, versus the
  accepted same-lane baseline 588.0/812.8/806.0/822.8. That is
  +1.0%/-4.8%/-1.7%/+1.7%, so keep this as cross-architecture writer hygiene,
  not an RPS breakthrough.
- Rejected knob-only experiment: raising `CAPSEM_MCP_INFLIGHT` from the live
  default 64 to 256 did not lift direct-vsock throughput. The scoped proof
  measured 560.2/758.2/801.6/827.4 RPS at c=1/10/50/200, all zero errors,
  versus the prior same-lane 593.6/773.8/792.4/836.2 after the writer cleanup.
  The in-flight semaphore cap is not the main limiter.
- Runtime security-engine attribution found the real high-concurrency limiter:
  live `everyday-work` sessions install a runtime security engine, while the
  host-only diagnostic had none. Before the fix, recorder snapshots showed
  `runtime_security_evaluate` growing from about 1.13ms p99 at low load to
  about 20-22ms p50/p99 under high concurrency, while parse, endpoint dispatch,
  response enqueue, and response write stayed sub-millisecond. Root cause:
  `capsem-process` wrapped a single mutable `SecurityEngine` in one
  `Mutex`, serializing every concurrent MCP security evaluation.
- Accepted runtime security-engine pool fix: `capsem-process` now installs a
  CPU-sized pool of identical compiled `SecurityEngine` instances with a shared
  `RuntimeRuleMatchAccumulator`. This preserves blocking, detection, and
  rule-match telemetry while removing the single evaluator mutex queue. Unit
  proof: `cargo test -p capsem-process mcp_runtime --bin capsem-process`
  passed 15 tests, including parallel rule-match aggregation. MCP frame proof:
  `cargo test -p capsem-core net::mitm_proxy::mcp_frame --lib` passed 13 tests
  plus one ignored diagnostic, including a test that a runtime MCP block still
  denies dispatch while recording `runtime_security_project` and
  `runtime_security_evaluate` histograms.
- Clean live proof after the pool fix: direct-vsock `mcp-load` measured
  586.0/3775.4/5564.0/5661.0 RPS at c=1/10/50/200, all zero errors, versus
  the accepted same-lane baseline 588.0/812.8/806.0/822.8. Deltas:
  -0.3%/+364.5%/+590.3%/+588.1%. p99 improved from 2.6/13.6/105.2/341.1ms
  to 2.0/3.6/15.4/42.3ms. Post-fix recorder proof with a 3s run showed
  `runtime_security_evaluate` around p50 2.0ms and p99 3.1ms at high
  concurrency instead of the previous 20-22ms queue.
- Audit-writer cleanup fast path landed as a follow-up that does not weaken
  logging or blockability: `DbWriter` now checks whether a resolved
  security-event ID already exists before deleting child rows. New MCP/security
  events avoid four empty cleanup deletes; repeated event IDs still remove and
  replace stale steps, findings, tags, and links before persisting the updated
  decision. The regression proof is
  `resolved_security_event_rewrite_removes_stale_child_rows`. Scoped live proof
  on the fresh build measured direct-vsock `mcp-load`
  593.4/3758.0/5630.6/5749.0 RPS at c=1/10/50/200, all zero errors, versus
  the post-pool baseline 586.0/3775.4/5564.0/5661.0. Deltas:
  +1.3%/-0.5%/+1.2%/+1.6%, so this is retained as audit-writer hygiene, not a
  new throughput breakthrough.
- Runtime security event-family routing landed as the next code-path
  improvement. `RuntimeSecurityEngine` now advertises whether it can evaluate a
  requested event family, and the MCP/HTTP/DNS/process exec paths only call the
  runtime CEL engine when the installed engine declares that family. Structured
  effective rules derive families from their callbacks; live runtime snapshots
  remain all-family because they do not carry callback metadata. The
  runtime-engine install log records the event-family scope so live sessions can
  be audited for `all` versus callback-derived scopes such as `dns,http`.
  Security proof: `non_mcp_runtime_scope_preserves_policy_block_and_logging`
  verifies a non-MCP runtime engine is not evaluated for MCP while MCP policy
  still blocks `local__echo` and persists the canonical `mcp.request` security
  event; `runtime_mcp_security_stages_are_recorded_without_bypassing_block`
  verifies an MCP-capable runtime block still denies dispatch. Scoped live
  proof on the fresh build measured direct-vsock `mcp-load`
  2188.2/10163.2/13934.4/13935.2 RPS at c=1/10/50/200, all zero errors, versus
  the immediately prior fresh-build audit-writer run
  593.4/3758.0/5630.6/5749.0. Deltas:
  +268.7%/+170.4%/+147.5%/+142.4%; p99 improved from
  2.2/3.8/15.2/42.7ms to 1.0/1.6/4.9/19.0ms.
- Post-routing attributed recorder run on `upbeat-beacon-tmp` used
  `CAPSEM_METRICS_DEBUG_INTERVAL_SECS=2` with 3s levels. Runtime install log
  confirmed `event_family_scope="dns,http"` for `everyday-work`, so MCP runtime
  CEL was not evaluated on this path. Recorder-overhead run measured
  direct-vsock 1901.3/8478.7/9431.0/9751.7 RPS and transport-only
  3023.0/10824.3/16355.7/20589.7 RPS at c=1/10/50/200. Stage snapshots for
  `tools/call local_echo` showed parse, endpoint dispatch, response enqueue,
  and response write all below ~0.1ms p99 in steady windows. The remaining
  visible backlog is post-response `telemetry_enqueue`: it grows from
  sub-0.1ms early to seconds under sustained load because durable session DB
  audit writes lag behind completed responses. This does not currently block
  guest-visible responses, but it is the next durability/resource-efficiency
  target before claiming the MCP path is clean.

## First Questions

- Is the Linux RPS gap actually in KVM/vsock, or in host-side MITM/security
  processing? Current answer: not raw KVM/vhost-vsock transport. Host-only
  framed MCP is fast, direct-vsock matches the raw relay cap, KVM ioeventfd
  queue-notify plus event-index wiring did not move the ceiling, and the new
  transport-only direct-vsock lane reaches up to 37k RPS in the same VM. The
  next trace target is real MCP policy/security/telemetry/inflight scheduling
  after frame parsing, not another KVM/vsock transport knob.
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
