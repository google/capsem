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

## First Questions

- Is the Linux RPS gap actually in KVM/vsock, or in host-side MITM/security
  processing?
- Why did `local__echo` regress to ~0.2x baseline throughput with >3x p99
  latency while producing zero errors? Trace guest stdio relay, framed vsock
  single-stream behavior, host MCP endpoint parsing, aggregator dispatch, and
  builtin stdio server round trips before changing code.
- Does TUI/status polling add measurable endpoint contention when sessions are
  active?
- Are weak RPS results correlated with VirtioFS workspace reads, policy-context
  file access, or session database writes?
- Are DNS and HTTP regressions separate, or both symptoms of the same
  guest-to-host bridge path?
