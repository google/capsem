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
- Remaining proof: run guest HTTP/proxy throughput in a real VM and confirm
  process-side vsock metrics move alongside existing MITM/DNS metrics, then
  expose the useful subset through status/session telemetry before making an
  RPS performance claim.

## First Questions

- Is the Linux RPS gap actually in KVM/vsock, or in host-side MITM/security
  processing?
- Does TUI/status polling add measurable endpoint contention when sessions are
  active?
- Are weak RPS results correlated with VirtioFS workspace reads, policy-context
  file access, or session database writes?
- Are DNS and HTTP regressions separate, or both symptoms of the same
  guest-to-host bridge path?
