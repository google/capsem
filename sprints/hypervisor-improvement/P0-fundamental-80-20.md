# P0 - Fundamental 80/20 Hypervisor Advances

Last updated: 2026-06-02

## Mission

Find and land the smallest set of source-backed changes that can move Capsem's
core performance across disk, network/RPS, CPU lifecycle, and memory pressure.
Benchmarks are acceptance evidence. They are not the discovery loop.

The P0 standard is:

- trace the real code path first;
- compare it to Firecracker and crosvm by lifecycle stage, not keyword hits;
- pick one coherent architecture slice;
- add counters that explain the slice in status and OTel-ready metric points;
- run focused smoke proof during development;
- run canonical `just benchmark` only when the mechanism is ready to accept or
  reject.

## Top Five Bets

1. Event-loop and worker ownership.
   Guest notifications, completion events, queue draining, used-ring
   publication, and interrupts need one coherent ownership model per device.
   This affects disk and likely affects network/RPS.
2. Guest-memory translation and buffer path.
   Keep proving that hot paths hand the host kernel validated guest memory
   ranges directly. Where Capsem already does this, stop chasing it and move to
   the next bottleneck.
3. Cache and rootfs ownership.
   EROFS DAX is the current leading rootfs stack. The remaining work is cache
   policy, page-fault behavior, direct I/O for writable/fallback block lanes,
   and whether compressed DAX is still the right default after tuning.
4. Control-plane polling and status overhead.
   Weak RPS can be hidden by service/gateway/TUI/status polling, proxy policy,
   DNS, or session telemetry. Split these lanes before optimizing network code.
5. vCPU lifecycle, SMP scheduling, and memory pressure.
   Firecracker's vCPU state machine, pause/resume, `immediate_exit`, and exit
   metrics are likely transferable. This matters for startup, suspend/resume,
   CPU overhead, and future Android/ARM portability.

## Execution Board

| Priority | Slice | Status | Acceptance |
| --- | --- | --- | --- |
| P0.1 | Block lifecycle mechanism table: Capsem vs Firecracker vs crosvm. | Active | Table covers notify, descriptor parse, memory translation, syscall/cache policy, async completion, used-ring publication, interrupt, and counters. |
| P0.2 | Pick first block architecture slice from the table. | Not Started | Slice names the changed mechanism before any benchmark, includes tests/counters, and avoids isolated knob tweaks. |
| P0.3 | Network/RPS lifecycle table. | Active | Splits guest network, vsock, MITM, DNS, security engine, gateway/service, status/TUI polling, and disk/workspace dependencies. |
| P0.4 | vCPU/SMP lifecycle table. | Not Started | Compares Capsem KVM vCPU control to Firecracker pause/resume/start/stop and names transferable pieces. |
| P0.5 | Memory/cache attribution table. | Not Started | Separates EROFS DAX page faults, host page cache, guest cache, direct I/O lanes, and snapshot/resume memory behavior. |

## Initial P0.1 Source Trace

This is the first pass from actual source paths. It is intentionally narrow:
block I/O first, because it already has measurements and existing counters.

| Stage | Capsem KVM | Firecracker | crosvm | Current Read |
| --- | --- | --- | --- | --- |
| Guest notify | `virtio_mmio` registers per-queue `KVM_IOEVENTFD`; `VirtioBlockDevice::activate` starts per-queue worker threads when async notify is available. The io_uring worker epolls both queue notify and completion eventfd. | Event manager routes queue eventfd to `VirtioBlock::process_queue_event`. | Worker owns queue event through `EventAsync`; `handle_queue` has one async task per queue in use. | All three have eventfd-based queue wakeups. Capsem is already closer to Firecracker than expected here. |
| Queue drain | `process_queue` and `process_queue_uring` loop with `pop_or_enable_notification`; used entries are deferred and flushed once per drain. | `process_queue` loops with `pop_or_enable_notification`, calls `advance_used_ring_idx` once, then `prepare_kick`. | `handle_queue` pops all available chains after one event and pushes each chain into `FuturesUnordered`. | Capsem and Firecracker have similar drain/batch shape for sync path. crosvm goes further by spawning async per-chain work within the queue handler. |
| Memory translation | Capsem validates GPA ranges through `GuestMemoryRef::gpa_range_to_host`, then builds `libc::iovec` for `preadv`, `pwritev`, and io_uring. | Firecracker parses requests against guest memory and the async engine finishes pending requests against guest memory. | crosvm uses virtio `Reader`/`Writer` helpers and async `write_all_from_at_fut` / `read_exact_to_at_fut`. | Capsem is already on the intended zero-copy vectored shape for block data. The next win is unlikely to be removing a data `Vec` clone in the main block path. |
| Syscall/cache policy | Capsem has sync `preadv`/`pwritev`, optional direct I/O, and an io_uring path with fixed registered file, opcode probe, restrictions, and completion eventfd. The ring is per block-worker queue. | Firecracker exposes sync/async file engines; async path uses an io_uring engine with fixed fds, read/write/fsync allowlist, completion eventfd, and queue-full throttling. | crosvm converts disk image into an async disk on an executor. On Linux, raw `File` becomes `SingleFileDisk` through `Executor::async_from`; with the uring executor this registers the source with a shared uring reactor. Other disk formats can fall back to `AsyncDiskFileWrapper` and a blocking pool. | The implementation question is not "add io_uring"; it is whether Capsem's per-worker ring, cache policy, and lane selection are better than crosvm's shared executor model for our workload. |
| Completion batching | `BlockIoUring::reap_completions` drains completions into deferred used entries, flushes once, prepares one kick, and retries queue processing after completions free capacity. Drain replies wait until `pending_len == 0`. | Firecracker drains async completions in a loop, advances used-ring index once, kicks once, then resumes queue processing if throttled. | crosvm completes each async chain with `add_used_with_bytes_written` and `trigger_interrupt` from the per-chain task. | Capsem is close to Firecracker on batching now. crosvm may trade more interrupt churn for per-chain parallelism; this is a candidate for a focused source-backed experiment, not a blanket rewrite. |
| Counters | Capsem has queue notification, drain, descriptor, used-entry, interrupt, request, request-byte, request-duration, async submission/completion/fallback/full/in-flight counters. | Firecracker has block metrics near queue events, throttling, execution, and event failures. | crosvm has tracing/logging around worker and async execution; direct comparable metrics are not yet mapped. | Capsem's counter coverage is good enough to validate a mechanism slice if status/OTel visibility remains wired. |

## First Hypotheses

- The old "Capsem must be copying block buffers into a `Vec`" theory does not
  match the current KVM block source. Capsem stores iovec metadata in Rust
  vectors, but the data path points at guest memory and hands those iovecs to
  host syscalls.
- The next high-leverage disk change is probably event/executor ownership or
  cache policy, not another isolated queue-size or segment-size change.
- Capsem's current io_uring worker is already lean and close to Firecracker:
  queue notify and completion fd share one epoll loop, submissions are kicked as
  a batch, completions are drained as a batch, and backpressured descriptors are
  retried after completion. A wholesale "make it Firecracker-shaped" rewrite is
  therefore unlikely to be the next 80/20 win.
- crosvm is interesting for a different reason: it can use a shared uring
  executor and fan descriptor chains into async tasks. That may help parallelism
  on some lanes, but it may also increase interrupt/task overhead. Treat this
  as a trace-backed candidate, not a default assumption.
- The same source-trace method should be applied to network/RPS before tuning
  proxy or socket constants, because the current weak RPS could come from
  status polling, gateway/service overhead, MITM policy, DNS, vsock, or disk.
- If a proposed change cannot improve at least one of disk, RPS, CPU overhead,
  memory/cache behavior, or user-visible telemetry, it is not P0.

## Initial P0.3 Control-Plane Trace

The first network/RPS read found a control-plane lane worth instrumenting before
any benchmark loop:

- `capsem-tui` defaults to a 1 second live refresh interval and calls
  `GatewayProvider::load()` on each refresh.
- `GatewayProvider::load_async()` fetches `/status`, then refreshes profile
  options through `/profiles`.
- `capsem-gateway` caches `/status` for 1 second and serializes refreshes with
  a mutex, but a refresh fans out to capsem-service `/list` plus `/info/{id}`
  for each running VM.
- MITM and DNS already have useful request, hook, upstream, body, DNS-cache,
  and DNS-duration metric names in `capsem-core`.

First implementation slice: add gateway `/status` cache, refresh, and
service-fan-out metrics so endpoint latency can be attributed to cache misses,
running-VM `/info` fan-out, or another lane.

Second implementation slice: add bounded gateway proxy request metrics for the
catch-all service proxy. These record endpoint class, method, status class, and
duration, so TUI `/profiles` refreshes and action traffic can be separated from
`/status` cache behavior and from guest network/MITM paths.

Third implementation slice: add bounded process-side vsock connection metrics.
The Linux guest HTTP path is `capsem-net-proxy` accepting redirected localhost
TCP, opening a host vsock connection for each client connection, and handing the
host fd to `capsem-process` dispatch before MITM/DNS/security work begins.
Capsem now records accepted/closed/active/duration metrics for
`terminal|control|sni_proxy|dns_proxy|audit|exec|lifecycle|unknown`, so weak RPS
can be separated into vsock dispatch pressure versus downstream MITM, DNS,
security-engine, or gateway/control-plane work.

First RPS mechanism change: the guest `capsem-net-proxy` process lookup no
longer walks every `/proc/<pid>/fd` directory per connection. It now resolves
the client TCP socket inode and consults a shared throttled socket-owner index.
This attacks guest CPU work that is independent of KVM versus Apple VZ and must
be validated with both throughput and process-attribution quality in a real VM.

## Immediate Next Slice

Finish P0.1 by tracing the remaining details:

- crosvm queue interrupt policy and how much per-chain completion churn appears
  under read-heavy loads.
- Firecracker async file engine request submission path, to compare exact
  syscall and pending-request ownership.
- Capsem vs crosvm executor ownership: per-queue ring/thread versus shared
  uring reactor and async task fan-out.

Only after that table is complete should we choose between:

- simplifying Capsem's block worker into a tighter Firecracker-style event loop;
- adopting crosvm-style per-chain async fan-out where it clearly helps;
- changing cache/direct-I/O policy for scratch and fallback block lanes;
- moving to network/RPS lifecycle tracing because disk is no longer the best
  80/20 target.
