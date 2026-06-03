version: 1.2.1780446192
---
### Changed
- Changed the endpoint-latency benchmark gate to keep strict default p95
  budgets while giving `/settings`, `/profiles`, and full `/logs/{id}` audit
  envelopes explicit endpoint budgets. The artifact now writes the real Capsem
  package version at top level in addition to canonical `project_version`,
  git-commit, dirty-state, and host CPU/RAM metadata.
- Changed runtime security-engine routing to advertise event-family capability
  and skip non-applicable runtime CEL evaluation for MCP, HTTP, DNS, and
  process exec paths. Structured effective HTTP/DNS rules no longer make MCP
  requests pay runtime CEL, while live runtime snapshots remain all-family and
  MCP policy blocks still log canonical `security_events`. The runtime-engine
  install log now records the event-family scope. Scoped Linux direct-vsock
  `mcp-load` improved from 593.4/3758.0/5630.6/5749.0 RPS to
  2188.2/10163.2/13934.4/13935.2 RPS at c=1/10/50/200, zero errors. This
  routing assumes effective MCP rules remain enforced by the MCP policy layer
  rather than converted into runtime CEL; any new security-engine merge must
  reassess the event-family contract and ensure MCP-capable rules advertise
  `Mcp` or all-family scope before preserving this optimization.
- Changed `DbWriter` resolved security-event inserts to skip stale child-row
  cleanup deletes on brand-new event IDs while retaining the cleanup path for
  repeated event IDs. This reduces per-event SQLite work on hot audit paths
  without dropping MCP/security logs or weakening blockability. A scoped Linux
  direct-vsock `mcp-load` proof measured 593.4/3758.0/5630.6/5749.0 RPS at
  c=1/10/50/200, zero errors, versus the post-pool baseline
  586.0/3775.4/5564.0/5661.0.
- Changed runtime security-engine evaluation in `capsem-process` from one
  global `Mutex<SecurityEngine>` to a CPU-sized pool of identical compiled
  engines with a shared rule-match accumulator. This preserves runtime MCP/HTTP
  blocking and detection behavior while removing evaluator lock queueing under
  concurrent framed MCP load. Linux direct-vsock `mcp-load` improved from the
  accepted 588.0/812.8/806.0/822.8 RPS at c=1/10/50/200 to
  586.0/3775.4/5564.0/5661.0 RPS, with zero errors.
- Changed framed MCP metrics to record runtime security-event projection and
  runtime security-engine evaluation as bounded `mitm.mcp_stage_duration_ms`
  stages, so policy/security cost is visible in the debug recorder and future
  OTel export without bypassing enforcement.
- Changed MCP session telemetry evidence parsing to avoid full JSON DOM
  allocation on the `DbWriter` thread when deriving `tools/call` argument
  evidence and result kind. MCP calls still write the same `mcp_calls`,
  `ai_mcp_execution_evidence`, and resolved security-event rows, and blocked
  MCP request logging remains covered by the framed MCP security tests.
- Changed Linux KVM vhost-vsock queue notifications to expose the RX/TX vhost
  kick eventfds and register them with `KVM_IOEVENTFD` at the virtio-mmio
  `QUEUE_NOTIFY` register. This matches the existing virtio-blk KVM shape and
  avoids the userspace `queue_notify` fallback on normal guest vsock writes.
  Live `mcp-load` still stayed near the prior ceiling: direct-vsock measured
  590.0/812.7/813.6/825.4 RPS at c=1/10/50/200 versus the previous scoped
  572.2/806.4/811.0/842.8 RPS, so the remaining RPS cap is not queue-notify
  trapping alone.
- Changed Linux KVM vhost-vsock to advertise `VIRTIO_RING_F_EVENT_IDX`, which
  the local `/dev/vhost-vsock` backend negotiated as `enabled_features =
  0x120000000`. This matches the interrupt-suppression shape already used by
  virtio-blk. A scoped live `mcp-load` proof still stayed in the same band:
  raw-single 591.6/767.8/773.6/818.0 RPS and direct-vsock
  589.6/782.0/789.8/834.2 RPS at c=1/10/50/200, so event-index is not the
  remaining RPS ceiling either.
- Changed related session telemetry emitters to use batched `DbWriter`
  submission helpers where they already construct multiple rows for one
  event. HTTP/model telemetry, filesystem monitor events, DNS decisions,
  process exec decisions, MCP file-tool restore logging, and framed MCP
  logging now avoid repeated sender locks/clones while preserving the same
  persisted audit/security rows.
- Changed framed MCP transport hot paths to avoid one inbound payload copy,
  reuse MCP telemetry JSON previews for byte counts, enqueue related MCP
  telemetry rows with one `DbWriter` sender clone, and batch ready response
  frames into one write/flush per connection. The scoped Linux `raw-single`
  `mcp-load` proof remains throughput-capped near 800 RPS, but c=200 p99
  improved from roughly 499ms to 358ms in the 5s scoped run, confirming
  per-frame response flushes contributed to tail latency while the remaining
  RPS ceiling is elsewhere.
- Changed framed MCP success responses to enqueue the already policy-checked
  response before waiting on session DB audit writes. MCP calls still record
  `mcp_calls` and resolved security events through the normal `DbWriter`, but
  saturated telemetry no longer holds the guest-visible response or the MCP
  in-flight permit on the hot path. Linux `mcp-load` improved from the
  previous fresh-initrd proof at 407/608/601/617 RPS for concurrency
  1/10/50/200 to 490/772/775/788 RPS, with zero errors.
- Changed deterministic `local__echo` MCP dispatch to bypass the
  process-to-aggregator and builtin-stdio/RMCP subprocess path at the MITM
  endpoint. The safe echo diagnostic tool still goes through framed MCP
  policy, response policy, telemetry, and session logging, while external
  tools and stateful/networked local builtins still route through the
  isolated aggregator/builtin subprocesses. A Linux VM proof improved
  c=1 `mcp-load` from the recorded ~312 RPS / 4.2ms p99 shape to 407 RPS /
  2.9ms p99, but c=10/50/200 remain capped around 600 RPS, so the remaining
  RPS ceiling is now tracked as framed guest/host transport or telemetry
  pipeline work rather than local builtin dispatch.
- Changed the guest `capsem-net-proxy` process-name lookup to use a shared
  throttled socket-owner index instead of walking every `/proc/<pid>/fd`
  directory for every proxied connection. This keeps process attribution
  best-effort while reducing per-connection guest CPU work on HTTP/RPS bursts.

### Fixed
- Fixed local Linux installs that rebuilt VM assets leaving `capsem status`
  blocked by stopped persistent VMs with missing pinned initrd/rootfs assets.
  The dev asset sync now preserves hash-named files from the previous installed
  asset backup, and default `capsem purge` removes stopped persistent VMs that
  are already unrecoverable because their pinned base assets are missing.
- Fixed the dev service startup recipe so it materializes local base profiles
  from the freshly repacked asset manifest before launching `capsem-service`.
  This prevents stale profile-pinned initrd hashes from forcing remote asset
  downloads during `just exec` after guest binary changes.

### Added
- Added a `direct-vsock-transport` attribution lane to `capsem-bench
  mcp-load`. It uses the same guest AF_VSOCK connection and framed MCP codec
  as `direct-vsock`, but handles a reserved diagnostic echo before MCP policy,
  endpoint dispatch, aggregator, or session DB writes. A scoped Linux VM proof
  measured 3,086.6/13,632.2/22,003.0/37,027.6 RPS at c=1/10/50/200 with zero
  errors, while the same-run `direct-vsock` tool path stayed at
  588.0/812.8/806.0/822.8 RPS. This isolates the current MCP RPS ceiling away
  from KVM/vhost-vsock transport and toward the real MCP policy/dispatch/
  telemetry path after frame parsing.
- Added an ignored host-only framed MCP throughput diagnostic that drives the
  production `serve_io` parser/policy/telemetry path over an in-memory duplex
  stream. The first Linux run processed 10,000 `local__echo` requests at
  roughly 25,290 RPS, isolating the current VM `mcp-load` ceiling away from
  host framed-MCP CPU and toward guest relay/vsock/KVM delivery.
- Added MCP load-test attribution lanes to `capsem-bench mcp-load`. The
  benchmark now reports FastMCP, raw JSON-RPC through one guest relay, raw
  JSON-RPC through four guest relay processes, and direct framed MCP over
  guest vsock:5002 in the same canonical output so MCP RPS investigations can
  separate Python/FastMCP, stdio relay, vsock, and host framed-MCP overhead.
- Added `CAPSEM_METRICS_DEBUG_INTERVAL_SECS`, an opt-in capsem-process
  diagnostic recorder that logs compact MCP stage histogram snapshots to
  `process.log`. This gives the H09 RPS investigation live stage attribution
  while the full OTLP exporter remains a separate telemetry sprint. The dev
  `just run-service` path now forwards the knob into the service environment
  so live benchmark runs can use it, and snapshots log under the normal
  `capsem_process::metrics_debug` target.
- Recorded the H09 MCP live-stage proof: the isolated Linux branch remains at
  roughly 770-800 RPS for deterministic `local__echo`, with or without the
  debug recorder. Stage histograms show endpoint/aggregator dispatch dominates
  while parse, telemetry enqueue, and response write stay sub-0.12ms p99.
- Added H09 MCP dispatch decomposition metrics across the process driver,
  aggregator subprocess, and builtin server. The new low-cardinality
  histograms split client channel send, driver queue wait, MessagePack
  encode/decode, frame read/write, aggregator handler queue, manager lookup,
  server RPC, response channel send, and builtin tool execution. The same
  `CAPSEM_METRICS_DEBUG_INTERVAL_SECS` knob now reaches the aggregator and
  stdio builtin child so live VM proof can compare those stages directly.
- Recorded the H09 MCP decomposition proof: the heavier multi-process
  diagnostic run reached 265/591/586/636 RPS at concurrency 1/10/50/200 with
  zero errors. The builtin `local__echo` body is effectively free
  (~0.015ms average), while aggregator `server_rpc` to the builtin stdio peer
  owns roughly 0.68-0.69ms average and 0.86-0.89ms p99, making local builtin
  stdio/RMCP collapse the next trace-backed RPS implementation target.
- Added OTel-ready MCP echo-path timing histograms for the H09 RPS
  investigation: framed MCP stage timing, MITM endpoint dispatch timing, and
  process-to-aggregator request timing. Labels are bounded by method kind,
  coarse tool kind, stage, and result so the deterministic `local__echo`
  regression can be traced without high-cardinality tool names.
- Added process-side vsock connection metrics with bounded port-kind labels,
  active-connection gauges, close-result counters, and handler-duration
  histograms. These split guest network/MITM pressure from gateway/status
  traffic and make control, terminal, SNI proxy, DNS, audit, exec, lifecycle,
  and unknown vsock lanes OTel-ready.
- Added gateway proxy request metrics with bounded endpoint-class labels,
  method labels, status classes, and request-duration histograms. This makes
  `/profiles`, action endpoints, files/history, and other service proxy traffic
  measurable without high-cardinality path labels.
- Added gateway `/status` control-plane metrics for the P0 hypervisor sprint:
  cache hit/miss/stale decisions, refresh count/duration, and service fan-out
  requests to `/list` and `/info`. These low-cardinality metrics make TUI and
  status polling overhead measurable before optimizing network/RPS lanes.
- Added the P0 Fundamental 80/20 Hypervisor Advances sprint board. It ranks
  the five source-traced performance bets across disk, network/RPS, CPU
  lifecycle, memory/cache, and control-plane overhead, and starts the block
  lifecycle comparison against Firecracker and crosvm before accepting another
  long benchmark loop.
- Added the H08 Disk Throughput Attribution sprint under Hypervisor
  Improvement, separating EROFS DAX rootfs, fallback virtio-blk rootfs,
  writable scratch, VirtioFS, and RPS-adjacent I/O lanes before accepting more
  speedups. The sprint requires request-shape/timing counters, status/OTel
  visibility, real VM counter proof, and canonical `just benchmark` artifacts
  with before/after percentages.
- Recorded the initial H08 artifact baseline: canonical Linux scratch
  sequential read is 0.08x macOS and 0.046x Linux host-native, while the active
  compressed EROFS DAX candidate already lifts rootfs random/small-file lanes
  well beyond the old canonical Linux rootfs artifact. HTTP RPS is 0.83x macOS
  and proxy throughput is 0.93x macOS, so disk remains the first-order gap.
- Added H08 KVM virtio-blk request-shape counters for disk attribution:
  completed block requests, request bytes, aggregate request/drain duration,
  max request bytes, max data descriptors per request, and max requests per
  queue drain. The counters flow through VM metrics snapshots, OTel-compatible
  metric points, service `/info`, gateway `/status`, and `capsem info`.
- Added an opt-in Linux KVM EROFS DAX experiment: `CAPSEM_KVM_ROOTFS_PMEM_DAX=1`
  maps the read-only rootfs image through virtio-pmem, `capsem.rootfs=erofs-dax`
  mounts `/dev/pmem0` with `-o dax`, and the rootfs-format grid records it with
  `--pmem-dax` for direct comparison against the tuned virtio-blk EROFS lane.
- Recorded the Linux x86_64 EROFS DAX benchmark artifact. The DAX lane mounts
  `/run/capsem-lower` from `/dev/pmem0` with `dax=always`, improves random
  rootfs reads and metadata-heavy lanes over the tuned virtio-blk EROFS lane,
  and keeps the experiment opt-in because large sequential reads regressed.
- Recorded an EROFS DAX compressed-vs-uncompressed comparison. Uncompressed
  EROFS DAX improved random rootfs IOPS and most AI CLI startup timings versus
  compressed EROFS DAX, while compressed DAX still led metadata, small-file,
  and large-binary throughput in the same run.
- Added an opt-in Linux KVM file-backed EROFS DAX lane:
  `CAPSEM_KVM_ROOTFS_PMEM_FILE_BACKED=1` maps an aligned rootfs image directly
  with host `mmap` instead of first copying it into anonymous pmem RAM. The
  rootfs-format grid exposes it with `--pmem-file-backed`, pads generated
  EROFS target images to KVM's 128 MiB pmem alignment, and records the backing
  mode plus padding metadata in the benchmark artifact.
- Recorded the file-backed EROFS DAX benchmark artifact. Against the previous
  anonymous-copy DAX artifact, uncompressed EROFS regressed sequential read
  2.9%, random IOPS 3.4%, and cold large-binary read 5.6%, while lower-rootfs
  metadata improved 5.6%. Compressed `erofs-lz4hc-c65536` regressed sequential
  read 3.1% and cold large-binary read 4.5%, but improved random IOPS 12.5%
  and small-JS reads 10.1%. This keeps file-backed DAX as a measured candidate,
  not yet the default throughput answer.
- Recorded compressed `erofs-lz4hc-c65536` + DAX as the current rootfs lead
  candidate because it balances image size with the strongest random/small-file
  behavior. Follow-up remains open for lz4hc retuning, EROFS zstd after a Linux
  6.11+ guest-kernel bump, and raw/cold throughput investigation.
- Tuned Linux guest rootfs read-ahead for the EROFS DAX pmem path. The initrd
  now applies a 16 MiB default to `/dev/pmem0` rootfs mounts while keeping
  ordinary virtio-blk devices at the existing 4 MiB default; the rootfs-format
  grid can sweep `capsem.rootfs_readahead_kb` and records observed `vda`/`pmem`
  queue state. The final rerun improved compressed file-backed DAX sequential
  read 2.8%, cold large-binary read 2.4%, and lower-rootfs metadata 2.3% versus
  the prior active artifact, while random IOPS and small-JS reads regressed
  3.7% and 4.7%.
- Added KVM file-backed pmem mapping policy knobs for the EROFS DAX rootfs:
  `CAPSEM_KVM_ROOTFS_PMEM_MADVISE=none|sequential|random|willneed` and
  `CAPSEM_KVM_ROOTFS_PMEM_POPULATE=1`. The rootfs-format grid can sweep these
  policies and records them in each cell. The startup-inclusive rerun kept the
  default at `none`: `willneed` improved sequential read by 5.9%, but regressed
  cold large-binary read 6.2%, small-JS reads 9.2%, lower metadata 4.7%, and
  Codex startup mean 14.3%.
- Added bounded Linux KVM virtio-blk shape knobs for queue count, queue size,
  segment limit, logical block size, and io_uring mode so rootfs/startup
  tuning can sweep coupled block-device settings instead of one-off constants.
- Added rootfs-specific and writable-device-specific Linux KVM virtio-blk
  shape knobs so read-only rootfs tuning can be tested without forcing the
  writable scratch block device onto the same queue geometry.
- Recorded a Linux x86_64 canonical `just benchmark` run with the rootfs-only
  KVM block-shape candidate on source commit `b834d554`, including refreshed
  in-VM, host-native/raw-system, endpoint-latency, lifecycle, fork, parallel,
  and security-engine artifacts.
- Added a KVM rootfs-format grid harness that materializes rootfs variants
  under `target/`, runs each format through the same virtio-blk shape matrix,
  and records storage, rootfs, startup, sysfs, and DAX capability metadata.
- Recorded the first KVM rootfs-format grid artifacts, showing uncompressed
  SquashFS improves large-binary reads and AI CLI startup against the current
  SquashFS zstd image while EROFS requires a guest-kernel capability slice.
- Enabled EROFS support in the arm64 and x86_64 guest kernel defconfigs so the
  rootfs-format benchmark matrix can compare EROFS on future rebuilt kernels.
- Extended the KVM rootfs-format grid harness with generated SquashFS zstd
  compression-level variants so compression level and block shape can be swept
  fairly together.
- Recorded KVM rootfs-format grid artifacts for EROFS and SquashFS zstd
  compression levels after rebuilding the x86_64 guest kernel with EROFS
  support. EROFS improved rootfs sequential read, random read, large-binary
  read, small-JS read, and AI CLI startup against the same-run SquashFS zstd
  baseline, while metadata stats regressed; zstd level 22 produced the
  smallest image with broadly similar runtime to the shipped zstd baseline.
- Extended the KVM rootfs-format grid harness with explicit EROFS compression
  variants for uncompressed, lz4, and lz4hc images so EROFS can be compared
  against uncompressed SquashFS as well as the shipped SquashFS zstd baseline.
- Recorded the EROFS compression comparison artifact. EROFS lz4hc was smaller
  and faster than uncompressed SquashFS on read and AI CLI startup lanes, while
  metadata-stat throughput remained substantially slower and needs follow-up
  before EROFS can be considered as a default rootfs format.
- Added a `capsem-bench rootfs` metadata diagnostic that measures both the
  normal overlay path and a benchmark-only direct lower rootfs bind mount at
  `/run/capsem-lower`, so EROFS metadata regressions can be separated into
  lower filesystem cost versus overlay amplification without exposing the lower
  mount during normal boots.
- Recorded the rootfs lower-metadata diagnostic artifact, showing EROFS lz4hc
  improves lower metadata traversal over uncompressed EROFS but still trails
  uncompressed SquashFS direct-lower metadata throughput by roughly half.
- Extended the KVM rootfs-format grid harness with EROFS lz4hc physical
  cluster-size variants so `mkfs.erofs -C` can be swept before testing broader
  EROFS inode, tail-packing, xattr, or chunked-file options.
- Recorded the EROFS lz4hc cluster-size sweep, showing 64K clusters improved
  read and overlay metadata lanes while shrinking the image versus the current
  lz4hc build, though tuned EROFS still trailed uncompressed SquashFS on
  metadata throughput.
- Added an opt-in Linux KVM rootfs direct-I/O ablation lane:
  `CAPSEM_KVM_BLK_ROOTFS_DIRECT_IO=1` opens the read-only virtio-blk backing
  file with `O_DIRECT`, and the rootfs-format grid can run it with
  `--direct-io`.
- Recorded the rootfs direct-I/O ablation artifact. Direct I/O worked but
  regressed random, small-file, and metadata-heavy rootfs lanes, so it remains
  a diagnostic lane rather than a default candidate.
- Recorded a startup-inclusive tuned EROFS artifact. `erofs-lz4hc-c65536`
  improved read and AI CLI startup lanes against uncompressed SquashFS while
  continuing to trail on metadata throughput.
- Documented the current Linux/KVM DAX feasibility result: the virtio-blk
  rootfs path does not expose a DAX-capable block device, so DAX requires a
  separate transport/kernel-capability experiment rather than a mount-option
  flip.
- Disabled guest `CONFIG_FUSE_DAX` while keeping `CONFIG_FS_DAX` enabled so
  the EROFS pmem DAX experiment does not force the existing virtio-fs overlay
  device into an unsupported DAX cache-window negotiation path.
- Added a focused KVM block-shape gridsearch harness that records structured
  artifacts for queue count, queue size, segment limit, logical block size,
  Linux sysfs queue state, and `capsem-bench rootfs` results.
- Surfaced live VM resource metrics through `capsem info` and the service
  `/info` response, including metrics schema/capture time, configured RAM and
  vCPUs, host process PID/RSS/CPU time/CPU percent, and session/workspace/rootfs
  disk counters when available.
- Surfaced KVM virtio-blk queue/backend counters through VM metrics snapshots,
  service `/info`, and `capsem info`, including queue notifications, queue
  drains, descriptors, used-ring entries, interrupt decisions, block ops,
  bytes, and async backend depth/fallback counters.
- Surfaced live VM resource and KVM block counters through the gateway
  `/status` feed and the `capsem-tui` session-info overlay so users can see
  resource use and I/O activity without leaving the terminal control surface.
- Added an OTel-compatible metric-point contract for VM metrics snapshots with
  stable Capsem metric names, units, counter/gauge kinds, and bounded
  attributes for resource and KVM block counters.
- Added explicit KVM virtio-blk io_uring submission-queue backpressure: full
  async queues now rewind the descriptor for retry instead of falling back to
  synchronous I/O, with queue-full counters exposed through VM metrics,
  `capsem info`, and the OTel metric-point contract.
- Improved KVM virtio-blk io_uring recovery so completions immediately retry
  a backpressured descriptor when submission capacity is freed, instead of
  waiting for another guest queue notification.
- Added the full Firecracker-shaped KVM virtio-blk async profile: io_uring is
  selected for read-only and writable block devices, uses a fixed registered
  backing file, probes required read/write opcodes, restricts the disabled ring
  before enabling it, and submits queued requests in batches from the block
  worker.
- Added a repeatable Firecracker comparison harness that runs Capsem's
  rootfs/startup benchmark workload under the official Firecracker release with
  the Capsem x86_64 kernel/rootfs assets and records structured JSON results.
- Added a repeatable crosvm reference benchmark harness for the same Capsem
  x86_64 kernel/rootfs rootfs/startup workload, with epoll, corrected-uring,
  direct-I/O, and multi-worker lanes recorded as structured artifacts.
- Recorded a fresh Linux x86_64 canonical `just benchmark` run from clean
  source commit `b6f9b6e2`, including refreshed active artifacts and a
  pre-rerun archive of the prior Linux artifacts for provenance.
- Added canonical `just benchmark` retention so same-architecture active
  artifacts are copied to `benchmarks/archive/` before reruns, superseded
  generated benchmark artifacts are zipped afterward, and active benchmark
  directories keep only the latest artifact for each category, architecture,
  and benchmark lane.
- Added the Hypervisor Improvement meta sprint to turn the Firecracker source
  audit into structured sub-sprints for KVM safety, event delivery,
  observability/status/OTel, CPU/SMP lifecycle, storage/rootfs experiments, and
  benchmark proof.
- Added a Linux KVM virtio-blk io_uring backend that submits read/write
  requests from the existing ioeventfd worker, reaps completions through a
  completion eventfd, preserves synchronous fallback, and records async
  submission/completion/in-flight metrics.
- Added OTel-ready KVM virtio-blk queue/backend metrics for notifications,
  drains, descriptor/used-ring volume, request bytes/duration, interrupt
  decisions, and quiesce drain timing.
- Added the Virtio Block Firecracker Path sprint to track KVM block
  notification suppression, async I/O depth, shared rootfs/benchmark work, and
  macOS comparison reruns as one measured performance stack.
- Recorded macOS arm64 benchmark data for `1.2.1779673506`, including
  in-VM, lifecycle, fork, and security-engine benchmark results.
- Recorded fresh macOS arm64 canonical `just benchmark` data for
  `1.2.1780103109` after merging the Linux support branch, including in-VM,
  endpoint-latency, host-native, lifecycle, fork, parallel, Criterion, and
  VM-originated security-engine benchmark artifacts.
- Added `just benchmark-compare` and `scripts/compare_benchmark_artifacts.py`
  to turn committed Linux/macOS benchmark artifacts into ratio and percentage
  comparisons while making missing lanes explicit.
- Added benchmark contract tests proving the canonical `just benchmark` path
  includes Criterion archiving plus the required serial artifact lanes,
  including host-native, lifecycle, fork, and VM-originated security benchmarks.
- Included `capsem-bench storage` in the default `capsem-bench all` path so
  canonical Linux and macOS benchmark artifacts both record storage attribution
  for rootfs, workspace, tmpfs, overlay, and queue/FUSE metadata.
- Added scatter/gather virtio-blk tests proving KVM block requests preserve
  multi-descriptor guest payload order.
- Added the initial `capsem-tui` crate with a fixture-backed standalone
  terminal control screen, global service light-bar state, per-session desktop
  indicators, and deterministic snapshot rendering for early UI proof.
- Added a `just dev-tui` standalone TUI shell with two fixture sessions,
  SVG snapshot export, and keyboard session switching that does not capture
  plain `q`.
- Added live `capsem-tui` gateway wiring against the installed Capsem HTTP
  gateway with token auth, periodic refresh, typed session mapping, fixture
  fallback, and HTTP provider tests.
- Added active-session terminal WebSocket wiring for `capsem-tui`, including
  gateway token reuse, terminal input forwarding, output buffering, resize
  messages, and basic ANSI cleanup for the Ratatui surface.
- Added hidden `capsem-tui` overlays for help, active-session statistics, and
  the session list so the normal terminal surface stays minimal.
- Added confirmed `capsem-tui` service actions for resuming, suspending,
  stopping, and deleting sessions through the installed HTTP gateway without
  blocking the terminal UI.
- Added `Alt+p` purge in `capsem-tui`, routed through the installed gateway's
  authenticated `/purge` endpoint for temporary and broken VM cleanup.
- Added a profile-aware `capsem-tui` new-session dialog with an editable
  prefilled `tmp-*` session name and live profile selection before
  provisioning.
- Added a `capsem-tui` fork dialog on `Alt+f` that asks for a fork name and
  sends the request through the installed gateway.
- Added `Alt+c` checkpoint/save as an explicit `capsem-tui` action, leaving
  `Alt+s` to mean suspend.
- Added `capsem-tui` to local install/package payloads so the TUI is available
  from `~/.capsem/bin/capsem-tui` after installation.
- Added `capsem_terminal_snapshot` to the Capsem MCP server so agents can
  inspect a session terminal/log surface through MCP with ANSI cleanup, grep,
  source selection, and tailing.
- Added an 8-live-VM host endpoint latency benchmark under
  `tests/capsem-serial/test_endpoint_latency_benchmark.py`, covering global
  service reads, per-VM detail/history/file/policy-context reads, and gateway
  health/token/status reads with committed `benchmarks/endpoint-latency/`
  results.

### Known Follow-ups
- Linux/KVM support is considered user-playable for the release branch after
  the normal merge gates pass (`just test`, `just benchmark`, installed
  smoke/doctor). Remaining Linux work is performance and observability, not a
  blocker for users to try the build.
- After the new all-CEL security engine and parser improvements merge, reassess
  runtime event-family routing. The optimization must only survive if the new
  engine carries explicit MCP rule scope or conservatively reports all-family
  scope for MCP-capable rules.
- The network branch should rerun canonical `just benchmark` and full
  `capsem-bench mcp-load` after merging the new engine/parser work, then use
  `CAPSEM_METRICS_DEBUG_INTERVAL_SECS=2` only for attribution if MCP regresses.
- The macOS team should pull the same merged branch, run `just benchmark` plus
  `capsem-bench mcp-load`, commit benchmark artifacts only, and compare with
  Linux through `just benchmark-compare`.
- Post-response MCP audit writes can still build a session DB backlog under
  high load. Guest-visible responses are no longer blocked by that backlog, but
  DB throughput, backlog visibility in status/OTel, and flush efficiency remain
  the next durability/resource-efficiency work.

### Changed
- Changed `capsem-bench disk` to default to `/var/tmp`, the writable
  scratch/system lane, instead of `/root`, the host-visible VirtioFS workspace.
  The packaged VM before/after path comparison showed `/var/tmp` improving
  sequential write by 43.5%, sequential read by 54.9%, random write by
  286.0%, and random read by 7028.9% versus `/root`; workspace/VirtioFS
  performance remains recorded by `capsem-bench storage`.
- Changed Linux KVM virtio-blk to advertise guest-visible segment-limit and
  logical block-size geometry, matching the conservative crosvm/Firecracker
  shape before larger queue or multi-queue experiments.
- Marked the Hypervisor Improvement H01 safety-and-queue-contracts slice
  complete after the full KVM unit gate and one-shot VM exec smoke passed, and
  opened H03 observability/status/OTel as the next active slice.
- Replaced the earlier opt-in/writable-only KVM virtio-blk io_uring gate with a
  full async profile for both rootfs and writable block devices, while keeping
  `CAPSEM_KVM_BLK_IO_URING=sync` as the explicit benchmark ablation and
  fallback path.
- Disabled in-VM shutdown commands. `capsem-sysutil` now only supports guest
  suspend, `capsem-init` removes `/sbin/shutdown`, `/sbin/halt`,
  `/sbin/poweroff`, and `/sbin/reboot` from the VM overlay, and the host
  ignores deprecated shutdown lifecycle frames for compatibility.
- Gated the Linux KVM virtio-blk io_uring backend to writable block devices
  after the first benchmark showed scratch sequential-read gains but rootfs and
  AI CLI startup regressions when io_uring was used unconditionally.
- Made the Linux KVM virtio-blk io_uring backend opt-in while measured default
  gates continue to show disk or rootfs regressions.
- Added KVM virtio-blk event-index negotiation and shared virtqueue
  notification-suppression helpers, with canonical Linux benchmark artifacts
  recording the mixed performance result for the Firecracker-path sprint.
- Split Google into its own `sprints/google/` meta sprint covering Gmail,
  Drive, gcloud, Firebase, Firebase Realtime DB remote comms, Jet Ski, Gemini,
  and Google AI.
- Routed x86_64 KVM virtio-blk queue notifications through `KVM_IOEVENTFD`
  with a dedicated block worker, so guest queue kicks no longer require vCPU
  MMIO exits while preserving synchronous fallback tests.
- Switched the KVM virtio-blk read/write data path from seek plus per-descriptor
  host I/O to `preadv`/`pwritev` over GPA-translated guest memory iovecs.
- Batched KVM virtio-blk used-ring publication so one queue notification writes
  `used.idx` once after draining all completed block descriptors.
- Added the Profile Foundation meta sprint with F00-F12 sub-sprints, a
  code-reality check, and a crosswalk from the old Profile V2 S-numbered
  boards.
- Made security plugins, dashboard improvements, Google/Gemini integration,
  OpenTelemetry, remote decisions, and remote alert logging explicit Profile
  Foundation scope.
- Renamed Foundation F07 around graph, dashboard, and observability so product
  relationships are a first-class contract instead of dashboard-only logic.
- Expanded Foundation Google scope to name Gmail, Drive, gcloud, Firebase, Jet
  Ski, Gemini, and Google AI credential/integration proof explicitly.
- Reframed S24 as the active post-ship Profile V2 meta sprint so every open
  Profile V2 item is tracked as in-scope child sprint work.
- Created S24 as the single post-ship Profile V2 sprint and migrated remaining
  release-hit-list proof, polish, and board cleanup work into it.
- Added a current Profile V2 sprint snapshot and reconciled the active board so
  S18 is the explicit release gate while S09, S11, S16, and S19 are marked
  closed for the bedrock release.
- Made `just benchmark` archive Rust Criterion microbenchmarks into
  `benchmarks/security-engine/` JSON artifacts, removed superseded historical
  benchmark JSONs, and refreshed benchmark docs so the repo only points at the
  current canonical artifact path.
- Extended benchmark artifacts with UTC timestamps plus richer host hardware and
  OS metadata, and added a host-native benchmark artifact to the canonical
  `just benchmark` path so VM performance is recorded beside the machine's
  local disk, startup, small-file read, and metadata-stat baselines.
- Split benchmark artifact git metadata into overall dirty state and
  `source_dirty`, so artifacts generated earlier in the same run do not hide
  whether the measured source tree itself was clean.
- Standardized benchmark execution around `just benchmark`, with `just bench`
  as an alias and no Linux-only benchmark recipe, so performance artifacts use
  one cross-platform recording path.
- Changed the guest rootfs build default to a configurable 128K squashfs block
  size, improving measured CLI startup and sequential rootfs reads while
  recording the chunk-size choice in `guest/config/build.toml`.
- Changed `capsem-tui` gateway refreshes to reuse the HTTP client and cached
  gateway token, so status polling measures the local status request instead of
  redoing auth bootstrap on every tick.
- Changed `capsem-process` live metrics snapshots to stay on in-memory
  counters instead of recursively scanning VM session directories on the
  service `/list` hot path.
- Changed service read hot paths so `/list` no longer calls per-VM live metrics,
  `/stats` uses an empty/read-only fast path, raw session DB queries use
  SQLite progress handlers instead of a 100ms watchdog-thread floor, and
  policy-context exports no longer duplicate one security event across multiple
  joined detail rows.
- Strengthened the suspend/resume lifecycle integration test so it now proves
  a background guest process keeps the same PID and continues writing after
  warm resume, giving Apple VZ and KVM the same long-term state-preservation
  contract.
- Added Linux host doctor smoke probes for `KVM_GET_API_VERSION` and
  `/dev/vhost-vsock` openability so bootstrap verifies usable KVM devices, not
  just filesystem permissions.
- Added structured `capsem-tui` help and session-list tables, an explicit
  `Alt+l` sessions overlay, and clearer `Alt+i` session info.
- Added focused-field highlighting to `capsem-tui` create and fork dialogs so
  the active input and selected profile are visible.
- Added an empty-state `capsem-tui` startup panel with CAPSEM ASCII art,
  first-launch shortcuts, and the inline create-session form so users can
  create their first VM directly from the empty screen.
- Changed the `capsem-tui` status hint to `help: alt+?` and moved it to the
  far right after active-session statistics, including the empty-session state.
- Changed `capsem shell` to launch `capsem-tui` as the single interactive VM
  control surface; `capsem shell <session>` now opens the TUI focused on that
  session instead of using the legacy direct PTY bridge.
- Added Linux KVM doctor coverage that creates and resolves symlinks under
  `/tmp`, keeping link-heavy cache/tool probes off the VirtioFS workspace while
  leaving snapshot symlink restore scoped to `/root`.
- Reduced the top-level sprint inventory to active Profile V2 work plus the
  credential detection pipeline, moving completed boards to `sprints/done/` and
  stale or superseded boards to `sprints/retired/`.
- Inventoried sprint planning docs and moved retired Profile V2, release, and
  legacy boards under `sprints/retired/` so active release planning starts from
  `sprints/policy-settings-profiles/`.

### Added
- Added rootfs benchmark sub-metrics for large binary sequential reads, small
  JS/package file reads, and metadata-heavy `lstat` walks so Linux/macOS rootfs
  gaps can be attributed to data reads versus loader-style metadata pressure.
- Added an opt-in `capsem-bench storage` diagnostic that records mount metadata
  and splits rootfs reads from writable-path I/O across workspace, tmpfs,
  overlay, and runtime directories for Linux/macOS performance comparisons,
  including detailed sequential and random IOPS/latency profiles per path and
  the booted squashfs compression/block-size, kernel cmdline, block queue, and
  FUSE connection metadata.
- Added Linux release-candidate benchmark artifact plumbing with arch-scoped
  output paths, host/git metadata, optional run IDs, and gross in-VM
  `capsem-bench` gates for disk, rootfs, CLI startup, HTTP, throughput, and
  snapshot operations.
- Added an in-guest `capsem-doctor` SMP diagnostic that compares `nproc` with
  `/proc/cpuinfo` and requires at least two visible vCPUs.
- Added live x86_64 KVM SMP boot support with synthetic ACPI RSDP/RSDT/MADT
  tables and guest CPUID topology so Linux discovers all configured vCPUs.
- Added x86_64 KVM checkpoint trait support for cooperative pause/resume,
  atomic guest-memory checkpoint writes, and checkpoint restore of guest RAM
  plus vCPU regs/sregs, with targeted vCPU kicks for blocking `KVM_RUN` pause
  and unsupported KVM restore paths failing closed instead of silently
  cold-booting.

### Fixed
- Fixed `capsem status` on Linux installs where the systemd unit references
  `~/.capsem/bin/*` symlinks that resolve to `/usr/bin/*`. The stale-unit
  checker now accepts canonicalized symlink targets, so a valid installed
  service no longer reports `service_unit_stale_path`.
- Fixed `capsem-tui` terminal rendering so the guest cursor remains visible at
  the active VM PTY cursor position, and removed the duplicate gradient
  `CAPSEM` word from the empty-state panel.
- Fixed Linux `.deb` post-install service registration in non-login shells by
  falling back from unavailable `systemctl --user` sessions to a real systemd
  system unit that runs Capsem as the installing user.
- Fixed Linux `.deb` post-install asset seeding so local dev symlinks under
  `~/.capsem/assets` are replaced before root copies package manifests.
- Fixed Linux `.deb` installs to remove an existing dpkg record before
  installing from the downloaded/local artifact, so same-version
  half-configured packages unpack the new maintainer scripts instead of
  rerunning stale dpkg control data.
- Fixed Linux service cleanup after `.deb` installs fall back to a systemd
  system unit: `capsem stop`, `capsem service uninstall`, and `just install`
  now try system `systemctl` without `--user` and remove the system unit before
  proving the old runtime is gone.
- Fixed Linux local `just install` service restart for hosts using the systemd
  system-unit fallback by restarting `capsem.service` through system
  `systemctl` with sudo and pointing failures at the system journal.
- Fixed asset rebuilds to remove stale generated manifest/signature metadata
  before regenerating checksums, recovering dev trees previously poisoned by
  privileged package hooks.
- Fixed the `capsem version` embedded build hash so Cargo reruns the build
  script when the current branch ref advances, not only when `.git/HEAD`
  changes.
- Fixed Linux local `just install` so the rebuilt `.deb` is passed to `apt`
  through an absolute file path instead of being interpreted as a package name.
- Fixed release/install version stamping on Linux by replacing BSD-specific
  `sed -i ''` invocations in `just install` and `just cut-release` with a
  portable Perl in-place edit.
- Fixed frontend release builds after privileged installs by cleaning stale
  `frontend/dist` output before `pnpm build`, with a sudo fallback for old
  root-owned artifacts.
- Fixed Linux `.deb` repacking on hosts with large numeric UID/GID values by
  normalizing Tauri's ar members before extraction when the generated archive
  header overflows classic ar owner/group fields.
- Hardened Linux KVM virtio-blk guest-memory handling so zero-copy block I/O,
  discard reads, request header parsing, get-id writes, and status writes
  validate the full `gpa + len` range before exposing guest pointers to host
  code.
- Hardened Linux KVM virtqueue descriptor-chain validation so out-of-range
  available heads, out-of-range `next` indices, cyclic chains, and
  non-power-of-two queue sizes fail closed before devices parse guest
  descriptors.
- Validated Linux KVM split-ring layout alignment and guest-memory coverage so
  malformed descriptor, available, or used ring placements fail closed instead
  of reaching modulo arithmetic or raw ring accesses.
- Hardened Linux KVM virtio-mmio activation so ready queues with invalid size,
  alignment, max-size, or guest-memory layout set device `FAILED` and do not
  start device workers with malformed queue state.
- Hardened Linux KVM guest-memory read/write helpers so hostile offsets that
  overflow `offset + len` return bounds errors instead of panicking before the
  bounds check.
- Hardened Linux KVM virtio-blk request accounting so aggregate descriptor data
  lengths that overflow `u32` return `IOERR` instead of panicking in the queue
  drain path.
- Fixed same-version host binary drift so `capsem status` and service startup
  compare IPC protocol/schema hashes, preventing suspend from failing against
  an incompatible `capsem-process` that reports the same package version.
- Fixed `capsem-tui` gateway refresh failure handling so unavailable live
  status clears stale VM tabs instead of preserving old sessions as actionable
  suspend/stop/delete targets, and now shows failed control actions in a popup
  with the full gateway error detail.
- Fixed `/profiles` so create surfaces only receive profiles with verified VM
  assets, and added `ui`/`tui`/`web` capability flags so terminal and web
  launchers can hide profiles that do not belong on that surface.
- Fixed service purge so `all=false` still removes defunct or profile-corrupted
  persistent VMs while preserving healthy persistent VMs, making TUI cleanup
  actually clear broken profile-pin sessions from refreshed VM lists.
- Fixed `capsem-tui` recovery for stopped VMs with corrupted profile pins:
  the inactive pane now explains that Enter creates a replacement VM, while
  `Alt+d` remains available to delete the bad VM entry.
- Fixed `capsem-tui` suspend feedback so `Alt+s` shows a full-pane
  `suspending...` state while the suspend action runs instead of only updating
  the bottom status bar.
- Fixed `capsem-tui` terminal input after suspend/resume so a failed or closed
  terminal WebSocket clears the connected marker, reconnects the active session
  after resume, and does not drop typed input into a stale terminal task.
- Fixed `capsem-tui` create flow focus so a newly provisioned VM becomes the
  active tab even when the first gateway refresh after `/provision` does not
  list the VM yet.
- Fixed `capsem-tui` corrupted profile-pin handling so non-resumable sessions
  are hidden from the bottom VM tab strip, still appear in the full `Alt+l`
  session inventory, and explain that the VM must be recreated from a signed
  profile if explicitly selected.
- Fixed `capsem-tui` service-offline startup so the TUI shows an offline
  service surface and asks to start Capsem before opening the new-session flow;
  confirming the prompt runs the local `capsem start` command and refreshes
  with a fresh gateway token.
- Fixed `capsem-tui` empty-session creation so the TUI no longer invents a
  `default` profile when `/profiles` is unavailable; the new-session modal now
  blocks Enter until a real profile list is loaded and has unit plus gateway
  E2E coverage for the profile-backed create contract.
- Fixed `capsem-tui` stopped-session rendering so stopped/suspended/failed
  tabs are greyed, the main pane shows a `Press Enter to resume` affordance
  instead of going blank, and the terminal bridge disconnects instead of trying
  to attach a WebSocket to an inactive VM.
- Fixed a `capsem-process` IPC file-descriptor leak where short-lived
  status/metrics connections left writer and lifecycle-forwarder tasks alive
  after the client disconnected.
- Fixed `capsem-tui` live gateway attention handling so sessions with
  `profile_status=current` are not marked stale, and proved the installed
  terminal WebSocket path against two running service sessions.
- Fixed `capsem-tui` terminal rendering to use a real VT/xterm parser with
  color/style preservation, adjacent output coalescing, and dirty-frame
  redraws instead of a hand-rolled ANSI text flattener.
- Fixed `capsem-tui` service latency rendering to reserve four digits so the
  bottom status bar does not shift as latency changes.
- Fixed `capsem-tui` service latency rendering to keep the status dot glued to
  the latency field, making the service block read as one unit.
- Fixed `capsem-tui` shell controls to use an app-owned Alt namespace:
  `Alt+Left/Right`, `Alt+1..9`, `Alt+n/f/r/s/c/t/d`, `Alt+?`, `Alt+i`,
  `Alt+l`, and `Alt+q`, instead of terminal-dependent Cmd/Ctrl forwarding or
  prefix fallbacks.
- Fixed `capsem-tui` help and modal handling by using `Alt+?` for help,
  rendering overlays through Ratatui modal widgets, and resending the active
  terminal geometry whenever the real terminal size changes.
- Fixed `capsem-tui` modal input ownership so `Esc` closes non-confirmation
  overlays, visible modals consume normal keys, and plain VM input resumes
  forwarding as soon as the modal closes.
- Fixed `capsem-tui` tab colors so the selected VM is yellow and every other
  VM tab is blue, removing the previous gray/attention color ambiguity.
- Fixed macOS release builds of the service debug report by widening filesystem
  block counts before computing disk byte totals.
- Fixed macOS release builds of `capsem-process` shutdown handling by returning
  the VM stop result from the main-thread stop task and avoiding a macOS-only
  unused signal receiver.
- Fixed install profile materialization so manifest aliases and legacy local
  alias directories do not make package assembly look for non-existent VM
  assets.
- Added Linux KVM virtio-blk discard handling so explicit guest discard/trim
  requests can punch holes in writable virtio block backing files.
- Refreshed local profile asset pins during dev service startup so benchmark
  runs after `_pack-initrd` use matching initrd/rootfs hashes.
- Expanded x86_64 KVM warm-restore groundwork by checkpointing VM interrupt
  controller, PIT, clock, extended vCPU, Virtio-MMIO transport, and vhost-vsock
  queue state, and by making guest snapshot preparation force a post-resume
  vsock reconnect. The durable process-preserving KVM resume contract still
  fails because restored guests stop making timer-driven forward progress.
- Improved Linux KVM VirtioFS throughput by negotiating 1 MB FUSE request
  pages and matching read-ahead when the guest kernel supports `FUSE_MAX_PAGES`,
  with structured init logging for the negotiated FUSE limits.
- Improved Linux KVM VirtioFS read/write handling by using positional host I/O
  for FUSE file operations, removing an extra seek from the hot path and
  keeping shared host file cursors stable across guest offset reads and writes.
- Fixed Linux `capsem-process` SIGTERM handling so external process death
  drains telemetry and exits instead of leaving the VM listed until service
  teardown.
- Fixed API file-upload observability by recording a synchronous `fs_events`
  row with ambient trace context, so service-originated writes do not depend
  solely on the polling filesystem monitor.
- Fixed Linux fork/snapshot fallback copies to preserve sparse VM disk holes
  when `FICLONE` is unavailable, avoiding 2 GB physical copies on filesystems
  without reflink support.
- Fixed full-test gate assumptions around KVM load by aligning VM-limit tests
  with the service's default eight-VM cap and giving suspend calls enough
  timeout budget to queue behind the host-wide save/restore lock.
- Fixed full-test setup/gateway harness contracts so `/setup/assets` may report
  per-asset download progress and mock terminal WebSocket teardown cannot race
  its shutdown event under parallel pytest.
- Fixed the local Python coverage gate to match the CI-owned 89% schema floor,
  with a regression test that prevents local/CI coverage threshold drift.
- Fixed serial benchmark gates for Linux KVM by separating backend-dependent
  provision latency from steady-state exec/delete latency and cleaning transient
  apt metadata out of the fork image-size workload.
- Fixed the serial log gate to accept early KVM ACPI/PCI boot messages and the
  guest banner when the log stream starts after the Linux version line.
- Fixed `just cross-compile` so its Linux boot test installs the repacked
  `.deb` with CLI/service companion binaries, packaged admin payload, signed
  manifest, payload verification, and Docker vsock permissions instead of the
  raw Tauri desktop package, with the package verifier isolated from the
  checkout venv, frontend dependencies isolated from the host checkout, install
  e2e Docker state isolated from host `.venv`/`node_modules` ownership, and
  session validation accepting current `*-tmp` VM names.
- Fixed the Linux full-test gate under current Rust by cleaning KVM, service,
  and app clippy warnings that were promoted to errors.
- Fixed native guest-agent rebuilds so readonly `target/linux-agent` outputs
  are replaced atomically instead of failing with `Permission denied`.
- Fixed host-side `capsem-pty-agent` exec tests by avoiding inaccessible
  `/root` working directories outside the guest.
- Fixed the PTY/vsock bridge to use nonblocking bidirectional polling with
  bounded buffers, preventing full-duplex terminal traffic from deadlocking or
  dropping queued bytes during peer shutdown.
- Fixed the full test harness to put pytest and VM temporary files under
  `target/tmp` instead of the host `/tmp` tmpfs, avoiding disk-pressure
  cascades during the four-worker VM integration phase.
- Fixed service settings reload isolation by pinning each service instance to
  its startup `service.toml` path, so tests and running services do not follow
  later `CAPSEM_HOME` environment changes.
- Fixed Linux KVM multi-VM vsock boot by allocating a per-VM host port block
  and passing the offset to guest agents through the kernel command line,
  preventing concurrent VMs from racing on fixed host ports 5000-5007.
- Fixed KVM suspend timing by giving the guest agent time to leave the
  pre-checkpoint vsock bridge and enter its post-resume reconnect loop before
  VM state is saved.
- Fixed x86_64 KVM process-preserving warm resume by checkpointing VM interrupt
  controller, PIT, clock, extended vCPU state, selected timer/paravirtual MSRs,
  Virtio-MMIO transport state, vhost-vsock queue state, and by restoring timer
  MSRs after LAPIC state so resumed guests keep making forward progress.
- Added warm-restore Virtio queue reconstruction and a pre-checkpoint
  VirtioFS quiesce hook with structured queue/IRQ telemetry so KVM checkpoints
  do not replay pre-suspend userspace FUSE work through fresh device workers.
- Improved x86_64 KVM checkpoint restore correctness by preserving vCPU MP
  state and avoiding cold-boot x86 setup writes over restored guest RAM.
- Fixed the Linux KVM full `capsem-doctor -x -v` gate, which now passes on the
  nested-KVM proving host after the SMP, VirtioFS, runtime cache, Git trust, and
  network proxy fixes.
- Fixed Git workflows in Linux KVM workspaces by adding guest system Git trust
  for VirtioFS-owned `/root` repositories, avoiding dubious-ownership failures
  when commands run as guest root.
- Fixed Linux KVM guest `uv pip install` by moving the uv cache off the
  VirtioFS workspace to `/var/cache/capsem/uv`, avoiding wheel/archive symlink
  failures under `/root/.cache/uv`.
- Fixed Linux KVM VirtioFS symlink reads by correcting the FUSE `READLINK`
  opcode from the `GETXATTR` slot to Linux opcode 5, which also stops xattr
  probes from being misrouted as symlink reads.
- Fixed Linux KVM VirtioFS rename-over-existing semantics so atomic CLI config
  rewrites keep the moved inode bound to the target path instead of making the
  rewritten file disappear from the guest dentry cache.
- Fixed KVM vCPU run-loop handling so application processors continue across
  guest HLT exits and transient `KVM_RUN` `EAGAIN` responses instead of
  silently dropping out of the VM.
- Fixed guest doctor readiness on Linux KVM by keeping the DNS and MITM network
  proxies alive across init shell transitions, failing closed when either proxy
  cannot start, and moving the Python virtualenv off the VirtioFS workspace to
  `/var/lib/capsem/venv`.
- Fixed the Gemini doctor wrapper lookup to use portable POSIX `command -v`
  instead of a shell-specific `type -P`.
- Fixed Linux developer bootstrap so fresh hosts install the C toolchain,
  Node/npm, and sqlite before cargo tool setup, and so pnpm is pinned to the
  lockfile-compatible 10.x installer path instead of picking up stale pnpm 11
  shims.
- Fixed `doctor --fix` VM asset setup to build the host architecture instead
  of requiring cross-architecture Docker emulation during first setup.
- Fixed KVM pure-logic regressions by correcting the vhost-vsock vring ioctl
  size and tightening VirtioFS namespace path handling.
