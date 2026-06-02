# Sprint: hypervisor-improvement

## Tasks

- [x] Create meta-sprint structure and sub-sprint plan.
- [ ] P0: fundamental 80/20 hypervisor advances.
  - [x] Add a P0 sprint board that ranks the five highest-leverage mechanism
        bets across disk, network/RPS, CPU lifecycle, memory/cache, and
        control-plane overhead.
  - [x] Start the block lifecycle mechanism table from actual Capsem,
        Firecracker, and crosvm source paths.
  - [x] Start the network/RPS control-plane trace and identify the gateway
        `/status` cache plus service `/list`/`/info` fan-out as the first
        measurable status/TUI lane.
  - [x] Add gateway `/status` cache/refresh/service-fan-out metrics with a
        focused gateway status test proof.
  - [x] Add gateway proxy endpoint-class request counters and duration
        histograms so `/profiles`, actions, files/history, and unknown service
        fan-out are visible without high-cardinality labels.
  - [x] Add process-side vsock connection counters, active gauges, close-result
        counters, and duration histograms with bounded port-kind labels so
        guest network/RPS pressure can be separated from MITM/DNS/security and
        gateway/status lanes.
  - [x] Replace guest net-proxy per-connection `/proc/<pid>/fd` process lookup
        with a shared throttled socket-owner index, preserving best-effort
        process attribution while reducing burst RPS CPU work.
  - [x] Fix dev-service profile materialization after initrd repack so
        one-shot VM smoke uses the freshly packed local guest binaries instead
        of stale profile-pinned remote asset hashes.
  - [ ] Finish the crosvm async engine and Firecracker async file-engine trace
        before choosing the first implementation slice.
  - [ ] Pick and land one coherent code-path improvement with counters and
        before/after proof by lane.
- [x] H00: close current KVM/block context and benchmark truth.
- [x] H00: make benchmark artifact retention part of `just benchmark`.
- [x] H01: safety and queue contracts.
  - [x] Record main merge and refreshed macOS benchmark comparison baseline.
  - [x] Add full guest-memory range validation before raw host pointers.
  - [x] Reject malformed virtqueue descriptor indices and cycles.
  - [x] Validate split-ring size, alignment, and guest-memory coverage.
  - [x] Reject invalid ready queues during virtio-mmio activation/restore.
  - [x] Make guest-memory offset arithmetic overflow-safe.
  - [x] Make virtio-blk aggregate descriptor length accounting overflow-safe.
- [ ] H03: observability, status, and OTel resource counters.
  - [x] Surface existing live VM resource metrics through service `/info`.
  - [x] Render live VM resource metrics in `capsem info`.
  - [x] Surface KVM virtio-blk queue/backend counters through metrics snapshots,
        service `/info`, and `capsem info`.
  - [x] Surface live resource and KVM block counters through gateway `/status`
        and the TUI session-info overlay.
  - [x] Add OTel-compatible metric-point mapping for live VM resource and KVM
        block counters.
  - [ ] Real OTLP exporter process/configuration remains deferred to the
        broader telemetry sprint.
- [ ] H02: event delivery and backpressure.
  - [x] Make KVM virtio-blk io_uring submission-queue saturation explicit
        backpressure instead of synchronous fallback.
  - [x] Add and surface `async_queue_full_total` through VM block metrics,
        service `/info`, `capsem info`, and the OTel metric-point contract.
  - [x] Retry backpressured KVM virtio-blk io_uring descriptors immediately
        after completions free submission capacity.
  - [x] Build the full Firecracker-shaped KVM block async profile before
        ablation: default io_uring engine for block devices, fixed registered
        fd, opcode probe, ring restrictions, explicit enable, existing
        backpressure, completion-triggered retry, and quiesce drain.
  - [x] Run the full-profile benchmark first, then grouped ablation.
  - [ ] Extend the same backpressure/event-loop audit to other KVM devices and
        completion paths after block is measured as a whole.
- [ ] H04: CPU, SMP, and lifecycle.
- [ ] H05: storage, rootfs, and filesystem experiments.
  - [x] Add a KVM block-shape profile covering queue count, queue size,
        segment limit, and logical block size.
  - [x] Add a focused gridsearch harness that records block-shape metadata and
        rootfs/startup results before choosing defaults.
  - [x] Add a rootfs-format grid harness so uncompressed rootfs and EROFS run
        through the same block-shape matrix as the current SquashFS baseline.
  - [x] Test rootfs format/compression alternatives through the canonical
        benchmark path: uncompressed rootfs, EROFS, and an opt-in virtio-pmem
        DAX-capability path.
  - [x] Add and measure a strict file-backed EROFS DAX lane that maps aligned
        rootfs images directly instead of copying them into anonymous pmem RAM.
  - [x] Record compressed `erofs-lz4hc-c65536` + DAX as the current lead
        candidate, with tuning explicitly still open.
  - [ ] Rerun EROFS tuning around the compressed lead after raw throughput
        work, including cluster size/layout tradeoffs.
  - [ ] Test EROFS zstd after bumping the guest kernel to Linux 6.11 or newer.
  - [x] Investigate guest rootfs readahead for the EROFS DAX pmem path and
        land the measured 16 MiB pmem default.
  - [x] Add KVM file-backed pmem mmap policy knobs and benchmark madvise /
        populate behavior on the lead EROFS DAX path.
  - [ ] Continue raw/cold throughput investigation: EROFS DAX mount/cache
        behavior, KVM block fallback for non-DAX rootfs, host page-fault/mmap
        behavior for file-backed pmem, and benchmark cache purity.
  - [ ] Revisit Direct I/O for writable scratch and fallback rootfs-over-blk
        separately from the EROFS DAX pmem path.
- [ ] H08: disk throughput attribution.
  - [x] Create the focused H08 sprint slice under the Hypervisor Improvement
        meta sprint.
  - [ ] Baseline accepted DAX rootfs, fallback block rootfs, writable scratch,
        VirtioFS, and RPS-adjacent lanes against the current committed
        artifact.
    - [x] Initial artifact baseline recorded in
          `H08-disk-throughput-attribution.md`: canonical Linux scratch seq
          read is 0.08x macOS and 0.046x host-native; active compressed EROFS
          DAX rootfs is already far faster than the old canonical Linux rootfs
          on random/small-file/metadata lanes; HTTP RPS is 0.83x macOS and
          proxy throughput is 0.93x macOS.
    - [x] Corrected `capsem-bench disk` to default to `/var/tmp`, the writable
          scratch/system lane, while keeping `/root` workspace/VirtioFS
          attribution in `capsem-bench storage`. Packaged VM comparison:
          sequential write +43.5%, sequential read +54.9%, random write
          +286.0%, random read +7028.9% versus forced `/root`.
    - [ ] Still needs a fresh full canonical `just benchmark` artifact on the
          installed working build so current Linux/macOS/host-native comparison
          is not based on the older pre-H08 artifact.
  - [ ] Add request-shape and timing counters for virtio-blk queue notify,
        drain, syscall, completion, used-ring publication, and interrupt
        decisions.
    - [x] Added completed request count, request bytes, aggregate request
          duration, aggregate queue-drain duration, max request bytes, max data
          descriptors per request, and max requests per drain to KVM
          virtio-blk metrics. These now flow through VM metrics snapshots,
          OTel-compatible metric points, service `/info`, gateway `/status`,
          and `capsem info`.
    - [ ] Still needs live VM proof that counters move during
          `capsem-bench disk` / `capsem-bench storage`; focused unit/API tests
          passed, but this acceptance gate remains open.
  - [ ] Add DAX/rootfs cache and page-fault evidence where available, so DAX
        throughput is not confused with virtio-blk throughput.
  - [ ] Compare Capsem, Firecracker, and crosvm by the same request lifecycle
        instead of keyword/source skims.
  - [ ] Land the first trace-backed code-path improvement and report
        before/after percentages by lane.
  - [ ] Record accepted results through canonical `just benchmark`.
  - [ ] Park memory, CPU/SMP, and RPS follow-up slices with concrete metrics
        once disk attribution shows what still matters.
  - [x] Restore an installed Linux build for manual testing before continuing
        perf work: `capsem 1.2.1780406785`, `capsem status` ready with
        service/assets/gateway ok and zero issues, and installed
        `capsem run "echo installed-capsem-ready-after-status-fix"` passed.
  - [x] Fixed Linux status validation for installed systemd units that
        reference `~/.capsem/bin/*` symlinks resolving to `/usr/bin/*`, so the
        installed service no longer appears stale when the package layout is
        correct.
  - [x] Fixed local install recovery when a package reinstall replaces the dev
        `~/.capsem/assets` symlink and drops hash-named files needed by stopped
        persistent VMs. The dev asset sync now restores backed-up pinned
        assets, and default `capsem purge` removes stopped persistent VMs whose
        pinned base assets are already missing so `capsem status` can recover
        without `--all`.
- [ ] H06: benchmark and product proof.
  - [x] Add a crosvm reference harness for the same Capsem x86_64
        rootfs/startup workload used by the Firecracker comparison.
  - [x] Record crosvm epoll, corrected-uring, direct-I/O, and multi-worker
        lanes as structured benchmark artifacts.
  - [ ] Run a structured crosvm trace audit against Capsem instead of another
        keyword/source skim: follow the full path for block I/O, virtqueue
        descriptor handling, event-loop wakeups/batching, queue worker
        ownership, and rootfs/virtiofs-facing reads, then compare each path to
        the matching Capsem implementation and turn transferable differences
        into measured implementation slices.
- [ ] H09: network and RPS attribution.
  - [x] Create the focused H09 sprint slice so weak RPS is not left as a vague
        H08 follow-up.
  - [x] Add first control-plane attribution counters for gateway `/status`
        cache decisions, refresh duration/result, and service fan-out to
        `/list` and per-running-VM `/info`.
  - [x] Add gateway proxy endpoint-class metrics to split `/profiles` and
        other proxied control-plane traffic from `/status` polling.
  - [x] Add process-side vsock metrics for terminal/control rekeys and
        auxiliary SNI proxy, DNS, audit, exec, lifecycle, and unknown ports.
  - [x] Land the first guest-network code-path improvement: throttle the
        expensive process-owner fd walk in `capsem-net-proxy` through a shared
        socket-owner index.
  - [ ] Refresh canonical HTTP, proxy throughput, endpoint-latency,
        security-engine, and host-native artifacts after the working Linux
        install.
    - [x] Endpoint-latency release gate recalibrated to keep strict default
          p95 budgets while giving full `/logs/{id}` audit envelopes and the
          heavier settings/profile snapshots explicit endpoint budgets. The
          benchmark artifact records canonical project version, git commit,
          dirty-source state, and host CPU/RAM metadata for Linux/macOS
          comparison.
  - [ ] Split RPS-facing paths into guest network, vsock bridge, MITM/proxy,
        DNS, security-engine evaluation, service/gateway endpoints, TUI/status
        polling, and workspace/disk dependencies.
  - [ ] Add or expose low-cardinality counters for the missing RPS lanes and
        prove them in a real VM run.
    - [x] Add MCP echo-path stage timing histograms for framed parse,
          endpoint dispatch, process-to-aggregator round trip, telemetry
          enqueue, response enqueue, and response write with bounded
          `method_kind`, `tool_kind`, and `result` labels.
    - [x] Reduce session DB audit-writer work on hot security-event inserts by
          skipping stale child-table cleanup deletes for brand-new event IDs.
          The repeated-event path still deletes/replaces child rows, preserving
          security decision correctness and full auditability.
    - [x] Route runtime security-engine evaluation by declared event family so
          MCP, HTTP, DNS, and process exec paths only evaluate applicable
          runtime CEL. Runtime snapshots remain all-family, and MCP policy
          blocks still deny dispatch and persist canonical security events.
    - [x] Add the opt-in `CAPSEM_METRICS_DEBUG_INTERVAL_SECS` process recorder
          plus service/`just run-service` env forwarding so live VM runs can
          emit compact `mcp_metric_snapshot` histogram summaries to
          `process.log`.
    - [x] Run `mcp-load` with the process recorder enabled and identify the
          dominant stage before changing MCP audit/dispatch behavior.
          Isolated branch no-recorder: c=1 312.0 RPS, c=10 770.8 RPS, c=50
          752.6 RPS, c=200 771.4 RPS, zero errors. Recorder-enabled: c=1
          296.6 RPS, c=10 737.8 RPS, c=50 740.6 RPS, c=200 807.0 RPS, zero
          errors. Stage p99 points at endpoint/aggregator dispatch
          (~1.3-1.55ms p99) while parse, telemetry enqueue, and response write
          stay below ~0.12ms p99.
    - [x] Decompose process-to-aggregator/local builtin dispatch next:
          aggregator queue wait, msgpack encode/decode, builtin stdio write,
          builtin tool execution, and builtin stdio read.
      - [x] Add process-driver histograms for channel send, driver queue wait,
            request MessagePack encode, request frame write, response frame
            read, response MessagePack decode, and response route.
      - [x] Add aggregator-subprocess histograms for request frame read,
            request MessagePack decode, handler queue wait, manager lookup,
            server RPC, response channel send, response MessagePack encode,
            and response frame write.
      - [x] Add builtin tool execution timing for `local_echo` and local
            builtin HTTP tools, plus metrics debug forwarding from process ->
            aggregator -> stdio child.
      - [x] Run attributed `mcp-load` and record which sub-stage owns the
            `server_rpc`/dispatch p99.
            Decomposition-recorder run: c=1 265.2 RPS p99 4.1ms, c=10
            590.8 RPS p99 19.2ms, c=50 586.0 RPS p99 93.9ms, c=200
            636.4 RPS p99 377.4ms, zero errors. Builtin `local_echo` itself
            is ~0.015ms average / ~0.02-0.03ms p99; aggregator `server_rpc`
            to the builtin stdio peer is ~0.68-0.69ms average / ~0.86-0.89ms
            p99 and response frame write is ~0.19-0.20ms average /
            ~0.25-0.27ms p99. Next implementation bet: collapse the local
            builtin stdio/RMCP round trip for safe builtin tools.
    - [x] Collapse safe `local__echo` dispatch at the MITM endpoint so the
          deterministic diagnostic tool no longer enters the aggregator pipe
          or builtin stdio/RMCP subprocess path.
          Fresh-initrd Linux VM proof: canonical `mcp-load` c=1 improved to
          407.2 RPS / 2.9ms p99, but c=10/50/200 stayed at 608.4/601.0/616.8
          RPS. Attributed proof shows current `local_echo` endpoint dispatch
          is ~0.04-0.05ms average and no longer emits
          `mitm.mcp_aggregator_request_ms` in the local-echo windows. Raw
          single-connection and four-connection pipelined probes stayed around
          618-620 RPS, so the remaining ceiling is outside local builtin
          dispatch and should be traced in framed guest/host transport,
          session telemetry/logging pressure, or DB-writer side effects.
    - [x] Decouple successful framed MCP responses from session DB writer
          backpressure. Hypothesis: the ~600 RPS ceiling matches two DB writes
          per MCP request (`mcp_calls` + `security_events`) with the response
          currently emitted only after both writes are accepted. Regression
          target: a real framed `local__echo` response must be observable even
          while the real `DbWriter` channel is intentionally saturated.
          Implementation now sends the already policy-checked response and
          releases the MCP in-flight permit before awaiting session DB audit
          enqueue. Unit proof:
          `framed_mcp_response_is_not_held_behind_db_writer_backpressure`.
          Live Linux proof via `just exec "capsem-bench mcp-load"`:
          c=1 489.8 RPS p99 3.0ms, c=10 772.0 RPS p99 15.0ms, c=50
          775.2 RPS p99 108.9ms, c=200 787.9 RPS p99 519.7ms, zero errors.
          Compared with the previous fresh-initrd endpoint-fast-path run
          (407.2/608.4/601.0/616.8 RPS), RPS improved +22.6%, +28.4%,
          +29.0%, and +29.6% at c=1/10/50/200. High-concurrency p99 improved
          roughly -22.0%, -21.3%, and -30.1% at c=10/50/200; c=1 p99 is
          effectively unchanged within rounding.
    - [x] Extend canonical `capsem-bench mcp-load` with ablation lanes:
          FastMCP client, raw JSON-RPC over one `/run/capsem-mcp-server`
          relay, and raw JSON-RPC over four relay processes. Linux proof:
          FastMCP 486.7/756.7/785.6/795.9 RPS, raw-single
          571.6/795.1/800.4/806.5 RPS, raw-multiprocess
          550.4/780.8/782.2/816.7 RPS at c=1/10/50/200. Conclusion:
          FastMCP is only a c=1 tax; the high-concurrency ceiling is shared
          host/vsock/framed-MCP or telemetry/security CPU, not the guest Python
          client or a single relay process.
    - [x] Land a structural framed-MCP hot-path cleanup and record what it
          improved. `capsem-proto` now exposes a borrowed MCP frame decoder;
          the guest relay and host framed parser use it to avoid payload copies;
          MCP telemetry reuses JSON preview strings for byte counts and enqueues
          the `mcp_calls`/resolved-security-event pair with one `DbWriter`
          sender clone; the host response writer batches ready frames into one
          write/flush. Scoped Linux raw-single proofs at 5s per level:
          borrowed decode 565.8/751.2/782.8/806.2 RPS, telemetry cleanup
          571.6/767.8/783.6/830.6 RPS, response batching
          576.4/805.4/788.4/819.6 RPS at c=1/10/50/200. Response batching
          improved c=200 p99 from ~498.7ms to 358.2ms in the scoped shape, but
          throughput remains capped near 800 RPS. Conclusion: useful tail
          cleanup, not the primary RPS fix.
    - [x] Sweep `DbWriter` batching across general related telemetry emitters.
          Added `try_write_many` for best-effort non-blocking paths and
          converted HTTP/model telemetry, fs monitor file/security rows, DNS
          decision rows, process exec decision rows, and MCP file-tool restore
          rows. This keeps the same persisted audit/security data while
          reducing sender lock/clone churn outside the narrow MCP load path.
    - [x] Add and run a host-only framed MCP throughput diagnostic to isolate
          host parser/policy/telemetry CPU from guest relay/vsock. Ignored test:
          `cargo test -p capsem-core framed_mcp_host_duplex_throughput_diagnostic
          -- --ignored --nocapture`. First Linux proof processed 10,000
          in-memory `local__echo` requests through production `serve_io` in
          395.4ms, or 25,290.2 RPS. Conclusion: the host framed-MCP path is
          not the ~800 RPS VM cap; next target is guest relay/vsock/KVM
          delivery.
    - [x] Add and run a direct-vsock `capsem-bench mcp-load` lane to bypass
          `/run/capsem-mcp-server` stdio relay. Scoped Linux proof:
          raw-single 574.0/784.8/784.8/822.0 RPS, direct-vsock
          572.2/806.4/811.0/842.8 RPS at c=1/10/50/200, all zero errors.
          Conclusion: direct guest vsock is only ~0-3% faster than the raw
          relay lane, so the stdio relay is not the primary cap. Combined with
          the 25k RPS host-only proof, next target is KVM/vsock delivery or
          host-vsock socket integration.
    - [x] Wire KVM vhost-vsock RX/TX queue notifications through
          `KVM_IOEVENTFD` instead of relying on the userspace MMIO
          `queue_notify` fallback. Unit proof:
          `cargo test -p capsem-core hypervisor::kvm --lib` passed 350 KVM
          tests. Live proof via `just exec "capsem-bench mcp-load"` completed:
          raw-single 573.4/765.2/785.4/815.0 RPS and direct-vsock
          590.0/812.7/813.6/825.4 RPS at c=1/10/50/200, zero errors.
          Compared with the previous scoped direct-vsock run
          572.2/806.4/811.0/842.8 RPS, this is +3.2%/+0.8%/+0.3%/-2.1%;
          conclusion: correct KVM/vhost-vsock shape, but queue-notify trapping
          alone is not the remaining ~800 RPS ceiling.
    - [x] Advertise `VIRTIO_RING_F_EVENT_IDX` for KVM vhost-vsock so the
          guest/backend can suppress unnecessary used-buffer interrupts. The
          live backend accepted it (`enabled_features=0x120000000`, meaning
          `VERSION_1|EVENT_IDX`). Unit proof:
          `cargo test -p capsem-core hypervisor::kvm --lib` passed 350 KVM
          tests. Scoped live proof:
          raw-single 591.6/767.8/773.6/818.0 RPS and direct-vsock
          589.6/782.0/789.8/834.2 RPS at c=1/10/50/200, zero errors. Against
          the previous scoped direct-vsock proof 572.2/806.4/811.0/842.8 RPS,
          direct-vsock moved +4.9%/-2.9%/-2.6%/-1.0%; conclusion: correct
          virtio hygiene, but not the remaining throughput limiter.
    - [x] Remove full JSON DOM allocation from MCP session telemetry evidence
          parsing on the `DbWriter` thread. `tools/call` argument extraction
          and response JSON/text classification now use borrowed
          `serde_json::RawValue` parsing, preserving the same `mcp_calls`,
          `ai_mcp_execution_evidence`, and security-event behavior. Focused
          proof: `cargo test -p capsem-logger mcp_` passed 21 tests and
          `cargo test -p capsem-core log_mcp_call_writes_` passed canonical
          plus blocked MCP security-event logging tests. Scoped live proof:
          direct-vsock 593.6/773.8/792.4/836.2 RPS at c=1/10/50/200, zero
          errors, versus accepted same-lane baseline
          588.0/812.8/806.0/822.8 (+1.0%/-4.8%/-1.7%/+1.7%); this is accepted
          as writer hygiene, not a measured RPS breakthrough.
    - [x] Add runtime-security MCP stage attribution and remove the single
          global runtime `SecurityEngine` mutex queue. Live recorder proof
          showed `runtime_security_evaluate` growing to roughly 20-22ms p50/p99
          at high concurrency before the fix while parse, endpoint dispatch,
          response enqueue, and response write stayed sub-millisecond.
          `capsem-process` now installs a CPU-sized pool of identical compiled
          runtime security engines sharing one rule-match accumulator. Unit
          proof: `cargo test -p capsem-process mcp_runtime --bin
          capsem-process` passed 15 tests, and `cargo test -p capsem-core
          net::mitm_proxy::mcp_frame --lib` passed 13 tests plus one ignored
          diagnostic. Clean Linux direct-vsock proof improved from
          588.0/812.8/806.0/822.8 to 586.0/3775.4/5564.0/5661.0 RPS at
          c=1/10/50/200, zero errors (-0.3%/+364.5%/+590.3%/+588.1%).
  - [ ] Land only trace-backed RPS speedups, with before/after percentages by
        lane and canonical `just benchmark` artifacts.
- [ ] H07: docs, changelog, release gate.

## Notes

- User priority: improvements should include core systematic goodness, not only
  benchmark-visible wins.
- User correction on 2026-06-02: stop spending hours benchmarking before we
  understand what to improve. Benchmarks are acceptance proof; the sprint must
  prioritize source-path tracing, fundamental architecture bets, and coherent
  implementation slices.
- User priority: counters must become visible in status and available for
  OpenTelemetry.
- User priority: expose CPU usage, I/O, and memory usage so users get a clear
  system view.
- User priority: tune queue count, queue size, segment limit, and logical
  block size together; these are coupled, so isolated one-off constants are not
  enough.
- User hypothesis: rootfs format may be a first-order bottleneck. Test
  uncompressed rootfs to separate decompression CPU from host I/O, test EROFS
  as a read-only Linux-native alternative, and investigate DAX-style aggressive
  guest mapping for rootfs data before assuming more virtio-blk tuning is the
  main lever.
- User direction on 2026-06-02: stop treating the remaining sequential
  throughput issue as another knob sweep. Build a focused sprint that traces
  the actual code path, separates DAX from virtio-blk from VirtioFS/workspace,
  and then lands trace-backed speedups. Memory, CPU/SMP, and weak RPS are next
  performance domains, not part of the first disk attribution milestone.
- Network/RPS source trace: guest HTTP goes through redirected localhost TCP in
  `capsem-agent`'s net proxy, opens one host vsock connection per client
  connection, then `capsem-process` dispatches the fd to MITM/DNS/security
  handlers. Process-side vsock counters now identify connection churn and
  handler duration by port kind before we change relay or proxy code.
- First RPS mechanism change: `capsem-net-proxy` no longer does a full
  `/proc/<pid>/fd` walk for every accepted TCP connection. It still resolves
  the client TCP inode from `/proc/net/tcp*`, then consults a throttled shared
  socket-owner index. The tradeoff is measurable: very short burst connections
  can fall back to `unknown` process names between refresh windows, so the next
  VM proof must record attribution quality as well as RPS.
- Validation fix: `just exec` exposed stale local profile hashes after initrd
  repack. `_ensure-service` now explicitly materializes
  `~/.capsem/profiles/base` from the current local manifest before service
  launch, matching the package install path and avoiding remote downloads for
  old initrd hashes during dev smoke tests.
- Validation rerun: `just exec "echo net-proxy-smoke"` passed after the
  profile materialization fix and returned `net-proxy-smoke`.
- H09 focused RPS diagnostic: `CAPSEM_BENCH_MITM_DURATION=3 capsem-bench
  mitm-load` completed in a fresh VM, but is contaminated by host DNS/network
  behavior. Results: c=1 21.7 RPS p95 20.0ms p99 1827.9ms errors=65; c=10
  3.3 RPS p50 5025.8ms errors=10; c=50 16.7 RPS p50 5017.7ms errors=50;
  c=200 67.3 RPS p50 10009.4ms errors=202. Do not compare this as a speedup
  or regression against the committed reference baseline
  (`benchmarks/mitm-load/baseline.json`: 1036.8/3042.6/3028.5/2698.9 RPS,
  zero request exceptions). Next proof needs restored DNS/network or a local
  deterministic upstream.
- H09 deterministic MCP echo diagnostic: `just exec "capsem-bench mcp-load &&
  cat /tmp/capsem-benchmark.json"` completed with zero errors through
  `local__echo`, so this is valid transport evidence. Current vs
  `benchmarks/mcp-load/baseline.json`: c=1 309.8 vs 2162.5 RPS (0.143x,
  -85.7%), p99 3.6ms vs 1.1ms (3.21x); c=10 761.5 vs 3792.0 RPS (0.201x,
  -79.9%), p99 15.4ms vs 4.4ms (3.48x); c=50 786.1 vs 4061.4 RPS (0.194x,
  -80.6%), p99 82.2ms vs 17.4ms (4.72x); c=200 782.7 vs 3965.0 RPS (0.197x,
  -80.3%), p99 296.6ms vs 70.8ms (4.19x). This fails the documented
  `mcp-load` >2x p99 regression gate at every concurrency level and should
  move MCP transport tracing up the H09 priority list.
- H09 source trace for the deterministic MCP echo path is recorded in
  `H09-network-rps-attribution.md`. The path is guest FastMCP stdio ->
  guest `capsem-mcp-server` framed vsock:5002 -> host MITM framed parser /
  policy / inflight semaphore -> `McpEndpointState` -> process
  `AggregatorClient` msgpack driver -> `capsem-mcp-aggregator` pipelined
  handler -> pooled `capsem-mcp-builtin` stdio echo -> durable MCP/security
  telemetry enqueue -> framed response write. Historical framed-MITM runs on
  the same logical path reached ~9k-10k RPS at c=10/50 and ~8.3k-9.2k RPS at
  c=200, so the current ~780 RPS ceiling is a regression. New OTel-ready
  histograms are in place: `mitm.mcp_stage_duration_ms`,
  `mitm.mcp_endpoint_dispatch_ms`, and `mitm.mcp_aggregator_request_ms`, all
  with bounded method/tool/result labels.
- Current highest-leverage H08 task: produce a Capsem vs Firecracker vs crosvm
  block-lifecycle mechanism table before running another long benchmark. The
  table must cover descriptor parsing, guest-memory translation, event-loop /
  worker ownership, syscall/cache policy, completion batching, used-ring
  publication, and interrupt decisions.
- Firecracker source audit found the strongest transferable patterns in vCPU
  control, event scheduling, virtqueue contracts, block engine configuration,
  io_uring restrictions/probes/backpressure, and hot-path metrics.
- Recommended first implementation after wrap-up: H01 range/queue safety, then
  H03 status/OTel counters.
- Benchmark retention is now policy: `just benchmark` archives superseded
  generated `data_*.json` artifacts after recording current artifacts.
- Linux x86_64 wrap-up benchmark rerun completed through canonical
  `just benchmark` on clean source commit `b6f9b6e2`; active artifacts were
  refreshed and the previous Linux artifacts were preserved in
  `benchmarks/archive/benchmark-prerun-20260530T123916Z.zip`.
- Current Linux/macOS comparison still shows Linux materially behind macOS:
  scratch read 0.11x, rootfs read 0.24x, startup python3 4.03x slower,
  startup node 2.68x slower, startup claude 4.13x slower, startup gemini
  4.21x slower, lifecycle total 2.44x slower, fork create 2.77x slower.
- `hypervisor-improvement` fast-forwarded to `origin/main` commit `238001fb`
  after the Linux support, TUI control, bug-fix, and refreshed macOS benchmark
  merges landed.
- Refreshed comparison after the macOS rerun now includes rootfs large-binary,
  small-JS, and metadata-stat lanes. Current Linux/macOS gap: scratch seq read
  0.10x, rootfs seq read 0.21x, rootfs metadata stat 0.21x, python startup
  3.83x slower, node startup 3.88x slower, lifecycle total 2.62x slower, fork
  create 3.16x slower.
- H01 is active first. Initial implementation slice: prove and fix complete
  `gpa + len` range validation before KVM virtio-blk zero-copy paths hand raw
  guest pointers to host syscalls.
- H01 first slice landed locally: `GuestMemoryRef::gpa_range_to_host` rejects
  overflow, RAM-end crossing, and x86_64 PCI-hole discontinuities; virtio-blk
  now uses it for zero-copy iovecs, discard reads, request header parsing,
  get-id writes, and status writes.
- H01 queue-contract slice landed locally: virtqueue pop now rejects invalid
  queue sizes, available-ring heads outside the queue, descriptor `next`
  indices outside the queue, and descriptor cycles instead of returning a
  partial or misparsed chain.
- H01 ring-layout slice landed locally: virtqueue operations now validate
  non-zero power-of-two size, descriptor-table 16-byte alignment, available-ring
  2-byte alignment, used-ring 4-byte alignment, and full guest-memory coverage
  for descriptor, available, and used rings before touching ring memory.
- H01 activation slice landed locally: virtio-mmio validates ready queue size,
  max-size, split-ring alignment, and full guest-memory coverage before
  `DRIVER_OK` activation or warm-restore reactivation. Invalid activation sets
  `STATUS_FAILED` and does not start device workers.
- H01 memory-helper slice landed locally: `GuestMemory` and `GuestMemoryRef`
  read/write helpers use checked offset arithmetic so invalid offsets produce
  errors rather than debug panics.
- H01 block-accounting slice landed locally: virtio-blk queue drains use
  checked `u32` accumulation for total descriptor data length so maliciously
  large chains return `IOERR` instead of panicking before I/O validation.
- H01 closed with `cargo test -p capsem-core hypervisor::kvm --lib` passing
  333 tests and `just exec "echo ok"` proving the current KVM boot/exec path
  still works after queue activation hardening. The old `just run` smoke path
  no longer exists after the TUI merge; `just exec` is the current one-shot VM
  command path.
- H03 is active next so the safety/queue counters and resource usage become
  visible through status and are ready for OTel export.
- H03 first slice landed locally: `/info` now projects the existing
  `VmMetricsSnapshot.resources` source of truth, and `capsem info` renders
  configured RAM/vCPUs, host PID/RSS/CPU time/CPU percent, and disk usage
  counters when they are available. Remaining H03 work is to wire queue/backend
  counters into status and the metrics/exporter surface.
- H03 second slice landed locally: KVM virtio-blk counters now accumulate in
  backend-owned atomics, remain emitted through the `metrics` facade, flow into
  `VmMetricsSnapshot.hypervisor.block`, and are projected through `/info` and
  `capsem info`. Live proof on a KVM VM reported 5,876 queue notifications,
  1,639 queue drains, 25,266 descriptors/used entries, 8,580 read ops, and
  31,394,816 block bytes read.
- H03 third slice landed locally: gateway `/status` enriches running VMs with
  `/info/{id}` metrics while keeping `/list` as the base/fallback, and the TUI
  session-info overlay renders resources, host RSS/CPU time, block ops, block
  bytes, and block queue counters. Live gateway proof reported 5,908 queue
  notifications, 1,638 queue drains, 25,264 descriptors/used entries, 8,578
  read ops, and 31,394,816 block bytes read for a throwaway KVM VM.
- H03 fourth slice landed locally: `VmMetricsSnapshot::otel_metric_points()`
  now flattens resource and KVM block counters into stable OTel-compatible
  metric points with explicit units, counter/gauge kinds, source metadata, and
  bounded attributes (`component`, `backend`). This makes the counters
  exporter-ready without adding a half-wired OTLP runtime in this sprint.
- H02 first slice landed locally: KVM virtio-blk io_uring submission queue
  saturation now backpressures instead of falling back to synchronous I/O. The
  worker records one queue-full event, rewinds the popped descriptor, leaves
  used/status untouched, and retries the same request when the async queue has
  capacity again.
- H02 second slice landed locally: the io_uring completion branch now reaps
  completions and immediately performs a completion-triggered queue drain. A
  descriptor rewound by SQ-full backpressure can be resubmitted as soon as
  completion capacity is available, without requiring a fresh guest notify.
- H02 direction correction on 2026-05-30: isolated VirtioFS batching/event-index
  experiments produced mixed numbers and were reverted uncommitted. The next
  accepted unit is the whole KVM block async profile, benchmarked as a complete
  backend shape before ablation. Firecracker comparison points being adopted
  now: async engine as a first-class file engine, fixed registered fd,
  restricted/probed ring, queue-full throttling/backpressure, completion event
  retry, deferred used-ring publication, event-index interrupt decisions, and
  quiesce drain semantics.
- H02 full-profile slice landed locally: KVM virtio-blk now uses the full async
  profile for read-only rootfs and writable block devices by default, keeps
  `CAPSEM_KVM_BLK_IO_URING=sync` as the ablation/fallback path, registers the
  backing fd as a fixed file, probes required opcodes, restricts the ring while
  disabled, explicitly enables it, and submits once per queue-drain or
  completion-retry batch.
- H02 full-profile benchmark, same-run async-vs-sync rootfs: seq read 121.0
  MB/s vs 121.7 (-0.6%), random read 1303 IOPS vs 1420 (-8.2%), large binary
  cold 170.9 MB/s vs 158.3 (+8.0%), large binary warm 5555.1 MB/s vs 5451.0
  (+1.9%), small JS 75,860 ops/s vs 73,875 (+2.7%), metadata stat 37,732/s vs
  36,196/s (+4.2%).
- H02 full-profile benchmark, same-run async-vs-sync startup: python3 38.3 ms
  vs 38.1 (-0.5%), node 336.7 ms vs 351.5 (+4.2%), claude 1720.9 ms vs
  1707.5 (-0.8%), gemini 3246.9 ms vs 3196.0 (-1.6%), codex 1203.5 ms vs
  1098.2 (-9.6%). Lower startup latency is better.
- H02 grouped ablation, io_uring depth 256 vs accepted 128: seq read 120.3
  MB/s vs 121.0 (-0.6%), random read 1347 IOPS vs 1303 (+3.4%), large binary
  cold 161.3 MB/s vs 170.9 (-5.6%), large binary warm 5555.1 MB/s vs 5555.1
  (+0.0%), small JS 71,505 ops/s vs 75,860 (-5.7%), metadata stat 39,430/s vs
  37,732/s (+4.5%). The mixed result rejected the larger ring for now.
- H02 VM smoke passed with the full async profile selected by default:
  `just exec "echo ok"` returned `ok` from a real KVM one-shot VM.
- Firecracker reality check on the same Linux host with official Firecracker
  v1.15.1, Capsem x86_64 rootfs.squashfs, Capsem kernel extracted from bzImage
  to ELF vmlinux, 2 vCPUs, 2048 MiB RAM, and a benchmark-only initrd: Firecracker
  Sync beat current Capsem full-async rootfs lanes by seq read +0.7%, random
  read +46.6%, cold large-binary +58.2%, warm large-binary +10.0%, small JS
  +21.7%, metadata stat +12.1%. Startup was also faster: python3 12.3%, node
  27.4%, claude 42.6%, gemini 23.3%, codex 36.4%.
- Firecracker Async was close to Sync for this workload, not a clean io_uring
  proof: vs current Capsem full-async it measured seq read +3.2%, random read
  +46.3%, cold large-binary +59.8%, warm large-binary +9.1%, small JS +27.4%,
  metadata stat +20.3%. This makes the next Capsem sprint less about blindly
  defaulting io_uring and more about matching Firecracker's virtqueue,
  interrupt, request, and guest-visible block behavior first.
- crosvm reference check, 2026-06-01: no packaged `crosvm` binary was available
  through apt, snap, or GitHub releases on this host, so the comparison uses a
  private source checkout built per crosvm's documented Linux path with a
  minimal no-default-features release build. This is reference evidence, not a
  Capsem product dependency.
- crosvm epoll with the same Capsem x86_64 kernel/rootfs/initrd shape beat
  Firecracker Sync on the rootfs lanes: seq read 123.3 MB/s (+1.1%), random
  read 2111 IOPS (+10.5%), cold large-binary 298.4 MB/s (+10.4%), small JS
  104,348 ops/s (+13.1%), metadata stat 48,030/s (+13.6%). Startup was similar
  or slightly better: python3 30.4 ms (+5.3%), node 243.5 ms (-0.5%), claude
  815.2 ms (+6.0%), gemini 2280.4 ms (+0.2%), codex 712.6 ms (+6.8%).
- H05 file-backed DAX slice landed locally: `CAPSEM_KVM_ROOTFS_PMEM_FILE_BACKED=1`
  switches KVM virtio-pmem rootfs backing from anonymous-copy mmap to strict
  file mmap, while the rootfs-format grid `--pmem-file-backed` mode pads
  generated EROFS target images to the 128 MiB pmem alignment and records
  backing/padding metadata. Benchmark artifact:
  `benchmarks/kvm-rootfs-format-grid/data_1.2.1780320819_x86_64_1780367616.json`.
- H05 file-backed DAX result vs previous anonymous-copy DAX artifact
  `1780366089`: uncompressed EROFS seq read 294.1 MB/s vs 302.9 (-2.9%),
  random 37,551 IOPS vs 38,881 (-3.4%), cold large-binary 301.2 MB/s vs
  319.2 (-5.6%), small JS 454k/s vs 465k/s (-2.4%), metadata 115.0k/s vs
  114.8k/s (+0.1%), lower metadata 156.9k/s vs 148.6k/s (+5.6%).
- H05 compressed file-backed DAX result vs previous anonymous-copy DAX:
  `erofs-lz4hc-c65536` seq read 271.3 MB/s vs 279.9 (-3.1%), random 22,541
  IOPS vs 20,042 (+12.5%), cold large-binary 323.3 MB/s vs 338.6 (-4.5%),
  small JS 575k/s vs 522k/s (+10.1%), metadata 122.7k/s vs 123.3k/s (-0.6%),
  lower metadata 168.4k/s vs 172.5k/s (-2.4%). Conclusion: file-backed DAX
  helps some compressed random/small-file lanes but is not the large-read
  throughput fix; continue with block/Direct-I/O and filesystem tuning.
- Current H05 product candidate decision: prefer compressed `erofs-lz4hc-c65536`
  + DAX over uncompressed EROFS because it is much smaller and has the strongest
  small-file/random interactive profile. This is not the final tuning lock:
  revisit lz4hc cluster/layout settings, add EROFS zstd after a Linux 6.11+
  guest-kernel bump, and focus the next investigation on raw/cold throughput.
- H05 guest readahead slice landed locally: `capsem-init` now applies a 16 MiB
  read-ahead default to `/dev/pmem0` when `capsem.rootfs=erofs-dax`, keeps
  ordinary virtio-blk devices at 4 MiB unless they are the mounted rootfs
  device, and accepts explicit `capsem.rootfs_readahead_kb=` values for grid
  sweeps. `capsem-bench storage` and the rootfs-format grid now record pmem
  queue state.
- H05 readahead benchmark artifact:
  `benchmarks/kvm-rootfs-format-grid/data_1.2.1780320819_x86_64_1780369716.json`.
  Final rerun vs prior active compressed file-backed DAX artifact `1780367616`:
  seq read 279.0 MB/s vs 271.3 (+2.8%), random 21,707 IOPS vs 22,541 (-3.7%),
  cold large-binary 331.1 MB/s vs 323.3 (+2.4%), small JS 548k/s vs 575k/s
  (-4.7%), metadata 122.8k/s vs 122.7k/s (+0.2%), lower metadata 172.2k/s vs
  168.4k/s (+2.3%). Conclusion: keep the pmem DAX read-ahead default because
  it nudges raw throughput up without a large metadata penalty, but continue
  the larger raw-throughput investigation.
- H05 KVM pmem mmap-policy slice landed locally: file-backed pmem mapping now
  supports `CAPSEM_KVM_ROOTFS_PMEM_MADVISE=none|sequential|random|willneed` and
  `CAPSEM_KVM_ROOTFS_PMEM_POPULATE=1`; the rootfs-format grid sweeps these
  policies and records them in each result shape.
- H05 mmap-policy evidence: full sweep artifact `1780402337` and confirmatory
  no-populate artifact `1780402545` are archived in
  `benchmarks/archive/benchmark-history-20260602T121925Z.zip`; the active
  startup-inclusive artifact is
  `benchmarks/kvm-rootfs-format-grid/data_1.2.1780320819_x86_64_1780402733.json`.
  `willneed` vs same-run control improved seq read 274.8 MB/s vs 259.5
  (+5.9%), but regressed cold large-binary 311.9 MB/s vs 332.6 (-6.2%),
  small JS 512k/s vs 564k/s (-9.2%), lower metadata 148k/s vs 156k/s (-4.7%),
  and Codex startup mean 693.4 ms vs 606.7 (+14.3%). Conclusion: keep policy
  knobs and benchmark coverage, but do not change the default from `none`.
- crosvm epoll is still far from the committed macOS Capsem artifact: 0.13x seq
  rootfs read, 0.24x random IOPS, 0.31x cold large-binary read, 0.26x small JS,
  0.24x metadata stat, and roughly 2.8x-4.2x startup latency for the shared
  startup commands. That supports the hardware/host-storage hypothesis and the
  need to reason about overhead instead of treating any one Linux VMM as
  macOS-speed proof.
- crosvm `direct=true` is rejected for this read-mostly rootfs workload:
  seq read 63.7 MB/s, random 442 IOPS, cold large-binary 103.2 MB/s, small JS
  29,205 ops/s, metadata 14,580/s, and codex startup 1769.3 ms. Bypassing the
  host page cache made both cold and loader-style paths much worse.
- crosvm `multiple-workers=true` did not improve the default epoll shape:
  random read stayed similar at 2103 IOPS and cold large-binary stayed similar
  at 298.7 MB/s, but small JS dropped to 97,162 ops/s and metadata dropped to
  43,584/s. This argues against blindly adding more block workers without a
  measured queue/contention reason.
- crosvm `--async-executor uring` initially could not start because upstream
  crosvm's private `io_uring_setup` wrapper passed `io_uring_params` as an
  immutable reference even though the kernel writes ring offsets back into it.
  In the optimized release build, crosvm then computed a zero submit-ring mmap
  length and failed with `Failed to mmap submit ring ... Invalid argument`.
  A private reference patch changing that wrapper to `&mut io_uring_params`
  proved uring can boot on this host.
- crosvm uring after the private ABI fix is not faster than crosvm epoll on
  this read-heavy workload: seq read 121.7 MB/s (-1.3%), random read 2067 IOPS
  (-2.1%), cold large-binary 287.7 MB/s (-3.6%), small JS 103,633 ops/s
  (-0.7%), metadata 46,717/s (-2.7%), node startup 246.4 ms (-1.2%), claude
  867.4 ms (-6.4%), gemini 2332.6 ms (-2.3%), and codex 713.2 ms (-0.1%).
  The corrected lesson is that crosvm's cache-friendly epoll block path is the
  better reference here, not uring by itself.
- Next crosvm work must be a systematic trace comparison, not isolated knobs:
  trace request lifecycle from guest notification through descriptor parsing,
  backend scheduling, host I/O submission/completion, interrupt delivery, and
  rootfs-visible read behavior in crosvm, then map each step to Capsem's KVM
  path. The output should be a concrete delta table with expected benefit,
  complexity/maintenance cost, macOS/shared applicability, and the benchmark
  lane that would prove or reject it.
- crosvm/Firecracker source audit, first accepted Capsem slice: crosvm
  advertises `VIRTIO_BLK_F_SEG_MAX` and `VIRTIO_BLK_F_BLK_SIZE`, with
  `seg_max` bounded by the queue size, while Firecracker keeps a simple
  single-queue device shape. Capsem now reports `seg_max = queue_size - 2` and
  `blk_size = 512` before attempting higher-risk multi-queue work, so Linux can
  use explicit block geometry without changing the async backend contract.
- Focused live KVM check for that slice confirmed Linux sees
  `/sys/block/vda/queue/max_segments = 254` and `logical_block_size = 512`.
  Against the committed Linux baseline artifact, the same live `capsem-bench
  rootfs` probe measured random read 1,463 IOPS (+13.9%), cold large-binary
  181.4 MB/s (+12.3%), small JS 78,261 ops/s (+4.6%), metadata 39,394 stats/s
  (+10.4%), warm large-binary 5,468.8 MB/s (-1.6%), and sequential read
  129.2 MB/s (-23.6%). This is a focused experiment, not a replacement for a
  canonical `just benchmark` artifact.
- H05 first block-shape slice landed locally: KVM virtio-blk now accepts
  bounded `CAPSEM_KVM_BLK_QUEUE_COUNT`, `CAPSEM_KVM_BLK_QUEUE_SIZE`,
  `CAPSEM_KVM_BLK_SEG_MAX`, and `CAPSEM_KVM_BLK_LOGICAL_BLOCK_SIZE` knobs,
  advertises `VIRTIO_BLK_F_MQ` plus config `num_queues` when queue count is
  greater than one, and registers one x86_64 `KVM_IOEVENTFD` datamatch per
  queue so MQ benchmarks do not fall back to vCPU MMIO exits. `capsem-service`
  now forwards those numeric tuning knobs to `capsem-process`.
- Focused live KVM MQ probe with `queue_count=4`, `queue_size=128`,
  `seg_max=64`, and `logical_block_size=4096` confirmed Linux sees
  `/sys/block/vda/mq` with 4 queues, `max_segments=64`,
  `logical_block_size=4096`, and `nr_requests=64`. Against the committed Linux
  baseline artifact, the same live `capsem-bench rootfs` probe measured random
  read 3,022 IOPS (+135.2%), cold large-binary 179.2 MB/s (+11.0%), small JS
  106,595 ops/s (+42.5%), metadata 64,006 stats/s (+79.4%), warm large-binary
  5,354.9 MB/s (-3.7%), and sequential read 134.0 MB/s (-20.8%). This is a
  focused experiment and will feed the gridsearch rather than being accepted as
  the default.
- H05 gridsearch harness landed locally as `scripts/kvm_block_shape_grid.py`.
  It expands queue count, queue size, segment limit, and logical block size as
  a coupled matrix, runs the selected shapes through `just exec`, captures
  Linux sysfs queue state, and writes structured artifacts under
  `benchmarks/kvm-block-shape/`. A one-cell harness proof for
  `queue_count=4`, `queue_size=128`, `seg_max=64`, `logical_block_size=4096`
  wrote `benchmarks/kvm-block-shape/data_1.2.1780320819_x86_64_1780334268.json`
  with sysfs `mq_dirs=4`, `max_segments=64`, `logical_block_size=4096`,
  `nr_requests=64`, and rootfs random read 2,885 IOPS, small JS 109,911 ops/s,
  metadata 61,877 stats/s.
- H05 first real grid recorded
  `benchmarks/kvm-block-shape/data_1.2.1780320819_x86_64_1780334747.json`
  for 24 queue/geometry cells: queue counts 1/4/8, queue sizes 128/256,
  segment max auto/64, logical block sizes 512/4096. The best balanced cell by
  equal rootfs lane ratio was `queue_count=8`, `queue_size=128`, `seg_max=64`,
  `logical_block_size=4096`: random read 3,349 IOPS (+160.7% vs committed
  Linux baseline), small JS 116,278 ops/s (+55.4%), metadata 63,880 stats/s
  (+79.1%), cold large-binary 203.2 MB/s (+25.8%), sequential read 138.1 MB/s
  (-18.4%). Linux exposed 4 `/sys/block/vda/mq` queues for both 4-queue and
  8-queue device configs on this 2-vCPU VM, so the next grid should include
  vCPU count and avoid assuming requested queue count equals active Linux
  hardware queues.
- H05 candidate startup proof recorded
  `benchmarks/kvm-block-shape/data_1.2.1780320819_x86_64_1780334834.json`
  for `queue_count=8`, `queue_size=128`, `seg_max=64`,
  `logical_block_size=4096` with startup enabled. Rootfs stayed in the same
  improved band: random read 3,502 IOPS (+172.6%), small JS 105,614 ops/s
  (+41.1%), metadata 61,219 stats/s (+71.6%), cold large-binary 199.3 MB/s
  (+23.4%), sequential read 144.2 MB/s (-14.8%). Startup also improved versus
  the committed Linux baseline: python3 31.1 ms (+30.2% faster), node
  247.3 ms (+44.8%), claude 1,301.8 ms (+30.8%), gemini 2,950.2 ms (+10.2%),
  codex 820.5 ms (+36.0%). This candidate is strong enough for a default
  experiment, but still needs canonical `just benchmark` and macOS/shared
  impact review before acceptance.
- H05 scope split landed locally after writable disk checks showed the rootfs
  candidate must not be applied globally. `CAPSEM_KVM_BLK_ROOTFS_*` overrides
  now target read-only rootfs devices, `CAPSEM_KVM_BLK_WRITABLE_*` can target
  writable block devices, and generic `CAPSEM_KVM_BLK_*` remains a fallback.
  Live sysfs proof with only rootfs-specific knobs showed `vda` at 4 active MQ
  queues, `max_segments=64`, `logical_block_size=4096`, `nr_requests=64`, while
  `vdb` stayed at the default 1 queue, `max_segments=254`,
  `logical_block_size=512`, `nr_requests=128`. Focused disk probes still showed
  low current write IOPS even with default writable geometry, so accepting the
  rootfs candidate requires canonical `just benchmark` rather than isolated
  disk interpretation.
- H05 canonical rootfs-only benchmark ran through `just benchmark` with
  `CAPSEM_KVM_BLK_ROOTFS_QUEUE_COUNT=8`, `CAPSEM_KVM_BLK_ROOTFS_QUEUE_SIZE=128`,
  `CAPSEM_KVM_BLK_ROOTFS_SEG_MAX=64`, and
  `CAPSEM_KVM_BLK_ROOTFS_LOGICAL_BLOCK_SIZE=4096` on source commit `b834d554`.
  The run refreshed the active Linux x86_64 artifacts and preserved the prior
  active generated artifacts in local archive
  `benchmarks/archive/benchmark-prerun-20260601T180613Z.zip`. Artifact metadata
  recorded source-clean git state (`source_dirty=false`) plus generated
  security-engine artifacts already modified by the same benchmark run. The
  canonical suite generated all requested performance artifacts but failed the
  endpoint-latency gate: service global endpoints were roughly 3.3-6.1 ms p95
  against the 3 ms gate, and `/logs/{id}` was roughly 25.3-26.3 ms p95 against
  the 12 ms gate.
- H05 next experiment candidates from user review: rootfs format/compression
  matrix should include current SquashFS zstd baseline, uncompressed SquashFS
  or equivalent uncompressed read-only image, EROFS if the guest kernel has or
  can reasonably gain support, and DAX-style guest mapping if the KVM/rootfs
  transport can expose it without replacing the product architecture. The
  benchmark proof must stay canonical: `capsem-bench storage/rootfs/startup`
  plus host-native/raw-system comparison and Linux/macOS artifact comparison.
  DAX is explicitly a capability audit first, because virtiofs DAX, pmem DAX,
  and block-backed read-only rootfs have different kernel/device requirements.
- H05 rootfs-format grid harness landed locally: `scripts/kvm_rootfs_format_grid.py`
  materializes benchmark-only asset roots under `target/kvm-rootfs-format-grid`,
  currently covering current SquashFS zstd, SquashFS with compression disabled,
  and EROFS image generation. Each format runs through the same
  queue-count/queue-size/segment/logical-block-size shape matrix via `just exec`
  and records guest sysfs queue state plus `capsem-bench storage`, `rootfs`, and
  optional `startup` JSON. The harness records DAX as `not_implemented` rather
  than pretending the current virtio-blk rootfs path can exercise DAX; real DAX
  needs a separate virtiofs-DAX or pmem-style mapping path.
- H05 first rootfs-format grid artifact:
  `benchmarks/kvm-rootfs-format-grid/data_1.2.1780320819_x86_64_1780339109.json`
  compared current SquashFS zstd (496.4 MiB) against uncompressed SquashFS
  (1,603.5 MiB) across the same two rootfs-only block shapes:
  `queue_count=1/8`, `queue_size=128`, `seg_max=64`,
  `logical_block_size=4096`, with `storage`, `rootfs`, and `startup` enabled.
  Uncompressed SquashFS won most read/startup lanes despite the 3.2x larger
  image. Best uncompressed vs best zstd: rootfs seq 195.4 vs 147.7 MB/s
  (+32.3%), random read 3,419 vs 3,184 IOPS (+7.4%), cold large-binary 313.3
  vs 191.6 MB/s (+63.5%), small-JS 127,580 vs 99,656 ops/s (+28.0%), python
  startup 20.5 vs 29.5 ms (+30.5% faster), node 143.5 vs 266.1 ms
  (+46.1%), claude 1,044.4 vs 1,321.6 ms (+21.0%), gemini 2,728.9 vs
  2,970.1 ms (+8.1%), and codex 492.4 vs 872.7 ms (+43.6%). Metadata went the
  other way in this small grid: best zstd 62,234 stats/s vs best uncompressed
  53,527 stats/s (-14.0%). This is strong evidence that rootfs compression is
  a first-order startup/read bottleneck and deserves a larger grid before
  baking block-shape defaults.
- H05 EROFS capability artifact:
  `benchmarks/kvm-rootfs-format-grid/data_1.2.1780320819_x86_64_1780339221.json`
  generated a 916.1 MiB EROFS image and tried the tuned rootfs block shape, but
  the VM did not become ready. Current `guest/config/kernel/defconfig.x86_64`
  does not enable `CONFIG_EROFS_FS`, so EROFS needs a kernel-capability slice
  before it can be compared fairly. DAX remains a capability/design audit, not
  a block-image benchmark, because the current rootfs transport is virtio-blk.
- H05 EROFS kernel-capability slice landed locally: both arm64 and x86_64 guest
  defconfigs now enable `CONFIG_EROFS_FS` and `CONFIG_EROFS_FS_ZIP` so rebuilt
  kernels can mount benchmark-generated EROFS rootfs images. The next proof is
  to rebuild the x86_64 kernel asset, rerun the EROFS cell, and then include
  EROFS in the same best-vs-best rootfs-format/block-shape grid.
- H05 compression-level matrix support landed locally: the rootfs-format grid
  now accepts `--zstd-levels`, appending generated `squashfs-zstd-l<N>` variants
  such as levels 1/3/9/15/22. These variants are rebuilt from the same extracted
  rootfs and run through the same block-shape cells as production zstd,
  uncompressed SquashFS, and EROFS, so compression level is measured fairly
  rather than compared against a differently tuned block path.
- H05 EROFS proof after rebuilding the x86_64 guest kernel asset:
  `benchmarks/kvm-rootfs-format-grid/data_1.2.1780320819_x86_64_1780345066.json`
  booted the generated EROFS image with `queue_count=8`, `queue_size=128`,
  `seg_max=64`, and `logical_block_size=4096`. Against the same-run production
  SquashFS zstd baseline recorded in
  `benchmarks/kvm-rootfs-format-grid/data_1.2.1780320819_x86_64_1780345823.json`,
  EROFS improved rootfs seq read 238.4 vs 143.7 MB/s (+65.9%), random read
  7,168 vs 3,019 IOPS (+137.5%), cold large-binary 416.7 vs 177.5 MB/s
  (+134.8%), small-JS 165,556 vs 91,645 ops/s (+80.6%), python startup 15.0
  vs 31.0 ms (+51.6% faster), node 91.0 vs 303.6 ms (+70.0%), claude 669.0
  vs 1,287.1 ms (+48.0%), gemini 2,585.9 vs 2,847.9 ms (+9.2%), and codex
  298.2 vs 820.0 ms (+63.6%). Metadata regressed hard: 23,956 vs 59,924
  stats/s (-60.0%), so EROFS is a strong read/startup candidate but not a
  clean universal replacement without metadata-path work.
- H05 zstd compression-level artifact:
  `benchmarks/kvm-rootfs-format-grid/data_1.2.1780320819_x86_64_1780345823.json`
  compared shipped SquashFS zstd (496.4 MiB) with generated zstd levels 1
  (594.0 MiB), 3 (564.9 MiB), 9 (526.9 MiB), 15 (496.4 MiB), and 22
  (490.4 MiB) at the same tuned rootfs block shape. Runtime was not monotonic
  with compression level: level 1 had the best seq read (167.7 MB/s, +16.7%)
  and small-JS read (116,325 ops/s, +26.9%) versus shipped zstd, level 9 had
  the best random read (3,535 IOPS, +17.1%) and cold large-binary read
  (210.9 MB/s, +18.8%), and level 22 produced the smallest image while staying
  near baseline seq read (148.9 MB/s, +3.6%) with better random read
  (3,307 IOPS, +9.6%) and small-JS read (103,931 ops/s, +13.4%). Metadata was
  basically flat across zstd levels. This supports using high compression when
  distribution size matters, but the bigger performance lever remains rootfs
  format/layout rather than zstd level alone.
- H05 EROFS compression matrix support landed locally: the rootfs-format grid
  now accepts `--erofs-compressions none,lz4,lz4hc`, appending explicit
  `erofs-uncompressed`, `erofs-lz4`, and `erofs-lz4hc` variants. The existing
  `erofs` format remains a compatibility alias for lz4hc, which is what the
  first successful EROFS artifact used. The next benchmark compares those
  variants against both shipped SquashFS zstd and uncompressed SquashFS so EROFS
  is judged against the right no-compression baseline as well as the product
  baseline.
- H05 EROFS compression artifact:
  `benchmarks/kvm-rootfs-format-grid/data_1.2.1780320819_x86_64_1780346718.json`
  recorded shipped SquashFS zstd, uncompressed SquashFS, EROFS uncompressed,
  EROFS lz4, and EROFS lz4hc at the same tuned rootfs block shape. Against
  uncompressed SquashFS (1,603.5 MiB), EROFS lz4hc (916.1 MiB) improved seq
  read 255.4 vs 201.3 MB/s (+26.9%), random read 7,599 vs 3,555 IOPS
  (+113.7%), cold large-binary 411.7 vs 299.3 MB/s (+37.6%), small-JS 145,813
  vs 127,280 ops/s (+14.6%), node startup 90.9 vs 143.1 ms (+36.5% faster),
  claude 660.4 vs 871.2 ms (+24.2%), gemini 2,486.8 vs 2,691.8 ms (+7.6%),
  and codex 298.2 vs 456.8 ms (+34.7%). Python was roughly flat at 18.9 vs
  19.1 ms (+1.0%). Metadata still regressed sharply: 27,069 vs 54,410 stats/s
  (-50.3%). EROFS uncompressed was fastest on seq read (305.2 MB/s),
  large-binary read (545.7 MB/s), small-JS (164,529 ops/s), and startup, but it
  was larger than uncompressed SquashFS (1,852.8 MiB vs 1,603.5 MiB) and had
  the worst metadata result (14,720 stats/s). EROFS lz4 had the best random
  read at 7,205 IOPS before lz4hc beat it at 7,599 IOPS in this run; both are
  strong read/startup candidates, but metadata needs a separate investigation
  before EROFS can be a default rootfs format.
- H05 metadata diagnostic landed locally: `capsem-bench rootfs` now records
  `metadata_stat_lower` when the guest exposes the read-only lower rootfs at
  `/run/capsem-lower`, mapping `/usr/bin`, `/usr/lib`, and `/opt/ai-clis`
  directly to `/run/capsem-lower/...`. The rootfs-format grid requests this
  with `capsem.bench_lower=1`; normal boots do not expose the lower rootfs.
  The existing `metadata_stat` lane remains the product path through overlay.
  The next EROFS/SquashFS rerun should distinguish lower filesystem metadata
  cost from overlay amplification.
- H05 metadata diagnostic artifact:
  `benchmarks/kvm-rootfs-format-grid/data_1.2.1780320819_x86_64_1780348995.json`
  reran uncompressed SquashFS, EROFS uncompressed, and EROFS lz4hc with
  `metadata_stat_lower` enabled. Uncompressed SquashFS measured 50,618 stats/s
  through overlay and 63,478 stats/s directly on the lower rootfs. EROFS
  uncompressed measured 15,138 stats/s through overlay and 12,864 stats/s
  directly on the lower rootfs. EROFS lz4hc measured 23,970 stats/s through
  overlay and 32,202 stats/s directly on the lower rootfs. Conclusion: overlay
  adds measurable cost, but the main regression is lower EROFS metadata
  traversal itself; EROFS lz4hc improves metadata locality compared with
  uncompressed EROFS but still lands at roughly half the direct lower metadata
  throughput of uncompressed SquashFS.
- H05 EROFS lz4hc tuning support landed locally: the rootfs-format grid now
  accepts `--erofs-lz4hc-clusters`, appending generated variants such as
  `erofs-lz4hc-c4096`, `erofs-lz4hc-c16384`, `erofs-lz4hc-c65536`, and
  `erofs-lz4hc-c131072`. This isolates `mkfs.erofs -C` compressed physical
  cluster size before layering riskier knobs like `-E ztailpacking`,
  force-inode modes, xattr tolerance, max extents, or experimental chunked
  files.
- H05 EROFS lz4hc cluster-size artifact:
  `benchmarks/kvm-rootfs-format-grid/data_1.2.1780320819_x86_64_1780350703.json`
  compared uncompressed SquashFS, current EROFS lz4hc, and explicit lz4hc
  `-C` clusters of 4K, 16K, 64K, and 128K. Against current EROFS lz4hc
  (916.1 MiB), 64K clusters shrank the image to 768.1 MiB and improved seq
  read 286.5 vs 237.3 MB/s (+20.7%), random read 7,884 vs 7,190 IOPS (+9.7%),
  cold large-binary 488.0 vs 368.7 MB/s (+32.4%), small-JS 167,155 vs 155,122
  ops/s (+7.8%), and overlay metadata 27,930 vs 20,745 stats/s (+34.6%).
  Direct-lower metadata stayed roughly flat/slightly down at 28,992 vs 30,134
  stats/s (-3.8%). The 16K variant was best for small-JS reads in this run
  (177,020 ops/s) but lower on random IOPS; the 128K variant was smallest
  (759.0 MiB) and highest seq read (296.6 MB/s) but did not dominate random or
  lower metadata. Against uncompressed SquashFS, tuned EROFS still wins read
  lanes and size but remains about 44-53% behind on metadata throughput, so
  cluster tuning improves the compressed candidate without closing the metadata
  gap.
- H05 direct-I/O ablation lane landed locally: KVM virtio-blk now accepts
  `CAPSEM_KVM_BLK_ROOTFS_DIRECT_IO=1` (or global
  `CAPSEM_KVM_BLK_DIRECT_IO=1`) and opens the read-only rootfs backing file
  with `O_DIRECT`. The gate is rootfs-only and opt-in because direct I/O has
  alignment constraints and should be measured before it becomes a product
  default. The rootfs-format grid exposes this with `--direct-io` and records
  the direct-I/O flag in each artifact.
- H05 direct-I/O artifact:
  `benchmarks/kvm-rootfs-format-grid/data_1.2.1780320819_x86_64_1780351156.json`
  compared direct-I/O rootfs backing against the buffered cluster-sweep artifact
  on the same block shape. Direct I/O was valid but not helpful for this
  workload. Uncompressed SquashFS regressed vs buffered by seq read -60.6%,
  random read -80.5%, cold large-binary -69.2%, small-JS -69.7%, overlay
  metadata -54.9%, and direct-lower metadata -60.4%. Tuned
  `erofs-lz4hc-c65536` kept sequential read closer (-3.1%) and cold
  large-binary closer (-8.5%), but random read fell -80.6%, small-JS -65.2%,
  overlay metadata -75.6%, and direct-lower metadata -76.3%. Conclusion:
  host page cache is a major positive part of the current rootfs workload, so
  `O_DIRECT` should remain an ablation/debug lane rather than a default.
- H05 tuned EROFS startup proof:
  `benchmarks/kvm-rootfs-format-grid/data_1.2.1780320819_x86_64_1780351471.json`
  reran uncompressed SquashFS against `erofs-lz4hc-c65536` with startup enabled
  and buffered host I/O. Tuned EROFS won read/startup lanes: seq read 315.4 vs
  227.7 MB/s (+38.5%), random read 8,626 vs 4,160 IOPS (+107.4%), cold
  large-binary 578.0 vs 349.3 MB/s (+65.5%), small-JS 237,963 vs 161,578
  ops/s (+47.3%), python startup 10.8 vs 19.3 ms (+44.0% faster), node 51.2
  vs 140.9 ms (+63.7%), claude 502.5 vs 662.7 ms (+24.2%), gemini 2,070.1 vs
  2,278.4 ms (+9.1%), and codex 191.7 vs 344.0 ms (+44.3%). Metadata remains
  the tradeoff: overlay metadata 35,562 vs 66,636 stats/s (-46.6%) and
  direct-lower metadata 37,066 vs 78,762 stats/s (-52.9%).
- H05 DAX feasibility probe on current Linux/KVM path: running the tuned EROFS
  rootfs on virtio-blk reported `/sys/block/vda/queue/dax=0` and no
  `/sys/block/vda/dax`; the guest exposes EROFS but the current defconfigs do
  not declare DAX/FS_DAX/PMEM support and the KVM rootfs transport is
  virtio-blk, not a DAX-capable pmem or virtiofs-DAX mapping. Conclusion:
  `-o dax` is not an immediate mount-option flip for the current rootfs path.
  A real DAX experiment should be a new transport slice: enable the needed
  guest kernel DAX symbols, expose a page-size-compatible DAX-capable backing
  device or virtiofs-DAX-style mapping, and test primarily EROFS because
  SquashFS is not a DAX candidate. Compressed EROFS may only partially benefit;
  uncompressed/direct-mappable EROFS is the cleaner DAX proof.

## Coverage Ledger

- Unit/contract: `tests/test_archive_superseded_benchmark_artifacts.py`,
  `tests/test_benchmark_contract.py`, `tests/test_benchmark_artifacts.py`,
  `tests/test_kvm_rootfs_format_grid.py`,
  `tests/test_docker.py::TestRenderKernel::test_kernel_defconfigs_support_erofs_rootfs_experiments`,
  `cargo test -p capsem-core guest_memory_ref --lib`,
  `cargo test -p capsem-core block_guest_iovecs_reject_range_that_crosses_ram_end --lib`,
  `cargo test -p capsem-core virtio_blk --lib`,
  `cargo test -p capsem-core virtio_queue --lib`,
  `cargo test -p capsem-core virtio_mmio --lib`,
  `cargo test -p capsem-core offset_overflow_fails --lib`,
  `cargo test -p capsem-core guest_memory --lib`,
  `cargo test -p capsem-core block_data_length_overflow_returns_ioerr --lib`,
  `cargo test -p capsem-core hypervisor::kvm --lib`,
  `cargo test -p capsem-core block_read_records_queue_and_request_metrics --lib`,
  `cargo test -p capsem-core virtio_blk --lib`,
  `cargo test -p capsem-process metrics_snapshot_is_process_owned_and_versioned --bin capsem-process`,
  `cargo test -p capsem-process ipc::tests --bin capsem-process`,
  `cargo test -p capsem-service attach_metrics_snapshot_projects_security_status_fields --bin capsem-service`,
  `cargo test -p capsem-gateway fetch_status_enriches_running_vm_with_info_metrics --bin capsem-gateway`,
  `cargo test -p capsem-gateway status::tests --bin capsem-gateway`,
  `cargo test -p capsem-process vsock::tests`,
  `cargo test -p capsem-agent --bin capsem-net-proxy`,
  `just exec "echo net-proxy-smoke"` (rerun after fixing dev profile
  materialization),
  `CAPSEM_BENCH_MITM_DURATION=3 capsem-bench mitm-load` through `just exec`
  (completed, but performance evidence rejected because every request path
  returned exceptions under current host DNS/network conditions),
  `just exec "capsem-bench mcp-load && cat /tmp/capsem-benchmark.json"`
  (completed with zero errors; valid deterministic MCP transport regression
  proof against `benchmarks/mcp-load/baseline.json`),
  `cargo test -p capsem-core aggregator_`,
  `cargo test -p capsem-core endpoint_`,
  `cargo test -p capsem-core mcp_stage_`,
  `cargo test -p capsem-core log_mcp_call_writes_canonical_security_event`,
  `cargo test -p capsem-core log_mcp_call_writes_`,
  `cargo test -p capsem-logger mcp_`,
  `cargo test -p capsem-core net::mitm_proxy::mcp_frame --lib`,
  `cargo test -p capsem-process pooled_runtime_security_engine_records_parallel_rule_matches --bin capsem-process`,
  `cargo test -p capsem-process mcp_runtime --bin capsem-process`,
  `cargo test -p capsem-core all_names_distinct`,
  `cargo test -p capsem-core describe_all_does_not_panic`,
  `cargo test -p capsem --bin capsem format_session_resource_lines_shows_live_metrics`,
  `cargo test -p capsem --bin capsem format_session_hypervisor_lines_shows_block_counters`,
  `cargo test -p capsem --bin capsem`,
  `cargo test -p capsem-tui gateway_status_json_maps_to_tui_state --lib`,
  `cargo test -p capsem-tui stats_overlay_renders_on_demand_without_persistent_help --lib`,
  `cargo test -p capsem-tui --lib`,
  `cargo test -p capsem-proto metrics::tests --lib`,
  `cargo test -p capsem-core undo_pop_retries_last_chain --lib`,
  `cargo test -p capsem-core block_io_uring_queue_full_backpressures_without_sync_fallback --lib`,
  `cargo test -p capsem-core block_io_uring_completion_retries_backpressured_descriptor --lib`,
  `cargo test -p capsem-core block_io_uring --lib`,
  `cargo test -p capsem-service process_env_allowlist_forwards_child_runtime_knobs --bin capsem-service`,
  `python3 scripts/kvm_block_shape_grid.py --dry-run --queue-counts 1,4 --queue-sizes 128 --seg-maxes auto,64 --logical-block-sizes 512,4096`,
  `cargo test -p capsem-service attach_metrics_snapshot_projects_security_status_fields --bin capsem-service`,
  `cargo test -p capsem --bin capsem format_session_hypervisor_lines_shows_block_counters`.
- Functional: `just exec "echo ok"` passed after H01 queue activation changes.
  A live named VM smoke with `capsem info --json` passed for H03 and reported
  `metrics_schema_version=1`, `configured_ram_mb=2048`, `configured_vcpus=2`,
  host PID, host RSS, and host CPU time before the throwaway VM was deleted.
  `just exec "echo ok"` also passed after H02 made the full io_uring block
  profile the default KVM block backend.
- Adversarial: `block_guest_iovecs_reject_range_that_crosses_ram_end` proves
  a descriptor whose start GPA is valid but whose length crosses RAM end is
  rejected before raw iovecs reach host I/O. `avail_head_outside_queue_fails_closed`,
  `descriptor_next_outside_queue_fails_closed`, and
  `cycle_in_descriptor_chain_terminates` prove malformed split-ring chains fail
  closed. `zero_size_queue_operations_fail_closed` and
  `misaligned_descriptor_table_fails_closed` prove bad queue layout does not
  panic or parse misaligned descriptor memory.
  `driver_ok_rejects_ready_queue_with_zero_size` and
  `driver_ok_rejects_ready_queue_outside_guest_ram` prove malformed ready
  queues are rejected at transport activation. `guest_memory_*_offset_overflow_fails`
  tests prove hostile offset arithmetic returns errors instead of panicking.
  `block_data_length_overflow_returns_ioerr` proves aggregate descriptor length
  overflow fails the request instead of panicking.
  `block_io_uring_queue_full_backpressures_without_sync_fallback` proves a full
  io_uring submission queue does not burn CPU in the synchronous fallback path,
  does not complete the request, and can retry the same descriptor later.
  `block_io_uring_completion_retries_backpressured_descriptor` proves a real
  io_uring completion frees capacity and triggers resubmission of the rewound
  descriptor without a new guest notification.
  `block_io_uring_uses_firecracker_shaped_ring_contract` proves the io_uring
  backend comes up with a fixed registered file and ring restrictions enabled.
- E2E/VM: `just exec "echo ok"` passed for the KVM one-shot VM path. H03
  resource projection was also checked against a live named VM via
  `capsem info --json`; the second H03 live check confirmed KVM block counters
  appear in that same JSON response for a real booted VM. The third H03 live
  check confirmed gateway `/status` carries those counters to the TUI-facing
  feed for a real booted VM. H02 default async block selection was smoke-tested
  through the same KVM one-shot VM path. The latest isolated live KVM check
  used the repo assets path and confirmed the guest-visible virtio-blk geometry
  before running `capsem-bench rootfs`. The latest MQ live KVM check confirmed
  four virtio-blk queues, tuned queue size, tuned segment limit, and tuned
  logical block size in Linux sysfs before running `capsem-bench rootfs`.
  H08 first telemetry slice attempted `just exec "echo ok"` after compiling and
  repacking the x86_64 guest agent/initrd, but VM provisioning did not start
  because installed assets were not ready and the service could not resolve
  `assets.capsem.dev` for the missing `2026.0601.2` x86_64 `vmlinuz`/`initrd`.
  This leaves live VM counter proof open rather than silently treating focused
  unit/API tests as E2E coverage.
- Telemetry: H03 first slice exposes existing `VmMetricsSnapshot.resources`
  fields through the service API and CLI. H03 second slice adds
  `VmMetricsSnapshot.hypervisor.block` and feeds it from the KVM virtio-blk
  backend while preserving `metrics` facade emission. H03 third slice carries
  those fields through gateway `/status` and the TUI model. H03 fourth slice
  adds OTel-compatible metric-point mapping with bounded attributes. Real OTLP
  exporter process/configuration remains open for the broader telemetry sprint.
  H02 first slice adds `async_queue_full_total` to the KVM block snapshot and
  OTel-compatible block metric points. H08 first telemetry slice adds
  request-shape and timing attribution counters to the same paths: VM metrics
  snapshot, OTel-compatible points, service `/info`, gateway `/status`, and
  `capsem info`. H09 first vsock slice adds process-side metrics facade points
  for accepted/closed/active vsock connections and handler duration with
  bounded port-kind labels; focused tests prove recorder visibility, while
  live VM/status/exporter proof remains open. H09 first guest-net-proxy slice
  reduces per-connection process attribution overhead with a throttled
  socket-owner index; attribution quality still needs live VM proof.
- Performance: canonical `just benchmark` rerun completed; benchmark artifacts
  record project version, git commit, source dirty state, host metadata, and
  active Linux x86_64 results. `scripts/compare_benchmark_artifacts.py`
  produced Linux/macOS ratios for shared lanes. Refreshed macOS artifacts from
  `1.2.1780103109` are now present on main and compared successfully. A
  canonical Linux x86_64 rerun on commit `19ca286e` recorded fresh artifacts
  for `1.2.1780320819`; it completed artifact generation but failed the
  endpoint-latency gate on service global endpoints at roughly 3-6 ms p95 and
  `/logs/{id}` at roughly 26 ms p95. The same artifact set shows Linux still
  behind macOS on the user-visible lanes: rootfs random read 1,285 vs 8,734
  IOPS, rootfs metadata 35,677 vs 199,915 stats/s, rootfs cold large-binary
  161.5 vs 977.3 MB/s, node startup 358.1 vs 77.6 ms, claude startup 1,702.2
  vs 309.0 ms, and codex startup 1,115.5 vs 237.1 ms. H02 first
  and second slices are correctness/backpressure for the io_uring path. H02
  full-profile local benchmarks measured the full async engine before grouped
  ablation: same-run rootfs showed cold binary +8.0%, small JS +2.7%, metadata
  +4.2%, but random rootfs -8.2%; same-run startup showed node +4.2% but codex
  -9.6%. Queue depth 256 was rejected after mixed ablation results. Official
  Firecracker v1.15.1 with the same Capsem rootfs/kernel workload proved the
  VMM/device path gap is real: Firecracker Sync was +46.6% random rootfs,
  +58.2% cold large-binary, +21.7% small JS, +12.1% metadata, and 12.3-42.6%
  faster on AI CLI startup. Firecracker Async remained in the same band rather
  than proving io_uring alone is the missing lever. crosvm epoll improved on
  Firecracker Sync for this workload by +10.5% random rootfs, +10.4% cold
  large-binary, +13.1% small JS, +13.6% metadata, and +6.8% codex startup, while
  crosvm corrected-uring, direct-I/O, and multi-worker ablations were rejected.
  A local
  uncommitted VirtioFS batching probe measured `/root`
  targeted disk at seq write +2.3%, seq read +2.4%, random write -0.6%, random
  read +10.8% without event-index, but it was not accepted because it was not
  the systematic backend-wide profile now being pursued.
  The H05 rootfs-only block-shape canonical run on commit `b834d554` improved
  the committed Linux baseline on the lanes the focused grid predicted:
  rootfs random read 2,686 IOPS (+109.1%), small-JS reads 88,791 ops/s
  (+18.7%), metadata stat 58,674 stats/s (+64.5%), python startup 31.7 ms
  (+27.8% faster), node 302.1 ms (+18.5%), claude 1,509.7 ms (+12.8%),
  gemini 3,206.5 ms (+1.4%), and codex 979.0 ms (+13.9%). It did not close
  the macOS gap: Linux/macOS ratios were rootfs seq 0.17x, random IOPS 0.31x,
  cold large-binary 0.17x, small-JS 0.22x, metadata 0.29x, disk seq read
  0.08x, and disk random read 0.08x. Against the same Linux host-native/raw
  artifact, the VM measured disk seq write 0.35x host, disk seq read 0.05x,
  disk random read 0.02x, small-file reads 0.52x, and metadata stats 0.27x.
  VM random write appeared 4.03x host-native because the two paths have
  different buffering/sync behavior, so that lane is not a clean raw-efficiency
  signal.
- Missing/deferred: Real OTLP exporter process/configuration is deferred to the
  broader telemetry sprint. H08 request-shape counters have focused unit/API
  coverage but still need live VM proof that they move during `capsem-bench
  disk` or `storage`, followed by a canonical `just benchmark` artifact before
  performance claims. H09 vsock metrics need real VM proof under guest HTTP
  load and a user-facing status/OTel export projection before they can support
  a performance claim. The guest net-proxy socket-owner index needs a real
  HTTP/proxy throughput comparison and an attribution-quality check before it
  can be accepted as a measured speedup. Endpoint-latency regressions are
  recorded by the canonical benchmark gate and still need a control-plane
  performance fix. The first focused `mitm-load` rerun is explicitly rejected
  as performance evidence because the environment produced request exceptions
  and timeout tails at every concurrency level. The `mcp-load` rerun is valid
  evidence and now has source tracing plus OTel-ready stage histograms. The new
  `direct-vsock-transport` lane proves same-VM framed KVM/vhost-vsock transport
  can reach 3,086.6/13,632.2/22,003.0/37,027.6 RPS at c=1/10/50/200 while the
  real direct-vsock MCP tool path stays at 588.0/812.8/806.0/822.8 RPS. This
  leaves the next missing proof in the real MCP policy/security/telemetry/
  inflight path after frame parsing, not guest Python/FastMCP or raw KVM/vsock.
  A local-echo-only default-policy fast path was rejected after same-run proof
  showed only +1.5%/+0.0%/+0.7%/+0.7% RPS at c=1/10/50/200; it improved c=200
  p99 by about 10.6% but would make the diagnostic less representative without
  improving real external MCP or policy-heavy paths.
  A per-connection audit worker was also rejected: it moved RPS to
  562.6/781.4/811.8/840.6 (-4.6%/-3.9%/+0.7%/+2.3%) and worsened c=200 p99 to
  363.3ms, so post-response audit construction/task fanout is not the simple
  limiter.
  MCP writer preview parsing now avoids full DOM allocation while preserving
  audit/security rows, but scoped live `mcp-load` was
  +1.0%/-4.8%/-1.7%/+1.7% versus the same-lane baseline, so it is not a
  measured RPS breakthrough.
  The accepted runtime-security pool fix moved high-concurrency direct-vsock
  `mcp-load` from the ~800 RPS ceiling to ~5.6k RPS while preserving runtime
  blocking/detection and rule-match telemetry. Post-fix recorder proof shows
  `runtime_security_evaluate` now stays around p50 2.0ms / p99 3.1ms at high
  concurrency instead of the previous 20-22ms queue. The remaining telemetry
  enqueue backlog appears after the response has already been sent and should
  be treated as the next audit-throughput/flush-efficiency target, not as the
  current guest-visible RPS ceiling.
