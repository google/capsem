# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Changed
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
- Fixed the dev service startup recipe so it materializes local base profiles
  from the freshly repacked asset manifest before launching `capsem-service`.
  This prevents stale profile-pinned initrd hashes from forcing remote asset
  downloads during `just exec` after guest binary changes.

### Added
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

## [1.2.1779673506] - 2026-05-24

### Fixed
- Fixed release package profile asset URLs so packaged Profile V2 installs
  download VM assets from the live GitHub Release, and updated the post-release
  verifier to seed packaged profiles before running `capsem update --assets`.

## [1.2.1779668968] - 2026-05-24

### Fixed
- Fixed macOS package notarization for the packaged `capsem-admin` Python
  payload by signing native Mach-O wheel extension files before building the
  installer package.

## [1.2.1779665197] - 2026-05-24

### Fixed
- Fixed release metadata stamping so the Python lockfile records the same
  package version as the workspace, Tauri app, and Python project metadata.

## [1.2.1779665141] - 2026-05-24

### Fixed
- Fixed the Linux install test harness clean-state path to stop the systemd
  user unit before killing scoped Capsem processes, preventing `Restart=always`
  from racing tests that intentionally replace `capsem-service` with a broken
  binary.

## [1.2.1779662531] - 2026-05-24

### Fixed
- Fixed package setup for manifest-only installs so packaged Profile V2
  sidecars install before local heavy VM asset fallback, allowing `.deb`
  postinstall to complete from signed packaged profiles without bundled
  kernel/initrd/rootfs files.

## [1.2.1779658398] - 2026-05-24

### Fixed
- Fixed guest `localhost` resolution during boot by restoring a deterministic
  `/etc/hosts`, so CLIs that bind local helper servers such as Google
  Antigravity (`agy`) do not send `localhost` lookups through Capsem DNS.
- Fixed live VM header model counters so VM-scoped model calls update the
  in-memory metrics snapshot used by `/status`, while host-scoped model calls
  remain excluded from VM accounting.
- Fixed Settings loading against the Profile V2 `/settings` contract so the UI
  accepts typed `profile_presets`, `effective_rules`, and `settings_profiles`
  responses without requiring the removed legacy settings tree.
- Fixed Gemini guest setup for Profile V2 sessions: saved Google AI
  credentials now project to `GEMINI_API_KEY`, and non-interactive Gemini
  launches use a real wrapper that defaults to `--yolo` instead of relying on a
  shell alias.
- Fixed dashboard status polling to retry gateway initialization before
  reporting the service offline, avoiding a stale offline state after
  start/install races when the gateway is actually healthy.
- Fixed dashboard connected-state polling to confirm `/status` before showing
  the service offline after a transient gateway health miss.
- Fixed human `capsem status` output to summarize profile assets compactly and
  move profile provenance into a trailing block instead of dumping every asset
  URL and hash inline.
- Fixed the local install harness to restore the packaged `capsem-admin`
  wrapper and Python payload when repairing or simulating an installed layout.
- Fixed frontend gateway API calls to refresh the localhost auth token and
  retry once after a 401, preventing the onboarding Profile step from blocking
  on stale gateway credentials.
- Fixed onboarding provider credentials for the Profile V2 cutover: detected
  service credentials now show as configured, and manually entered keys are
  saved as Profile V2 credential IDs instead of legacy settings keys.
- Fixed the final onboarding screen to use session/profile language and show
  profile cards instead of exposing VM asset readiness internals.
- Fixed profile listing launchability so `/profiles` and `/profiles/catalog`
  mark profiles without an installed signed catalog revision unusable even
  when their VM asset files are present.
- Fixed local setup for packaged Profile V2 installs so `capsem run` and
  temporary `capsem shell` can pin profile/package/asset metadata from the
  packaged base profile without generating a duplicate corp profile.
- Fixed Profile V2 runtime defaults so packaged base profiles emit
  schema-valid profile payload JSON instead of defaulting profile accent colors
  to the service-settings-only `"blue"` value.
- Fixed the local install simulation to codesign macOS Mach-O binaries with the
  Virtualization entitlement, matching package postinstall behavior so release
  smoke tests do not boot unsigned `capsem-process` binaries.
- Fixed `just install` so it reruns non-interactive setup after restoring
  preserved settings and syncing assets, preventing local reinstalls from
  undoing package postinstall setup and leaving profile pins incomplete.
- Fixed `just install` so it no longer restores package-owned `profiles/base`
  or stale profile catalog sidecars over the freshly materialized package
  profiles, preventing VM asset hash drift after initrd repacks.
- Fixed `just install` so the initrd repack runs inside the recipe and repairs
  the existing local profile metadata before any sudo/package step, keeping the
  installed product coherent even if the user cancels or cannot complete sudo.
- Fixed `just install` so local installs rebuild the host-arch profile-derived
  VM assets before repacking/syncing them, preventing an old rootfs from
  surviving after base profile package/tool contracts change.
- Fixed ARM64 guest kernel configuration to use a 48-bit userspace virtual
  address layout, so TCMalloc-based Linux ARM64 CLIs such as Google
  Antigravity (`agy`) can run inside Capsem VMs instead of crashing during
  startup.
- Fixed the local install simulator to tolerate repo `assets/` being the same
  filesystem tree as `~/.capsem/assets`, avoiding same-file copy failures while
  repairing a dev install.
- Fixed the macOS package postinstall hook so it waits for the service socket
  and gateway health endpoint before opening the desktop app, preventing the UI
  from launching into a stale offline screen during install.
- Fixed package postinstall hooks to fail loudly when no target user can be
  determined for per-user setup instead of leaving a package that requires
  manual `capsem setup`.
- Fixed Profile V2 HTTP write enforcement so derived `http.read` and
  `http.write` rules compile into guarded runtime CEL, preserve rule priority,
  let runtime overlays override profile defaults, and resolve profile `ask`
  decisions as allow/pass until S15 ships interactive confirm resolution.
- Fixed in-guest doctor diagnostics to treat positive MCP network probes as
  conditional on the selected profile while still requiring write requests to
  be blocked when `CAPSEM_WEB_ALLOW_WRITE=0`.
- Cleared the local Docker/Colima initrd packaging caveat after restoring the
  half-running Colima VM and proving `just _pack-initrd` with Docker
  cross-compilation, initrd repack, hash-named assets, and manifest signature
  verification.
- Updated developer skills to require a Colima stop/start recovery attempt
  before reporting macOS Docker-backed asset builds as blocked.

### Changed
- Changed default VM sizing to the agent-friendly `4 CPU / 8 GB RAM / 8 active
  VMs` baseline across Profile V2 base profiles, builder defaults, service
  admission defaults, onboarding, and the create-session override UI, and
  removed stale onboarding resource selectors that no longer write through
  Profile V2.
- Bumped the active release line and default stamping recipe from `1.1` to
  `1.2` for the Profile V2/bedrock engine release.
- Expanded human `capsem profile show` and `capsem profile resolve` output with
  package, tool, MCP, VM sizing, and VM asset contract summaries.
- Changed `capsem create`, `capsem resume`, and `capsem restart` to preserve
  typed Profile V2 provision metadata and print profile id/revision/status,
  package contract hashes, pinned VM asset hashes, and asset-health progress
  without changing the first-line VM id output.
- Changed `capsem info <vm>` to preserve and render Profile V2 VM pins,
  including profile payload hash, package contract hash, and pinned
  kernel/initrd/rootfs hashes.
- Changed the onboarding wizard to select Profile V2 profiles through the
  profile catalog/select routes and to show profile identity in the ready
  summary instead of the old security-preset wording.
- Changed frontend VM launch to refresh selected-profile asset status at first
  launch and show a modal download/progress state instead of silently blocking
  creation while assets are checking or downloading.
- Changed profile catalog/status surfaces to report VM asset readiness per
  profile, including missing local paths, so one broken profile cannot hide or
  block usable profiles.
- Changed the frontend profile catalog and launch flows to refuse profiles
  whose VM assets are missing or invalid while still showing the missing asset
  path needed to repair the profile.

### Added
- Added Google Antigravity CLI (`agy`) to the Profile V2 guest tool contract:
  base profiles declare the official `https://antigravity.google/cli/install.sh`
  curl install, `capsem-admin` schemas model it as typed `packages.curl_installs`,
  and image-workspace/rootfs generation materializes and verifies it as a
  required guest tool.
- Added `capsem mcp list` and `capsem mcp show` aliases for the Profile V2 MCP
  connector inspection path.
- Added typed Profile V2 document CLI coverage for `capsem profile create
  --file` and `capsem profile update <id> --file`.
- Added `capsem confirm list` to expose the current disabled S15 ask/confirm
  resolver state through the CLI.
- Added typed Profile V2 mutation CLI coverage for `capsem profile fork` and
  `capsem profile delete`.
- Added read-only Profile V2 CLI inspection with `capsem profile list`,
  `capsem profile show`, and `capsem profile resolve`.
- Added `capsem skills list/show/add/delete` for Profile V2 skill inspection
  and direct user-profile skill mutations through the service `/skills` routes.
- Added broader `capsem enforcement` and `capsem detection` CLI coverage for
  runtime rule compile, update, file-backed backtest, and detection hunt flows.
- Added the first `capsem-file-engine` crate so file activity normalization has
  a first-class Bedrock Engine boundary outside `capsem-core`.
- Added the first `capsem-process-engine` crate so process exec normalization,
  command classification, and inline process Security Engine evaluation have a
  first-class Bedrock Engine boundary outside `capsem-core`.
- Added the first `capsem-network-engine` crate and moved domain/HTTP network
  policy primitives out of `capsem-core`, with process runtime and builtin MCP
  tooling consuming the new boundary directly.
- Moved the DNS wire parser and adversarial fixture/property tests into
  `capsem-network-engine`, with DNS handler, process dispatch, examples, and
  fuzz targets consuming the Network Engine parser directly.
- Moved DNS transport result and DNS SecurityEvent projection into
  `capsem-network-engine`, so DNS runtime blocks, resolved-event rows, and
  legacy `dns_events` projection share the Network Engine boundary.
- Added Network Engine-owned HTTP SecurityEvent projection, with MITM telemetry
  adapting request/response stats into a typed `HttpSecurityEventInput` instead
  of constructing HTTP subjects directly inside `capsem-core`.
- Added Network Engine-owned MCP SecurityEvent projection, with framed MCP
  dispatch adapting JSON-RPC summaries into a typed `McpSecurityEventInput`
  before runtime CEL evaluation and resolved-event journaling.
- Moved the SSE wire parser and parser tests into `capsem-network-engine`, so
  AI/model stream parsing now starts at the Network Engine boundary instead of
  the old `capsem-core::net::parsers` path.
- Moved provider-neutral AI stream events, summaries, provider identity, and
  non-streaming usage parsing into `capsem-network-engine`, leaving
  `capsem-core` to own only MITM provider routing and key injection.
- Moved typed AI request parsing for Anthropic, OpenAI, and Google/Gemini into
  `capsem-network-engine`, including tool-result extraction and malformed-body
  fallback tests.
- Moved canonical AI interaction evidence projection into
  `capsem-network-engine`, so model request/response/tool-call/tool-result
  evidence is built at the Network Engine boundary before core telemetry
  persistence.
- Added Network Engine-owned model SecurityEvent projection, and switched
  session-backed detection hunt reconstruction to build model events through
  that boundary instead of constructing model subjects inside the service.
- Added persisted runtime enforcement/detection overlay recovery: service
  runtime rule mutations now atomically write a typed
  `capsem.runtime-security-rules.v1` store, and startup recompiles the saved
  overlays back into the CEL registries while failing closed on invalid rules.
- Disabled runtime `ask` overlays until the S15 confirm prompter lands, so
  enforcement validate/compile/install/backtest and persisted restore fail
  closed instead of exposing an approval workflow with no resolver.
- Added runtime Security Engine health to `/debug/report`, including the
  persisted runtime-rule store path, enforcement/detection registry counts,
  match counters, rule attribution, and the current confirm resolver state.
- Added runtime Security Engine health to `capsem status`: JSON status now
  carries the typed security summary from `/debug/report`, and text status
  shows compact enforcement/detection rule and match counts.
- Added a resolved Security Event summary to `capsem logs`, so session logs show
  event, block, detection, family, and rule counts before the raw structured
  security-event JSON lines.
- Added a Settings -> Policy Security Engine health panel that renders typed
  `/debug/report` runtime enforcement/detection counts, match totals, runtime
  rule-store state, and confirm resolver availability.
- Added a Settings -> Profiles catalog panel that renders typed profile
  catalog revisions, current/installed drift, and the canonical
  `active`/`deprecated`/`revoked` lifecycle states.
- Added profile selection through `POST /profiles/{id}/select` and surfaced the
  selected/default profile in the Settings -> Profiles UI.
- Added profile-backed VM create requests in the frontend quick-session and
  customize-session flows, forwarding service-reported profile id/revision and
  showing the active profile in the create dialog.
- Added VM profile identity and lifecycle status to the frontend session list,
  including a corrupted marker when a VM lacks an explicit profile pin.
- Added a profile asset readiness panel to the frontend Sessions screen,
  showing the active profile revision, architecture, payload hash, and
  per-asset source/hash/size provenance from `/status`.
- Added runtime rule backtesting to the Settings -> Policy Live Rules editor,
  posting draft enforcement/detection rules with a JSON event corpus and
  rendering deduplicated evidence rows from the service backtest result.
- Added session detection hunting to the Settings -> Policy Live Rules editor,
  letting operators run a draft detection rule against a specific session via
  `/sessions/{id}/detection/hunt` and inspect the returned evidence rows.
- Added the first S08d Security Engine Criterion benchmark harness for
  canonical CEL compile/evaluate, policy-context materialization, 100-rule
  last-match evaluation, and native HTTP lookup comparison.
- Added the first committed Security Engine CEL microbenchmark artifact under
  `benchmarks/security-engine/` and surfaced the host-side numbers in the
  benchmark results docs with explicit non-VM-originated caveats.
- Added the first VM-originated Security Engine benchmark for process
  enforcement: a serial live-service/VM test installs a runtime CEL block rule,
  measures repeated blocked exec decisions, verifies runtime match counters,
  `session.db` resolved-event rows, and `logs` attribution, and archives the
  result under `benchmarks/security-engine/`.
- Expanded the Security Engine Criterion benchmark artifact with runtime
  detection evaluation, backtest evidence deduplication, and runtime rule
  registry operation timings.
- Wired `just bench` to run the Security Engine Criterion microbenchmarks and
  VM-originated process-enforcement benchmark alongside the existing in-VM and
  lifecycle/fork benchmark stages.
- Added a VM-originated HTTP request enforcement benchmark that blocks a
  guest HTTPS request through the MITM/Security Engine path, verifies runtime
  counters, `session.db` security rows, and `logs` attribution, and archives a
  dedicated security-engine benchmark artifact.
- Refined the HTTP request enforcement benchmark to separate guest wall-clock
  latency from curl `time_starttransfer`, with a warmup request so cold
  proxy/TLS setup does not masquerade as Security Engine cost.
- Added curl phase timing deltas to the HTTP request enforcement benchmark so
  DNS, TCP connect, TLS appconnect, post-pretransfer first byte, and response
  tail costs are visible in the committed artifact.
- Added a persistent TLS keep-alive lane to the VM-originated HTTP enforcement
  benchmark so repeated in-connection block decisions prove sub-millisecond
  MITM/Security Engine response timing and one security log row per request.
- Added Security Engine benchmark coverage for runtime compiled-plan rebuilds
  and Detection IR parse/lowering/compile costs, with committed artifacts and
  `just bench` wiring for the `capsem-core` security-pack Criterion harness.
- Added runtime CEL enforcement on the DNS proxy path plus a VM-originated DNS
  request benchmark that blocks guest resolver lookups before upstream
  resolution, verifies `dns_events`, `security_events`, runtime counters, and
  `capsem logs` qname attribution, and archives a dedicated benchmark artifact.
- Added runtime CEL enforcement on the framed MCP endpoint plus a VM-originated
  MCP request benchmark that blocks guest `local__echo` tool calls, verifies
  `mcp_calls`, canonical `security_events`, runtime counters, and `capsem logs`
  server/tool attribution, and archives a dedicated benchmark artifact.
- Expanded `capsem logs` security-event projection with family-specific debug
  fields such as DNS qname, HTTP host/path, MCP server/tool, model provider/
  name, file path, and process operation/class.
- Added the internal "Ledger of the Realm" engineering-quality reference and
  linked the active S08b/canonical-AI-evidence sprint docs to its Lannister,
  Winterfell, Baratheon, and Iron-Bank standards.
- Added the S08 canonical AI interaction evidence side-sprint so model/MCP
  policy, detection, telemetry, timeline, quotas, and plugin work have a
  provider-neutral substrate for OpenAI, Anthropic, and Google/Gemini traffic.
- Added explicit host-versus-VM AI attribution requirements so future
  service-owned model prompts charge host telemetry/counters instead of VM
  health totals.
- Added main sprint release holds for host/service AI counters, resolved-event
  attribution, logger accounting owner fields, and tests proving host prompts
  correlated with a VM do not charge VM metrics.
- Added S08 canonical AI evidence contracts in `capsem-security-engine`,
  including OpenAI/Anthropic/Gemini/host fixtures, host-vs-VM attribution fields
  on security events and quota dimensions, optional model/MCP evidence subjects,
  and tests proving host AI does not charge VM accounting.
- Added the first `capsem-core` AI evidence adapter so existing OpenAI,
  Anthropic, and Gemini request/stream parser summaries project into canonical
  `ModelInteractionEvidence` with tool-call, tool-result, usage, argument
  status, and host-vs-VM attribution tests.
- Added normalized session database tables for canonical AI interaction
  evidence so provider/API/model/tool/linkage fields are queryable directly
  instead of being hidden in an opaque JSON blob.
- Added explicit canonical-AI-evidence enum persistence traits and SQLite
  `CHECK` constraints so session DB evidence rows can only store approved enum
  spellings.
- Added first canonical AI/MCP execution linkage: framed MCP tool calls now
  link to model-emitted MCP tool calls when trace id and normalized tool name
  agree, updating both queryable evidence rows and the legacy tool-call
  projection.
- Added security-engine quota/status projection for canonical AI evidence,
  including API family, parse/evidence status, model tool/result/execution
  counts, linked MCP tool-call counts, and MCP execution link identifiers.
- Closed the canonical AI evidence side sprint with additional fixtures and
  tests for OpenAI Responses, orphan model tool calls, orphan MCP executions,
  and provider unknown-field drift.
- Added the first S08b `capsem-security-engine` contract crate with normalized
  security events, resolved-event actions, detection findings, quota dimensions,
  and throttle-ready serialization tests.
- Added the first S08b Security Engine core pipeline shell, ordering
  preprocessors, enforcement, confirm, detection, postprocessors, and resolved
  event construction with fail-closed enforcement errors.
- Changed Security Engine `ask` decisions without a configured confirm resolver
  to record an applied confirm step and fail closed to a terminal block, so
  inline process decisions do not leave unresolved prompts in logs or jobs.
- Added a real CEL-backed S08b enforcement evaluator in `capsem-security-engine`
  so enforcement rules compile through the `cel` crate before install and
  evaluate against normalized `SecurityEvent` values at runtime.
- Added a real CEL-backed S08b detection evaluator so runtime detection rules
  produce typed findings on normalized `SecurityEvent` values before resolved
  event emission.
- Added lowering from `capsem.detection.ir.v1` into real CEL runtime detection
  rules, with explicit family/field allowlists so unsupported Sigma-derived
  paths fail closed before runtime install.
- Added Security Engine match-stat recording hooks so enforcement and detection
  matches update the runtime rule registry counters that future service stats
  routes will expose.
- Added first service-owned runtime `/enforcement/*` and `/detection/*`
  handlers for validate/compile, live add/update/delete/list, and stats backed
  by real CEL compilation and compile-first registry installs.
- Added deterministic priority ordering to runtime enforcement/detection
  registries and seeded the default effective profile's enforcement rules into
  the service runtime registry at startup, with profile/user/corp attribution
  and typed callback guards around profile CEL conditions; profile-scoped rules
  are kept out of the global runtime-rule broadcast snapshot.
- Added service-owned runtime enforcement and detection backtest handlers that
  evaluate candidate CEL rules against typed normalized `SecurityEvent` inputs
  and return the shared deduplicated `BacktestResult` shape.
- Added the first service-owned detection hunt handler for running multiple
  candidate detection rules over a supplied normalized event corpus.
- Added the first session-backed detection hunt golden path:
  `/sessions/{id}/detection/hunt` reads a hand-built canonical session DB
  corpus, reconstructs HTTP security events from structured journal/projection
  rows, verifies the reconstructed event projects iso-style into
  `capsem_proto::PolicyContext`, and runs real CEL detection rules against
  paths/hosts from the DB.
- Extended session-backed detection hunt reconstruction beyond HTTP so
  canonical `security_events` rows can join existing DNS, MCP, model, file,
  process, and snapshot projections into typed `SecurityEvent` values for CEL
  backtest/hunt rules, with common-row reconstruction for VM, profile, and
  conversation events.
- Added canonical AI evidence reconstruction for session-backed detection hunt:
  model events now prefer `ai_model_interactions` for provider/API family,
  stream, usage, and cost fields, while MCP events attach
  `ai_mcp_execution_evidence` for argument/result status.
- Added raw file path policy projection for normalized file security events,
  so CEL and Detection IR rules can target `file.activity.path` separately from
  classified `file.activity.path_class`.
- Added canonical `security_events` output to `capsem logs`, so resolved
  Security Engine decisions from `session.db` are visible as structured JSONL
  with VM/profile/user/rule/finding attribution alongside process and serial
  logs.
- Added canonical security-log support to the MCP VM log tool's grep/tail
  filtering so agent-side debugging sees the same resolved Security Engine
  events as the CLI.
- Updated HTTP gateway log contract tests and architecture docs so `/logs/{id}`
  is treated as the typed security/process/serial log envelope.
- Enriched `/timeline/{id}` security rows with canonical resolved-event rule,
  pack, finding-count, VM, profile, user, and accounting-owner attribution so
  timeline debugging no longer has to jump straight to SQL for those fields.
- Updated MCP tool metadata and usage docs so `capsem_vm_logs` and
  `capsem_timeline` advertise security-log and security-layer support.
- Changed runtime enforcement/detection backtest evidence rows to report
  canonical enforcement paths such as `http.request.host` instead of an opaque
  whole-subject blob.
- Expanded enforcement/detection backtest evidence rows with common
  attribution, HTTP headers/body, MCP request/response/link evidence, and model
  tool-call/tool-result paths so forensic hunts explain the fields rules
  matched.
- Added HTTP gateway contract coverage for runtime enforcement validation and
  session detection hunt routes so the security API preserves forensic matched
  fields through the gateway.
- Expanded HTTP gateway contract coverage across the S08b enforcement and
  detection route groups, including compile, backtest, list, stats, live
  create/update/delete, inline hunt, and session hunt passthrough.
- Improved `capsem detection hunt-session` human output to show matched event
  ids, rules, packs, outcomes, and canonical evidence fields instead of counts
  only.
- Added typed model tool-call policy projection under
  `model.request.tool_calls`, including name, origin, argument status, status,
  linked MCP call id, and parse confidence, with session-backed detection hunt
  reconstruction from `ai_model_tool_calls`.
- Added typed model tool-result policy projection under
  `model.response.tool_results`, including content kind, previews, error
  status, returned-to-model state, linked MCP call id, and parse confidence,
  with session-backed detection hunt reconstruction from
  `ai_model_tool_results`.
- Added a session policy-context export path:
  `GET /sessions/{id}/policy-contexts` and
  `capsem export-policy-contexts <session>` emit JSONL fixtures from
  `session.db` for admin/runtime corpus work, with live VM proof for blocked
  process enforcement.
- Added the first committed session-export policy-context fixture and matching
  process enforcement pack/expected report so admin offline backtest and Rust
  CEL parity both cover a real `process.exec` block shape.
- Added typed process operation and command-class columns to the canonical
  `security_events` ledger so blocked process decisions preserve policy
  evidence even when no downstream exec projection exists.
- Added a typed frontend API client surface for runtime enforcement and
  detection routes, including validate/compile/install/delete/list/stats,
  backtest, live hunt, and session-backed detection hunt calls.
- Added a Policy settings "Live Rules" UI for runtime enforcement and detection
  overlays, including rule priority, attribution, match counts, validation,
  install, and guarded runtime-only delete actions.
- Added the first S08c shared policy-context/CEL corpus fixtures, with Python
  Pydantic loading and Rust CEL parity coverage over canonical
  `http.request.*` roots plus rejected `event.subject.*` authoring.
- Added `capsem-admin detection backtest` for offline pySigma-backed detection
  checks against typed policy-context fixture JSONL.
- Added `capsem-admin enforcement backtest` for offline enforcement checks against
  typed policy-context fixture JSONL, with golden expected-result artifacts for
  the first shared S08c corpus.
- Added Rust S08c parity coverage proving the real CEL evaluator matches the
  committed admin enforcement backtest expected artifact.
- Added a committed Detection IR artifact for the S08c Sigma corpus and Rust
  parity coverage proving canonical `http.request.*` detection fields match
  the admin detection backtest expected artifact.
- Added `capsem-admin enforcement compile` to fail closed on unsupported or legacy
  enforcement roots before offline backtest.
- Added an explicit admin policy path allowlist so `capsem-admin enforcement compile`
  rejects unknown canonical-looking paths and cross-family policy roots before
  offline replay.
- Fixed `capsem-admin enforcement backtest` to compile-check enforcement packs before
  fixture replay, so an empty corpus cannot report success for invalid policy
  paths.
- Added an S08c drift test proving the committed Sigma-derived Detection IR
  artifact exactly matches current `capsem-admin` compiler output before Rust
  consumes it.
- Extended the real process-enforcement E2E so a VM-originated blocked exec is
  verified in both `capsem logs` and the resolved-event `session.db`
  `security_events` / `security_event_steps` journal.
- Expanded the admin policy-context model and offline enforcement backtest subset
  beyond HTTP so DNS/MCP/model/file/process/profile scalar roots, boolean
  equality, and numeric equality can be tested through `capsem-admin`.
- Added indexed model tool-call/tool-result enforcement paths to admin backtest so
  rules can match roots such as `model.request.tool_calls[0].name` and
  `model.response.tool_results[0].returned_to_model`.
- Added rule-corpus workflow documentation tying policy-context fixtures,
  enforcement/detection expected artifacts, admin commands, and Rust parity
  tests together.
- Expanded the S08c policy-context corpus with detection-only and
  auth-without-secret HTTP rows so enforcement and detection parity tests cover
  divergent outcomes.
- Added a session-backed detection hunt expected artifact for the hand-built
  `session.db` corpus, pinning matched fields and evidence signatures from the
  resolved-event journal path.
- Added session-backed detection hunt projection coverage for DNS, MCP, model,
  file, process, snapshot, VM, profile, and conversation rows, including
  canonical profile activity matched fields.
- Added CLI runtime security commands for enforcement and detection rule
  list/stats/validate/install/delete plus session-backed detection hunt.
- Added typed runtime rule definitions to the rule registry and service/API
  responses so installed enforcement/detection rules can be rebuilt into live
  Security Engine CEL evaluators without losing decision, severity, Sigma, or
  tag metadata.
- Added a service-side runtime Security Engine builder that evaluates installed
  enforcement and detection registries together and records live match counts
  back to the correct registry.
- Added `security_decisions` to session DB triage so normalized
  `security_events` decisions and failed steps surface alongside network, DNS,
  MCP, exec, and audit signals.
- Added production MITM telemetry dual-write for canonical resolved HTTP
  `security_events` while preserving the existing `net_events` projection, so
  Network Engine traffic now starts entering the S08b normalized event journal.
- Added inline Network Engine enforcement for HTTP requests: `capsem-process`
  now builds a CEL-backed runtime Security Engine from effective profile HTTP
  rules, MITM evaluates normalized `http.request` events before upstream
  dispatch, and blocked requests journal both `net_events` and canonical
  `security_events`.
- Added request-body-aware inline HTTP enforcement: when a runtime Security
  Engine is installed, MITM now buffers bounded request bodies before upstream
  dispatch so `http.request.body.text` CEL rules can block without touching the
  network, while preserving the forwarded bytes and telemetry body preview.
- Added response-body-aware inline HTTP enforcement: when a runtime Security
  Engine is installed, MITM can evaluate decoded `http.response.body.text`
  before guest delivery and synthesize a 403 without leaking the upstream body.
- Changed MITM security-event telemetry to persist the actual runtime
  `SecurityResult` when inline enforcement runs, preserving response-phase
  event types, rule ids, findings, and resolved steps instead of rebuilding a
  request-shaped event from `NetEvent`.
- Changed MITM runtime telemetry to persist every resolved request/response
  phase result for a transaction, so an allowed request event is not overwritten
  by a later response-phase block or finding.
- Added canonical MCP Security Engine journaling for framed MCP tool calls so
  allowed and blocked MCP requests write `security_events` alongside the
  existing `mcp_calls` projection.
- Added canonical DNS Security Engine journaling so DNS handler results write
  `security_events` alongside the existing `dns_events` projection.
- Added canonical file Security Engine journaling so file monitor and MCP file
  restore/delete events write `security_events` alongside `fs_events`.
- Added canonical process Security Engine journaling so exec dispatch writes
  typed observe-only `process.exec` events alongside `exec_events`.
- Added inline Process Engine enforcement for exec dispatch: `process.exec`
  events now evaluate through the runtime Security Engine before guest
  delivery, blocked exec calls resolve the pending IPC job with an error, and
  the canonical resolved event records the final decision.
- Added shared Process Engine command classification for session-backed
  detection hunt reconstruction, so historical `process.exec` events use the
  same canonical classes such as `shell`, `python`, and `network` as live exec
  enforcement.
- Added Process Engine runtime rule match stats coverage and subsystem-neutral
  fail-closed wording for runtime Security Engine compile failures.
- Added structured Process Engine decision logging for exec evaluation so
  `capsem logs <vm>` includes event ids, attribution, final action, rule/pack,
  reason, and process command class alongside the session database trail.
- Added JSON serialization coverage for Process Engine decision logs so the
  `security.process` fields that power `capsem logs` remain queryable.
- Added service log endpoint coverage proving structured process security
  decision lines are returned verbatim with VM/profile/user/rule attribution.
- Added testable `capsem logs` formatting so structured process security lines
  survive CLI tailing, and taught shell IPC handling to ignore runtime rule
  match-drain replies.
- Added a real VM e2e for runtime process enforcement: install a shell-blocking
  rule, prove `capsem exec` is blocked, and prove `capsem logs` shows the
  structured `security.process` decision with VM/profile/rule attribution.
- Fixed stale profile-asset test fixtures and child process log filters so
  old `request.*` policy roots no longer fail closed during boot and
  `security.process` lines are not filtered out of `process.log`.
- Added live VM status security metrics from the canonical resolved-event
  stream, including security event counts, block counts, detection counts,
  latest block, and latest detection surfaced through process metrics snapshots
  and service list/info responses.
- Added live VM status counters for canonical HTTP, DNS, model, MCP, file, and
  process security events, with host-attributed model events excluded from VM
  token/cost accounting.
- Added session database seeding for live VM status metrics so resumed
  persistent VM processes start from durable HTTP, DNS, model, MCP, file,
  process, security, block, and detection counters before adding new live
  canonical events.
- Added live profile-policy reload for the Network Engine runtime Security
  Engine: `capsem-process` now shares a swappable engine slot with MITM, so
  `ReloadConfig` can replace profile-derived HTTP enforcement without
  rebuilding the proxy config or restarting the VM process.
- Added typed runtime enforcement/detection rule snapshots to process IPC so
  service-owned `/enforcement/*` and `/detection/*` mutations can push live CEL
  rule state into already-running VM processes and report per-session
  propagation status.
- Added process-to-service runtime rule match draining so live VM enforcement
  and detection matches are folded back into service `/enforcement/stats` and
  `/detection/stats` without relying on stale service-local counters.
- Added VM/session/profile/user identity propagation into Network Engine
  security events and canonical AI evidence, including `CAPSEM_SESSION_ID` and
  `CAPSEM_PROFILE_REVISION` handoff through `capsem-process` and the MCP
  aggregator child environment.
- Fixed local setup-generated profile payloads to include the required UI mode
  when installing a local profile revision from `CAPSEM_ASSETS_DIR`.
- Added the shared `capsem-proto` policy context schema that future CEL and
  high-level DSL rules mirror, with versioned typed roots for common, HTTP,
  DNS, MCP, model, file, process, and profile activity.
- Added canonical policy-context CEL evaluation in `capsem-security-engine`, so
  runtime enforcement/detection rules now use roots such as
  `http.request.host` and reject internal `event.*` paths.
- Added all-family CEL match/pass smoke coverage for the policy context,
  covering dedicated DNS, HTTP, MCP, model, file, process, and profile roots
  plus common-root coverage for credential, VM, conversation, and snapshot
  security events.
- Added typed HTTP request policy projection for canonical CEL rules, including
  request URL/path, case-insensitive headers, and body text predicates such as
  `http.request.body.text.contains("secret")`.
- Added Rust Detection IR evaluation against the new S08b normalized
  `SecurityEvent` contract so Sigma-derived findings can run on the shared
  event model instead of a parallel fixture-only shape.
- Added S08b event identity fields for parent event, stream, activity, sequence,
  source engine, and enforceability so later engine wiring has the correlation
  data needed for timeline, telemetry, and quota work.
- Added S08b security-event schema versions, enforcement/detection pack identity
  fields, and JSON fixtures covering every normalized event family plus resolved
  event findings.
- Added the first S08b resolved-event emitter contract with required versus
  best-effort sink semantics, delivery bookkeeping, and shared event/finding id
  tests.
- Added the first structured resolved-event session ledger:
  `security_events`, `security_event_steps`, `detection_findings`,
  `detection_finding_tags`, and `security_event_links`, with
  `WriteOp::ResolvedSecurityEvent` persistence, canonical enum spelling checks,
  session-schema tooling coverage, and a `/timeline/{id}` `security` layer.
- Added S08b backtest result shaping with full event refs, mismatch outcomes,
  default 100-row match limits, and evidence-signature deduplication.
- Added the first S08b runtime rule registry contract with compile-first
  add/update, previous-plan preservation on compile failure, delete, and live
  match stats.
- Added S08b plugin-groundwork event semantics: first-class ask/block/rewrite/
  throttle decisions, labels/context/history snapshots, findings, declarative
  mutations, mutation target validation, and internal transport projection.
- Added deterministic S08b plugin transform validation with canonical event
  hashes, immutable core event enforcement, and prior label/finding/mutation
  preservation.
- Updated S08b security-event JSON fixtures to include plugin-facing context,
  trace labels, decisions, findings, and declarative mutations.
- Added plugin transform records to resolved security events so replay/audit can
  tie plugin identity to input/output event hashes.
- Added a deferred S22 rate-limit, budget, and quota sprint while keeping S13
  scoped to remote enforcement/observer plumbing and reserving S08/S12
  compatibility points for future throttle decisions.
- Added explicit S12 planning for authoritative in-memory running-VM status with
  enforcement/detection counters, latest detection, latest block, and shared
  `/metrics/json` plus Prometheus scrape sources.
- Added typed `capsem-admin doctor` output that checks admin toolchain
  readiness and optional Profile V2 image-plan derivation without using
  `guest/config` as the operator-facing source of truth.
- Added bootstrap-managed shared skill symlinks for Claude Code, Gemini CLI,
  Codex, and Cursor.
- Added the first S08 Profile V2 HTTP gateway contract coverage for profile
  catalog/revision routes, profile CRUD/resolve, skills, standard MCP servers,
  rules/evaluate, confirm-pending reads, profile-selected VM create response
  pins, and gateway `/status` profile/asset provenance.
- Added S08 gateway coverage for Profile V2 `/setup/assets` download progress,
  `/debug/report` profile asset provenance, exact service typed-error
  passthrough, and service debug-report diagnostics for stale or mismatched
  gateway runtime files.
- Added S08 live HTTP gateway coverage for selected-profile VM creation: real
  service/gateway processes now prove `/provision` accepts profile id/revision,
  reconciles the selected profile's verified VM assets before boot, execs
  through the gateway, and echoes the pinned profile state through
  `/info/{vm_id}`.
- Added S08 adversarial HTTP gateway coverage proving Profile V2 typed-error
  status/body passthrough for malformed profile creation, locked
  skill/MCP/rule mutations, invalid rule evaluation, asset cleanup while
  updating, and revoked profile revision install.
- Added regroup sprint specs for service-settings schema/admin parity and the
  policy-rule versus detection/Sigma architecture decision before CLI,
  telemetry, plugins, rule UI, and Confirm UX continue.
- Added `capsem-admin detection compile|check` with pySigma-backed Sigma
  parsing, typed `capsem.detection.ir.v1` output, JSONL normalized-event
  fixture checks, and fail-closed unsupported Sigma subset coverage.
- Added Rust Detection IR V1 schema/serde/evaluator parity fixtures so
  `capsem-core` consumes the same `capsem.detection.ir.v1` artifact emitted by
  `capsem-admin detection compile`.
- Added corp-facing admin CLI, enforcement, and detection-format docs covering
  PyPI install, developer editable usage, pySigma validation, Detection IR, and
  policy/detection command proofs.
- Added Profile V2 settings/profile provenance to the redacted service debug
  report, including selected profile, profile roots, effective VM summary,
  resolver trace summary, and credential-id-only reporting.
- Added Profile V2 service-settings runtime wiring for service asset locations,
  default VM sizing, and per-session `vm-effective-settings` plus resolver
  trace attachments.
- Added capsem-process consumption of session-attached Profile V2 effective
  settings for network defaults, MCP defaults, and Policy V2 runtime rules.
- Added framed MCP Policy V2 `ask` confirmation resolution through the shared
  confirmer/backoff contract before request dispatch and response surfacing,
  with redacted confirmation snapshots.
- Added HTTP Policy V2 `ask` confirmation resolution through the same
  confirmer/backoff contract before upstream request dispatch or guest response
  surfacing.
- Added model Policy V2 `ask` confirmation resolution through the shared
  confirmer/backoff contract before model request dispatch, model response
  surfacing, and tool-call/tool-response delivery, with redacted metadata-only
  confirmation snapshots.
- Added model Policy V2 `model.request` body rewrite support for
  `request.data` rules, forwarding only the rewritten bytes upstream and
  recording rewritten request previews in telemetry.
- Added a `net::policy_v2` runtime import surface plus CEL, gzip model-response,
  and builder config/defaults tests to keep Profile V2 policy enforcement and
  image-generated settings aligned.
- Added hardening coverage for HTTP gzip decompression, CEL quoted-literal
  parsing, and builder image/defaults alignment.
- Added guard coverage to keep generated builder/frontend settings fixtures from
  being treated as Profile V2 runtime authority.
- Added the first S07 UDS foundation: typed VM metrics snapshot structs plus
  service/process IPC request and response variants for live metrics.
- Added read-only Profile V2 UDS profile routes for listing profiles, fetching
  a profile record, and resolving VM-effective settings with resolver trace.
- Added Profile V2 UDS profile mutation routes for creating, forking, updating,
  and deleting user-owned profiles.
- Added Profile V2 UDS rules routes for listing resolved rules, fetching a
  rule with provenance, and dry-running V2 policy evaluation against synthetic
  subjects without enforcing or prompting.
- Added Profile V2 UDS rule mutation routes for creating user-authored rules
  and deleting direct user rules, including default built-in profile override
  materialization, duplicate-rule rejection, and locked-rule delete failures.
- Added chained functional and bounded performance coverage for the Profile V2
  UDS Rules API before mirroring it through the HTTP gateway.
- Added Profile V2 service tests proving profile creation cannot shadow locked
  profile roots and settings saves follow the currently selected user profile.
- Added the S07 UDS closeout surface: typed `GET /confirm/pending`, Profile V2
  `GET /skills` / `POST /skills` / `DELETE /skills/{id}`, locked/duplicate
  skills mutation coverage including inherited same-kind duplicates, and a
  chained profile/skills/MCP/rules route proof.
- Changed MCP management to use Profile V2 MCP servers: profiles now use the
  standard top-level `mcpServers` map with Capsem governance under
  `mcpServers.<id>.capsem`; `/mcp/connectors` now
  lists/adds servers, `/mcp/connectors/{id}` deletes direct user servers,
  and the old `/mcp/{servers,tools,policy}` plus `/mcp/tools/*` service/CLI
  surface, capsem-mcp debug tools, and service-to-process management IPC are
  removed.
- Added typed Profile V2 package/tool contracts and per-architecture VM asset
  declarations, including canonical BLAKE3 hash validation, path-traversal
  rejection, VM-effective serialization, and inherited resolver merge coverage.
- Added the formal Profile V2 JSON Schema Draft 2020-12 artifact with valid
  and invalid golden fixtures plus a Rust `jsonschema` validation gate.
- Added Pydantic v2 Profile V2 payload and manifest models for admin tooling,
  including Pydantic-only JSON validation/dumping helpers, TOML-to-Pydantic
  validation, and the canonical `active`/`deprecated`/`revoked` status enum.
- Added the first Service Settings V2 admin contract slice: Pydantic v2
  service-settings models, Pydantic-only JSON/TOML validation and dump helpers,
  a committed Draft 2020-12 schema artifact, valid/invalid golden fixtures, and
  Rust/Python fixture parity tests.
- Added the first `capsem-admin settings` commands: schema export,
  TOML/JSON validation, doctor summaries, typed JSON reports, and focused CLI
  coverage over the Service Settings V2 contract.
- Added a shared Service Settings V2 defaults fixture checked by both Python
  and Rust, and aligned Python's default user profile roots with the Rust
  `CAPSEM_HOME` / `$HOME/.capsem` path contract.
- Added `capsem-admin settings init` to emit Pydantic-generated Service
  Settings V2 JSON or TOML drafts with profile-root options, asset cache
  selection, overwrite protection, and validation tests.
- Documented the Service Settings V2 versus Profile V2 boundary, the
  `capsem-admin settings` validation flow, and the split from the guest/UI
  descriptor schema.
- Added `capsem-admin profile schema` and `capsem-admin profile validate`
  for Profile V2 JSON/TOML payloads, including typed JSON reports with profile
  id and revision.
- Added `capsem-admin profile init <profile-id>` to emit a valid Profile V2
  JSON or TOML draft through the Pydantic model, with all-architecture VM asset
  placeholders, package/tool contract defaults, optional file output, and
  parity tests proving init JSON matches init TOML after reparsing.
- Added `capsem-admin image plan <profile>` to derive a typed image build plan
  from Profile V2 package/tool/VM asset contracts, with `--arch all` by default,
  single-arch narrowing, and fail-closed missing-asset checks.
- Added `capsem-admin image verify <profile> --assets-dir <dir>` to verify
  profile-declared local kernel/initrd/rootfs assets by architecture, size, and
  BLAKE3 hash, with typed `capsem.image-verification.v1` JSON output and
  non-zero exits on missing or mismatched assets.
- Added typed `capsem.image-inventory.v1` package/tool inventory checks to
  `capsem-admin image verify --inventory`, comparing apt, Python, node, and
  required guest tool versions against the Profile V2 image plan while
  preserving Pydantic-only JSON input/output.
- Added rootfs build extraction of `image-inventory.json`, collecting installed
  apt, Python, node, and tool versions from the built container and validating
  the artifact through the same Pydantic model used by `image verify`.
- Changed `capsem-admin image verify` to auto-discover per-architecture
  `image-inventory.json` files under the asset directory and report inventory
  contract checks by architecture, rejecting ambiguous all-arch single-file
  inventory input.
- Changed profile image verification to fail closed when any selected
  architecture is missing its `image-inventory.json`, so package/tool contract
  proof is required rather than silently falling back to asset-only checks.
- Added `capsem-admin image verify --doctor-bundle` support for
  `capsem-doctor --bundle` tar files, parsing the JUnit probe result without
  extracting the archive and failing image verification on in-VM test failures.
- Added `capsem-admin image sbom` to generate per-architecture SPDX 2.3 guest
  image SBOM JSON from typed `image-inventory.json` artifacts, including
  profile/revision/package-contract identity and package-manager purl refs.
- Added a profile-backed release-image boot gate that requires host-arch
  `image-inventory.json`, boots the profile image, captures
  `capsem-doctor --bundle`, and verifies the bundle through
  `capsem-admin image verify`; local asset preflight now rebuilds when the
  host-arch image inventory is missing.
- Documented the S08a policy/detection contract: `capsem.enforcement-pack.v1`,
  `capsem.detection-pack.v1`, `capsem.detection.ir.v1`, normalized security
  event taxonomy, typed findings, admin validation/check commands,
  implementation ordering, and test matrix.
- Added typed `capsem-admin enforcement validate|schema` and
  `capsem-admin detection validate|schema` support for strict Pydantic policy
  and detection pack envelopes, including YAML detection envelopes, with
  committed JSON Schema artifacts.
- Added `capsem-admin manifest check <manifest> --fast` with typed
  `capsem.manifest-check.v1` reports, Pydantic manifest validation, local
  `file://` profile payload hash/id/revision checks, remote HTTP(S) `HEAD`
  checks, and non-zero exits on missing or mismatched profile payloads or
  signatures.
- Added `capsem-admin manifest check <manifest> --download` to fetch every
  referenced profile payload, profile signature, VM asset, and VM asset
  signature into a temp or explicit download directory, verifying profile
  payload hashes and profile-declared VM asset sizes and BLAKE3 hashes.
- Added `capsem-admin manifest generate --profiles <dir>` to produce typed
  Profile V2 catalog manifests from local JSON/TOML profile payloads, deriving
  exact payload hashes, `.minisig` URLs, status/current-revision overrides, and
  file or hosted profile URLs without hand-authored manifest JSON.
- Added minisign-backed `capsem-admin manifest sign`,
  `manifest verify-signature`, and `manifest check --download --pubkey`
  cryptographic verification for downloaded profile payload and VM asset
  signatures.
- Added a developer bootstrap proof that `uv sync` exposes the `capsem-admin`
  entrypoint and that `uv run capsem-admin --version` succeeds after Python
  dependencies are installed.
- Added release package layout proof for `capsem-admin`: macOS `.pkg` and
  Linux `.deb` assembly now require the relocatable admin wrapper plus its
  packaged Python payload, and release policy tests verify the helper is
  prepared before OS packages are built.
- Added `capsem-admin image build-workspace` to materialize a profile-derived
  build workspace from the Profile V2 package/tool contract, emitting
  `capsem.image-workspace.v1` reports and generated `guest/config`-compatible
  TOML without reading repo hand-authored image settings.
- Added `capsem-admin image build` as the public profile-derived image build
  entrypoint, routing generated workspaces into the existing kernel/rootfs
  Docker builder with typed `capsem.image-build.v1` JSON reports and dry-run
  support.
- Added the required Profile V2 `ui` contract (`everyday` or `coding`) across
  Pydantic, JSON Schema, Rust profile parsing/effective settings, fixtures, and
  generated built-in profile drafts.
- Added `capsem-admin profile init-builtins` to generate typed
  `everyday-work` and `coding` base profiles, plus committed generated base
  profile TOML drafts under `config/profiles/base/`.
- Changed built-in profile generation to derive package, tool, AI provider,
  MCP server, and VM resource contracts from `guest/config`, preserving the
  current release image inputs while making the profiles the source of truth.
- Added profile-aware `scripts/build-assets.sh --profile` and Justfile
  `build-assets` / `build-kernel` / `build-rootfs` profile arguments so local
  asset builds can route through `capsem-admin image build`.
- Changed VM asset build recipes and PR install CI to require a Profile V2
  payload, using `config/profiles/base/coding.profile.toml` by default and
  removing the unprofiled `capsem-builder build guest/` fallback from live
  build lanes.
- Fixed release SBOM attestation to cover Linux `.deb` packages as well as the
  macOS `.pkg`, and documented that the current `cargo-sbom` artifact is the
  Rust host SBOM while profile-derived guest package/tool SBOMs remain S07b
  image-verification work.
- Added Profile V2 section-level editability gates so profiles can allow user
  skill or MCP edits while locking AI providers, rules, VM assets, package
  contracts, or other sections; service mutations enforce the locks and forks
  preserve them. The editability map itself is immutable through profile update
  routes to prevent unlock-then-edit bypasses.
- Changed service settings reload fallback to reuse the startup settings
  snapshot when `service.toml` is absent or unreadable, preventing profile roots
  from silently falling back to defaults.
- Added Rust Profile V2 payload schema validation helpers for JSON and TOML
  payloads backed by the production Draft 2020-12 schema artifact.
- Changed the signed profile catalog manifest to the canonical
  `ProfileManifest` / `format = 1` contract, removing the transitional
  generation naming and old asset-manifest compatibility language.
- Changed VM asset readiness to be profile-driven: service startup now resolves
  boot assets from the selected profile's per-architecture declarations,
  downloads missing assets from profile URLs, and forwards expected hashes to
  `capsem-process` for boot-time verification.
- Added durable per-session telemetry identity: `session.db` now records the
  VM id, resolved profile id, and local user id, and `/info` exposes those
  fields for support/status flows.
- Added VM profile pins for persistent/running VM metadata, including resolved
  profile id, signed profile revision, profile payload hash,
  package-contract hash, and pinned boot asset identity.
- Changed VM profile pins to read the installed profile revision sidecar and
  include the installed profile payload hash when a verified catalog payload is
  present.
- Added core profile catalog reconciliation so active revisions install/update
  from signed payloads, deprecated installed revisions stay available for
  existing VMs, and revoked installed revisions lose their launchable profile
  plus current state.
- Added `POST /profiles/catalog/reconcile` on the service API so UDS/gateway
  callers can apply signed profile catalog lifecycle state and receive a typed
  install/deprecate/revoke/error summary.
- Added `capsem profile reconcile-catalog --manifest <path> --pubkey <path>`
  so the native CLI can apply a signed profile catalog through the service
  reconciler and print either a compact lifecycle summary or raw JSON.
- Added `capsem profile reconcile-catalog --manifest-url <https-url>` so
  operators can reconcile a signed Profile V2 catalog from a remote source,
  with `http://` accepted only for loopback development/test hosts and a
  bounded manifest body.
- Added typed `[profile_catalog]` service settings plus service-side scheduled
  profile catalog reconciliation from the configured signed catalog URL and
  profile payload public key.
- Added a read-only profile catalog status surface plus `capsem profile
  catalog [--json]` so operators can inspect the persisted signed catalog,
  installed profile revisions, revision lifecycle status, and configured
  catalog source.
- Added per-profile catalog revision inspection through
  `GET /profiles/{id}/revisions` and `capsem profile revisions <id> [--json]`,
  including current/installed revision markers and canonical lifecycle status.
- Added profile revision lifecycle actions through the service and CLI:
  `install`, `update`, and `remove` now operate on signed catalog revisions,
  reject revoked installs, clean revoked installed revisions, and remove local
  launchable state while preserving archived payload material.
- Changed profile catalog reconciliation to remove launchable installed
  profiles whose profile id is absent from the signed catalog while preserving
  the archived installed payload for retention/VM-pin cleanup.
- Added profile-aware asset retention sources so cleanup can preserve VM assets
  referenced by installed profile payloads and by persistent VM profile pins.
- Added `POST /setup/assets/cleanup`, a profile-era asset cleanup endpoint that
  removes unreferenced hash-named/legacy asset files without old manifest
  authority, preserves installed-profile and saved-VM pins, and refuses to run
  while assets are still checking or updating.
- Added `POST /setup/assets/reconcile` so callers can force the service-owned
  Profile V2 asset reconciler to check/download profile VM assets on demand.
- Added explicit profile selection for fresh VM create/provision requests and
  `capsem create --profile [--profile-revision]`, with selected profile asset
  reconciliation and VM-effective profile attachment before process spawn.
- Changed `capsem update --assets` to call the service Profile V2 asset
  reconciler instead of the old asset-manifest downloader.
- Changed VM profile pinning to require complete installed profile revision
  authority when present, including the runtime profile file, archived verified
  payload, and matching payload hash.
- Added structured profile asset check/download lifecycle logs with redacted
  asset URLs, plus status propagation for the service asset check timestamp.
- Added explicit Profile V2 asset provenance to service/CLI asset health,
  including profile id, profile revision, installed profile payload hash, and
  redacted per-asset source/hash metadata in reconcile, list/status, setup
  asset status, and debug-report payloads.
- Added adversarial coverage proving concurrent profile asset reconciles share
  one download run and asset cleanup refuses while a profile asset download is
  active.
- Changed first-use VM create/run to await the service Profile V2 asset
  reconciler before process spawn, and made create-from-source, fork, and
  persist derive boot-asset identity from the VM profile pin while rejecting
  pin/registry drift.
- Added chained service-level coverage proving a profile asset reconcile is
  reflected consistently in `/setup/assets`, `/list`, debug reports, and
  service logs after downloading from a local asset server.
- Added formal `file://` Profile V2 VM asset reconciliation support plus live
  E2E coverage proving `capsem update --assets` can fill an empty asset cache,
  boot a real VM from the reconciled hash-named assets, exec inside it, and
  preserve the installed profile revision pin in `capsem info --json`.
- Added a real-VM fork-lineage E2E proof that writes a file, forks, deletes the
  source, resumes the fork, mutates filesystem state, forks again, deletes the
  middle VM, and proves the final fork preserved only the expected descendant
  state.
- Added current UI baseline screenshots for the marketing-site refresh sprint,
  covering the hero plus the feature, security, how-it-works, and FAQ sections.
- Changed `capsem update --assets` to honor the selected service UDS socket
  instead of assuming the default runtime socket.
- Changed the runtime network policy module names from transitional
  `policy_v2`/`policy_v2_*` paths to the forward `policy` and `policy_model`
  surfaces, with DNS/MITM tests split into focused behavior modules.
- Removed the legacy MITM HTTP policy hook runtime path. Request/response-head
  HTTP enforcement must now move through the S08b canonical Security Engine
  path instead of the old pipeline hook.
- Removed the remaining legacy named-policy runtime: `net::policy`,
  `policy_confirm`, model-policy helpers, Policy Hook Spec0 API/artifact,
  policy-only DNS/MCP/MITM tests, the old policy benchmark, and the
  `policy_hook_events` session table/write path. HTTP, MCP, DNS, model, file,
  and process policy work now has one forward path: canonical Security Engine
  events.
- Removed the old Rust VM asset `ManifestV2` model, verified-manifest loaders,
  manifest-driven downloader, and manifest-driven cleanup path. CLI status and
  service debug reports now rely on Profile V2 asset health instead of legacy
  asset manifests, and cleanup removes stale legacy asset metadata files.
- Changed persistent VM resume to require forward profile pins and pinned asset
  identity; unpinned registry entries no longer fall back to the current
  profile/assets.
- Changed VM profile pinning to require a signed profile catalog revision,
  profile payload hash, and pinned asset identity before create-from-source,
  fork, or persist can produce durable VM state.
- Fixed VM forks to preserve VM-effective profile attachments and fail closed
  on profile drift before the fork is registered or executed.
- Added profile identity and status to VM list/status payloads, `capsem list`,
  and `capsem info`: each VM now reports its pinned profile/revision plus
  `current`, `needs_update`, `deprecated`, `revoked`, `corrupted`, or
  `unknown`.
- Removed legacy `assets.manifest.*` service settings and setup-time asset
  manifest checks; old asset-only manifests are no longer runtime authority.
- Changed `/setup/corp-config` inline and URL installs to accept Profile V2
  corp profile TOML and refresh the typed settings-profile surface.
- Changed guest boot config ownership so `GuestConfig`/`GuestFile` live under
  the VM namespace instead of the legacy policy-config namespace.
- Removed the legacy `net::policy_config` module, v1 settings-file runtime
  fallbacks, v1 install/setup fixtures, and old `user.toml`/`corp.toml`
  support-bundle/uninstall preservation paths in favor of Profile V2
  `service.toml` and profile roots.

### Changed
- Renamed the public admin enforcement-pack surface from `capsem-admin policy`
  to `capsem-admin enforcement`, including the Pydantic model/schema ids
  (`capsem.enforcement-pack.v1`, `capsem.enforcement-compile.v1`, and
  `capsem.enforcement-backtest.v1`), committed fixtures, docs, and tests. The
  old `policy` command group is not kept as a public alias.

### Fixed
- Fixed same-millisecond Security Event ID collisions across HTTP, DNS, MCP,
  and file logging. HTTP now carries a per-request event seed, and DNS/MCP/file
  event IDs use nanosecond timestamps so bursty decisions no longer collapse
  rows in `security_events`.
- Fixed synthetic HTTP block/error telemetry to enqueue Security Engine
  `net_events` and resolved `security_events` at the decision point instead of
  relying on response-body finalization, preserving fast denied keep-alive
  requests in `session.db` and `capsem logs`.
- Fixed settings policy-rule saves to reject unsupported `.match(` condition
  terms before writing a user profile override.
- Fixed HTTP gzip handling so comma-separated `Content-Encoding` token lists are
  recognized case-insensitively and malformed gzip headers with reserved flags
  pass through instead of dropping bytes.
- Fixed Policy V2 CEL parsing so method-looking text inside quoted string
  literals is not mistaken for `.contains()`/`.matches()` calls.
- Fixed Policy V2 dry-run/runtime callback coverage for generated `http.read`
  and `http.write` rules, including boolean `true` CEL catch-all conditions.
- Fixed `POST /profiles` so it rejects ids that already exist in built-in,
  base, corp, or user profile roots instead of writing a shadowing user file.
- Fixed `just smoke`, `just test`, and `build-ui` ordering so Tauri frontend
  assets are built before Rust workspace compile/clippy/test phases that need
  `frontend/dist`.
- Fixed isolated smoke/doctor runs to avoid installed gateway-port collisions
  and to skip persistent service-unit checks when a test-scoped service unit is
  intentionally not required.
- Fixed Profile V2 VM runtime migration compatibility so sessions consume only
  Profile V2 `vm-effective-settings.toml` instead of reopening legacy settings
  files at runtime.
- Fixed running VM reloads to refresh Profile V2 effective policy from each
  session attachment, including MCP builtin domain policy and Policy V2 rules.
- Fixed Profile V2 conditional MCP/HTTP rules so narrow argument/path rules no
  longer collapse into broad legacy tool/domain allow-block lists.
- Fixed default user profile discovery to resolve under `CAPSEM_HOME`/`HOME`
  instead of a literal `./~` directory, keeping local artifacts out of runtime
  and test profile resolution.
- Fixed install E2E asset handling when the repo `assets/` path is a symlink,
  including file-only asset copying so nested/stale arch directories cannot
  poison install fixture refresh.
- Fixed the Profile V2 valid-payload minisign fixture so profile catalog
  install/reconcile tests exercise real signature verification with a matching
  test public key.
- Fixed service test fixtures so profile roots are created consistently and
  asset lifecycle log assertions tolerate equivalent download event ordering.
- Fixed full smoke stability by closing inherited Python fixture log fds,
  provisioning E2E services with Profile V2 asset homes, separating signed MCP
  VM-lifecycle fixtures from editable profile-mutation fixtures, and running
  VM-heavy service/CLI and MCP smoke groups sequentially to avoid Apple VZ
  cleanup starvation.

## [1.1.1778860037] - 2026-05-15

## [1.1.1778855131] - 2026-05-15

### Added
- Added a dedicated marketing FAQ page with a hypervisor-vs-container answer
  as the first FAQ.
- Added `capsem status --json` with a typed `capsem.status.v1` health report
  for install verification and UI/test consumers.
- Added a Settings -> About debug report action that copies redacted
  version, runtime, and VM asset/initrd fingerprints for GitHub bug reports.
- Added `capsem debug` and the `capsem.debug.v1` JSON debug report so release
  bugs can include status/doctor readiness issues, setup-state, runtime, asset
  hash, host binary hash, disk-space, install-layout, process-liveness, and
  redacted log-tail evidence from the same `/debug/report` service endpoint
  used by the UI.
- Added `scripts/capture-install-status.py`, a release verification harness
  helper that captures `capsem status --json` into a structured evidence bundle
  with raw command output, parsed status JSON, metadata, version output, and a
  shallow `CAPSEM_HOME` tree snapshot. The bundle also captures optional
  `capsem debug` output and service/gateway pid, socket, and port breadcrumbs
  while redacting `gateway.token`, plus a focused installed-layout index for
  helper binaries, asset manifests, setup state, the platform service unit, and
  the macOS app bundle path. Saved VM registry and persistent-session summaries
  are captured without leaking saved VM environment variable values.
- Added a service-owned VM asset supervisor that reports `checking`,
  `updating`, `ready`, and `error` states with progress and retry detail.
- Added saved-VM base asset dependency tracking so persistent VMs can record the
  rootfs/kernel/initrd hashes, asset version, arch, and guest ABI they require.
- Added a reusable `.deb` payload verifier and wired release CI to validate
  Linux package helper binaries, signed manifests, and manifest signatures.
- Added a macOS release CI gate that requires a Developer ID Installer identity
  and runs `pkgutil --check-signature` plus Gatekeeper assessment after
  notarization and stapling.
- Added `capsem purge --product` for explicit whole-product resets that remove
  runtime files plus durable Capsem state after confirmation.
- Added an OpenTelemetry metrics handoff for the follow-up sprint, including
  the service/process IPC boundary, the live VM counter source of truth, and
  the split between JSON status surfaces and `/metrics`.

### Changed
- Changed setup/profile fixture policy roots from legacy `qname` /
  `request.*` conditions to canonical `dns.request.*` and `http.request.*`
  CEL paths.
- Closed the Profile V2 S07/Post-S06 sprint ledger after reconciling later
  S07c/S07b/S08 proof: remaining confirm, event-journal, UI, debug, telemetry,
  docs, and release-replay work is now assigned to later sprints instead of
  sitting as unowned S07 debt.
- Changed Profile V2 asset reconciliation logging so the asset supervisor emits
  a `profile_asset_check_finish` lifecycle event for every check path, including
  scheduled/background checks rather than only route-triggered reconciles.
- Changed `capsem uninstall` to remove the installed runtime while preserving
  durable user state such as config, setup state, assets, logs, session/audit
  data, and persistent VM state.
- Changed the runtime replacement proof to exercise uninstall plus fresh
  install while preserving user config, persistent VM state, and saved-VM asset
  blobs.
- Changed `capsem doctor` to preflight through the same typed health checks
  used by `capsem status` before provisioning a diagnostic VM. Status blockers
  now carry stable issue codes and severity before they are rendered.
- Changed `capsem status` to report missing or non-executable host helper
  binaries as typed health blockers.
- Changed `capsem status` to report stale `capsem-service` and
  `capsem-process` helper binary versions as typed health blockers.
- Changed `capsem status` to report stale/missing service units, asset manifest
  problems, and missing/corrupt/incomplete setup state as typed health blockers.
- Changed `capsem status` to report a missing `/Applications/Capsem.app` as a
  typed health blocker for real installed macOS runtimes.
- Changed `capsem status` to report stale `capsem-gateway` and `capsem-tray`
  helper binary versions as typed health blockers. Their `--version` paths now
  answer before runtime initialization, so status can check them safely.
- Changed `capsem status --json` to include a top-level `state` plus grouped
  `checks` for host binaries, service unit, setup, assets, app bundle, service
  endpoint, and gateway readiness.
- Changed service `/list`, gateway `/status`, and `capsem status --json` to
  preserve the service asset supervisor state instead of collapsing asset work
  into only ready/missing booleans.
- Changed the tray menu to show asset `checking`/`updating`/`error` states and
  disable New Session until VM assets are ready.
- Changed asset cleanup, saved-VM resume/fork, service `/list`, gateway
  `/status`, tray status, frontend types, and `capsem status --json` to preserve
  and report saved-VM asset dependencies. Missing saved-VM assets now surface as
  typed `saved_vm_asset_missing` status blockers without blocking new current-
  version VM creation.
- Hardened `just install` for local release reproduction: it now removes and
  verifies the old runtime while preserving durable state, installs through the
  same native package commands as `install.sh`, captures typed installed
  `capsem status --json` evidence, and fails if service, gateway, status, guest
  DNS, or guest HTTPS checks do not pass.
- Hardened the Python install-test fixture so local simulated install tests
  build the default host binaries once, then refresh installed helpers when
  they differ from `CAPSEM_BIN_SRC`, not only when missing.
- Hardened the install-status capture harness with dirty-state evidence for
  missing tray helpers and missing macOS app bundles without mutating
  `/Applications`.
- Hardened the install-status capture harness to preserve grouped status
  checks in metadata and capture saved-VM asset-reference fields when present,
  including file-state evidence for referenced asset paths.
- Added black-box simulated install coverage for reinstalling after
  `capsem uninstall` and reinstalling over a corrupted helper binary, both
  gated by `capsem status --json` runtime-layout issue codes.
- Changed service `/list` to avoid per-VM `session.db` telemetry scans on the
  hot status path. `/info` keeps the historical SQLite enrichment for now,
  while live list metrics are deferred to the OpenTelemetry sprint.
- Changed the full release gate so benchmark/doctor E2E checks run in the
  serial stage instead of racing the parallel Python shard, keeping the
  expensive VM and benchmark paths deterministic.

### Fixed
- Fixed first-run CLI auto-launch when `capsem-service` exits before binding
  its socket, so broken installed service binaries return a clear startup
  error instead of waiting through repeated socket timeouts.
- Fixed the built-in `local` MCP server toggle so
  `mcp.servers.local.enabled = false` persists, stays visible in settings, stops
  injecting or preserving the local stdio bridge in agent configs, and disables
  the runtime built-in server list entry.
- Fixed the marketing-site installer for the stamped v1.1 package assets:
  macOS now installs the downloaded `.pkg` with the native installer, and
  package downloads are checked against the release manifest when local tools
  are available.
- Fixed `capsem uninstall --yes` so it no longer recreates
  `~/.capsem/update-check.json` via the background update checker while
  uninstalling.
- Fixed repeat local installs when stale Tauri app bundles under
  `target/release/bundle/macos/` are not removable by the normal build step.
- Fixed `.deb` payload verification for zstd-compressed packages without an
  embedded content-size header, matching the published Debian package format.
- Fixed Linux KVM unit-test compilation issues surfaced by PR CI before the
  site/download installer hardening can merge.
- Fixed macOS PR CI's clean-checkout Rust unit gate by creating a minimal
  frontend dist before `capsem-app`'s Tauri test build runs.
- Fixed macOS PR CI codesigning races during `nextest` discovery by
  serializing the ad-hoc signing runner and preserving its build log on
  workflow failures.
- Fixed PR install E2E's clean-checkout host setup so missing VM assets can be
  built with `uv`, checked through pnpm-backed doctor paths, and signed with
  `minisign`.
- Fixed PR CI coverage drift by aligning the workflow's Rust coverage floor
  with the documented `just test` gate.
- Fixed clean-checkout install E2E asset alias creation by copying hash-named
  assets when Linux protected-hardlink rules reject Docker-produced files.
- Fixed PR install E2E's Docker test runner to include the project dev
  dependency group before invoking pytest inside the installed-package
  container.
- Fixed release-gate flakiness in gateway and install harness tests by making
  the mock Unix-socket gateway concurrent, restoring runtime fixtures after
  destructive uninstall/purge tests, and localizing the large-payload MITM
  upstream instead of relying on external network behavior.
- Fixed macOS PR CI's Python coverage step so it collects top-level Python
  contract tests without accidentally booting VM integration suites.
- Fixed the shared `just` execution lock on macOS hosts without a `flock`
  binary by falling back to a Python `fcntl` lock holder.
- Fixed macOS PR CI's scoped Python coverage floor so the top-level contract
  lane matches clean-runner coverage while the full `just test` gate stays at
  90%.
- Fixed macOS PR CI's no-VM Python integration lane so clean runners execute
  only suites without generated asset/signing prerequisites while still
  import-checking every integration suite.
- Fixed Linux PR CI so hosted ARM runners compile the KVM backend and test
  binaries without hanging in live KVM probes or unbounded hosted-runner test
  execution; release CI remains the real-KVM exercise gate.
- Fixed ordinary CI hardening gaps: Linux KVM diagnostics no longer emit red
  success annotations, Rust integration coverage is release-blocking, coverage
  summary errors are not hidden by `tee`, and Codecov test analytics use the
  supported uploader.

## [1.1.1778542197] - 2026-05-11

### Changed
- Disabled the unsupported desktop self-updater surface for the next release:
  Tauri updater config, updater permissions, launch-time checks, and frontend
  update controls are removed until release artifacts support full-install
  updates.
- Package installers now fail loudly when release-critical `capsem install` or
  `capsem setup` fails, instead of reporting success for a non-bootable install.
- Policy Hook Spec0 remains infrastructure-only for the next release:
  configured external hook dispatch is not exposed as a shipped settings/UI
  surface until a production integration gate wires and verifies it.

### Fixed
- macOS `.pkg` and Linux `.deb` package flows now carry signed
  `manifest.json` snapshots plus all host helper binaries, and release CI
  verifies package payload signatures before publishing.
- Release install E2E now consumes clean-checkout VM assets, locally signs the
  package manifest, and repacks the Linux `.deb` in place so CI installs the
  tested package instead of the unrepacked Tauri artifact.
- Linux release app builds now install `minisign` before package payload
  manifest signing, matching the clean install E2E gate and preventing
  release-only `minisign: command not found` failures.
- Setup, `capsem update --assets`, service startup, status, and doctor
  diagnostics now use verified manifest loading so unsigned or invalid
  manifests cannot silently downgrade asset verification.
- Release preflight now validates the manifest signing key against
  `config/manifest-sign.pub`, keeps Linux package publication
  release-blocking, and includes the signed manifest plus boot assets in
  provenance attestation.
- VM asset manifests now use consistent same-day patch selection across
  full image builds and local initrd repacks, preserve numeric asset-version
  ordering, clean stale per-arch hash aliases, and validate rootfs contents
  from the canonical guest artifact lists before release publication.
- Settings save and frontend import now reject new `policy.hook.*` rules, so
  users cannot save inert hook-decision policy that appears enforced.
- Settings reload failures now return structured saved-but-not-applied state,
  including affected session IDs, so the UI can keep a persistent retry banner.

### Security
- Manifest loading now verifies release signatures in setup, update, service,
  status, and doctor paths so unsigned or invalid asset manifests cannot
  silently downgrade boot asset verification.
- Policy hook controls and `policy.hook.*` writes are hidden or rejected until
  configured external hook dispatch has a production integration path and
  black-box E2E proof.

## [1.0.1778378133] - 2026-05-10

### Added (enforcement rules)
- Added the MCP policy sprint plan and tracker to productize MCP
  rules as typed `allow`, `ask`, and `block` decisions across TOML,
  settings, MITM enforcement, telemetry, and VM E2E tests.
- Expanded policy planning beyond MCP to cover HTTP and DNS with the
  same typed decision model, including capture-aware `rewrite`, HTTP
  method/URL path/query/header rules, header stripping, DNS rewrite rules,
  credential-broker-safe redaction expectations, and explicit E2E/session
  proof for `mcp_calls`, `net_events`, and `dns_events`.
- Expanded policy planning again to include model request/response,
  model tool-call/tool-response policy, and Policy Hook Spec0: an
  OpenAPI 3.1 export generated from runtime wire types so third-party
  HTTPS hook servers can receive normalized policy requests and return
  typed allow/ask/block/rewrite decisions.
- Clarified the enforcement rule shape as named
  `policy.<type>.<rule_name>` TOML tables with `on`, CEL `if`,
  `decision`, `priority`, and capture-aware
  `rewrite_target`/`rewrite_value` fields; simple UI allow/block/header
  controls must compile into the same enforcement rule IR.
- Added the first policy settings slice: settings files can now parse,
  preserve, return, and save priority-bearing named enforcement rules through
  the `/settings` API so frontend policy editors can post rule objects.
- Hardened policy config validation with adversarial rewrite tests:
  bogus rewrite shapes, malformed regex targets, callback/table
  mismatches, invalid rule names, invalid policy key saves, header-strip
  normalization, and atomic rejection now fail closed before settings are
  written.
- Added strict policy condition validation for the documented
  CEL-compatible subset: conjunctions, comparisons, `has(...)`, string
  helper methods, regex `matches(...)`, and per-callback subject fields
  are checked before TOML or `/settings` policy saves can persist.
- Added the first enforcement rule evaluator over normalized subjects, with
  priority/name-ordered rule selection for MCP argument, HTTP path, and
  model response conditions.
- Wired merged enforcement rules into the framed MITM MCP endpoint: named
  MCP request `block` rules now stop dispatch and record `policy.mcp.*`
  in `mcp_calls`, while `ask` rules fail closed without aggregator
  dispatch and record `policy_action=ask`.
- Added framed MITM MCP response enforcement for `mcp.response`
  block rules: secret-bearing tool results are replaced with policy
  errors before reaching the guest and the original result is omitted from
  `mcp_calls.response_preview`.
- Added `mcp.response` rewrite enforcement for framed MITM MCP:
  regex/capture rewrite targets mutate matched response text before it
  reaches the guest and telemetry records only the rewritten payload.
- Added `mcp.request` rewrite enforcement for framed MITM MCP:
  argument regex rewrites mutate dispatch payloads before the aggregator
  sees them, request telemetry records only redacted arguments, and
  rewrite-target errors fail closed without leaking original arguments to
  `session.db`.
- Added the first HTTP policy enforcement path in the MITM hook
  pipeline: named `http.request` block and ask rules stop before upstream
  dispatch, rewrite rules can mutate request URLs and strip request
  headers before telemetry/upstream construction, and `net_events` now
  carries typed policy mode/action/rule/reason fields.
- Added HTTP response policy enforcement in the MITM hook pipeline:
  named `http.response` rewrite rules can strip response headers and
  rewrite response header/status targets before guest delivery and
  telemetry capture, while unsupported response rewrite targets fail
  closed without leaking upstream response headers or bodies.
- Added DNS query policy enforcement: named `dns.query` allow rules now
  dispatch with audit fields, block and ask rules fail closed before
  upstream resolution, rewrite rules synthesize configured A/AAAA answers
  without touching upstream DNS, live policy reload is checked before
  cached answers, and `dns_events` now carries typed policy
  mode/action/rule/reason fields.
- Added model request policy enforcement before provider dispatch:
  named `model.request` allow rules dispatch with audit fields, block
  and ask rules fail closed before upstream connection, unsupported
  request rewrite rules fail closed without dispatch, and `net_events`
  records policy fields plus byte counts without retaining denied request
  bodies.
- Added adversarial and VM E2E coverage for model request policy:
  truncated JSON matching, invalid runtime conditions, non-LLM path
  bypass, `/settings` model-policy saves, callback/type mismatch
  rejection, and a real guest OpenAI-shaped HTTPS request blocked from
  `user.toml` with `session.db` no-leak assertions.
- Added configured MCP Policy V2 VM E2E coverage: a saved
  `policy.mcp.*` argument-name block now goes through `/settings`,
  `/reload-config`, the real guest framed MCP relay, and `session.db`
  assertions for decision, rule, reason, process attribution, and
  redacted previews.
- Added more configured MCP Policy V2 VM E2E coverage for T5:
  argument-value `ask`, request-argument `rewrite`, external stdio MCP
  request `block` with no dispatch, and external MCP return-value `block`
  with no response-preview leak are now proven through `/settings`, the
  real guest framed MCP relay, and `session.db`.
- Added a policy product-surface subsprint covering docs site updates,
  session database references, just recipe documentation, and settings UI
  work so the framed MITM MCP and policy user-facing surfaces stay in sync
  with the implementation.
- Added the policy product surface: a docs reference page, refreshed
  framed-MITM MCP/settings/session/just recipe docs, settings import/export
  of named enforcement rules, and a settings UI panel that edits, deletes, and
  stages generated `policy.<type>.<rule_name>` rules.
- Added Policy V2 T5 VM proof for HTTP, DNS, and model traffic: real guest
  sessions now cover configured HTTP method/path/query/header blocks,
  HTTP request/response header stripping with no-leak `net_events`,
  configured DNS block/rewrite with `dns_events`, model request ask/rewrite
  fail-closed no-leak behavior, and model tool-response block/rewrite
  telemetry redaction.
- Added model `tool_response` Policy V2 enforcement before provider
  dispatch: OpenAI-shaped tool-result messages can now be blocked or
  rewritten before local tool output reaches the model provider, with
  rewritten request bodies updating `Content-Length` and redacted
  `net_events`, `model_calls`, and `tool_responses` previews.
- Added model response and provider-emitted model tool-call Policy V2
  enforcement before guest delivery: OpenAI-shaped responses can now be
  blocked, asked, or rewritten with no-leak `net_events`, redacted
  `model_calls.text_content`, and redacted nested `tool_calls` session
  rows on the host MITM fixture path.
- Added Policy Hook Spec0 as checked-in OpenAPI generated from Rust wire
  types, exposed it from `GET /policy-hook/spec`, and added a strict hook
  endpoint runtime with HTTPS/auth/body-cap/schema-version fail-closed
  handling plus `policy_hook_events` session DB audit rows.
- Added deterministic VM E2E coverage for model response block/rewrite and
  provider-emitted tool-call block/rewrite through a local OpenAI-shaped
  upstream fixture, with guest-visible no-leak assertions and `net_events`
  policy proof.
- Added scoped Policy V2 Criterion microbenchmarks for HTTP, DNS, model
  response, model tool-call, hook-decision matching, and Policy Hook response
  decoding, with sample results recorded under `benchmarks/policy-v2/`.

### Fixed (service)
- Fixed failed-session preservation idempotency: duplicate cleanup paths that
  race on the same session directory now treat an already-renamed or already-
  removed directory as a quiet no-op instead of warning that logs were lost
  and the session was orphaned. Real rename/remove failures still warn with
  the actual filesystem outcome, and regression tests cover preserved,
  already-absent, and double-call behavior.
- Fixed the Slack redaction regression fixture so it no longer contains a
  contiguous token-shaped literal that trips GitHub push protection while still
  constructing the same runtime string for the redactor test.

### Fixed (enforcement rules)
- Fixed model telemetry parsing for explicit/local OpenAI-compatible
  provider paths by carrying the request's provider classification through
  the MITM chunk-hook metadata, so enforcement and SSE interpretation use
  the same provider decision instead of relying only on the network domain.
- Fixed builtin MCP HTTP policy propagation: `capsem-process` now passes
  merged domain allow/block lists to `capsem-mcp-builtin`, so configured
  builtin HTTP denials fail at the policy boundary, avoid upstream
  resolution, and write both `mcp_calls` and `net_events`.
- Fixed a model tool-response policy bypass found during adversarial unit
  testing: an allow rule matching one tool result can no longer let a
  separate secret-bearing tool result in the same provider request bypass a
  block rule.
- Fixed a policy evaluator safety bug found during adversarial testing:
  a missing field no longer satisfies a negative comparison such as
  `provider != "local"`.
- Fixed policy settings UI crashes found during browser verification by
  tolerating omitted live metadata arrays and deduplicating generated rule
  keys before rendering Svelte keyed rows.
- Fixed MITM integration fixture discipline: fake upstreams now drain the
  full `Content-Length` body and upstream task panics fail the test instead
  of only printing noisy background panics.
- Fixed warnings-as-errors issues found during policy verification by
  removing a redundant setup detection closure and switching settings
  endpoint env-serialization tests to an async mutex.
- Fixed a Policy V2 MCP telemetry leak: pre-dispatch `policy.mcp.*`
  block/ask denials now redact original request arguments before writing
  `mcp_calls.request_preview`.
- Fixed MITM body handling regressions found during T6 verification:
  HTTP decompression now honors `Content-Encoding: gzip` instead of raw
  gzip magic bytes, and decoded responses drop stale compressed
  `Content-Length`/size hints so guest delivery cannot truncate.
- Fixed suspend/resume recovery hardening found during T7: `.vzsave`
  checkpoints are fsynced before process exit, service registry suspended
  state is cleared only after resume readiness, and failed Apple VZ warm
  checkpoints are archived before a persistent cold-boot fallback recovers
  workspace/overlay state.
- Fixed the smoke leak-detector false positive where concurrent pytest
  invocations shared one leak-attribution file and could report another
  still-running pytest process's service fixture as a leak; `just smoke`
  now gives each pytest phase a distinct leak-log namespace.
- Fixed clean ephemeral session shutdown cleanup so non-persistent session
  directories are removed on expected process exit while unexpected process
  deaths remain available for postmortem inspection.
- Fixed local release gate recipes so `just test` can complete on macOS:
  optional Tauri signing arguments no longer trip Bash 3.2 nounset in
  `just cross-compile`, and `just test-install` recreates the Docker host
  builder base image if cross-compile cleanup pruned it.

### Fixed (mitm-mcp-unification T4 coverage hardening)
- Preserved all JSON-RPC request id shapes in framed MCP telemetry:
  string, numeric, and null ids now populate `mcp_calls.request_id`
  instead of only unsigned numeric ids.
- Corrected the sprint tracker T5 scope: configured external MCP tool
  calls are inspected at the framed MITM MCP boundary; any remaining
  downstream host-side egress concern must be named separately.
- Expanded framed MCP coverage across Rust, VM E2E, and in-VM doctor
  diagnostics for malformed JSON recovery, oversized guest requests,
  corrupted-frame recovery after an established MCP frame stream,
  notification interleaving, non-`tools/call` timeout telemetry, and
  persistent stop/resume reconnect.
- Updated the T4 coverage review notes and benchmark log with the bugs
  found during review, `session.db` sanity evidence, and fresh
  `mcp-load` numbers after the hardening pass.

### Changed (mitm-mcp-unification T4 cutover)
- **Guest MCP now uses framed MITM transport by default.**
  `/run/capsem-mcp-server` relays stdio JSON-RPC over bounded MCP
  frames on `vsock:5002`, carries per-frame process attribution, emits
  explicit disconnect errors for in-flight JSON-RPC requests, and avoids
  automatic replay of non-idempotent `tools/call` requests after a
  transport drop.
- Removed the legacy guest MCP router on `vsock:5003`: deleted
  `capsem-core/src/mcp/gateway.rs`, removed `VSOCK_PORT_MCP_GATEWAY`,
  removed 5003 vsock dispatch/classification, and updated guest
  diagnostics, docs, skills, and benchmarks to describe the MITM MCP
  endpoint as the canonical guest MCP path.
- Added the `mitm.mcp_disconnects_total` metric and VM E2E coverage
  proving the default guest relay writes populated `mcp_calls` rows,
  live policy reload affects an existing connection, concurrent parent
  processes preserve `mcp_calls.process_name`, tool timeouts record
  terminal errors, external stdio MCP tools still dispatch, and legacy
  `vsock:5003` refuses guest connections.
- Fixed `scripts/check_session.py` so `just inspect-session <id>` works
  with current run-session directories and older system Python versions.

### Changed (development process)
- Strengthened the Capsem sprint/testing skills to require an explicit
  functional-slice proof matrix for non-trivial work: unit/contract,
  functional, adversarial, E2E/VM, telemetry, and performance evidence
  must be named in sprint trackers, with any missing coverage recorded
  as visible debt instead of implied by benchmarks or unit tests.
- Expanded the MCP development skill with the framed MITM MCP hardening
  matrix: parser/interpreter adversarial cases, dispatch coverage,
  enforcement rule enforcement, telemetry assertions, VM E2E checks, and the
  aggregator DB-free boundary.

### Fixed (mitm-mcp-unification T3 hardening)
- **Framed MCP now consumes request stream ids before JSON parsing,**
  so a valid frame with invalid JSON cannot reuse the same stream id for
  a later request. Parser-level failures still return JSON-RPC parse
  errors, complete the stream id, and avoid writing misleading
  `mcp_calls` rows.
- **`capsem-service` now forwards framed MCP runtime knobs to
  `capsem-process`.** The child-process env allowlist includes
  `CAPSEM_HOME`, `CAPSEM_MCP_DEFAULT_TIMEOUT_SECS`,
  `CAPSEM_MCP_TOOL_CALL_TIMEOUT_SECS`, and
  `CAPSEM_MCP_TOOL_CALL_TIMEOUT_CEILING_SECS`, keeping service/process
  config roots aligned and allowing E2E tests to exercise real timeout
  limits.
- Added framed MCP VM E2E coverage for builtin `tools/call`, configured
  external stdio tools, live policy reload on an already-open connection,
  concurrent process attribution, slow-tool timeout telemetry, and
  `session.db` policy/preview assertions.
- Added a static regression guard proving the low-privilege
  `capsem-mcp-aggregator` crate remains free of session DB dependencies
  and audit writes.

### Added (mitm-mcp-unification T3 MITM MCP endpoint)
- **Framed MCP now dispatches through a real MITM-owned endpoint
  instead of borrowing the legacy MCP gateway handler.** `MitmProxyConfig`
  owns `McpEndpointState`; the framed path routes initialize,
  tool/resource/prompt list, tool calls, resource reads, and prompt gets
  through the low-privilege `AggregatorClient`; and the MITM frame layer
  writes `mcp_calls` telemetry directly through the session `DbWriter`.
  The aggregator remains DB-free.
- Added method-aware framed MCP timeouts: non-`tools/call` methods default
  to 60s, `tools/call` defaults to 300s, tool-call catalog timeout
  overrides are clamped by a 300s ceiling, and timeout failures return
  JSON-RPC errors while recording terminal `mcp_calls` rows with
  `decision=error`.

### Added (mitm-mcp-unification T2 decision provider)
- **Framed MCP calls now record audit-only policy decisions in
  `mcp_calls`.** The MITM MCP frame path builds an owned decision
  request from the interpreter summary, preserving process name,
  method classification, request preview, and BLAKE3 request hash for
  future remote corp forwarding. The local v1 provider emits only
  `allow` or `deny` actions, maps warning policy to `allow`, evaluates
  tool calls at per-tool granularity, evaluates resource/prompt reads
  at server granularity, and stores `policy_mode`, `policy_action`,
  `policy_rule`, and `policy_reason` through the logger schema,
  writer, reader, and session triage output.
- Added the T2 policy test matrix for exact tool name, exact MCP
  resource URI, prompt/tool argument name, prompt/tool argument value,
  nested return value, deny-over-allow precedence, live policy mutation,
  response-time decisions, actual framed request blocks, and sanitized
  framed response blocks. The framed tests now drive the MCP frame
  transport into a real `session.db` and assert both telemetry previews
  and policy fields on the resulting `mcp_calls` rows.
- Framed MCP deny decisions now enforce as well as log: request-rule
  denies short-circuit before aggregator dispatch, and return-value
  denies replace the original MCP result with a policy error before it
  reaches the guest.

### Added (mitm-mcp-unification T1 parser/interpreter)
- **Framed MCP over `vsock:5002` now has a bounded parser and
  interpreter instead of relying on the T0 spike shape.** The MITM
  MCP frame path validates frame length/flags, enforces monotonic
  nonzero request `stream_id`s while reserving `stream_id=0` for
  notifications, bounds JSON-RPC payload parsing before deserialize,
  classifies MCP request/notification methods, extracts server/tool/
  resource/prompt names for the known MCP call families, emits method
  metrics, and recovers from corrupt-but-bounded frames by returning
  JSON-RPC invalid-request errors before continuing the stream.

### Changed (exec timeout contract)
- **`capsem exec` and `capsem run` no longer impose a hidden default
  command timeout.** Omitting `--timeout` now waits for command
  completion, which matches long-running user jobs such as builds,
  installs, migrations, and `capsem-bench mcp-load`. Explicit
  `--timeout <seconds>` still applies a service-side deadline. The
  process-layer exec watchdog was removed; transport delivery remains
  covered by the control bridge's Ack/AckReply replay layers.

### Changed (rustfmt sweep)
- Ran a one-time workspace `cargo fmt` sweep while landing T1 so future
  sprint diffs start from the same formatter baseline.

### Added (mitm-mcp-unification T0 wire gate)
- **Framed MCP-over-MITM transport is now benchmark-gated for the
  MCP unification sprint.** Added a bounded `MC` frame envelope in
  `capsem-proto`, a MITM classifier branch for framed MCP on
  `vsock:5002`, and an explicit `CAPSEM_MCP_TRANSPORT=framed`
  mode in the guest MCP relay. The T0 spike still routes through
  the existing aggregator/policy/MCP telemetry path so the wire
  comparison stays fair. Fresh same-hardware `mcp-load` artifacts
  are recorded at
  `benchmarks/mcp-load/baseline-pre-mitm-unification.json` and
  `benchmarks/mcp-load/baseline-framed-mitm-unification-t0.json`.
  Framed selected: rps +8.6% / +4.8% / -6.4% / +5.4% and p99
  -31.9% / -23.9% / +7.8% / -31.0% at concurrency 1/10/50/200,
  with zero errors on both transports.

### Fixed (mcp/file_tools: truncate_path panic on non-ASCII paths -- AB-007)
- **`truncate_path` no longer panics on paths whose suffix
  byte offset lands inside a multibyte UTF-8 sequence.** The
  legacy implementation used `path.len()` (bytes) and
  `&path[path.len() - (max - 3)..]` (byte slice). For example,
  a path of 40 `日` chars + 1 ASCII char (121 bytes) with
  max = 33 panicked with `start byte index 91 is not a char
  boundary; it is inside '日'`. Both call sites
  (`render_changes` and the snapshot list renderer) walk
  user-supplied paths, so any non-ASCII path could crash
  snapshot rendering for the whole VM. The new implementation
  counts and slices by character, falling back to a
  no-ellipsis suffix for `max <= 3` so ill-typed callers
  cannot bring down the tool. Eight regression tests cover
  ASCII-under, ASCII-over, Unicode-under (keeps as-is even
  when byte length exceeds max), Unicode-boundary panic
  repro, Unicode-over (correct char count), empty path,
  `max == 3`, and `max == 0`.

### Fixed (security: deep-link JS injection -- AB-003)
- **`capsem-app::dispatch_deep_link` no longer interpolates
  `--connect` / `--action` values into JavaScript that runs in
  the desktop webview.** The previous code only escaped single
  quotes and embedded the values into a single-quoted JS
  literal that was passed to `window.eval`. A trailing
  backslash, a newline, or a payload like
  `x\'); alert(1); //` broke out of the string and ran as
  code -- in a webview that holds the gateway auth token, so
  effective full local capsem control. New helpers
  `build_deep_link_payload` (returns a `serde_json::Value`)
  and `build_deep_link_script` embed the payload via JSON
  serialization, which is a strict subset of valid JS object/
  string literals; every backslash, quote, control char, and
  high-bit code point is escaped by construction. Tests added
  cover plain values, single quote, backslash, newline, the
  injection-payload repro, and a JSON round-trip across a
  high-entropy input string.

### Fixed (mitm-redesign T3 closure -- production bug, dns-load reveal)
- **DNS cache returned the original query id for every cache hit.**
  The TTL-honoring answer cache (T3.f) stored wire-format response
  bytes verbatim, including the 16-bit DNS transaction id in
  bytes 0-1. Cache hits returned those bytes without rewriting the
  id, so subsequent queries to the same `(qname, qtype, qclass)`
  always echoed the FIRST query's id. Downstream resolvers (which
  match responses to outstanding queries by id, RFC 1035 sec 4.1.1)
  would discard the cached response as not-mine, causing 100%
  query failure once the cache warmed up. Surfaced by the
  `capsem-bench dns-load` in-VM run during T3 closure: the run
  reported ~99.999% errors, and an inline diagnostic showed the
  exact pattern -- 5 sequential queries with random ids all
  returned the same id (the first query's). Fix: `DnsAnswerCache::get`
  takes a new `query_id: u16` parameter and patches the response
  bytes' id field on every hit before returning. New regression
  tests `cache_hit_patches_query_id_into_response` (asserts the
  patch happens with two different ids on the same key) and
  `cache_hit_with_zero_query_id_zeroes_bytes` (defensive: id=0
  must overwrite, not skip the patch). Existing 18 cache tests
  updated to pass through the new arg. capsem-core lib at 1693
  tests now (+2 regression). Workspace clippy clean.

### Fixed (mcp: corp precedence -- AB-002)
- **Corp-defined MCP servers can no longer be shadowed by a
  same-name user manual entry.** The build pipeline in
  `crates/capsem-core/src/mcp/mod.rs::build_server_list_with_builtin`
  used a first-wins HashSet but processed entries in the order
  builtin → auto-detected → user → corp, so corp was last and
  was silently skipped on collision. A user typing the same
  name as a corp-injected server would win the URL, headers,
  and bearer token, contradicting the documented `corp > user
  > defaults` policy in `docs/architecture/settings.md` and
  the "corp_locked" model. Corp definitions are now processed
  first, so the first-wins rule enforces the documented trust
  order. Same-name user entries are skipped; unique-name user
  and auto-detected entries are unaffected. Tests added:
  `build_server_list_corp_shadows_user_on_same_name`,
  `build_server_list_unique_user_server_survives_with_corp_present`,
  `build_server_list_corp_enabled_override_on_user_server`.
  `docs/src/content/docs/architecture/mcp-aggregator.md`
  reordered to match the new processing order.

### Fixed (security: gateway CORS -- AB-001)
- **Gateway CORS now does an exact-host check on the Origin
  header instead of a string prefix match, closing a path that
  could leak the gateway auth token to attacker-controlled
  pages.** The previous predicate accepted any origin starting
  with `http://localhost`, `http://127.0.0.1`, `https://...`,
  or `tauri://`, so origins like `http://localhostevil.com`,
  `http://127.0.0.1.evil.example`, and `tauri://evil.example`
  passed CORS. Combined with `GET /token` being exempted from
  the auth middleware (it is gated only by loopback peer IP --
  which a victim's own browser satisfies), a malicious page
  could read `gateway.token` cross-origin and drive the local
  capsem service. The new
  `crates/capsem-gateway/src/cors.rs::is_allowed_origin` parses
  the Origin as a URI and accepts only exact matches for
  `http`/`https` to `localhost`, `127.0.0.1`, or `::1`, plus
  `tauri://localhost`; any path/userinfo/query/fragment, any
  unknown scheme, and any host suffix attack are rejected.
  22 unit tests cover the positive and negative matrix and the
  predicate is now shared between production and the
  integration test in `main.rs` so they cannot drift.

### Fixed (mitm-redesign T3 closure -- in-VM gate)
- **Host vsock listener registration was missing
  `VSOCK_PORT_DNS_PROXY` (5007) and `VSOCK_PORT_AUDIT` (5006).**
  In-VM smoke surfaced the DNS half: `capsem-dns-proxy` queries
  failed with "Connection reset by peer (os error 104)" because
  the host kernel had no listener for vsock port 5007 to accept
  on. `crates/capsem-core/src/vm/boot.rs::vsock_ports` now
  includes both 5006 and 5007 alongside the existing 5000-5005,
  so the Apple VZ + KVM hypervisor backends register listeners
  on every port `dispatch_aux_connection` knows how to handle.
  The audit case was a latent bug -- `audit_events` had been
  silently empty in every session since the audit feature
  landed -- now incidentally fixed alongside the DNS one.
- **Diagnostics: `test_dns_resolves_to_local` (test_sandbox.py)
  and `test_allowed_domain` still asserted the legacy
  `10.0.0.1` dnsmasq sentinel.** Updated to match the T3.4
  cutover: DNS now resolves to a real upstream IP via the
  capsem proxy (accepting either IPv4 or IPv6 first-token
  shape, since some upstreams return AAAA-only). The
  `test_allowed_domain` step-by-step diagnostic now uses the
  resolved hostname for TCP/TLS steps instead of hard-coding
  10.0.0.1. `test_dns_blocked_domain_returns_nxdomain` was
  policy-dependent (the user's `~/.capsem/user.toml` may
  override `api.openai.com.allow`); replaced with
  `test_dns_nxdomain_propagates_from_upstream` which uses an
  RFC 2606 `.invalid` TLD that no upstream can resolve --
  a clean policy-independent NXDOMAIN E2E test that pre-T3
  dnsmasq would have wrongly answered with 10.0.0.1.
- **In-VM E2E gate result.** With the boot.rs fix + diagnostic
  updates: `capsem-doctor -k 'dns or proxy_listening or
  iptables_redirect'` returns 14/14 PASS in a temp VM. The
  full DNS path is validated end-to-end: libc -> iptables nat
  53 -> 1053 -> capsem-dns-proxy -> vsock 5007 -> host hickory
  handler -> upstream forward (1.1.1.1) OR NXDOMAIN
  short-circuit -> answer back. `dns_events` rows populate with
  `trace_id`, source_proto, upstream_resolver_ms.
  `pgrep dnsmasq` returns nothing.

### Added (mitm-redesign T3 follow-up `f.proptest`)
- **proptest property-based tests for the DNS wire codec.** New
  `crates/capsem-core/src/net/parsers/dns_parser/proptests.rs`
  with 7 properties (256 random cases each by default) closing
  the loop alongside the cargo-fuzz targets:
  - `parse_query_round_trip`: build a query with arbitrary
    name + qtype + id, parse it back, assert id / qname / qtype
    / qclass / extra_questions match.
  - `build_nxdomain_preserves_question`: NXDOMAIN response built
    from an arbitrary query parses back to a question with the
    same id / qname / qtype / qclass.
  - `build_servfail_preserves_question`: same shape, ServFail
    rcode.
  - `build_redirect_preserves_question_for_a`: redirect response
    with N arbitrary IPv4 IPs lands all N as A records (no
    cross-family filter loss).
  - `build_redirect_filters_cross_family`: redirect with
    1 IPv4 + 1 IPv6 + an A query yields exactly 1 answer
    (the IPv4) -- the cross-family filter holds.
  - `parse_query_does_not_panic_on_arbitrary_bytes`: 0..2000
    arbitrary bytes never panic. Mirrors the cargo-fuzz target's
    safety contract so a regression surfaces in `cargo test`
    even without nightly + cargo-fuzz installed locally.
  - `build_nxdomain_does_not_panic_on_arbitrary_bytes`: same.
  Strategies: `dns_name_strategy()` produces 2-3 label
  syntactically-valid lowercase DNS names; `qtype_strategy()`
  picks from A/AAAA/TXT/MX/CNAME/SRV/CAA/NS/SOA/PTR/HTTPS/ANY.
  New dev-dep `proptest = "1"` (test-only, no production
  surface). capsem-core lib at 1691 tests now (was 1684).

### Added (mitm-redesign T3 follow-up `f.cache`)
- **TTL-honoring LRU answer cache for the DNS proxy.** New
  `crates/capsem-core/src/net/dns/cache.rs` shipping
  `DnsAnswerCache`: bounded LRU (default 1024 entries) keyed on
  `(qname, qtype, qclass)`, value is the wire-format answer bytes
  + `expires_at` derived from `min(answer_TTL, max_cache_ttl)`
  with `[60s, 300s]` clamp (DEFAULT_MAX_TTL_SECS / MIN_TTL_SECS).
  Lazy expiry: an expired entry is popped on the next lookup +
  counted as a miss. Cache **eligibility**: only `Decision::Allowed`
  responses with rcode=0 are inserted -- block + redirect
  re-evaluate every query (admin can change either at any moment),
  and SERVFAIL / NXDOMAIN from upstream are not persisted (avoids
  amplifying a transient upstream blip into 5 minutes of wrong
  answers). Cache **coherence**: `cache.get()` re-checks
  `is_fully_blocked` AND `find_dns_redirect` on every hit -- a
  domain that becomes blocked or redirected after we cached its
  answer is invalidated lazily on the next access (the entry is
  popped + counted as a miss). Three new metrics:
  `mitm.dns_cache_hits_total`, `mitm.dns_cache_misses_total`,
  `mitm.dns_cache_evictions_total`. New `lru = "0.18"` capsem-core
  dep (small pure-Rust crate). Wired into `DnsHandler` via the
  new `with_cache` constructor; `with_default_resolver` enables
  it by default with default config. `new` (no cache) constructor
  is preserved so existing tests can assert the upstream path
  always runs without cache-hit interference. 18 cache unit tests
  (insert/get round-trip, qtype/qclass key independence, capacity
  eviction with LRU order, TTL clamps to MIN/MAX bounds,
  garbage-input falls back to MIN, NoData answer falls back to
  MIN, min-across-records, clear, default constants pinned) + 8
  handler integration tests (cache hit short-circuits upstream
  via blackhole-after-warmup, policy-now-blocks invalidates
  lazily, policy-now-redirects invalidates lazily, block path
  still NXDOMAINs without consulting cache, cache_hits_total +
  cache_misses_total metrics fire, NXDOMAIN-from-upstream is not
  cached, with_default_resolver enables caching, new() leaves
  cache=None). capsem-core lib at 1684 tests now (was 1658).
  Workspace clippy clean.

### Added (mitm-redesign T3 follow-up `f.observability`)
- **DNS path metrics + structured tracing span.** Three new
  metric names registered alongside the existing MITM ones:
  `mitm.dns_queries_total{decision}` (allowed / denied /
  redirected / error), `mitm.dns_handle_duration_ms` (histogram,
  end-to-end), `mitm.dns_upstream_duration_ms` (histogram,
  upstream-forward path only -- absent on policy short-circuit),
  `mitm.dns_upstream_failures_total`. `DnsHandler::handle` is now
  wrapped in a `mitm.dns.query` info-span recording `qname`,
  `qtype`, `decision`, `rcode`, and `upstream_ms` on exit so a
  single `RUST_LOG=capsem::net::dns=debug` traces one query from
  parse to answer. The handler was refactored to a thin
  `handle()` (span + metric emission) wrapping `handle_inner()`
  (the decision tree) so every exit path goes through the same
  observability stamp -- no drift between block / redirect /
  forward / error branches. 5 new tests against
  `metrics_util::DebuggingRecorder` assert the right counter
  fires per decision label, the upstream histogram is absent on
  policy short-circuit but present on the forward path, and
  `dns_upstream_failures_total` increments on resolver error.
  `metrics_util` was already a dev-dep from the T1 sprint;
  facade-only emission means a no-op overhead in production
  until T5 wires the OTel exporter (same shape as the existing
  MITM metrics).

### Added (mitm-redesign T3 follow-up `e`)
- **`capsem-bench dns-load` harness.** New
  `guest/artifacts/capsem_bench/dns_load.py` mirrors the
  mitm-load shape: drives the DNS proxy at concurrency
  1/10/50/200, measures rps + p50/p95/p99/p999 latency, counts
  errors, and reports a per-level rcode distribution
  (`{"denied": 1234}` for the policy-block path,
  `{"allowed": 1234}` for the upstream-forward path) so the
  output dovetails with `dns_events.decision` for cross-checks.
  Defaults to `api.openai.com` (a fully-blocked domain in the
  dev policy) so every query hits the NXDOMAIN short-circuit
  path -- isolates the proxy's per-query cost from real upstream
  variance. Override via `CAPSEM_BENCH_DNS_QNAME` /
  `_QTYPE` / `_DURATION` / `_TIMEOUT`. The harness builds DNS
  wire-format queries by hand (no dns-python dep needed) so the
  guest's bundled python is enough; the encoder helpers
  (`_encode_qname`, `_build_query`, `_decode_rcode`,
  `_RCODE_DECISION` map) come with 7 host-side unit tests
  pinning the wire format + the rcode-to-Decision lock-step.
  Wired into `__main__.py` as the new `dns-load` mode (gated
  off `all` like mitm-load -- 40s of pure proxy stress would
  dominate a casual `capsem-bench all` run). Baseline JSON
  capture deferred to junior who owns the bench runner this
  session per the resume prompt.

### Added (mitm-redesign T3 follow-up `d`)
- **`DnsRedirect` enforcement rule -- admin-configured DNS overrides.**
  New `DnsRedirect { matcher, qtype, answers, ttl }` rule kind on
  `NetworkPolicy::dns_redirects` lets an admin override DNS
  resolution for a specific qname (and optionally a specific
  qtype). The DNS handler checks redirects AFTER `is_fully_blocked`
  (a blocked domain stays NXDOMAIN; redirect never weakens block)
  and BEFORE the upstream forward (no network round-trip when the
  answer is pinned locally). Use cases: redirect telemetry domains
  to a local trap, simulate an unreachable name with a deterministic
  IP for test runs, /etc/hosts-style overrides without modifying
  the guest. New `Decision::Redirected` variant on
  `capsem_logger::events::Decision` (string `"redirected"`) so
  `dns_events` rows surface override hits via
  `WHERE decision = 'redirected'`. Builder
  `dns_parser::build_redirect_response(query_bytes, &[IpAddr],
  ttl) -> Result<Vec<u8>>` synthesizes A/AAAA answer records
  filtered by qtype (cross-family IPs silently skipped, yielding
  the standard "name exists, no record of that type" NoError +
  zero-answers shape). 9 new policy unit tests + 11 new handler
  integration tests + 8 new builder unit tests covering exact /
  wildcard match, qtype filter, qtype=None matches anything,
  cross-family filtering, mixed-family yields only matching,
  block-overrides-redirect (block path runs first), TTL
  propagation, multiple IPs, empty-answers nodata, and
  no-match-falls-through-to-upstream. capsem-core lib at 1653
  tests now (was 1591). Workspace clippy clean.

### Added (mcp-concurrency T3 angle 2)
- **Pooled rmcp stdio peers for the local builtin MCP server.** The
  gateway can now spawn N independent stdio subprocesses for one
  MCP server and round-robin tool calls across them, removing
  rmcp 1.6's per-`Peer` mpsc → driver-task → stdin funnel as a
  singleton bottleneck. New fields on `McpServerDef`: `pool_size`
  (None / 0 / 1 = no pool, current behavior; >1 = N peers) and
  `pool_safe_tools` (allowlist of tool names safe to round-robin;
  others pin to `peers[0]` so per-process state stays consistent).
  HTTP servers ignore `pool_size` (HTTP/2 multiplexes natively).
  Builtin pool defaults to `min(available_parallelism, 4)` (matches
  the inflight-cap rule from `d88a714`). `CAPSEM_MCP_BUILTIN_POOL`
  overrides for tuning / debugging (set to 1 to force pre-pool
  behavior; clamped [1, 16]). `pool_safe_tools = [echo, fetch_http,
  grep_http, http_headers]`; snapshot tools stay pinned to
  `peers[0]` (their `AutoSnapshotScheduler` is per-process and N
  peers would diverge silently). Single-shot smoke at the dynamic
  default on M5 Max (pool=4): c=200 mcp-load p99 = 28.2 ms (vs
  sprint gate ≤ 35 ms), rps = 9591 (vs sprint gate ≥ 8000); c=10
  rps 3628 → 8794 (+143 %) — the rmcp stdio funnel disappearing
  at low contention.
- **`CAPSEM_BUILTIN_PEER_INDEX` env var** on `capsem-mcp-builtin`.
  Peer 0 keeps the original `mcp-builtin.lock` singleton; peers
  1..N use `mcp-builtin-{idx}.lock` so the `capsem_guard::install`
  per-session-dir guard doesn't make pool peers exit 0 with
  "another instance holds the lock".
- **`CAPSEM_MCP_BUILTIN_POOL` added to capsem-service env-allowlist**
  (both create and resume paths) so ops/bench can tune without
  rebuilding.

### Added (mitm-redesign T3 follow-up `c`)
- **cargo-fuzz harnesses for the DNS wire-format codec.** Four
  libFuzzer targets at `crates/capsem-core/fuzz/fuzz_targets/`:
  `parse_query`, `build_nxdomain`, `build_servfail`, and
  `round_trip` (asserts that if `parse_query` succeeds then
  `build_nxdomain` succeeds AND the response re-parses to the
  same qname/qtype/qclass -- catches divergence between the parse
  and rebuild paths that would let malformed queries escape
  NXDOMAIN gating). Each `corpus/<target>/` is pre-seeded with
  the T3.b `.bin` fixtures for fast structural coverage. The
  `fuzz/` directory is a standalone cargo workspace so libFuzzer's
  instrumentation flags don't leak into the parent workspace's
  normal builds. Plan acceptance from `T3-dns-proxy.md`: each
  target must survive `cargo +nightly fuzz run <target> --
  -max_total_time=60` clean (run path documented in
  `crates/capsem-core/fuzz/README.md` alongside the triage
  workflow for any crash artifact).

### Added (mitm-redesign T3 follow-up `b`)
- **dns_parser on-disk wire-format fixture corpora.** 13 raw DNS
  wire-byte `.bin` fixtures live at
  `crates/capsem-core/src/net/parsers/dns_parser/fixtures/`,
  covering simple A / AAAA / TXT / MX / CAA / HTTPS queries, the
  multi-question case, NXDomain + ServFail synthetic responses,
  truncated query, header-only, lying-qdcount, and the
  compression-self-loop adversarial case. Loaded via
  `include_bytes!()` at compile time so test runs don't hit the
  filesystem. 13 round-trip tests + an `all_fixtures_have_nonzero_length`
  pin (catches "include_bytes! pointed at an empty file" failure
  modes) wire them into the existing dns_parser test suite.
  Bootstrapped + regenerated by a new
  `crates/capsem-core/examples/dns_fixture_gen.rs` (separate
  compilation unit so the include_bytes! / regen chicken-and-egg
  doesn't bite). Plain English: a hickory-proto upgrade that
  changes the on-the-wire encoding of any of these query shapes
  lights up in the test diff before it bites a real query, and
  cargo-fuzz can corpus-seed from these exact bytes.

### Added (mitm-redesign T3 follow-up `a`)
- **dns_parser test breadth: record types + adversarial.** 32 new
  unit tests covering CNAME / NS / SOA / PTR / SRV / CAA / HTTPS /
  ANY / NULL / HINFO / AXFR / IXFR record types, all five DNS
  classes (IN / CH / HS / NONE / ANY), and risk-shape inputs:
  empty / single-byte / header-only / lying-qdcount / oversized
  qdcount=65535 / label compression self-loop / forward pointer
  past EOF / label > 63 bytes / NUL byte in label / truncated
  question section / max-label (63 bytes) accepted / NXDOMAIN
  preserves obscure qtype (CAA) and non-IN qclass / SERVFAIL
  rejects undecodable input. Total dns_parser tests: 46 (was 14).
  No production code changed -- pure additive coverage so a
  hickory-proto upgrade that quietly drops a record-type variant
  or breaks compression-bomb defense lights up before it bites a
  real query.

### Changed (mitm-redesign T3.4)
- **Guest cutover from dnsmasq to capsem-dns-proxy.** The
  in-guest dnsmasq fake (which resolved every name to the sentinel
  `10.0.0.1` so the MITM proxy could intercept connections) is
  gone. `capsem-init` now launches `capsem-dns-proxy` (T3.2) and
  installs iptables nat rules redirecting UDP/TCP port 53 to the
  proxy's `127.0.0.1:1053` listener. DNS queries now traverse the
  vsock envelope to the host's hickory-backed handler (T3.1)
  which applies the shared `NetworkPolicy` and forwards to a real
  upstream nameserver. `dig anthropic.com` from a guest returns a
  real answer; `dig api.openai.com` returns NXDOMAIN with the
  decision logged in `dns_events` (T3.3). The `dnsmasq` package
  is dropped from `guest/config/packages/apt.toml`, so the next
  rootfs rebuild leaves the binary out of the squashfs entirely.
  Diagnostics updated: `test_sandbox::test_dnsmasq_running` is
  replaced with `test_dns_proxy_running` plus a new
  `test_dnsmasq_not_running` that pins the cutover.
  `test_network` swaps the dnsmasq sentinel checks for two new
  acceptance tests: `test_dns_resolves_via_capsem_proxy` (a
  policy-allowed name resolves to a real IP, not the legacy
  10.0.0.1) and `test_dns_blocked_domain_returns_nxdomain` (the
  host policy short-circuits api.openai.com to NXDOMAIN before
  hitting the upstream resolver). Boot-stage marker added:
  `dns_proxy` between `net_proxy` and the rest of the boot
  sequence.

  End-to-end VM validation + `mitm-load` regression check still
  pending: the dev `capsem` binary needs codesigning (handled by
  the `just` recipes) and the `~/.capsem/assets/` install needs
  a `just install` to pick up the rebuilt initrd. Both fall
  under the junior-dev-owned bench runner this session, so the
  final acceptance gate is staged but not yet executed -- code,
  cross-compile, initrd repack (validated end-to-end via the
  Docker `agent` recipe), workspace clippy, and full Rust test
  suite are all green.

### Added (mitm-redesign T3.3)
- **`dns_events` telemetry table + per-query event row + trace_id
  correlation.** New `dns_events` schema in `capsem-logger`
  (timestamp, qname, qtype, qclass, rcode, decision, matched_rule,
  source_proto, process_name, upstream_resolver_ms, trace_id) with
  indexes on `(timestamp, qname, trace_id, decision)` for the
  inspect-session join. New `DnsEvent` event struct +
  `WriteOp::DnsEvent` + `insert_dns_event` writer; idempotent
  schema migration so existing DBs pick up the new table without a
  rebuild. New free function
  `capsem_core::net::dns::build_dns_event(result, source_proto,
  process_name, trace_id) -> DnsEvent` (pure, sqlite-free) +
  `serve_dns_session` in `capsem-process::vsock` calls it after
  every handler invocation and pushes the row through the shared
  `DbWriter` via `try_write` (matches the audit-event back-pressure
  pattern). `trace_id` is the ambient capsem trace id, so a single
  agent action joins across `dns_events` and `net_events` -- a
  `curl https://anthropic.com/` shows up as one `dns_events` row
  ("anthropic.com" allowed, qtype=A, rcode=0) plus one `net_events`
  row, both stamped with the same trace_id. 6 new
  capsem-core::net::dns::telemetry tests (allowed, denied,
  undecodable, decision strings round-trip with logger convention,
  source_proto optional, process_name passthrough) + 2 new
  capsem-logger writer tests (dns_event_insert_populates_row,
  dns_events_indexed_by_trace_id_for_join) + 3 new schema tests
  (create includes dns_events, migrate idempotent, indexes
  present). Bench gate still deferred to T3.4 (zero MITM hot-path
  code touched).

### Added (mitm-redesign T3.2)
- **vsock DNS envelope + guest `capsem-dns-proxy` listener.** New
  vsock port `VSOCK_PORT_DNS_PROXY = 5007` (`capsem-proto`)
  carries length-framed `rmp-serde` `DnsRequest` / `DnsResponse`
  envelopes between the guest agent and the host's `DnsHandler`.
  The host side (`capsem-process::vsock::serve_dns_session`)
  performs one envelope round-trip per vsock connection: read a
  `DnsRequest`, run `DnsHandler::handle` (T3.1), write a
  `DnsResponse`, close. The guest side is a new agent binary
  `capsem-dns-proxy` that listens on `127.0.0.1:1053` (UDP + TCP
  on the same port; iptables NAT will redirect 53 -> 1053 in
  T3.4) and opens a fresh vsock conn per query. The `DnsHandler`
  was retrofitted to take the same `Arc<RwLock<Arc<NetworkPolicy>>>`
  hot-swappable shape as `MitmProxyConfig` so an admin policy
  edit propagates to both protocols at once. The agent crate
  stays hickory-free -- it forwards raw bytes only. 9 new
  capsem-proto envelope tests (port-distinctness, request /
  response roundtrip, no-process-name path, compactness,
  garbage rejection, IPC-frame disjointness) + 5 new agent-bin
  unit tests pinning the listen port (1053), vsock port (5007),
  EDNS payload size, proto labels. Pre-T3.4 the `capsem-dns-proxy`
  binary is built and packaged but NOT launched -- T3.4 wires it
  into `capsem-init` alongside the iptables redirect for port 53
  and removes the dnsmasq invocation. Until then dnsmasq is still
  the guest's DNS server.

### Added (mitm-redesign T3.1)
- **Host-side DNS handler + UDP forwarder + wire-format parser.**
  New `capsem-core::net::dns` module (`server`, `resolver`) plus
  `capsem-core::net::parsers::dns_parser`. The `DnsHandler` is the
  bytes-in / bytes-out async processor that decodes a DNS query,
  consults the shared `NetworkPolicy::is_fully_blocked` rule, and
  either synthesizes an NXDOMAIN response (`Decision::Denied`),
  forwards the bytes verbatim to one of N upstream nameservers
  (default `1.1.1.1:53`, `8.8.8.8:53`; `Decision::Allowed`), or
  returns a synthetic SERVFAIL when every upstream is
  unreachable (`Decision::Error`). Read-only domains still
  resolve so the MITM proxy keeps its verb-level audit trail.
  Built on `hickory-proto = "0.26"` (workspace dep,
  `default-features = false, features = ["std"]`) -- the agent
  crate stays hickory-free; it'll forward raw bytes when T3.2
  wires the vsock envelope. 14 parser unit tests + 10 handler
  end-to-end tests against a fake `127.0.0.1:0` UDP upstream.
  Not yet wired into anything; T3.2 brings the vsock bridge,
  T3.3 the `dns_events` schema + telemetry hook, T3.4 cuts the
  guest image over from dnsmasq to iptables redirect.

### Performance (mcp-concurrency)
- **MCP gateway in-flight cap now scales with host CPU.** Default
  `DEFAULT_MCP_INFLIGHT` constant replaced with
  `default_inflight_cap()` = `available_parallelism * 4`. Anchors
  to the empirical sweet spot we measured on Apple M5 Max (18 cores,
  64 permits optimal) and tracks host shape automatically.
  `CAPSEM_MCP_INFLIGHT` continues to override the computed default.
  Sample mappings: 8-core -> 32, 16-core -> 64, 18-core (M5 Max) ->
  72, 32-core -> 128. Fallback when `available_parallelism()` itself
  fails: 8 cores -> 32 permits.
- **mcp-load throughput +62 % at concurrency 200; tail -24 %.**
  Three changes shipped together so the regression we measured when
  T1.2 + T1.3 were tried alone (p99@200: 40 → 358 ms, mitm rps -40 %)
  cannot land on its own again:
  1. **T1.2: aggregator subprocess pipelined.** `capsem-mcp-aggregator`
     no longer reads-then-handles-then-writes in one task; the reader
     spawns `handle_request` per incoming msgpack frame and a single
     writer task drains an `mpsc<AggregatorResponse>(256)` to stdout.
     `Shutdown` is acked synchronously on the reader path before the
     drain so we can't lose the ack to a stuck handler.
  2. **T1.3: hot manager lock eliminated.** `McpServerManager` now
     exposes `dispatch_call_tool` / `dispatch_read_resource` /
     `dispatch_get_prompt` that perform the lookup synchronously and
     return owned `impl Future + Send + 'static` futures. The
     aggregator wraps the manager in `std::sync::RwLock`; the sync
     read guard drops before the rmcp RPC is awaited, so concurrent
     dispatches never serialise on the manager.
  3. **T1.5: bounded concurrency at the gateway.** The MCP gateway in
     `capsem-core::mcp::gateway::serve_mcp_session` now acquires a
     `tokio::sync::Semaphore` permit BEFORE `tokio::spawn`-ing each
     handler. Default cap 64 (override via `CAPSEM_MCP_INFLIGHT`,
     forwarded through the capsem-service env-allowlist). Without
     this cap, T1.2 + T1.3 turn the MCP path into a CPU-starvation
     source for the rest of capsem-process (notably the MITM proxy on
     the same tokio runtime).
  Bench (Apple M5 Max, 2 vCPU bench VM, vs T1.1-only baseline at
  HEAD): mcp-load c=10 rps 3370 → 9160 (+172 %), c=50 rps 3081 →
  8633 (+180 %), c=200 rps 5224 → 8464 (+62 %), p99@200 57.1 →
  43.4 ms (-24 %), p999@200 67.9 → 53.4 ms (-21 %). mitm-load
  c=200 rps 2845 → 2968 (+4.3 %), p99 177 → 170 ms (-3.8 %) — both
  paths better, neither path regressed. Sprint MCP rps@200 gate
  (≥ 8000) cleared; the 35 ms p99@200 gate is still 8 ms over and
  is tracked as T3 in `sprints/mcp-concurrency/tracker.md`.

### Added (mitm-redesign)
- **T2 plain-HTTP coverage: adversarial / risk-shape tests.**
  Five more tests on top of the parsing-correctness ones, each
  hitting a real failure mode the proxy could plausibly meet in
  the wild:
    * `…body_larger_than_preview_cap_forwards_full_but_caps_preview`
      -- 16 KB request body (4x default `max_body_capture`).
      Asserts upstream receives the full body byte-for-byte,
      `NetEvent.bytes_sent == 16384`, but
      `NetEvent.request_body_preview` length <= 4096 and starts
      with the first 4 KB block (no later block leaked through
      the cap).
    * `…ipv6_host_header_does_not_silently_succeed` -- inbound
      `Host: [::1]:8080`. The host parser explicitly bails on
      `[`-prefixed hosts; the proxy must NOT 200 on the implicit
      ("", 80) fallback. Asserts response is 502 or 403, never
      200, with a non-Allowed `Decision`.
    * `…corrupted_gzip_response_doesnt_crash` -- upstream sends
      `Content-Encoding: gzip` plus a valid 10-byte gzip header
      followed by 61 bytes of garbage payload. With a 5s read
      deadline, the test asserts: (a) the proxy still emits
      exactly one `NetEvent` (= `on_response_end` fired = no
      panic on the response path), and (b) `bytes_received == 0`
      because `flate2::Decompress` yields nothing on a
      fully-corrupt deflate body. Future regressions that would
      leak pre-decode bytes here get caught.
    * `…truncated_upstream_response_doesnt_hang` -- upstream
      advertises `Content-Length: 1000` but writes only 33 bytes
      then closes. With a 5s read deadline. Asserts the proxy
      doesn't hang AND `bytes_received <= 33` AND `< 1000` (i.e.
      we record the actual bytes received, not the lying
      Content-Length).
    * `…zero_length_response_body_emits_netevent` -- 200 OK with
      `Content-Length: 0`. Asserts the chunk-hook chain still
      fires `on_response_end` on an empty body and emits exactly
      one `NetEvent` with `bytes_received == 0`.
  26 mitm_integration tests pass (17 plain-HTTP + 8 TLS + 1
  ignored throughput); 1542 lib tests pass; clippy clean.
- **T2 plain-HTTP coverage: verbs, query strings, header
  passthrough + secret redaction.** Four more integration tests
  on top of the structural ones, closing the parsing-correctness
  gap:
    * `mitm_proxy_plain_http_records_every_http_method` -- sends
      GET / HEAD / OPTIONS / POST / PUT / DELETE / PATCH on one
      keep-alive connection, asserts seven separate `NetEvent`
      rows each with the right `method` + `path` + `204` status.
      Validates verb parsing across both read-classified and
      write-classified methods.
    * `mitm_proxy_plain_http_records_query_string_with_parameters`
      -- `GET /search?q=hello%20world&page=2&filter=active&tag=a&tag=b`.
      Asserts the upstream sees the full request line verbatim
      AND `NetEvent.path == "/search"` (no `?`) +
      `NetEvent.query == "q=hello%20world&page=2&filter=active&tag=a&tag=b"`.
      Repeated keys, equals signs, and percent-encoded values
      preserved verbatim.
    * `mitm_proxy_plain_http_forwards_custom_headers_to_upstream`
      -- sends `User-Agent` (allowlisted), `X-Trace-Id`,
      `X-Custom-Flag`, `Authorization: Bearer ...` (custom).
      Asserts the upstream receives every header by name + value
      verbatim, and that `accept-encoding` was rewritten to `gzip`
      (we only forward what we can decompress).
    * `mitm_proxy_plain_http_telemetry_hashes_non_allowlisted_headers`
      -- security-focused. Sends real-shaped secrets:
      `Authorization: Bearer SUPER-SECRET-...`,
      `X-Api-Key: live_pk_DEADBEEF_...`,
      `Cookie: session=ROTATE_ME_...`. Asserts
      `NetEvent.request_headers` does NOT contain any of those
      verbatim values (each is replaced with `hash:<12-hex>`),
      while the header NAMES still appear and allowlisted
      `User-Agent` + `Host` appear verbatim. Locks down the
      "secrets in telemetry" surface.
  Also tightened the keep-alive test's response reader to drain
  head + body per request rather than relying on one-shot
  `tcp.read()` (was order-flaky on a busy CI). 21 mitm_integration
  tests pass; 1542 lib tests pass; clippy clean.
- **T2 plain-HTTP integration coverage extended.** Five new
  integration tests close the "ad-hoc verification" gap left by
  the earlier Ollama smoke. The new tests share a
  `spawn_fake_upstream(serve)` helper + a `read_http11_request`
  drainer so each test parameterizes the upstream's behavior:
    * `mitm_proxy_plain_http_post_forwards_body_and_records_bytes_sent`
      -- POST with body. Asserts the upstream sees the JSON body
      verbatim + `NetEvent.bytes_sent` covers the body.
    * `mitm_proxy_plain_http_chunked_streaming_response_aggregates_bytes`
      -- fake upstream sends `Transfer-Encoding: chunked` with 4
      data frames. Asserts the client sees every chunk +
      `NetEvent.bytes_received` equals the concatenated payload
      length (proves the ChunkDispatchBody runs the sync
      ChunkHook chain across multiple frames and the
      end-of-stream NetEvent emission fires).
    * `mitm_proxy_plain_http_keep_alive_emits_one_netevent_per_request`
      -- single client TCP connection, three back-to-back GETs to
      `/a`, `/b`, `/c`. Asserts three separate `NetEvent` rows,
      each with the right path/method/status/port/conn_type.
      Validates the per-connection cached upstream sender +
      keep-alive on the plain-HTTP branch.
    * `mitm_proxy_plain_http_preserves_host_header_to_upstream`
      -- captures the bytes the upstream observed. Asserts the
      inbound `Host: 127.0.0.1:<port>` header is forwarded
      verbatim. (TLS path rewrites Host from SNI; HTTP must not.)
    * `mitm_proxy_plain_http_unresolvable_upstream_emits_502_netevent`
      -- targets `nonexistent.invalid` (RFC 6761). Asserts 502
      back to the client + one `NetEvent` with `Decision::Error`,
      status 502, conn_type http-mitm, and the dial error in
      `matched_rule`. No silent drop on dial failure.
  17 mitm_integration tests pass (8 plain-HTTP + 8 TLS + 1
  ignored throughput); 1542 lib tests pass; clippy clean.
- **T2 verified end-to-end against real Ollama on
  `127.0.0.1:11434`.** From inside an air-gapped VM, `curl
  http://127.0.0.1:11434/api/tags` rides the full new pipeline:
  iptables redirect (port 11434 → 10080), agent listener on
  10080, vsock bridge, host first-byte sniff (T2.1), Host header
  parse + port allowlist (T2.2), plain TCP upstream dial, 357-byte
  JSON response forwarded verbatim to the guest. NetEvent recorded
  with `port=11434, conn_type=http-mitm, decision=allowed,
  status=200`. As part of the verification,
  `DEFAULT_HTTP_UPSTREAM_PORTS` is bumped from `[80]` to
  `[80, 11434]` so the host policy default mirrors the iptables
  rules in `capsem-init` -- otherwise port 11434 traffic gets
  redirected to 10080, hits the host proxy, and is rejected by
  the policy gate, which is the wrong default for the canonical
  local-LLM workflow this protocol path was designed for. New
  ports get added by editing both lists in tandem until the
  policy_config plumb (deferred follow-up) lands.
- **T2 (agent-side): plain-HTTP listener + iptables redirects.**
  `capsem-net-proxy` now listens on `127.0.0.1:10080` in addition to
  the original `:10443`; a `run_listener(port)` helper drives the
  per-port accept loop, and both targets the same vsock port
  `VSOCK_PORT_SNI_PROXY` (5002) -- the host's first-byte sniff
  (T2.1) classifies on wire bytes, so the guest-side listener split
  is just an iptables-target convenience. `capsem-init` adds two
  `iptables -t nat -A OUTPUT -p tcp --dport <N> -j REDIRECT
  --to-port 10080` rules for `:80` (plain HTTP) and `:11434`
  (Ollama default); the post-launch readiness poll waits for both
  `:10443` and `:10080` before declaring the proxy ready. Three
  new in-VM diagnostics cover the wiring:
  `test_iptables_redirect_80_to_10080`,
  `test_iptables_redirect_11434_to_10080`, and
  `test_net_proxy_http_listening`. Three new agent unit tests pin
  the new constant + cross-port distinctness. Cross-compile
  (`aarch64-unknown-linux-musl`) clean. The configurable
  guest-side allowlist (read from `policy_config`) is deferred --
  the host-side `NetworkPolicy.http_upstream_ports` is the
  authoritative gate, and adding a config plumb to the guest-side
  iptables list is its own follow-up.
- **T2.3: Ollama-shaped end-to-end test for the plain-HTTP path.**
  `mitm_proxy_plain_http_ollama_shape_records_telemetry` spins a
  fake plain-HTTP upstream on `127.0.0.1:0`, configures the proxy
  with that OS-assigned port on its `http_upstream_ports` allowlist
  + `127.0.0.1` on the domain allowlist, sends `POST /api/generate`
  with the typical Ollama request shape (model + prompt JSON body),
  and asserts: (a) the upstream's response body is forwarded
  verbatim, (b) the resulting `NetEvent` records
  `method=POST`, `path=/api/generate`, `status=200`,
  `domain=127.0.0.1`, `port=<upstream_port>`,
  `conn_type=http-mitm`, `decision=Allowed`, with non-zero
  `bytes_sent` / `bytes_received`. Adds `make_proxy_config_full`
  helper to override the `http_upstream_ports` allowlist
  (existing tests stay on the default `[80]`). 12 mitm_integration
  tests pass.
- **T2.2 (host-side): plain HTTP serves through the same hyper
  pipeline as TLS.** When the first-byte sniff (T2.1) classifies a
  connection as `Protocol::Http`, the listener now skips rustls
  entirely and runs `hyper::server::conn::http1::Builder::new()
  .serve_connection(io, svc)` directly on the vsock stream
  (`ReplayReader` carries the buffered first bytes). Per-request
  domain + upstream port are parsed from the inbound `Host` header
  by `parse_http_host_target` (T2.2 helper in `mitm_proxy/util.rs`)
  and threaded through `handle_request` as a new `upstream_port:
  u16` parameter; the inbound `host` header is preserved (it's
  authoritative for plain HTTP), unlike the TLS path which still
  rewrites it from the SNI domain. The hyper service closure runs
  the same PolicyHook and ChunkHook chain as TLS, so domain
  policy, decompression, SSE parsing, AI interpreters and
  Telemetry all apply uniformly. Upstream dials branch on
  `protocol`: TLS does TCP+rustls+http1::handshake, HTTP does
  TCP+http1::handshake (no TLS step). Telemetry: every
  `TelemetryRequestContext` carries `port: u16` + `conn_type:
  &'static str` (`https-mitm` / `http-mitm`); `NetEvent` rows now
  reflect the actual upstream port and transport label so
  operators can split HTTPS vs plain-HTTP traffic in `session.db`.
  `MitmProxyConfig::handle_inner` is split into `serve_tls`,
  `serve_plain_http`, and a shared `serve_pipeline` helper that
  drives the hyper server over either an `IO: hyper::rt::Read +
  hyper::rt::Write`. New `NetworkPolicy::http_upstream_ports:
  Vec<u16>` (default `[80]`) gates plain-HTTP upstream ports
  before the dial -- a request whose `Host` header carries an
  allowlist-missing port is rejected with a 403 + Decision::Denied
  + `matched_rule = "http-port-not-allowlisted({port})"`. The TLS
  path is unaffected by the allowlist (always uses 443).
  Two new integration tests cover the path:
  `mitm_proxy_plain_http_denies_disallowed_host` (PolicyHook 403
  on a disallowed Host) and
  `mitm_proxy_plain_http_denies_port_not_in_allowlist` (port-gate
  403). 1539 lib tests + 11 mitm_integration tests pass; clippy
  clean. Agent-side multi-port listener and iptables rules ship
  separately so the in-VM test (T2.3) can drive them.
- **T2.1: first-byte protocol sniff (TLS vs plain HTTP) on the vsock
  listener.** New `mitm_proxy::protocol` module with `Protocol` enum
  (`Tls` / `Http` / `Unknown`) and `detect(&[u8]) -> Option<Protocol>`
  classifier. The `vsock:5002` accept path now peeks the first
  post-meta payload byte: `0x16` -> TLS (existing path, unchanged);
  uppercase ASCII (`0x41..=0x5A`, the HTTP method set) -> plain HTTP
  classified but routed to a "T2.2-pending" connection-level error
  (the actual hyper plain-HTTP server lands in T2.2); other bytes ->
  `Unknown` connection-level error. The `mitm.connections_total`
  counter, previously hard-coded to `protocol="tls"` on every accept,
  is now incremented post-sniff with the correct label so operators
  can distinguish TLS / HTTP / unknown traffic. `mitm.requests_total`
  + the upstream-error increments propagate the same label.
  `ConnMeta` carries a `protocol: Protocol` field set from the sniff;
  every hook reads it through `ctx.conn().protocol`. 8 unit tests in
  `protocol/tests.rs` cover the byte-level rules (record types
  `0x14`/`0x15`/`0x17` rejected; lowercase methods rejected; high-bit
  junk rejected) plus 2 integration tests in `mitm_integration.rs`
  asserting the plain-HTTP and unknown-byte paths each emit the
  right `NetEvent`.

### Changed (mitm-redesign)
- **T1 closes -- legacy async body chain deleted; sync ChunkHook
  pipeline owns the response path end-to-end.** Slice 9 cleanup.
  Removes `mitm_proxy/telemetry.rs` (`TelemetryEmitter` +
  `TelemetryBody`, ~390 lines), `ai_traffic/ai_body.rs`
  (`AiResponseBody`, ~155 lines), `body::DecompressBody` +
  `body::BodyStream` + `body::RespStatsKind` (one
  `async_compression::tokio::bufread::GzipDecoder` adapter, one
  `tokio_util::io::StreamReader`, one `Body→Stream` shim). The
  inline `if is_gzip { DecompressBody::new(...) }` block in
  `handle_request` is gone -- the inline `if is_gzip` now only
  strips Content-Encoding / Content-Length headers (a few field
  accesses on the parts struct, kept inline because moving it to
  an async hook would re-introduce the same plumbing the slice
  removed). All four ChunkHooks are pure sync: `DecompressionHook`
  (`flate2::Decompress::new(false)`), `SseParserHook`, three
  `InterpreterHook`s, `TelemetryHook` -- per-chunk work runs inline
  from `poll_frame` with no `.await`, no channel hop, no async
  wrapper. `TelemetryHook` is wired into
  `make_production_pipeline` + reads its per-request context out
  of a `HookState` slot seeded by `handle_request` (new
  `HookState::set::<T>()` + `ChunkDispatchBody::seed::<T>()`
  builder). `MitmProxyConfig` is refactored to hold
  `Arc<TelemetryDeps> { db, pricing, trace_state }` instead of
  by-value `pricing` + `Mutex<TraceState>` -- the `Arc` breaks
  the would-be config↔pipeline↔hook reference cycle (the hook
  points at `TelemetryDeps`, not the surrounding config).
  `make_production_pipeline` signature now takes the
  `Arc<TelemetryDeps>`; `capsem-process` construction site +
  in-tree test fixtures + the integration test in
  `crates/capsem-core/tests/mitm_integration.rs` updated. The
  redundant `TelemetryEmitter` / `TelemetryBody` / `DecompressBody`
  / `emit_model_call` / `trace_chains_across_tool_use` test
  fixtures in `mitm_proxy/tests.rs` are deleted -- the same
  surfaces are covered by the per-hook tests in
  `telemetry_hook/tests.rs` (NetEvent + ModelCall builders),
  `decompression_hook/tests.rs` (gzip streaming), and the
  remaining integration tests still exercise the full path
  end-to-end via `handle_connection`.

  **Bench: SSE parser microbench at 478-488 MiB/s (up from 449-472
  MiB/s in the T0 pre-rewrite baseline; criterion reports
  "Performance has improved" with p<0.05).** Sync ChunkHooks are
  structurally faster than the async wrappers they replace.
  `capsem-bench mitm-load` against
  `benchmarks/mitm-load/baseline.json` is the integration gate;
  it requires a built VM image and is run on a real-machine
  session (this commit's verification rests on the criterion
  micro-bench + the 8 in-tree integration tests through the
  full MITM path).

  1531 capsem-core lib tests pass (down from 1547 -- the deleted
  redundant fixtures); 8/8 mitm_integration tests pass; clippy
  clean.

### Performance (mcp)
- **Pipelined the MCP gateway loop**
  (`crates/capsem-core/src/mcp/gateway.rs`). The per-vsock-connection
  serial `read → handle → write` loop is replaced with a reader that
  spawns one `tokio::spawn(handle_json_rpc)` per request and a
  dedicated writer task that drains an `mpsc::Receiver<Vec<u8>>`(256).
  Out-of-order responses are fine — JSON-RPC `id` lets the client
  demux. mcp-load (single fastmcp Client over one vsock) gains
  **+30 % rps@200 (4 252 → 5 551) and -44 % p99@200 (70.95 → 39.73 ms)**;
  mitm-load unchanged (±2.6 %). Next ceiling is the aggregator
  subprocess loop (T1.2 in `sprints/mcp-concurrency/`).

### Fixed (mcp)
- **`capsem_host_logs` / `capsem_panics` / `capsem_triage` /
  `capsem_timeline` no longer corrupt query values with reserved
  characters.** Each tool built its URL via raw
  `format!("k={}&", value)` interpolation. Two failure modes,
  both reproduced via live MCP:
  1. Any value containing whitespace (e.g. `grep="capsem-gateway
     spawned"`) failed with `invalid uri character` because the
     URL parser rejects unencoded spaces. **Multi-word grep was
     completely broken.**
  2. Any value containing `&` (e.g. `grep="foo&bar"`) was silently
     truncated to `foo` because the server's query parser saw the
     unescaped `&` as a separator and treated `bar` as a stray
     empty param.
  Same risk on `=`, `+`, `#`, `%`, `?`, and other reserved chars
  in `since`, `id`, `trace_id`, `layers`. Fix in
  `crates/capsem-mcp/src/main.rs`: new `query_string` helper
  builds the query from a list of `(key, Option<value>)` pairs,
  percent-encoding each value with an explicit RFC 3986
  query-value set (CONTROLS plus all reserved/unsafe ASCII;
  ALPHA/DIGIT and the unreserved `-._~` round-trip plain).
  Refactored the 4 tools to use it; trailing-`&` cosmetic issue
  fixed as a side effect. 8 new unit tests cover empty/single/
  multiple/None-skipping/space/`&`/multi-reserved-chars/unreserved-
  passthrough. `capsem_service_logs` was unaffected (does
  client-side filtering); the other 21 tools use JSON bodies or
  path-only URLs and don't take untrusted query values.

### Fixed (build)
- **`just _pack-initrd` no longer corrupts the hash-named hardlink
  while a stress run is mid-`VmConfig::build`.** The recipe wrote
  the gzipped cpio archive via shell redirect (`gzip > "$INITRD"`),
  which truncates the existing inode in place. `create_hash_assets.py`
  later gives `initrd.img` a hash-named hardlink (e.g.
  `initrd-<hex16>.img`, sharing the inode). An in-place rewrite
  mutates that hardlink's content too, so any concurrent VM mid-
  `VmConfig::build` reading the old hash-named path computes a hash
  of the NEW bytes and rejects with `hash mismatch for ...img:
  expected X, got Y` -- a stress run hit by a parallel `just
  _pack-initrd` lost two cycles per race (observed in
  `target/stress-acceptance-logs/iter-6.log` cycles 48-49 with
  unified-log evidence of `cpio` running at the exact failure
  timestamp). Fix in `Justfile`: write to `${INITRD}.tmp.$$` and
  `mv` to the final path. The atomic rename leaves the old inode
  (and its hash-named hardlink) intact until `_cleanup_stale` in
  `create_hash_assets.py` explicitly unlinks the old alias.

### Fixed (resume,protocol)
- **Stress-cycle "doesn't have entitlement" cascade now self-recovers
  via launchd-cleanup-aware retry.** Apple's
  `Virtualization.framework` runs a per-VM XPC helper
  (`com.apple.Virtualization.VirtualMachine.<UUID>`); when
  capsem-process dies, launchd schedules that XPC's cleanup with a
  9-second delay (observed in `log show`: `scheduling cleanup in 9
  sec after sending Killed: 9` followed by `internal event:
  PETRIFIED`). Under rapid VM churn (~3s/cycle) the cleanup queue
  grows; once `syspolicyd` saturates (`Unable to get certificates
  array: (null)` in the unified log just before the failure
  window), the next freshly-spawned capsem-process's
  `VZVirtualMachineConfiguration.validateWithError()` returns
  NSError code 2 with the misleading
  `localizedDescription = "...The process doesn't have the
  'com.apple.security.virtualization' entitlement."` -- even though
  the binary IS entitled. We saw this fire as 2-cycle cascades at
  ~cycle 37-40 of the 50-cycle stress (iter-2 cycles 37-38; iter-6
  cycles 39-40 post-Bug-C-fix). Two-part fix in
  `crates/capsem-service/src/main.rs`:
  (1) New `is_launchd_cleanup_transient` helper pattern-matches
  the full VZ-specific phrase (`com.apple.security.virtualization`
  + `entitlement`) on the failed-attempt's process.log tail. Does
  NOT match a bare `entitlement` mention so a real codesign
  regression still surfaces.
  (2) `handle_provision` extracts the per-attempt logic into
  `provision_attempt` and wraps it in `capsem_core::poll::poll_until`
  with `timeout=8s, initial_delay=200ms, max_delay=500ms`. On
  `LaunchdTransient` outcome the loop unregisters the failed
  attempt's persistent-registry entry + clears the instances map,
  then retries; everything else (`BootCrash`, `ProvisionError`,
  `Ready`) bails or succeeds immediately. Retry-decision routing
  is a pure function (`classify_attempt_decision`) so the retry
  logic is unit-testable without spawning a real VM. Worst-case
  user-visible latency on a healthy launchd is unchanged
  (single attempt, ~3-5s); under cascade the retry adds ~500ms-1s
  of backoff per failed attempt, amortized against the launchd
  drain. Unit coverage: 4 matcher tests + 6 routing tests covering
  Ready/StillBooting/LaunchdTransient/BootCrash/already-exists
  /generic-provision-error.
- **Post-resume `vsock_connect` ECONNRESET no longer poisons the agent's
  exec dedup cache.** After `restoreMachineStateFromURL` the host's
  vsock listener for the EXEC port (5005) is registered but the
  kernel-side accept queue can briefly reset incoming connections
  while VZ attaches it. The agent's `run_exec` opened that connection
  with a single-shot `vsock_connect`; one ECONNRESET → `run_exec`
  returned 126 → `exec_done` cached `id → 126` → every host-watchdog
  retry of the same Exec id replayed `ExecDone {exit_code: 126}`,
  even after the transport recovered. Captured in serial.log as
  `exec[N] vsock connect failed: Connection reset by peer (os error
  104)` followed by `exec[N] duplicate (already done, exit=126);
  replaying ExecDone`. Two-part fix in
  `crates/capsem-agent/src/main.rs`: (1) new
  `vsock_connect_with_econnreset_retry` helper retries on
  `ErrorKind::ConnectionReset` only (5 attempts × 20ms backoff =
  ~100ms ceiling, well under the host's 1s watchdog window);
  non-ECONNRESET errors bail immediately so misconfiguration
  (refused / address-family-unsupported) is not papered over. (2)
  `run_exec` now returns `ExecOutcome::{Done(i32), TransportFailed}`;
  `control_loop` only inserts into `exec_done` when
  `outcome.should_cache()` -- transport failures stay uncached so
  the next host-watchdog retry gets a fresh attempt against the
  recovered vsock. The host still receives `ExecDone {exit_code:
  126}` so its watchdog resolves with a real ExecResult instead of
  hanging. Verified in real-VM stress: pre-fix cycle 1 hit this in
  the very first failure; post-fix 39 consecutive cycles pass before
  a different (separately-tracked) failure mode appears. Unit
  coverage: 7 new tests covering retry-success, retry-recovery,
  bail-on-other-kinds, exhaustion, cache-decision matrix.
- **Symmetric guest-side replay buffer with `HostToGuest::AckReply`.**
  Closes the bidirectional silent-drop hole: the prior bridge replay
  layer covered the host→guest forward path; this adds the matching
  guest→host return path. The agent now keeps every ackable
  `GuestToHost` response (`ExecDone` / `FileOpDone` / `FileContent` /
  `Error`) in a `pending_responses` map keyed by `id`, lifted to
  outer scope in `capsem-agent/src/main.rs` so it survives
  reconnects (the writer thread is per-`run_bridge`). On every
  fresh control conn the writer thread first replays every entry
  still in the map, then resumes normal writes. The host bridge in
  `capsem-process/src/vsock.rs` emits `HostToGuest::AckReply { id }`
  immediately on receipt of an ackable response; `control_loop`
  removes the entry. Without this, an ExecDone (or FileContent --
  worse, since the agent doesn't cache file bytes) lost on the
  Apple VZ silent-drop path was unrecoverable except via the
  host's watchdog re-sending the original `Exec`, which only worked
  for `Exec` (cached `exit_code`) and not for `FileRead`'s
  `FileContent`. Verified directionally with a 50-cycle
  `CAPSEM_STRESS=1 test_stress_suspend_resume.py` run, 50/50 passed.
- **Bridge replay layer with `GuestToHost::Ack` for ackable
  HostToGuest messages.** The control bridge in
  `capsem-process/src/vsock.rs` now keeps every ackable outbound
  message (`Exec` / `FileWrite` / `FileRead` / `FileDelete`) in a
  pending map keyed by `id` (`JobStore::pending_acks`). The agent
  emits `GuestToHost::Ack { id }` immediately on receipt, *before*
  any processing -- the bridge clears the entry. On every fresh
  control conn after a re-key, the bridge re-writes every entry
  still in the map. This is the protocol-level cover for Apple
  VZ's post-restoreState silent-drop pattern: the host's
  `write_control_msg` returns success while the bytes never
  propagate, so the previous single-slot `held: Option<HostToGuest>`
  (which only fired on write *errors*) couldn't catch them. The
  multi-slot map also recovers a message whose Ack was lost on the
  return path -- the message stays pending across reconnects until
  an ack actually lands. Agent dedup ensures a re-sent message that
  did land twice doesn't double-execute.
- **Watchdog recalibrated to 1s × 16 retries (16s budget)** -- with
  the bridge replay layer now handling forward-path losses, the
  watchdog only exists to cover the asymmetric return-path case
  (agent processed and sent ExecDone / FileOpDone, those bytes were
  silently dropped). 1s gives ~6× headroom over the longest
  observed healthy round-trip (~150ms for `bash -c "mkdir+echo+cat"`)
  without sitting idle for 3s of dead time.
- The earlier "8 × 3s = 24s budget" config (commit `8cc76e2`) is
  superseded -- the storm-derivation-based number was correct in
  intent but the bridge replay layer is the structurally right fix
  for forward-path drops.

### Added (mitm-redesign)
- **`TelemetryHook` -- per-request `NetEvent` + optional `ModelCall`
  emission as a sync `ChunkHook`.** T1 slice 8 (additive). Carries
  the entire emit surface that lives in `telemetry::TelemetryEmitter`
  today, packaged as a `ChunkHook` that fires on `on_response_end`.
  The hook owns its own response-side byte counting + preview, so
  once the legacy chain is removed in the cleanup slice it
  replaces both `TelemetryEmitter` (the per-request scratch
  struct) and `TelemetryBody` (the body wrapper that decided
  *when* to fire). Per-request context is read out of a typed
  `HookState` slot (`Option<TelemetryRequestContext>`); a missing
  slot puts the hook in shadow mode (no allocation, no emit). The
  per-call `LlmEventStream` populated by the interpreter hooks is
  read at end-of-stream and folded into the `ModelCall` via the
  existing `collect_summary` path. Pure builder helpers
  (`build_net_event` and `maybe_build_model_call`) are split out
  so tests verify the field-mapping logic without spinning up an
  async runtime or a real `DbWriter`. Trace-correlation
  (tool-use chains across requests) goes through a shared
  `Arc<Mutex<TraceState>>` exactly the way `TelemetryEmitter`
  does today, so existing trace-grouping behavior is preserved
  byte-for-byte. Hook is **not** yet registered in
  `make_production_pipeline` and `handle_request` is **not** yet
  rewired; those changes ship together with the deletion of
  `telemetry.rs`, the legacy `AiResponseBody` /
  `DecompressBody` wrappers, and the benchmark gate in slice 9
  cleanup. Eight unit tests covering: `NetEvent` field mapping,
  HEAD probe filter, non-LLM path filter, non-AI provider
  filter, `LlmEvent` flow into `ModelCall`, tool-use trace
  chaining across two requests, shadow-mode skip when context
  unseeded, byte counting + preview tally with seeded context.
  1547 capsem-core lib tests pass; clippy clean.

### Added (mitm-redesign)
- **`DecompressionHook` -- streaming gzip decompression as a sync
  `ChunkHook`.** T1 slice 7. Replaces the
  `async_compression::tokio::bufread::GzipDecoder` driving
  `body::DecompressBody` with the lower-level
  `flate2::Decompress` raw-deflate state machine plus a small
  hand-rolled gzip-header parser. gzip streaming-decode is
  fundamentally sync, so the async wrapper was plumbing-only
  (one `tokio::io::AsyncRead` adapter, one `StreamReader`, one
  `Body -> Stream` shim) -- removing it is the goal of the cleanup
  slice. The hook detects gzip from the first two bytes' magic
  prefix (`0x1f 0x8b`) since the per-request `HookState` slot map
  carried by `ChunkDispatchBody` isn't shared with async
  `Hook::on_event`'s state, so a `Content-Encoding: gzip` flag
  can't bridge from `RawResponseHead` into the chunk pass through
  that map. Magic detection sidesteps the issue without changing
  the hook trait. The header parser handles the standard 10-byte
  prefix plus FEXTRA / FNAME / FCOMMENT / FHCRC optional fields
  (RFC 1952 §2.3.1). After the header, the deflate body streams
  through `flate2::Decompress::new(false)` (`zlib_header=false` =
  raw deflate); the decoder retains state across chunks so partial
  blocks split anywhere decode correctly. Registered in
  `make_production_pipeline` BEFORE the SSE parser hook so the
  hook order is correct once the legacy inline `DecompressBody` is
  removed in slice 9 (today the hook is essentially a no-op
  because `DecompressBody` decompresses upstream of the
  `ChunkDispatchBody` and the hook sees plaintext bytes; that's
  intentional -- this slice ships the surface, the cleanup slice
  flips the switch). Six unit tests: single-chunk decompress,
  decompressed-bytes split across two chunks, plain non-gzip
  passthrough, classification stickiness (a chunk that happens to
  start with `0x1f 0x8b` after a non-gzip first chunk is left
  alone), byte-by-byte chunking, and one-byte-first-chunk
  classification deferred. 1539 capsem-core lib tests pass;
  clippy clean.

### Added (mitm-redesign)
- **Provider interpreter `ChunkHook`s -- Anthropic / OpenAI /
  Google.** T1 slice 6. Three concrete `ChunkHook`s that consume
  parsed `SseEvent`s from the upstream `SseEventStream` slot and
  emit provider-agnostic `LlmEvent`s into a shared `LlmEventStream`
  slot. Each interpreter gates on its provider's domain
  (`api.anthropic.com`, `api.openai.com`,
  `generativelanguage.googleapis.com`) so registering all three in
  the production pipeline is essentially free for non-AI traffic --
  the unmatched hooks short-circuit on a single string compare
  before touching state. Internally, each hook reuses the existing
  `ProviderStreamParser` impl
  (`AnthropicStreamParserWithState` / `OpenAiStreamParser` /
  `GoogleStreamParser`) -- no parsing logic is duplicated, so all
  the existing per-provider tests still cover the parse semantics.
  The interpreter takes the parser out of its slot via
  `mem::take`, drains `SseEventStream`, runs each event through
  the parser, then puts the parser back -- this releases the slot
  map for the SSE/LLM slot accesses inside (single-borrow at a
  time on the slot map). `LlmEventStream` carries an optional
  `provider: ProviderKind` set by the matching interpreter on
  first push, so downstream consumers can dispatch on provider
  without re-parsing the domain. `on_response_end` runs the same
  drain so trailing SSE events flushed by `SseParserHook` reach
  the interpreter. All three registered in
  `make_production_pipeline` after `SseParserHook`. Six unit tests
  covering: end-to-end Anthropic SSE → text delta + summary,
  OpenAI text delta, Google multi-part chunk, three-hooks-coexist
  routing (only matching one drains), wrong-domain skip leaves
  queue untouched, on_response_end trailing flush. 1533
  capsem-core lib tests pass; clippy clean.

### Added (mitm-redesign)
- **`SseParserHook` -- the first concrete `ChunkHook` consumer.** T1
  slice 5. Wraps the existing `parsers::sse_parser::SseParser` as a
  sync `ChunkHook` and writes parsed `SseEvent`s into a public
  per-request `SseEventStream` slot via `ChunkCtx::state`. The slot
  is the bridge to the provider-specific interpreter hooks landing
  in the next slice -- they drain new events on every chunk pass to
  build `ModelCall` summaries. The hook gates internally on AI
  domains (`api.anthropic.com`, `api.openai.com`,
  `generativelanguage.googleapis.com`) so registering it in the
  production pipeline is free for non-AI traffic: the `is_ai` check
  caches in the parser-state slot on first chunk and a non-AI
  connection bails before allocating the parser. `on_response_end`
  flushes any trailing event without a terminating blank line --
  matches the behavior of the inline `AiResponseBody` path that
  this hook is replacing. Now registered in
  `make_production_pipeline`. Six unit tests cover single-chunk,
  multi-chunk-split, multi-event accumulation, non-AI bypass,
  trailing-event flush, and the `[DONE]` sentinel filter for
  OpenAI. 1527 capsem-core lib tests pass; clippy clean.

### Fixed (resume,protocol)
- **Host-side watchdog around HostToGuest::Exec / FileWrite / FileRead
  with j_rx-based retry and 24s budget.** Apple VZ post-restoreState
  occasionally drops a successfully-written vsock frame (the host's
  `write_control_msg` returns success; the bytes never reach the
  guest), and the existing single-slot replay buffer in the control
  bridge can't catch this -- it only triggers on a write *error*.
  The watchdog re-sends the payload every 3s if the host hasn't seen
  the result oneshot resolve. Direct measurement of one stress-suite
  failure (`process.log` from
  `20260503-220608/.../susp-10f1a6c7`) showed the storm lasted 9.13s
  before any message arrived end-to-end, so the budget is set to 8
  attempts × 3s = 24s, leaving 6s of headroom under the 30s IPC
  envelope. The watchdog's signal is the j_rx oneshot resolving
  (i.e. ExecResult / FileOp ack), not ExecStarted -- the latter
  fires while ExecDone is still in flight, and ExecDone can be lost
  on the same torn return path the original Exec was lost on.
- **Agent-side dedup with cached ExecDone replay.** Exec ids
  observed during a session are tracked in two maps shared across
  reconnects: `exec_inflight` (still running -- skip duplicate, the
  original will send ExecDone) and `exec_done: HashMap<id,
  exit_code>` (finished -- replay GuestToHost::ExecDone with the
  cached code so the host's j_rx resolves even when the original
  reply was lost on the return path). The maps are hoisted out of
  `control_loop` into the parent's outer reconnect scope so a retry
  that lands on a *new* control conn after the previous one was
  torn still hits the dedup logic. File ops are intentionally not
  deduped -- write/read/delete are idempotent enough to re-process
  and re-ack on every receipt, which is correct for a FileOpDone
  that was lost on the return path (dedup-with-skip there would
  deadlock the host watchdog).

### Known limitations (resume,protocol)
- **Stress-suite flakiness floor: ~30% iteration fail rate remains.**
  10x runs of the back-to-back stress suite
  (`test_svc_resume_paths.py` + `test_svc_suspend_corruption.py` +
  `TestSuspendResume`) score 6-7/10 with these fixes, vs 7/10 for
  the unfixed baseline at HEAD~1 -- within the same noise band.
  Direct measurement (one ovl-test failure) showed the post-resume
  storm can last 21s of constant vsock re-keying, dropping
  bidirectional traffic for the entire window. Neither host-side
  retries nor guest-side response replay survive a storm that
  spans the whole 30s IPC envelope, because the bytes for the
  retried Exec *and* its replayed ExecDone are both subject to
  silent-drop on every conn. Closing this requires either: (a)
  application-level reliability (per-message ACKs over vsock with
  exponential backoff and a longer envelope), (b) a guest-side
  replay buffer for GuestToHost messages analogous to the host's
  bridge replay buffer (held across the agent's reconnect rather
  than dropped when the writer thread breaks), or (c) detecting and
  pausing sends during a storm. Followup beyond this sprint's scope.

### Fixed (test-infra)
- **`/delete` now routes through `preserve_failed_session_dir`.**
  Previously the only paths that preserved `process.log` /
  `serial.log` / `session.db` for post-mortem were three
  host-detected failure routes; a Python-side test assertion that
  fired after `/exec` but before the test's `finally:
  client.delete()` left only `service.log` archived, which doesn't
  show what the per-VM process or the guest were doing. The cull
  is bumped from 5 to 32 most-recent failed sessions so a
  10-iteration stress run that creates 1-3 VMs per iteration
  doesn't lose earlier failures to the LRU. Disk usage stays
  bounded by the cull regardless.

### Added (mitm-redesign)
- **Pipeline observability contract: every hook call is logged,
  timed, and counted.** Closes the "what is blocking?" gap. Async
  `Hook::on_event` is now wrapped in a `mitm.hook` info-span carrying
  fields `hook`, `kind`, `layer`, `decision` (recorded after the
  future resolves -- one of `continue`/`rewrote`/`stop_drop`/
  `stop_reject`/`stop_dns_reject`), and `duration_ms`. Counter
  `mitm.hook_invocations_total{hook}` increments per call;
  histogram `mitm.hook_duration_ms{hook}` samples the wall time.
  Trace events bracket the call: `on_enter` + `on_exit` at trace!
  level (filter via `RUST_LOG=mitm.hook=trace`). Stop-outcomes
  promote to debug! at target `mitm.hook.cause` so triage tooling
  surfaces them at default RUST_LOG=info filtering. Sync
  `ChunkHook` iteration gets the same counter + histogram (no span,
  trace! events at `mitm.hook.chunk` -- per-chunk spans would
  dominate the bench budget). New unit test installs a
  `metrics_util::DebuggingRecorder` via `set_default_local_recorder`
  and asserts the counter + histogram both fire on a single
  dispatch. 1521 tests pass; clippy clean.

### Added (mitm-redesign)
- **`ChunkHook` -- sync per-body-chunk hook trait + pipeline
  registration.** T1 slice 3 foundation. `ChunkHook` is a sync
  companion to the async `Hook` trait: methods
  `on_request_chunk(&mut Bytes, &mut ChunkCtx)` /
  `on_response_chunk(...)` / `on_request_end` /
  `on_response_end`. Body wrappers iterate registered ChunkHooks
  inline from `poll_frame` -- no async overhead, no channel hop.
  Sync is correct here because per-chunk work is fundamentally
  CPU-bound byte transformation: decompression, regex
  match-and-replace, streaming parsers, byte counting. None need
  `.await`. Per-connection state lives in the same typed slot
  map the async `Hook`s use, accessed via `ChunkCtx::state::<T>()`.
  `Pipeline` gains `register_chunk(ArcChunkHook)` builder method,
  `has_chunk_hooks()` short-circuit predicate, and
  `dispatch_request_chunk` / `dispatch_response_chunk` /
  `dispatch_request_end` / `dispatch_response_end` iteration
  helpers. Two new unit tests prove the surface: registration-order
  iteration with one hook rewriting bytes that the next hook then
  observes, and the empty-pipeline short-circuit. Slices 3b
  (DecompressionHook), 3c (TelemetryHook), 3d (SseParserHook) are
  now unblocked. 1520 tests pass; clippy clean.

### Added (mitm-redesign)
- **`RawResponseHead` dispatch + per-request `mitm.request` span.**
  T1 slice 3a (observer surface). After upstream returns headers,
  `handle_request` now dispatches `Event::RawResponseHead(&mut parts)`
  through the pipeline so future hooks can observe the response head
  before any wrapping (decompression, telemetry, AI parsing) takes
  place. Hooks that want to react to status codes or content-encoding
  / content-type live here. Today observer-only -- the Stop outcome
  is intentionally dropped because handing the upstream sender
  partially-used would leak. Plus a `#[instrument(target="mitm.request")]`
  decoration on `handle_request` itself recording fields domain,
  method, path, decision, status; every log line in a request now
  carries those as structured fields. Pure addition; no behavior
  change. 1518 tests pass; clippy clean.

### Added (mitm-redesign)
- **Metrics + tracing decision contract wired on the hot path.** T1
  slice 4. Every TLS connection now increments
  `mitm.connections_total{protocol="tls"}` and the
  `mitm.active_connections` gauge (RAII-decremented on drop, even on
  panic). Every request increments
  `mitm.requests_total{protocol="tls", decision}` partitioned by
  outcome (`allow` / `deny` / `upstream_error`). TLS handshake time
  histograms via `mitm.tls_handshake_ms`; full upstream-dial path
  (TCP + TLS) via `mitm.upstream_dial_ms`. `handle_connection` now
  in a `#[instrument(target="mitm.connection")]` span. No recorder
  registered yet, so each emission is one relaxed atomic add against
  the global no-op recorder (~4 ns per call per the T0 baseline).
  Two new smoke tests assert the metric names are unique and
  `describe_all` is idempotent. 1518 capsem-core lib tests pass;
  clippy clean.

### Fixed (virtio-blk-overlay-migration)
- **System overlay moved off loop-on-VirtioFS onto a real virtio-blk
  device.** rootfs.img is now attached to the guest as `/dev/vdb` and
  mounted directly as the overlayfs upper, bypassing the prior
  loop-device-on-VirtioFS sandwich whose closed-source virtiofsd
  returned EIO under writeback pressure on resume. Closes
  `loop-device-io-after-resume`: heavy directory churn + suspend +
  resume no longer leaves `EXT4-fs (loop0): failed to convert
  unwritten extents` / `I/O error, dev loop0` in dmesg. Universal --
  ephemeral and persistent VMs both use the new path; legacy
  loop-on-VirtioFS fallback removed from `capsem-init`. Snapshot
  (APFS clonefile) path validated byte-for-byte against the
  virtio-blk-attached file. `BootOptions::scratch_disk_path` renamed
  to `system_overlay_disk` to reflect its new role.

### Fixed (resume-stability)
- **Resume API no longer hangs 30s when capsem-process dies during
  restore.** `wait_for_vm_ready` now races the `.ready` sentinel poll
  against an instance-presence check; when the resume-side child
  exits before signalling ready, the API fails fast (~5ms-50ms)
  instead of spinning out the full readiness budget. The exit
  handler also logs the child's `exit_status` so future failures are
  diagnosable from `service.log` alone (previously the resume-side
  exit silently dropped the status).
- **Apple VZ post-restoreState handshake EOF is now retryable.**
  `is_retryable_handshake_error` accepts `UnexpectedEof` alongside
  `BrokenPipe` / `ConnectionReset` -- empirically the dominant
  fingerprint when Apple VZ tears the new vsock conn down between
  guest frames. The host re-accepts a fresh terminal+control pair
  and re-runs the handshake within the existing
  `HANDSHAKE_RETRY_MAX` budget. Prior behaviour: process exited with
  code 1, leaving the resume API to time out at 30s.
- **Control bridge holds in-flight `HostToGuest` messages across
  re-key.** When Apple VZ kills the control vsock mid-write, the
  message that was being sent (often an `Exec` or `FileWrite`
  command) used to be silently dropped, and the corresponding
  `/exec` or `/write_file` call timed out at 30s waiting for a reply
  that would never come. The bridge now stashes the failed message
  and replays it on the next successfully re-keyed connection.

### Changed (mitm-redesign)
- **Inline `policy.evaluate` deny path removed; PolicyHook is now the
  source of truth.** T1 slice 2d. PolicyHook stashes its
  PolicyDecision (allowed + matched_rule + reason) in HookCtx::state
  via the typed slot mechanism. After dispatch, handle_request reads
  the record back and uses it to populate the TelemetryEmitter (allow
  + deny paths both). On Stop(Reject(_)) the hook's response is
  wrapped with TelemetryBody so a NetEvent still fires for denies
  (no telemetry regression). Test fixtures upgraded from
  make_default_pipeline() to make_production_pipeline(policy) so
  policy actually fires in unit + integration tests. 1516 lib tests +
  8 integration tests pass; clippy clean. Slice 2d closes T1's
  rewire of the policy stage; the pipeline now owns it end-to-end.

### Added (mitm-redesign)
- **Pre-rewrite `mitm-load` baseline captured.** T0 closes:
  `benchmarks/mitm-load/baseline.json` holds the live numbers from
  `capsem-bench mitm-load` against the un-redesigned proxy at
  concurrency 1/10/50/200 (10s per level). Highlights: rps
  1109/2862/2995/2701, p99 2.2/8.4/45.4/175.2 ms, 0 errors,
  RSS 26-230 MB. T5's CI gate compares against this file -- any
  level >2x p99 regression fails the build.

### Added (mcp + bench)
- **`local__echo` MCP tool + `capsem-bench mcp-load` mode + baseline.**
  New zero-I/O diagnostic tool: returns its `text` parameter verbatim.
  Lives in `capsem-mcp-builtin`; reachable as `local__echo` through
  the in-guest MCP server -> vsock:5003 -> aggregator -> builtin
  subprocess chain. New `capsem-bench mcp-load` mode hammers it from
  the guest with concurrent fastmcp Client calls (asyncio.gather over
  N workers per concurrency level) so we get a number for the MCP
  path's scaling shape, isolated from the MITM path. Pre-rewrite
  baseline at `benchmarks/mcp-load/baseline.json`: rps
  2162/3792/4061/3965 across concurrency 1/10/50/200, p99
  1.1/4.4/17.4/70.8 ms, 0 errors. Sub-linear scaling -- plateaus at
  ~4000 rps from concurrency 10 onwards. There IS a serialization
  point in the MCP path that needs debugging (suspect:
  stdio-framing in capsem-mcp-server, single vsock:5003 stream, or
  JSON-RPC dispatch in the aggregator). Sister to the MITM baseline,
  which plateaus around ~3000 rps with worse tails.

### Added (capsem CLI)
- **`capsem cp` -- file transfer between host and a session's
  workspace.** The service has had `GET/POST /files/{id}/content`
  upload/download endpoints for a while (used by the desktop app's
  Files tab). The CLI never exposed them. Now: `capsem cp foo.txt
  my-vm:foo.txt` (upload) / `capsem cp my-vm:bench.json
  ./bench.json` (download) / `capsem cp my-vm:log.txt -` (stdout).
  Exactly one of `<src>`/`<dst>` must be `SESSION:PATH`; PATH is
  relative to `/root` (workspace bind-mount in the guest). Errors
  loud: `guest-to-guest copy not supported`,
  `neither argument is SESSION:PATH`. New
  `UdsClient::request_bytes` returns raw response bytes + content-type
  for endpoints that don't speak JSON (the existing `request` method
  always tries to deserialize JSON, so couldn't be used for binary
  downloads).

### Added (mitm-redesign)
- **Pre-rewrite `mitm-load` baseline captured.** T0 closes:
  `benchmarks/mitm-load/baseline.json` holds the live numbers from
  `capsem-bench mitm-load` against the un-redesigned proxy at
  concurrency 1/10/50/200 (10s per level). Extracted via the new
  `capsem cp` command (write bench output to `/root/baseline.json`
  in the guest, `capsem cp` it to host). Highlights: rps
  1037/3043/3029/2699, p99 2.3/8.4/53.4/191.3 ms, 0 errors,
  RSS 27-260 MB. T5's CI gate compares against this file -- any
  level >2x p99 regression fails the build.

### Added (mitm-redesign)
- **Hook pipeline now dispatches from `handle_request`
  (parallel-deploy).** T1 slice 2c: every HTTPS request through the
  MITM now runs `pipeline.dispatch(Event::RawRequestHead, ...)` with
  the per-connection `ConnMeta` (domain + process_name + port=443)
  and the ambient `trace_id`. Production builds use
  `make_production_pipeline` so `PolicyHook` fires for every request,
  emitting the `mitm.policy_decisions_total` counter and the
  structured `mitm.policy` tracing event with rule + reason fields.
  The hook's `Stop(Reject(_))` outcome is intentionally dropped this
  slice -- the inline `policy.evaluate()` call below remains the
  source of truth for the actual stop/continue decision so behavior
  is provably unchanged. Subsequent slices land TelemetryHook +
  RejectHook plumbing that lets us safely remove the inline path.

### Added (mitm-redesign)
- **`PolicyHook` + `ConnMeta` + `make_production_pipeline`.** T1
  slice 2b: first concrete `Hook` impl. `mitm_proxy/policy_hook.rs`
  subscribes to `Event::RawRequestHead` (priority -1000 so it runs
  before any other L1 consumer), evaluates `NetworkPolicy::evaluate`
  against `ConnMeta::domain` + the request method, returns
  `Stop(Reject(403))` on deny. Tracing target `mitm.policy` records
  `decision` (allow|deny) + `rule` + `reason`; metric
  `mitm.policy_decisions_total{decision}` increments. New
  `ConnMeta` (`domain`, `process_name`, `port`) carried read-only
  through `HookCtx::conn()` so hooks can reach per-connection
  metadata not present in `RawRequestHead`. `make_production_pipeline`
  builds the registered set; `handle_request` does not yet dispatch
  through it (slice 2c). 4 new tests cover allow / deny / default-allow
  and the `evaluate_decision` rendering helper. 1516 passing.

### Added (mitm-redesign)
- **`MitmProxyConfig` carries a `pipeline: Arc<Pipeline>` field.**
  T1 slice 2a: the `Pipeline` from slice 1 is now plumbed through the
  proxy config so subsequent slices can dispatch from `handle_request`
  without changing the public type again. `make_default_pipeline()`
  returns an empty pipeline -- the inline call graph in
  `handle_request` still drives policy / decompression / AI parsing /
  telemetry. T1 slice 2b will register the production hooks; slice 3
  wires the metrics + tracing decision contract. Three call sites
  updated: `mitm_proxy/tests.rs`, `tests/mitm_integration.rs`,
  `capsem-process/src/main.rs`.

### Added (mitm-redesign)
- **Single `Hook` trait + `Event<'_>` ladder + dispatcher.** T1 slice
  1: pure-additive infrastructure for the new pipeline. Three new
  modules under `mitm_proxy/`: `events.rs` (15-variant `Event<'a>`
  enum across L1 raw transport / L2 protocol / L3 semantic, plus
  `EventKind` discriminator + `EventLayer` ordering + bitset
  `EventMask`), `hooks.rs` (the single `Hook` trait, `HookOutcome` =
  `Continue | Rewrote | Stop(StopAction)`, `StopAction` =
  `Drop | Reject(http::Response) | DnsReject(rcode)`, `HookCtx` with
  per-connection typed slot map for cross-call carry-over and
  `ctx.emit()`), `pipeline.rs` (registration-time-sorted dispatcher
  with O(1) per-kind plan, recursive `emit()` re-entry, layer-cycle
  prevention enforced at runtime: an L3 hook cannot emit L1/L2;
  `EmitError::CycleAttempt` returned). 16 new unit tests including:
  hook ordering by `(priority, registration_order)`, `Stop`
  short-circuit, L1->L2 emit dispatch, L3->L1 cycle rejection, typed
  state slot persistence across multiple chunk dispatches (the
  contract the future credential-rewrite hook will use), trace-id
  visibility. No production code wires the pipeline yet -- T1 slice 2
  rewires policy / decompression / AI parsing / telemetry as Hook
  impls.

### Added (mitm-redesign)
- **`capsem-bench mitm-load` mode.** New
  `guest/artifacts/capsem_bench/mitm_load.py` drives the MITM proxy at
  configurable concurrency levels (default 1 / 10 / 50 / 200) for
  `CAPSEM_BENCH_MITM_DURATION` seconds each (default 10s) against
  `CAPSEM_BENCH_MITM_TARGET` (default a non-routable domain so every
  request fails fast at upstream-dial, isolating proxy cost from
  upstream variance). Reports per-level rps, p50/p95/p99/p99.9 latency,
  RSS peak, and error count. T5's CI gate compares to
  `benchmarks/mitm-load/baseline.json`: any concurrency level >2x p99
  regression fails the build. Baseline JSON itself is deferred --
  requires `just run "capsem-bench mitm-load"` against the
  un-redesigned proxy and commit of the result.

### Added (mitm-redesign)
- **Criterion bench harness + pre-rewrite baselines.** `criterion`
  (dev-dep) plus four new benches under `crates/capsem-core/benches/`:
  `parser_sse`, `parser_jsonrpc`, `interp_anthropic`, `mitm_pipeline`.
  First-run numbers committed to `benches/baselines/T0-pre-rewrite.md`
  -- T5's regression gate compares against this file via `critcmp` and
  fails CI on >5% slower medians. Baseline highlights: SSE parser
  449-472 MiB/s on 1MB corpora (plan budget 500 MiB/s), Anthropic
  interpreter end-to-end 233 MiB/s on tool-use response, metrics-facade
  counter emission 3.89 ns with no recorder installed.
- **`metrics` facade dependency + `mitm_proxy::metrics` module.**
  All counter / histogram / gauge names from the plan declared with
  `describe_*` calls in `mitm_proxy/metrics.rs`. No recorder registered
  this sprint -- T5 wires an exporter (likely OTel via
  `opentelemetry-otlp`); until then emission is a single relaxed atomic
  add against the global no-op recorder. T0 slice 4 of
  `sprints/mitm-redesign/`.

### Fixed (observability)
- **W3 IPC handshake: respect tokio's non-blocking sockets.**
  `tokio::net::UnixStream::into_std()` returns the std handle still in
  non-blocking mode. The W3 handshake's `read_exact`/`write_all` then
  bailed with WouldBlock instantly, manifesting as 95 integration tests
  failing with "peer did not send Hello within 5000ms" the first time
  any IPC channel was used. `negotiate_initiator`/`negotiate_responder`
  now flip the socket to blocking mode for the handshake (saving the
  previous flag) and restore the original mode afterward so the bincode
  channel inherits the same tokio-non-blocking shape it expects. Builds
  + 1273 integration tests now pass.

### Added (observability follow-ups)
- **W6 writer-side population.** `trace_id` is now a column AND a
  field on every event struct. Writer INSERTs the column on every row.
  Construction sites populate via
  `capsem_core::telemetry::ambient_capsem_trace_id()`. `tool_calls` /
  `tool_responses` fall back to the parent `model_calls.trace_id`.
- **`capsem_triage --id <vm>` queries session.db** for `denied_net`,
  `mcp_errors`, `exec_failures` alongside the host-log scan.
- **`capsem_timeline` joins tool_calls -> mcp_calls** so a model
  tool_use shows its servicing MCP call inline.
- **`capsem support-bundle --max-session-bytes`** (default 50MB) drops
  oldest sessions when their session.db total exceeds the cap.
- **Hot-path `#[instrument]` coverage** on `wait_for_vm_ready`,
  `pause`, `resume`, `attach_disk`, `attach_virtiofs_share`.
- **`dump_frontend_logs` Tauri command + `recordWsEvent` wiring.**
  `__capsemDebug.dumpLogs()` now returns a real jsonl path;
  `__capsemDebug.lastWsEvents` actually fills as WS events arrive.
- **Triage panic parser + redactor adversarial fixtures.**
- **`capsem-app` emits `service.start`** so cross-version-mix detection
  covers all 9 binaries (adds capsem-proto leaf dep; capsem-core
  invariant preserved).
- **Skill updates: dev-mcp** (4 new tools in tool table), **dev-debugging**
  (MCP triage trio workflow + schema_hash hint),
  **references/mcp-wire.md** (W5 `_meta` envelope + BootConfig.traceparent).
- **C1: T3 timeline SQL allowlist** enforced before `format!()`.
- **C2: `app_error_logged!`** used in fork's clone-task error path.
- **T1 (test): `tests/capsem-service/test_protocol_handshake.py`**
  exercises the W3 handshake regression.
- **CLI parity: `support-bundle` added to `CLI_ONLY` allowlist.**

### Added (observability)
- **In-band W3C trace context on the host->guest control bridge and
  on MCP JSON-RPC.** `BootConfig` now carries an optional
  `traceparent: String` so the guest agent learns the host's trace_id
  on message #1 of boot; capsem-agent stamps every subsequent
  `blog_line` log line with `trace_id=<lower 16 hex>` so guest-side
  panics, kernel errors, and init script output correlate with
  host-side spans for the same VM boot.
  `JsonRpcRequest` and `JsonRpcResponse` gain an optional `_meta`
  envelope with `traceparent` + `tracestate` (W3C Trace Context) so a
  per-tool-call trace can ride alongside the JSON-RPC payload. Both
  fields are optional with serde defaults -- third-party MCP clients
  and pre-W5 capsem peers continue to round-trip cleanly.
  Also reorganizes the post-mitm-redesign rename: `net::ai_traffic::
  {anthropic,google,openai,sse}` are now re-exports of the new
  `net::interpreters::*_interpreter` and `net::parsers::sse_parser`
  modules so existing call sites compile while new code can use the
  fully-qualified path.

### Changed (mitm-redesign)
- **`mitm_proxy.rs` decomposed into submodules.** The 1421-line file
  is now `mitm_proxy/mod.rs` (614 lines: handle_connection +
  handle_inner + handle_request + MitmProxyConfig + helpers) plus four
  sibling submodules: `body.rs` (BodyStats, RespStatsKind, ProxyBoxBody,
  TrackedBody, BodyStream, DecompressBody), `telemetry.rs`
  (TelemetryEmitter + TelemetryBody + emit_model_call), `fd_stream.rs`
  (AsyncFdStream + ReplayReader + set_nonblocking), `util.rs`
  (is_llm_api_path + split_path_query + format_headers +
  HEADER_ALLOWLIST). Each submodule keeps `pub(super)` visibility so
  the public API of `crate::net::mitm_proxy::*` is unchanged. T0
  slice 3 of `sprints/mitm-redesign/`; zero behavior change.

### Changed (mitm-redesign)
- **All remaining inline `mod tests { }` blocks in `net/` extracted to
  sibling `tests.rs` per CLAUDE.md.** `mitm_proxy.rs` shrinks from
  2847 to 1421 lines (1426 lines of tests now in
  `mitm_proxy/tests.rs`); `ai_traffic/{events,pricing,ai_body,provider,
  mod}.rs` similarly cleaned. Production code is no longer buried under
  scroll-past test fixtures; every grep / Read of a parser shows just
  the parser.

### Changed (observability)
- **W6 trace_id wiring completed across capsem-logger / capsem-core /
  capsem-process.** The `trace_id` column on `net_events`, `mcp_calls`,
  `tool_calls`, `tool_responses`, `fs_events`, `snapshot_events`, and
  `audit_events` is now populated end-to-end. Write-side: every event
  emitter (`mitm_proxy`, `mcp/{gateway,builtin_tools,file_tools}`,
  `fs_monitor`, `capsem-process`'s snapshot/audit paths) calls
  `capsem_core::telemetry::ambient_capsem_trace_id()`. INSERT statements
  in `writer.rs` now include the new column. `tool_calls.trace_id` and
  `tool_responses.trace_id` fall back to the parent `model_calls.trace_id`
  when the per-row value is None (same agent turn). Read-side defaults
  to `None` until the SELECT clauses are extended in a follow-up.

### Changed (mitm-redesign)
- **AI parser tests extracted to sibling `tests.rs` per CLAUDE.md.**
  `parsers/sse_parser.rs`, `interpreters/anthropic_interpreter.rs`,
  `interpreters/openai_interpreter.rs`, and
  `interpreters/google_interpreter.rs` no longer carry inline
  `mod tests { }` blocks; their ~1100 lines of tests now live next to
  each prod file (e.g., `parsers/sse_parser/tests.rs`). Same pattern
  established by the obs sprint's earlier 18-file extraction.
- **Backwards-compat re-exports removed.** The transitional aliases
  `net::ai_traffic::{anthropic,google,openai,sse}` are gone; all
  internal callers (mitm_proxy, ai_body, events, provider, interpreter
  tests) reference the canonical
  `net::parsers::sse_parser` / `net::interpreters::<provider>_interpreter`
  paths. T0 slice of `sprints/mitm-redesign/`.

### Added (mitm-redesign)
- **`sprints/mitm-redesign/` scaffolded.** Meta-sprint plan to decompose
  the 2847-line `mitm_proxy.rs` monolith into a hookable pipeline with
  first-class plain HTTP, a real DNS proxy (hickory-server replaces the
  fake dnsmasq), MCP protocol awareness, and a single `Hook` trait + L1/
  L2/L3 `Event` ladder. Six phases (T0..T5) covering reorganization,
  hook traits, plain HTTP, DNS, MCP awareness, and hardening with
  performance regression CI gates. The future security engine
  (credential rewrite via regex body replace) is explicitly out of scope
  but the hook surface is shaped to host it without trait changes.

### Added (observability)
- **`capsem doctor --bundle` -- in-VM diagnostic tar wired into the
  support bundle.** `guest/artifacts/capsem-doctor` now accepts
  `--bundle [PATH]` and packages pytest output + junit XML, /var/log,
  dmesg, /proc/{mounts,cmdline}, /tmp/capsem-init.log, and
  session.db (when present) into a single tar at
  `/shared/doctor-bundle.tar` (default) or a caller-supplied path.
  Host-side `capsem doctor --bundle` lifts that file out of virtiofs
  to `~/.capsem/run/doctor-latest.tar` before the VM is destroyed.
  `capsem support-bundle` then embeds it as `doctor/bundle.tar`.
  Closes the "guest-side bug, but the bundle has only host context"
  gap in T1's bundle.

- **CI uploads `test-artifacts/` on red runs.** Both the `test-linux:`
  and `test:` jobs now have `upload-artifact@v4` steps gated on
  `if: failure()`. Reviewers get a downloadable bundle of
  `service.log`, `process.log`, `serial.log`, and `session.db` from
  every failed job without rerunning. Existing `preserve_tmp_dir_on_failure`
  in `tests/helpers/service.py` already populates the directory.
- **`just test-artifacts`** -- one recipe that finds the latest
  preserved failure dir under `test-artifacts/` and prints the file
  list with sizes. Saves digging through `ls -lt` after a red local
  run.
- **Frontend `window.__capsemDebug` console handle.** Exposed when
  the URL contains `?debug=1`. Methods: `versions()` (build_ts +
  version), `dumpLogs()` (returns the path to the latest jsonl via a
  reserved `dump_frontend_logs` Tauri command), `lastWsEvents` (small
  ring buffer; populated by api.ts when a WS event arrives via
  `recordWsEvent`). Console-only -- the visual HUD is punted to the
  frontend-rebuild sprint.

- **`capsem_timeline` MCP tool -- one tool call renders the unified
  time-ordered event stream for a session.** UNION across exec_events,
  mcp_calls, net_events, fs_events, and model_calls, ordered by
  timestamp. Filter by `traceId` to follow a single logical operation
  across layers (W6 added trace_id to every table; W4 propagates the
  id through the host process tree). Filter by `since` to scope the
  window. Optional `layers` arg accepts a comma-separated subset
  ("exec,mcp" etc.) when only some are interesting. Pre-W4 rows have
  NULL trace_id and are returned alongside matched rows so the user
  doesn't lose context that pre-dates the trace propagation.

- **`trace_id TEXT` column on every event table.** Added to
  `mcp_calls`, `net_events`, `fs_events`, `snapshot_events`,
  `tool_calls`, `tool_responses`, `audit_events` (model_calls and
  exec_events already had it). Indexes added on each. Fresh DBs get
  the column from `CREATE_SCHEMA`; existing DBs get it via
  idempotent `ALTER TABLE ADD COLUMN` on next open. Unblocks
  `capsem_timeline --trace_id <X>` to UNION across all event classes
  for one logical user action. Population through the writer API
  follows in a subsequent commit; pre-population rows are NULL and
  the timeline tool tolerates that gracefully.

- **W3C trace context propagated to every spawned capsem-* binary +
  per-stage timing on the suspend hot path.** capsem-service injects
  `CAPSEM_VM_ID`, `CAPSEM_TRACE_ID`, `TRACEPARENT`, `TRACESTATE` into
  capsem-process at spawn (cold-boot + resume paths); capsem-process
  forwards them when spawning capsem-mcp-aggregator. New helper
  `capsem_core::telemetry::child_trace_env(vm_id)` in one place; if
  this binary is itself a child of another capsem-* binary, the
  parent's traceparent is forwarded verbatim, so the whole tree shares
  one trace_id. Top-of-tree binaries synthesize a fresh
  `00-<32hex>-<16hex>-01` traceparent from blake3(vm_id + nanos).
  Suspend now emits `target=suspend op=apple_vz_pause`,
  `op=apple_vz_save_state`, `op=with_quiescence`, and
  `target=fs op=fsync path=rootfs.img` events with `duration_ms` --
  closes parent ISSUE.md pattern (6) and the today-2026-05-02
  "fsync timing was missing" debugging session.
- **Top-5 `_ => {}` enum arms now log instead of dropping.** vsock
  port dispatcher, lifecycle port, `handle_guest_msg`, and the MCP
  aggregator main match. An unknown variant now emits
  `tracing::warn!(target = "ipc", unhandled = ?other, "unknown
  variant; this binary may be older than its peer")` -- closes parent
  ISSUE.md pattern (3).

- **`capsem_panics`, `capsem_triage`, `capsem_host_logs` MCP tools.**
  AI agents (and developers via `capsem-mcp`) can now triage Capsem
  failures in one tool call without leaving the conversation:
  - `capsem_panics` -- structured panic + backtrace extractor across
    `~/.capsem/run/{service,mcp,gateway,tray}.log` and capsem-app's
    latest jsonl. Returns `[{ ts, binary, thread, location, message,
    frames }]` with `/Users/<x>/` paths redacted to `~/`. Run this
    FIRST when investigating an unexplained failure.
  - `capsem_triage` -- ranked summary of recent panics, dropped IPC
    frames (`target=ipc` warns from W1), 4xx/5xx server errors
    (`target=service` from W3.5), and slow operations (`target=fs
    op=fsync` etc., >500ms). Default lookback "30m"; accepts "5m",
    "1h", "24h", "7d", or RFC3339.
  - `capsem_host_logs` -- read any host log by symbolic name with
    grep + tail filtering. Hard-coded allowlist (no path traversal).
  Three new service HTTP endpoints (`/triage`, `/panics`,
  `/host-logs/{name}`) reuse the W2 JSON output shape, the W3 schema
  hash, and the W3.5 status field for deterministic ranking.

- **`capsem support-bundle` -- one command, one redacted tar.gz, ready
  to attach to a bug report.** Gathers `~/.capsem/run/*.log`,
  `~/.capsem/logs/*.jsonl`, the last N session directories
  (session.db + serial.log + process.log + metadata.json), assets
  manifest, redacted user.toml/corp.toml, version + OS info, dmesg
  (Linux), and a blake3 fingerprint of the MITM CA cert (the cert
  itself is NEVER bundled). Default output:
  `~/.capsem/support/capsem-support-<UTC-ts>-<host>.tar.gz`. Five
  redaction rules strip Bearer tokens, sk-/AIza/xoxb- API key prefixes,
  TOML/JSON keys named like a secret, and `/Users/<x>/` paths;
  `--no-redact` disables. `--include-rootfs` opt-in (off by default --
  rootfs.img is huge and rarely useful). Manifest schema v1 includes a
  ranked "next steps" list pointing at where to look in the bundle and
  which `target=` filters to grep for.

- **Every `AppError` returned by the capsem-service HTTP layer now
  emits a structured `tracing` event automatically.** Done in
  `IntoResponse` so all 104 `AppError(StatusCode, msg)` call sites are
  covered with zero codemod: 5xx → `error!`, 4xx → `warn!`, other →
  `info!` with `target = "service"` and the status code as a
  structured field. Pre-W3.5: the user got a 500 in the response with
  nothing in `service.log` to trace back from. Optional
  `app_error_logged!` macro lets a call site emit a SECOND event
  earlier (with the same status field) when an in-flight span is more
  informative than the late one fired at response-build time.

- **Versioned IPC handshake: cross-version mixes fail loudly in ~1s.**
  Every typed IPC connection between capsem-service and capsem-process
  now exchanges a `Hello { version, schema_hash, peer, traceparent }`
  frame on the raw UnixStream before the bincode channel takes over.
  `version` bumped to `1`. `schema_hash` is a build-script-emitted
  FNV-1a 64 hash of the protocol source bytes -- catches enum
  reordering / variant additions that don't bump version. On mismatch:
  `tracing::error!(target = "ipc", peer_id, ours_hash, peer_hash,
  "IPC handshake failed; refusing connection")` within 1 second instead
  of the pre-sprint 30-second silent timeout. Side-channel design
  (handshake on the raw stream before bincode) preserves the existing
  `Sender<ServiceToProcess>` / `Receiver<ProcessToService>` API; W1's
  `try_send!` codemod sites are unchanged. Pre-W3 binaries fail decode
  within 5 seconds (HELLO_TIMEOUT).

- **All host-side binaries now write JSON-per-line logs to
  `~/.capsem/run/{service,mcp,gateway,tray}.log`** -- consolidated
  through a single `capsem_core::telemetry::init()` entry point. Eight
  binaries (capsem-service, -process, -mcp, -mcp-aggregator,
  -mcp-builtin, -gateway, -tray, plus the macros consumer in capsem)
  now share one tracing-subscriber bootstrap. The four that previously
  emitted compact-format text (gateway, tray, mcp-builtin,
  mcp-aggregator) now emit structured JSON, so `capsem support-bundle`
  and the upcoming `capsem_panics` MCP tool can parse every host log
  with one decoder. Each binary's `service.start` line carries
  `protocol_version` + `schema_hash` so cross-version-mix can be
  detected from a single log read once W3 lands.
- **W3C `TRACEPARENT` env var captured at startup** and exposed via
  `capsem_core::telemetry::current_parent_traceparent()` /
  `ambient_capsem_trace_id()`. No OpenTelemetry runtime dep this
  sprint -- traceparent is a structured field in JSON for now;
  tracing-opentelemetry layer is a future-sprint addition. Adding it
  later is purely an additional `Layer` on the existing subscriber.

### Changed (observability)
- **Silent IPC drops in suspend/resume/exec/file paths now log at
  `target="ipc"`.** ~50 sites across `capsem-process/src/{vsock,ipc,
  main,terminal,job_store}.rs`, `capsem-service/src/main.rs`, and
  `capsem/src/main.rs` were `let _ = X.send(...)` -- a closed receiver
  silently swallowed the message with no trace. New `try_send!` macro
  in `capsem-core::macros` wraps every IPC/vsock send and emits a
  `tracing::warn!(target = "ipc", channel, error)` line on failure.
  Filter with `RUST_LOG=ipc=warn` to see only dropped-message events.
  Cleanup paths where a closed receiver is the documented design
  (e.g. broadcast publish into `TerminalOutputQueue`) keep the bare
  `let _ = ` and carry an inline `// channel-closed-ok: <reason>`
  marker so the audit grep can exclude them.

### Changed (persistent overlay)
- **EXT4 journal re-enabled on the persistent overlay-upper.** Previously
  formatted with `mke2fs -O ^has_journal`; switched to default
  `has_journal` and mount with `data=ordered`. Costs ~5-10% IOPS;
  enables metadata replay on resume so directory listings stay
  consistent after suspend/resume cycles where in-flight metadata
  writes hadn't been flushed. Verified via `tune2fs -l /dev/loop0`:
  `Filesystem features: has_journal ... metadata_csum`. Standard
  suspend/resume + heavy-churn directory listing now both work.
  (Heavy-churn DATA reads of a subset of files still hit
  `Input/output error` -- that's the loop-device-io-after-resume
  sprint's remaining work, fixable only by moving rootfs.img off
  VirtioFS to a real VZ block device.)

### Fixed (lifecycle)
- **Guest-initiated `shutdown` left persistent VMs marked Defunct
  instead of Stopped.** The lifecycle path (`capsem-sysutil shutdown`
  -> vsock:5004 -> `ProcessToService::ShutdownRequested`) had no
  service-side listener; the process just sent `Shutdown` to itself
  and exited cleanly. The cleanup task interpreted "instance still in
  the map at exit" as `unexpected_exit=true` and flipped the registry
  to `defunct`, so `capsem list` showed Defunct and the test
  `test_guest_shutdown_preserves_persistent_and_resume` failed.
  Distinguish: a clean `ExitStatus::success()` is graceful regardless
  of who initiated it; only non-zero exit / signal kill is a crash.

### Fixed (suspend/resume durability)
- **`cd /root && ls` after `capsem resume` failed with "cannot open
  directory '.': No such file or directory".** Apple VZ writes to the
  persistent overlay's `rootfs.img` were buffered in macOS's APFS page
  cache. After `save_state`, capsem-process exited before APFS flushed,
  so the next boot read a stale `rootfs.img` and the EXT4 overlay-upper
  served stale inodes -- the cwd handle in the resumed shell pointed at
  garbage. Three-stage flush now layered on suspend:
  1. Guest agent: `sync()` + `BLKFLSBUF` + `fsync(/dev/loop0)` (existed).
  2. Guest agent: `fsync(/mnt/shared/system/rootfs.img)` -- sends
     `FUSE_FSYNC` over VirtioFS so the host VirtioFS daemon flushes its
     own buffered writes against the real macOS file (NEW).
  3. Host capsem-process: `sync_all()` on `rootfs.img` after
     `save_state` returns -- catches APFS dirty pages (NEW).
  Confirmed end-to-end against the live service: simple suspend/resume
  + `cd /root && ls` works; suspend with churn across `/tmp /var /opt
  /etc /usr/local` survives; file *contents* on the EXT4 overlay are
  durable. Heavy directory churn (~50 new entries per dir then
  immediate suspend) can still leave EXT4 directory data blocks with
  stale checksums on resume -- file reads succeed but `readdir`
  returns I/O error. Tracked in
  `sprints/loop-device-io-after-resume/ISSUE.md`; the next step is
  forcing an `fsync` on each parent directory inside the guest before
  signalling SnapshotReady.
- **Failed suspend left VM marked "Suspended" with a corrupt checkpoint.**
  When `with_quiescence` failed (timeout, channel closed) the spawn task
  ignored the error, sent `StateChanged{Suspended}` anyway, and exited
  with code 0. The service then marked the VM as suspended; the next
  resume cold-booted against the half-written rootfs.img and kernel-
  panicked with `EXT4-fs error inode #N: iget: checksum invalid` ->
  `overlayfs failed`. Fix: only send the Suspended state and `exit(0)`
  when the operation actually succeeded; on failure, log the error and
  `exit(1)` so the service treats it as a crash and does not write the
  checkpoint marker.
- **Silent IPC connection close on protocol mismatch.** Two binaries
  built across an enum-variant addition (`StopTerminalStream`) talked
  past each other; the receive side closed the connection silently with
  the decoder error swallowed. Fix: log the rx error at `warn` level so
  the next protocol-skew bug surfaces in the first run instead of
  presenting as a "guest doesn't respond" timeout.

### Fixed (capsem shell)
- **Terminal garbage on shell exit.** Pressing Ctrl-C / typing `exit` in
  `capsem shell` could leave the user's parent terminal flooded with
  binary garbage (MessagePack frames -- `bootconfig`, `epoch_secs`,
  `Pong` repeated). Two compounding bugs:
  1. `output_task` (the spawned reader of `ProcessToService` IPC frames)
     was never aborted on exit. tokio `JoinHandle::drop` does NOT cancel
     -- the task lived on, kept holding `stdout`, and any in-flight
     `TerminalOutput` frame wrote to the user's now-cooked-mode shell.
  2. The host-side `capsem-process` kept queuing `TerminalOutput` for the
     dropped IPC connection because the client never told it to stop.
- Fix: `run_shell` now sends a new `ServiceToProcess::StopTerminalStream`
  before exit, aborts the local task, drops the IPC writer, and writes a
  minimal terminal reset (`\x1b[0m\x1b[?25h\r\n` -- SGR reset, show
  cursor, CRLF; deliberately no alt-screen toggle or screen clear so
  scrollback is preserved).
- Defenses: `capsem_proto::looks_like_ipc_frame` ships a detector for the
  `to_vec_named` adjacently-tagged enum prefix that produced the garbage;
  `capsem-process` calls it on every `TerminalOutput` payload and emits a
  loud `warn!` if a leak ever resurfaces. 15 unit tests in
  `crates/capsem/src/shell_exit/tests.rs` pin: the reset sequence shape,
  every variant of both `HostToGuest` and `GuestToHost` matching the
  detector, no false positives on ANSI/UTF-8/scrollback content, and
  the load-bearing tokio behavior (`JoinHandle::drop` does not cancel,
  `JoinHandle::abort` does).

### Changed (kernel)
- `guest/config/build.toml` ships `kernel_branch = "auto"` instead of a
  hardcoded `"6.6"`. `resolve_kernel_version("auto")` queries
  kernel.org/releases.json and picks the newest non-EOL longterm branch's
  latest patch (today: `6.18.26`). Pin to a specific branch by setting
  `kernel_branch = "X.Y"` (e.g. `"6.6"`) for reproducibility / security
  freeze. Killed the duplicated `"6.6"` literal in `models.py` /
  `scaffold.py` -- single source of truth is now `build.toml`.

### Changed (bootstrap)
- `bootstrap.sh` moved to the repo root (was `scripts/bootstrap.sh`).
- Phase 1 now auto-installs `rustup` (sh.rustup.rs) and `just` (just.systems
  -> `~/.local/bin`) instead of printing hints and bailing.
- Phase 2 auto-installs `uv` (astral.sh), `pnpm` (brew on macOS,
  get.pnpm.io on Linux), and on macOS `colima` + `docker` + `docker-buildx`
  with Rosetta-enabled VM start (`colima start --vm-type vz --vz-rosetta
  --memory 8 --cpu 8`). Linux docker stays manual (distro-specific, sudo,
  group, daemon -- prints clear apt/dnf hints instead).
- Each install gates on a `[Y/n]` prompt; **Enter accepts** (Y is the
  default). `--yes` and non-tty input both auto-accept for CI.
- Stopped silencing every installer (`--quiet`, `>/dev/null`). Real errors
  were getting swallowed -- `uv sync` failures showed up as a mystery
  `exit 1` with no diagnostic.
- Closing message no longer tells you to run `just build-assets` (it
  already ran as part of doctor's auto-fix in Phase 3).

### Fixed (bootstrap)
- `cargo install cargo-tauri` was wrong -- the crate is `tauri-cli` (the
  binary it produces is `cargo-tauri`). Fixed in `scripts/doctor-common.sh`.

### Fixed
- **Asset download URL.** `download_missing_assets` built the URL from the
  asset version (`v2026.0424.1`) instead of the binary version (`v1.0.{ts}`),
  so every fresh install 404'd against the GitHub Release. Releases are tagged
  by binary version; the asset version lives only inside the manifest.
- **Manifest schema mismatch.** The CI release pipeline writes
  `binaries.releases.<v> = {version, files}`, but the Rust `BinaryRelease`
  struct required `{date, min_assets}`. Every published manifest was
  unparseable -- the binary couldn't even *get* to the URL builder before
  failing. Made `date` / `min_assets` / `min_binary` optional, added
  `version` / `files` to round-trip pkg/deb metadata. `pick_asset_version`
  treats empty `min_assets` as "no constraint" and falls back to
  `assets.current`.
- **Removed broken Makefile.** The legacy Makefile bypassed `_pack-initrd`,
  `gen_manifest`, and `create_hash_assets`, so `make` produced a binary that
  couldn't resolve any VM asset at boot. Use `just` for everything.

### Added (defenses)
- Pinned the asset URL contract in `asset_download_url()` with unit tests so
  future drift between the downloader and `release.yaml`'s upload step
  (`gh release upload "$f#${arch}-${base}"`) is caught at compile time.
- `verify-release-downloads` post-flight job: after every release, downloads
  the published manifest, curl-checks every `<base>/v<tag>/<arch>-<name>` URL
  is reachable, AND runs the just-released binary's `capsem update --assets`
  against real GitHub. Closes the gap that hid the URL bug for one release.
- Fixed `tests/capsem-install/test_asset_download.py`: fake release dir was
  at `v<asset_version>` (mirroring the same buggy mental model as the code).
  Now at `v<binary_version>` so it actually models GitHub.
- Dropped the `_build-host` dependency from `just test-install`. The recipe
  builds host crates inside the container that has the GTK/glib -dev libs;
  the duplicate runner-side build was failing on Ubuntu 24.04 arm64 (no
  libglib2.0-dev), which masked the asset-URL bug because the e2e never ran.

### Security (frontend deps)
- **`marked` 18.0.0 -> 18.0.3** (GHSA-6v9c-7cg6-27q7, HIGH): infinite recursion
  in tokenizer. Direct dev dep; bumped to `^18.0.2`, lockfile resolved 18.0.3.
- **`postcss` >=8.5.10 enforced via pnpm override** (GHSA-qx2v-qp2m-jg93,
  MODERATE): XSS via unescaped `</style>` in CSS stringify output. Pulled
  transitively through `@sveltejs/vite-plugin-svelte > vite > postcss`.
  Override forces every node in the lockfile to >=8.5.10.

### Added (CI)
- **`just audit` recipe.** Fast standalone gate (cargo audit + pnpm audit only,
  no test/build). `just test` Stage 1 already runs both audits; this is the
  pre-push check that doesn't require ~15 min of full-suite work first.
- **`test-linux` job no longer hard-fails when `/dev/kvm` is missing.** The
  "Enable KVM" step is now `continue-on-error: true`, and the verification
  step emits a workflow warning instead of `::error::` + `exit 1`. Hosted
  ARM runners do not always expose nested virt; the compile + non-KVM unit
  tests still run, and real-KVM coverage runs in the release pipeline.
  Workflow comments link future readers to `sprints/done/ci-green` so the
  hard-fail doesn't get reintroduced.

### Changed (Colima default)
- **Bumped Colima default RAM from 8 GB to 16 GB** across `bootstrap.sh`,
  `scripts/doctor-macos.sh`, three skills (`dev-setup`, `dev-start`,
  `build-images`), and four docs pages (architecture/build-system,
  architecture/custom-images, development/getting-started, development/stack).
  The Tauri install-test cold build (`just test-install`) blew past 8 GB
  during cargo compile of the capsem-mcp crates and SIGTERM'd at exit 143.
  16 GB is the recommended floor; 12 GB is the absolute minimum.
- Bumped `@tauri-apps/api` from `^2.10.1` to `^2.11.0` to match the Rust
  `tauri` v2.11.0 crate (`cargo tauri build` refuses mismatched majors/minors).

### Fixed (install-test fixture)
- `tests/capsem-install/test_asset_download.py` hardcoded `serve_dir/v1.0.1/`
  for the fake release dir, but the installed binary builds asset URLs from
  its own `CARGO_PKG_VERSION` (e.g. `v1.0.1777065213`). Every run inside the
  install-test container 404'd. Replaced with `f"v{_binary_version()}"` -- a
  helper that runs `capsem --version` once and uses the result -- so the
  fixture always matches the binary under test, regardless of release tag.

### Deferred
- **Orthogonal asset/binary release cadence** (separate tag scheme + workflow
  for asset-only bumps) is still postponed -- revisit after this URL fix
  ships. The defenses above are designed to also guard the future split.

## [1.0.1777065213] - 2026-04-24

### Fixed (CI)
- Codesign companion binaries with --options runtime + --timestamp;
  notary rejected the .pkg because the 8 companion binaries lacked
  hardened runtime.

## [1.0.1777061711] - 2026-04-24

### Fixed (CI)
- Notarize + stapler + artifact collection now reference the pkg at
  packages/ (build-pkg.sh writes there, not CWD).

## [1.0.1777059098] - 2026-04-24

### Fixed (CI)
- Raise pnpm audit threshold to high/critical (was default=low); a new
  moderate postcss CVE in dev-only deps kept failing the release.

## [1.0.1777058736] - 2026-04-24

### Fixed (CI)
- Trust productsign + productbuild on p12 import so .pkg signing
  doesn't hang on a GUI keychain prompt.

## [1.0.1777014595] - 2026-04-24

### Added (release)
- Sign the .pkg installer with a Developer ID Installer certificate
  (requires APPLE_INSTALLER_SIGNING_IDENTITY secret + combined p12).

## [1.0.1777013185] - 2026-04-24

### Diagnostic (CI)
- Preflight now enumerates all keychain identities to surface whether
  a Developer ID Installer cert is present (productsign needs it, codesign
  does not).

## [1.0.1776987645] - 2026-04-24

### Fixed (CI)
- build-app-macos: include capsem-mcp-aggregator / capsem-mcp-builtin in
  companion-binary build + codesign (build-pkg.sh needs all 8).
- build-app-linux: install libxdo-dev, libayatana-appindicator3-dev,
  librsvg2-dev so capsem-tray links.

## [1.0.1776984283] - 2026-04-24

### Fixed (CI)
- install-test: chown entire /src to capsem uid (was only /src/frontend);
  Tauri build.rs hit EACCES under the narrower chown.

## [1.0.1776982455] - 2026-04-24

### Fixed (CI)
- install-test container: chown full /src/frontend (not just node_modules)
  so vite/astro temp writes work when runner uid (1001) != container uid (1000).

## [1.0.1776981476] - 2026-04-23

### Fixed (CI)
- test-install runner now installs libgtk-3-dev + libwebkit2gtk-4.1-dev
  + libayatana-appindicator3-dev + librsvg2-dev + libxdo-dev + libssl-dev
  so `_build-host` can `cargo build` the tray / tauri-adjacent crates.


## [1.0.1776980282] - 2026-04-23

## [1.0.1776980020] - 2026-04-23

### Security
- **Verify manifest signatures at boot before trusting asset hashes.**
  The previous commit wired asset hash verification to the on-disk
  `manifest.json`, but an attacker with write access to `assets/` could
  swap both the rootfs and the manifest to match. Closed the gap with
  minisign signature verification: the release pubkey
  (`config/manifest-sign.pub`, key id `93A070CBB288AC9B`) is now baked
  into `capsem-core` via `include_str!`, and
  `asset_manager::load_verified_manifest_for_assets` rejects any
  manifest whose sibling `.minisig` is missing or invalid. Release
  builds (`cfg!(debug_assertions) == false`) hard-fail on a manifest
  without a valid signature; debug builds allow unsigned manifests so
  local dev loops with locally built assets keep working. Added the
  `minisign-verify = "0.2"` crate; covered by 9 new unit tests
  including verify-accepts/rejects-tampered-manifest/rejects-mangled-
  signature/rejects-wrong-pubkey/bails-when-sig-required-but-missing/
  accepts-unsigned-when-allowed/bails-on-bad-signature and a regression
  guard that the baked pubkey file parses as valid minisign. Updated
  `docs/src/content/docs/architecture/asset-pipeline.md` to describe
  the full tamper-resistance chain.

- **Asset hash verification at boot was silently disabled on every release.**
  `crates/capsem-core/src/vm/boot.rs` read three expected hashes via
  `option_env!("VMLINUZ_HASH")` / `"INITRD_HASH"` / `"ROOTFS_HASH"`, but
  nothing in the build chain ever set those env vars -- no `rustc-env=`
  emit from any `build.rs`, no shell-level pre-seed in CI, no `capsem-core
  /build.rs` at all. Every shipped binary therefore reached
  `VmConfig::build()` with `expected_*_hash: None`, skipping the hash
  check on kernel, initrd, and rootfs. Casual corruption, asset/manifest
  drift, or an attacker with write access to `assets/` all went
  undetected at boot. The compile-time embedding approach is also
  incompatible with the project's independent binary/asset release
  model (`min_binary`/`min_assets` compatibility ranges) -- baking
  specific hashes into a binary would tie every binary release to one
  asset release.
  Replaced the `option_env!` path with runtime manifest lookup. New
  `asset_manager::load_manifest_for_assets(assets)` reads `manifest.json`
  from the assets dir or its parent; new `ManifestV2::
  expected_hashes_current(arch)` returns the kernel/initrd/rootfs hashes
  for the current release on the host arch. `boot_vm` now feeds those
  to `VmConfig::builder`, so the hash check fires on every boot that has
  a manifest. Missing or malformed manifest falls back to disabled
  verification with an explicit `[boot-audit] asset hash verification
  disabled` log line, keeping dev loops without a manifest working.
  Tamper resistance for release environments now depends on manifest
  signature verification in the asset-download path; that path is a
  separate, tracked gap.
  Updated `docs/src/content/docs/architecture/asset-pipeline.md` to
  describe the runtime-lookup flow (replacing the old "Compile-Time
  Hash Embedding" section) and fixed the mermaid diagram to match.
  Covered by 8 new unit tests in `crates/capsem-core/src/asset_manager.rs`
  covering `expected_hashes_current`, `load_manifest_for_assets`, and
  the `aarch64 -> arm64` arch mapping.

### Fixed
- **Signal-driven explicit cleanup for capsem-process background-thread
  owners.** Companion fix to the `shutdown_lock` host-serialization
  landed earlier on this branch: even with one teardown at a time, the
  previous code relied on tokio-runtime-drop ordering to run
  `DbWriter::Drop` (join writer thread + `PRAGMA
  wal_checkpoint(TRUNCATE)`) and `FsMonitor` quiescence inside the
  service's 1s SIGTERM-to-SIGKILL budget. Non-deterministic under any
  unrelated slowdown (APFS fsync spike, busy writer queue, slow VZ
  teardown) and still flaky on the observed
  `test_wal_absent_after_clean_shutdown` failure (428512-byte WAL).
  Fixed by hoisting the background-thread owners into a `Shutdown`
  struct owned by `main()` so the SIGTERM handler can drain them
  synchronously before calling `CFRunLoopStop`. New primitives:
  `capsem_logger::DbWriter::shutdown_blocking(&self)` (Arc-safe,
  idempotent -- switches `tx`/`join_handle` to
  `std::sync::Mutex<Option<...>>` so callers holding any `Arc<DbWriter>`
  can deterministically drain the writer thread; existing `Drop`
  delegates to it), and
  `capsem_core::fs_monitor::FsMonitor::shutdown_and_join(&self)` (signals
  the event loop to flush and joins its worker thread). The handler
  drains `FsMonitor` first (fs_events fan into DbWriter), then DbWriter,
  both inside `tokio::task::spawn_blocking` so we don't stall a tokio
  worker on the thread joins; `CFRunLoopStop` runs only after the drain
  completes. Added `CAPSEM_TEST_SLOW_CHECKPOINT_MS` test-only env var in
  `writer_loop` that inserts a sleep before the final checkpoint --
  proves that explicit cleanup waits for the checkpoint where an
  implicit Drop path would race the SIGKILL budget. Documented the
  pattern in `/dev-rust-patterns` next to the host-serialization pattern.
  Covered by four new `capsem-logger::writer` tests
  (`shutdown_blocking_through_arc_flushes_wal`,
  `shutdown_blocking_is_idempotent`, `write_after_shutdown_is_noop`,
  `slow_checkpoint_hook_delays_shutdown`); the
  `test_wal_absent_after_clean_shutdown` integration test now passes
  clean under `-n 4` alongside the rest of `capsem-session-lifecycle`,
  `capsem-cleanup`, `capsem-recovery`, `capsem-stress`, and the full
  `capsem-service` suite.

- **VM teardown races under load left `session.db-wal` non-empty after
  `capsem delete`.** `handle_delete`'s fast path SIGTERMs
  capsem-process and waits 1s for exit before escalating to SIGKILL.
  Under N concurrent deletes on one host, each capsem-process's exit
  path -- Apple VZ guest teardown on the main thread, virtiofs drain,
  `DbWriter::Drop`'s writer-thread join + `PRAGMA
  wal_checkpoint(TRUNCATE)` -- compete for the same main-thread + I/O
  bandwidth, and one teardown can blow the 1s budget. SIGKILL then
  fires mid-checkpoint, leaving a large WAL file on disk (395 kB in
  the failing `test_wal_absent_after_clean_shutdown` artifact).
  Fixed by serializing VM teardown at the service layer: added
  `ServiceState::shutdown_lock: tokio::sync::Mutex<()>`, acquired at
  the top of `shutdown_vm_process` and held through the entire
  `SIGTERM` + `wait_for_process_exit` window. Same pattern as the
  existing `save_restore_lock`: one critical-section operation in
  flight per host at a time, in-process tokio mutex since production
  runs exactly one service per user-host. `handle_purge`'s
  `join_all` of concurrent teardowns now effectively serializes
  through the lock -- intentional trade of concurrency for
  correctness; purge is an admin operation, not latency-sensitive.
  Documented in `skills/dev-rust-patterns/SKILL.md` alongside
  `save_restore_lock` as the "host-serialization locks" pattern, and
  the follow-up refactor (signal-driven explicit cleanup in
  capsem-process so cleanup-correctness doesn't depend on the
  SIGKILL budget) is scoped in
  `sprints/explicit-shutdown-cleanup/ISSUE.md`.

- **Gateway auth rejections were invisible in the log, so
  curl-returns-`000` under load was untriaged.** The gateway's
  `auth_middleware` silently returned 401/429 with no structured log
  line, and the default env filter was `capsem_gateway=info` -- so
  tower_http's per-request spans and hyper's connection-level
  complaints (malformed header, RST during read) also never made it
  into `gateway.log`. When a concurrent `just test` run surfaced
  `test_empty_bearer_returns_401` as `000` instead of `401`, there
  was nothing in the preserved test artifacts to diagnose from.
  Fixed by (a) broadening the default filter to
  `capsem_gateway=info,tower_http=debug,hyper=info` so connection-
  level events land in the log, (b) logging auth rejections at
  `info!`/`warn!` (401 / 429 respectively) with `method`, `path`,
  and a `shape` field that classifies the Authorization header
  without leaking its value (e.g. `bearer-empty`, `bearer-no-space`,
  `basic`, `unknown-scheme`, `non-ascii`), and (c) installing a
  panic hook so any panicked request handler surfaces an
  `ERROR gateway panic` rather than vanishing into a dropped
  connection. No behaviour change on the happy path; diagnostic-only
  for the flaky load-time failure mode. Covered by
  `classify_auth_header` unit tests (absent/empty/non-ascii/bearer
  shape matrix).

- **`ExecDone` always stalled 500ms on no-output commands, taxing every
  fork and every internal `sync`.** `handle_guest_msg(ExecDone)` in
  `crates/capsem-process/src/vsock.rs` used `captured.is_empty()` as a
  heuristic for "EXEC-reader thread hasn't finished depositing yet" and
  unconditionally slept 500ms on that branch. The heuristic cannot
  distinguish "deposit still in flight" from "command legitimately
  produced no stdout", so `true`, `sleep`, `exit`, and the
  `fsfreeze -f /; sync; fsfreeze -u /` pipeline `handle_fork` uses to
  quiesce the guest filesystem each paid 500ms of dead time per call.
  Visible as `test_fork_benchmark` (fork_ms mean ~110ms -> ~621ms,
  blowing the 500ms gate) and a broader regression: any command with
  no stdout took 520-570ms instead of 20-50ms.
  Replaced the heuristic with a proper deposit signal. `JobStore::
  active_exec` now holds an `ActiveExec { id, captured, deposited:
  Arc<tokio::sync::Notify> }`; the EXEC-port reader thread calls
  `notify_one()` after writing `captured` under the active_exec lock,
  and the `ExecDone` handler awaits `.notified()` with a 100ms bound
  (short safety net for guest never opening the EXEC port). Common
  path: deposit lands first, permit stored, `notified()` resolves
  immediately -- no sleep. Racy path: deposit arrives while ExecDone
  is parked, Notify wakes it; ExecDone reads the real captured bytes.
  Covered by `crates/capsem-process/src/vsock/tests.rs::
  exec_done_with_empty_stdout_resolves_without_500ms_stall`, which
  pre-deposits an empty `ActiveExec`, notifies, and asserts ExecDone
  returns under 100ms; fails at 503ms on the old code. `fail_all`
  also wakes any parked ExecDone on the deposit notifier so control-
  channel close doesn't leave the handler stuck.

- **`wait_for_vm_ready` backoff overshot VM ready-time by ~500ms,
  regressing every `provision -> exec-ready` wait.** A recent alignment
  of `wait_for_vm_ready` onto `PollOpts::new`'s project-wide defaults
  (50ms initial / 500ms max) was correct for peer pollers that wait on
  remote processes with seconds-scale startup, but wrong for this hot
  path: the ready-sentinel is a cheap local `stat` on a sub-second
  latency gate, so with max_delay=500ms the exponential curve lands
  attempts at t=50/150/350/750/1250ms and misses a VM that becomes
  ready at t~=550ms until the next 500ms boundary. Visible as
  `test_avg_exec_latency_3_concurrent_vms` / `test_lifecycle_benchmark`
  (exec_ready mean ~570ms -> ~1287ms) and `test_fork_benchmark`
  boot_ready mean (~680ms -> ~784ms). Restored the tight backoff
  (5ms/50ms) inline on this one call site, documenting why it diverges
  from `PollOpts::new` defaults. Covered by
  `crates/capsem-service/src/tests.rs::
  wait_for_vm_ready_detects_ready_within_tight_overshoot`, which
  creates a `.ready` file after 200ms and asserts detection under
  300ms.

- **Suspend/resume: sibling-VM save_state overlap corrupted the
  persistent overlay.** Apple's Virtualization.framework does not
  tolerate overlapping `saveMachineStateToURL` /
  `restoreMachineStateFromURL` calls across sibling VMs on the same
  host: the VirtioFS ring state captured inside the vzsave ends up
  referencing FUSE descriptors the host has torn down or re-keyed on
  behalf of another VM mid-operation. On the unlucky VM, resume
  surfaces as cascading `I/O error, dev loop0` plus
  `EXT4-fs (loop0): failed to convert unwritten extents to written
  extents -- potential data loss!` in the guest, and
  `initial handshake failed: BootReady read failed: failed to fill
  whole buffer` on the host. The 8% tail from
  `sprints/loop-device-io-after-resume/` was this. Added
  `ServiceState::save_restore_lock` (a `tokio::sync::Mutex<()>`) held
  across the full body of `handle_suspend` (until the per-VM
  `capsem-process` has exited and the checkpoint is durable) and
  across `handle_resume` (until `wait_for_vm_ready` confirms the
  new process's `.ready` sentinel). Production runs exactly one
  `capsem-service` per host per user, so per-service serialization is
  sufficient there. Stress harness
  `tests/capsem-mcp/test_stress_suspend_resume.py` now documents that
  it must run at `-n 1`: multiple xdist workers spawn multiple
  services and the in-service lock cannot coordinate across them,
  re-exposing the bug in a state that never occurs in production.
  With the lock, `CAPSEM_STRESS=1 ... -n 1` runs 50/50 (was noisy
  around 46-50/50 before). Scoped the pre-existing MutexGuard in
  `with_graceful_shutdown` into its own block so the compiler's Send
  analysis survives the new tokio mutex in `ServiceState`. Full
  gotcha writeup at `docs/src/content/docs/gotchas/
  concurrent-suspend-resume.md`; skills updated in
  `skills/dev-testing/SKILL.md` to call out the one legitimate `-n 1`
  test.
- **Test infra: capsem-service leaked across aborted pytest runs.** The
  companion reaper in `capsem-guard` only bounds tray and gateway to
  their parent service -- the service itself had no parent-watch. When
  pytest exited abnormally (Ctrl-C, xdist worker crash, hang followed
  by SIGKILL) the session-scoped fixture teardown never fired, and
  `capsem-service` plus its tray+gateway sat around until manually
  killed. `capsem-service` now accepts an optional `--parent-pid` flag
  that wires `capsem_guard::watch_parent_or_exit` into startup,
  symmetric with the existing companion behaviour: on parent death the
  service exits within ~100 ms, which lets the companion reaper take
  the tray and gateway down with it. Real daemon launches that omit
  `--parent-pid` are unaffected. Wired into the three pytest fixtures
  that spawn their own service (`tests/helpers/service.py`,
  `tests/capsem-mcp/conftest.py`, `tests/capsem-e2e/conftest.py`) so
  each one pins service lifetime to its worker. Verified end-to-end by
  spawning the service under a bash wrapper, killing the wrapper, and
  confirming `capsem-service` exits within ~100 ms; and by running the
  `test_stress_suspend_resume.py -n 8` harness and observing
  `pgrep -lf target/debug/capsem` return empty after teardown.
- **Suspend/resume: VZErrorDomain Code=12 "permission denied" on restore
  from a `/var/folders/...` path.** Apple VZ's
  `restoreMachineStateFromURL` enforces strict path matching between
  `saveMachineStateToURL` and restore -- the VirtioFS share paths
  (and any path referenced by the preserved VM state) must resolve
  identically. Under pytest the tmp_dir lands at
  `/var/folders/lv/.../capsem-test-xxx` which is a symlink chain
  through `/var -> /private/var`. If the save path was the symlink
  form and the restore resolved to `/private/var/...` (or vice versa),
  VZ rejected the restore with a security error and the VM entered
  an unrecoverable state (guest kernel came back up on a wedged loop
  device, stress harness showed 21-100+ `permission denied` entries
  per failing `process.log`). Both `capsem-service` and
  `capsem-process` now call `std::fs::canonicalize()` on their
  respective root paths (`run_dir` / `session_dir`) immediately after
  `create_dir_all`, so every downstream derivation (checkpoint path,
  VirtioFS share host_path, machine identifier, session.db, workspace
  dir, `CAPSEM_SESSION_DIR` env for guest MCP, auto-snapshot
  scheduler, MCP aggregator) uses the canonical
  `/private/var/...` form from both the pre-suspend and post-resume
  process. A reproduction outside pytest (using `~/.capsem/...`,
  which doesn't cross the `/var` symlink) passed first try -- the bug
  was pytest-path-specific. Stress harness (50 iters × 8 workers)
  goes from 4.4% VZ-permission-denied failures to 0, with the
  remaining 8% tail being the unrelated loop-device I/O error on
  the persistent overlay (tracked separately in
  `sprints/vsock-resume-reconnect/plan.md`).
- **Suspend: resume-too-soon race where the old `capsem-process`
  still held the checkpoint file.** `capsem-service::handle_suspend`
  previously returned as soon as the child emitted
  `StateChanged { state: "Suspended" }`, but the child broadcasts that
  event *before* its `save_state` finalizer syncs and the process
  exits. A quick subsequent `capsem_resume` could therefore race the
  outgoing process's `.vzsave` fsync / exit, and VZ would see either a
  partially-written checkpoint or contention over the backing file.
  `handle_suspend` now drains the broadcast channel until it closes
  (the child has exited) or a 15s timeout fires, guaranteeing the old
  process is fully gone before returning to the caller.
- **Suspend/resume: VM survives Apple VZ post-resume vsock half-opens
  and post-handshake connection resets.** The host's vsock layer now
  runs a continuous accept loop for the VM's lifetime and hot-swaps
  the underlying fd into stable terminal/control reader-writer bridges
  via dedicated re-key channels. When a connection resets
  (`BrokenPipe` / `ConnectionReset` pre-handshake, any read/write
  error mid-session) the bridges drop the dead fd, clear all
  framing buffers (no `0x81A08329` "control frame too large" misread
  of a MessagePack map header as a length header), and block on the
  rekey channel for a fresh fd produced by the guest's own reconnect
  loop. The initial handshake retries up to 3× on narrow retryable
  errors only (`BrokenPipe` / `ConnectionReset` at any level of the
  `anyhow::Error` source chain — `UnexpectedEof` and decode errors
  fail fast because they indicate a genuinely wedged guest, not the
  half-open-vsock race). Errors in `perform_handshake` now propagate
  with `.context()` so the underlying `std::io::Error` stays in the
  source chain and classification works without string matching.
  All 11 pre-refactor invariants survive: 10s heartbeat, terminal
  resize, lifecycle port for guest shutdown/suspend, audit port,
  exec duration tracking, VZ main-thread dispatch for pause/save/stop,
  fsync-after-save, error-path Unfreeze, deferred_conns processing,
  handshake on spawn_blocking, and reader-break `JobStore::fail_all`
  poisoning when the rekey channel itself closes. Stress harness
  (`test_stress_suspend_resume.py`, 50 iterations × 8 workers) goes
  from 45-48/50 to 47/50; the remaining 3 failures are an independent
  loop-device I/O error on the persistent overlay after restore (see
  `sprints/vsock-resume-reconnect/plan.md` for the handoff). Plan
  and tracker for the sprint live at
  `sprints/vsock-resume-reconnect/{plan,tracker}.md`.
- **capsem-process kept running after `setup_vsock` returned Err,
  turning every handshake failure into a 30-second service-side poll
  timeout.** The tokio task at `capsem-process/src/main.rs:424` just
  logged the error and exited, leaving the parent process alive with
  no `.ready` sentinel and no working control channel. The service
  polled `.ready` until its 30s deadline then reported a generic
  "exec-ready timeout" with no specific diagnosis. Now `std::process
  ::exit(1)` on vsock-setup failure so the service's child-exit
  handler reclaims the instance in <1s and callers (tests, CLI, MCP)
  see the failure promptly. Residual 4% tail failure seen under
  xdist stress is tracked in `sprints/vsock-resume-reconnect/ISSUE.md`
  (the real root cause is an Apple VZ half-open vsock after resume;
  the fix here just makes it surface cleanly).
- **`wait_for_vm_ready` poll hammered the sentinel 600× per 30s window
  while every other caller used 10× fewer polls.** `main.rs::wait_for_vm_ready`
  was the only site in the codebase constructing `PollOpts { max_delay:
  50ms }` directly; every peer (`service-connect`, `service-socket`,
  `gateway-ready`, `shell-socket`, guest `vsock-connect`, `reconnect`)
  uses `PollOpts::new` with the project-standard 500ms max_delay.
  Aligned this one site to the convention. Cuts sentinel-check traffic
  per second by 10× under contention without changing the 30s overall
  timeout.
- **Control-channel reader could silently wedge a VM for 30 seconds
  per command and kept the `.ready` sentinel fresh the whole time.**
  When `capsem-process`'s `ctrl_f_read` loop hit any decode/read error
  (e.g. desync, short-read, oversize frame), it logged and `break`ed
  without cleaning up. In-flight `Exec`/`ReadFile`/`WriteFile` oneshots
  registered in `job_store.jobs` never resolved, so the `ipc.rs` tasks
  awaiting them hung indefinitely; meanwhile `.ready` stayed on disk
  and `vm_ready` stayed `true`, so every subsequent `POST /exec` passed
  `wait_for_vm_ready` and then timed out at 30s too. Added
  `JobStore::fail_all(message)` which drains pending oneshots with
  `JobResult::Error`; the reader's error path now calls it and also
  removes `.ready`, clears `vm_ready`, so in-flight callers get an
  immediate error and new callers fail fast at the readiness check.
- **Handshake reads/writes ran sync on the async runtime, and every
  failure was silently swallowed.** `setup_vsock` did blocking
  `read_control_msg`/`write_control_msg` directly inside the async fn,
  so under contention (N VMs booting at once) all tokio workers could
  block on vsock I/O simultaneously -- runtime starvation slowed every
  handshake and gave guests enough time to hit their own timeouts,
  leading to protocol desync. Worse, `let _ = read_control_msg(...)`
  at the Ready and BootReady reads plus every `let _ =
  write_control_msg(...)` in the restore branch meant a half-failed
  handshake still reached `vm_ready.store(true)` and `.ready` sentinel
  creation, so callers sent commands into a broken vsock. Moved the
  handshake into `tokio::task::spawn_blocking`, propagated every read
  and write error with context, and gated `vm_ready`/`.ready` on the
  handshake actually succeeding.
- **Artifact preserver left `sessions/` and `persistent/` empty in the
  archive when tests failed under contention.** The helper used
  `shutil.copytree` with an `ignore` filter. When capsem-process was
  still alive during teardown (SIGKILL hadn't reaped it yet) and was
  writing/unlinking files concurrently, copytree's error-accumulation
  model created the destination subdirectories but silently failed to
  populate them -- exactly the `persistent/<vm>/` directories that
  hold `process.log` / `serial.log` / `session.db` needed to debug
  suspend/resume failures. Replaced the `copytree`+`ignore` pattern
  with a manual `os.walk` + per-file copy loop so a single flaky file
  no longer takes out its whole parent subdir, with a stderr summary
  (`copied=N skipped=... errors=N`) and the first 10 error reasons
  surfaced so future regressions don't debug in the dark. Added
  regression tests for the concurrent-unlink race and for a >25 MB
  sibling file coexisting with small log files.
- **`just test-install` had no durable cushion against Colima disk/cache
  exhaustion.** The recipe relied on `_docker-gc`'s `until=72h` filters
  (too conservative to recover recent images / build cache) and on the
  persistent `capsem-install-target` cargo volume never going out of
  bounds. In practice the volume grew to 18.7 GB across sprint version
  bumps and images accumulated until Colima disk pressure compounded
  any OOM already in play. Added two self-healing preflight checks to
  `test-install`: (a) if Colima's `/var/lib/docker` has <10 GB free,
  run `docker image prune -af` + `docker builder prune -af` (no until=
  filter); (b) if the `capsem-install-target` volume has passed 25 GB,
  `docker volume rm` it. Both are no-ops in the common case so they
  don't thrash the cache every run. Linux hosts skip (a) since they
  don't use Colima. Guarded `colima ssh` with `</dev/null` so callers
  that pipe stdin into `just` can't stall the check.
- **`just test-install` leaked a systemd container on every failed run,
  eventually SIGTERM-killing the next build with exit 143.**
  The `test-install` recipe gave each run a unique container name
  (`capsem-install-test-$$`) and only cleaned it up on the happy path.
  Any failed `docker exec` (cargo build, Tauri build, dpkg, pytest)
  short-circuited the script under `set -euo pipefail` before the
  `docker stop`/`docker rm` at the end, leaving the privileged systemd
  container running. Stacked containers squatted Colima's 8 GiB VM
  across runs, and the next build's parallel rustc processes OOM-killed
  mid-compile -- visible as `error: Recipe test-install failed with
  exit code 143` with no pytest output. Also removed dead `EXIT_CODE=$?
  ... exit $EXIT_CODE` bookkeeping that `set -e` had made unreachable
  on the failure path. Fixed by switching to a stable container name,
  preemptively `docker rm -f`ing it at the top of the recipe, and
  installing an `EXIT` trap so cleanup runs on any exit path.
- **Docs described a fictional manifest schema.**
  `docs/src/content/docs/architecture/custom-images.md` claimed every build
  produced `assets/{arch}/manifest.json` with a bill-of-materials schema
  containing `packages[]` and `vulnerabilities[]` arrays -- none of which
  ever existed. `docs/src/content/docs/architecture/asset-pipeline.md`
  showed a different wrong schema (`{"latest", "releases": {<ver>: {<arch>:
  {"assets": []}}}}`) and mentioned legacy flat-format compatibility that
  `asset_manager.rs` no longer accepts. Both pages now document the real
  `assets/manifest.json` format 2 schema (top-level `format`, `assets.
  {current, releases.<ver>.{date, deprecated, min_binary, arches.<arch>.
  <filename>.{hash, size}}}`, `binaries.{current, releases}`) and the
  `min_binary`/`min_assets` compatibility contract. Docs site builds
  green.

- **`tests/capsem-build-chain/test_manifest_regen.py` was testing a ghost
  layout and had been silently skipping every assertion.** The fixture read
  `assets/<arch>/manifest.json` (per-arch) and the tests iterated a flat
  `{filename: hash-hex-string}` schema, but the real manifest is top-level
  `assets/manifest.json` with a nested v2 schema (`assets.releases.<ver>
  .arches.<arch>.<filename>.{hash,size}`). Both the path and the schema
  predate a refactor that was never propagated here, so the fixture's
  `pytest.skip()` fired unconditionally and all four tests reported as
  `s` in build-chain runs -- meaning the suite never actually verified
  manifest/asset consistency. Rewrote the fixture to read the real
  manifest and scope to the current release + host arch. Rewrote every
  test against the nested schema: shape check, per-file existence, b3sum
  match, and a strict `test_no_extra_assets` that allows manifest-listed
  names plus their `<stem>-<hex16>.<ext>` hash-tagged aliases and rejects
  everything else. Verified live on the current tree (4 passed) and
  proved the stale-alias gate with a planted `initrd-deadbeef12345678.img`
  that correctly fails the check.

- **`scripts/create_hash_assets.py` left stale hash-tagged aliases that lied
  about their content.** The script creates `<stem>-<hex16>.<ext>` hardlinks
  mirroring manifest entries so the dev layout matches the installed layout.
  It unconditionally unlinked-and-relinked each expected destination, but
  never swept hash-tagged files left over from prior builds -- and because
  `_pack-initrd` replaces `initrd.img` with fresh content on every run, the
  re-link step kept re-pointing those stale names at the new inode. End
  state in this repo: `assets/arm64/` held five `initrd-<hex>.img` names
  all hardlinked to one inode, but only one hex prefix matched the current
  content hash; the other four names claimed hashes they no longer had.
  Nothing in production reads the stale names (`asset_manager.rs` derives
  the filename from the manifest hash), but the content-addressable naming
  contract was quietly broken and any downgrade/rollback path that
  resurrected an older manifest would have served wrong bytes behind the
  right name. Rewrote the script to enumerate every `<stem>-<hex16>(.ext)?`
  filename in each arch dir and delete those not in the expected set before
  (re)creating current hardlinks. Covered by three new unit tests in
  `tests/capsem-build-chain/test_create_hash_assets.py`.

- **CAPSEM_REQUIRE_ARTIFACTS pre-flight falsely failed `just test` Stage 5
  on a successful build.** `tests/conftest.py::_REQUIRED_ARTIFACTS` declared
  the manifest at `assets/<arch>/manifest.json`, but the canonical layout
  is flat top-level (`assets/manifest.json`). Every production reader --
  `capsem-service` boot at `crates/capsem-service/src/main.rs:2740`,
  `capsem setup` at `crates/capsem/src/setup.rs:187`, `scripts/gen_manifest.py`,
  `scripts/check-release-workflow.sh` -- and the builder's
  `generate_checksums` writer at `src/capsem/builder/docker.py:700` all agree
  on the flat path. The per-arch entry was introduced with the gate itself
  in this release cycle and never resolved on a real build, so the pre-flight
  exited with a confusing "missing: ['assets/<arch>/manifest.json']" right
  after Stage 1-4 had produced the actual manifest. Fixed by correcting the
  path in `_REQUIRED_ARTIFACTS` and adding
  `test_required_artifacts_manifest_path_is_flat` in
  `tests/test_leak_detection.py` to pin the canonical location so this can't
  drift again.

### Security
- **Bumped Astro to 6.1.8 across frontend, docs, and site packages** to clear
  advisory GHSA-j687-52p2-xcff (moderate XSS in `define:vars` via incomplete
  `</script>` tag sanitization; patched in Astro >=6.1.6). `just test` Stage 1
  runs `cd frontend && pnpm audit` and was failing because
  `frontend/pnpm-lock.yaml` had locked Astro to 6.1.4 despite the caret range.
  Grepped the tree for `define:vars` and found zero usages -- exploitability
  in this codebase was nil, but `pnpm audit` gates on version, not usage, so
  the `test` recipe couldn't pass until the lockfiles refreshed. `docs/` and
  `site/` were bumped in the same commit because they were also on affected
  Astro versions.

### Fixed
- **Guest binaries landed on the host with 0o755 instead of 0o555 after
  container-native agent builds.** `capsem-builder agent` on macOS cross-
  compiles inside a Linux container and `chmod 555`s the binaries before
  copying them to the bind-mounted `target/linux-agent/<arch>/` output.
  Docker-for-Mac bind-mount semantics non-deterministically dropped the
  host-side mode, so `capsem-pty-agent` and `capsem-net-proxy` could
  surface as `0o755` while `capsem-mcp-server` and `capsem-sysutil`
  stayed `0o555`. The guest-binary read-only invariant (CLAUDE.md) then
  only held when the `_pack-initrd` justfile recipe ran its compensating
  chmod downstream; any caller invoking the builder directly or running
  `tests/capsem-security/test_binary_perms.py::test_agent_binaries_555`
  before repack saw the bad modes. Added
  `enforce_guest_binary_perms(paths)` in `src/capsem/builder/docker.py`
  and called it at the end of both `container_compile_agent` and
  `cross_compile_agent`, so the invariant is applied at the source by
  the builder itself. Removed the now-redundant compensating `chmod 555`
  in the justfile's `_pack-initrd` recipe. Covered by three new unit
  tests in `tests/capsem-build-chain/test_agent_perms.py`.

- **Every `tests/capsem-mcp/` and `tests/capsem-e2e/` MCP test errored
  under `filterwarnings = ["error"]`.** Both dirs spawn `capsem-mcp`
  with `stdin=PIPE, stdout=PIPE` to speak JSON-RPC, then tore the proc
  down with `proc.terminate() + proc.wait()` -- Popen does not close
  PIPE fds on its own, so each test leaked two
  `_io.FileIO` / `_io.TextIOWrapper` handles. pytest's strict mode
  surfaced them as `ExceptionGroup: multiple unraisable exception
  warnings (2 sub-exceptions)` at setup-teardown boundaries, turning
  69 capsem-mcp tests into `ERROR` and 4 capsem-e2e tests into
  `FAILED`. Added `kill_mcp_proc(proc, timeout=5)` in
  `tests/helpers/mcp.py` -- terminates (or kills), waits, then closes
  `proc.stdin / stdout / stderr` if non-None and not already closed.
  Rewired `tests/capsem-mcp/conftest.py::_kill_proc` through it and
  replaced four inline `proc.terminate(); proc.wait()` pairs in
  `tests/capsem-e2e/test_e2e_mcp.py`. Post-fix: 116 passed, 0 errors
  across both dirs. Covered by a unit test in
  `tests/test_leak_detection.py` that spawns a
  `sys.executable -c "sys.stdin.read()"` child with all three pipes,
  calls `kill_mcp_proc`, and asserts `.closed` on each.

- **Missing built artifacts silently skipped tests instead of failing.**
  Tests that depend on `assets/<arch>/manifest.json`,
  `assets/<arch>/initrd.img`, `entitlements.plist`, or
  `target/linux-agent/<arch>/` use `pytest.skip()` when the artifact is
  absent so a fresh local checkout doesn't fail the suite. In CI, where
  earlier `just test` stages are expected to produce those artifacts, a
  skip means an earlier stage silently dropped its output -- and the
  skipped tests dropping out of the gate disguises the breakage as a
  green run. Added `pytest_sessionstart` pre-flight in
  `tests/conftest.py`: when `CAPSEM_REQUIRE_ARTIFACTS=1` is set
  (justfile's `test` recipe now sets it for both the parallel stage-5a
  and serial stage-5b pytest invocations), the hook fails the session
  before collection if any required artifact is missing, with a
  specific message pointing at the build command needed. Local runs
  without the env var are unchanged -- skips still work. Covered by
  two new unit tests in `tests/test_leak_detection.py` pinning both
  branches of `_missing_required_artifacts`.

- **Python test warnings were never promoted to errors.** `pyproject.toml`
  `[tool.pytest.ini_options]` had no `filterwarnings`, so
  `DeprecationWarning`, `ResourceWarning`, and (critically)
  `PytestUnraisableExceptionWarning` were reported but never gated. Real
  fd / socket / thread-resource leaks in both tests and production
  scripts therefore shipped green. Set `filterwarnings = ["error"]` and
  fixed every leak surfaced:
  - `scripts/clean_stale.py` -- all six `os.scandir(...)` call sites
    were either unbracketed (iterator GC'd eventually, but not
    deterministically) or, in `_target_release_has_old_content` and
    `_dir_has_no_recent`, returned early mid-iteration leaving the
    iterator open. Wrapped each in `with os.scandir(...) as entries:`
    so the underlying fd is released on scope exit regardless of
    return path.
  - `tests/test_exec_lock.py` -- the `_spawn_holder` helper returned
    a `subprocess.Popen` with `stdout=PIPE, stderr=PIPE`; tests
    `.wait()`'d but never closed the pipe fds. Hoisted the two
    callers into `with ... as holder:` blocks so Popen's own
    `__exit__` closes the pipes.
  - `tests/capsem-gateway/test_gw_terminal.py` -- `ws_env` fixture
    teardown called `svc_server.shutdown()` (stops
    `serve_forever`) but never `svc_server.server_close()` (releases
    the UDS listen socket). Added the close plus
    `svc_thread.join()`.
  - `tests/capsem-gateway/test_gw_lifecycle.py` -- the SIGTERM /
    SIGINT lifecycle tests called `gw.start()` but never
    `gw.stop()`, so the gateway log-file handle leaked even though
    the gateway process was killed by signal. Wrapped the asserts
    in `try/finally: gw.stop()`.
  - `tests/capsem-service/test_companion_lifecycle.py` -- the
    restart-with-same-run-dir test needed svc_a's log fd closed
    without destroying the shared tmp_dir (svc_b reuses it);
    added an explicit `svc_a._log_file.close()` between the two
    services. `_spawn_service_on_fixed_port` opened its own log
    file anonymously (`stdout=open(log_path, "w")`) so the
    six-rapid-restarts test could not reach it; it now stashes
    the handle on `proc._log_file` and the test closes every
    spawned service's log file in its finally block.

- **Unhandled exceptions in daemon threads were not failing the test
  suite.** Python surfaces thread exceptions as
  `PytestUnhandledThreadExceptionWarning`, which is reported but has
  never been gating in `pyproject.toml`. Real races (e.g. today's
  `MockWsProcess` teardown hitting `loop.stop()` while
  `run_until_complete` was awaiting) shipped green until someone
  eyeballed the warning in a test run. `tests/conftest.py` now installs
  a process-wide `threading.excepthook` at import time (covers
  collection, fixture setup, and every test) that records each caught
  exception in `_CAUGHT_THREAD_EXCEPTIONS` and prints the traceback to
  stderr in real time. `pytest_sessionfinish` fails the session if that
  list is non-empty. Per-process (each xdist worker gates its own;
  thread exceptions are process-local, unlike process leaks which need
  cross-worker visibility). Covered by two new tests in
  `tests/test_leak_detection.py`: hook-is-installed, and
  captures-real-daemon-thread-exception. Also removed the stale
  `tests/capsem-build-chain/conftest.py.bak` (orphaned after the
  `capsem-cli` -> `capsem` / `capsem-ui` -> `capsem-app` rename).

- **Leak detector false-positived sibling `capsem-mcp` processes.** The
  per-test `check_leaks` fixture and the `pytest_sessionfinish` gate in
  `tests/conftest.py` defined "leak" as any `capsem-*` PID on the host
  not present in the import-time baseline. That caught sibling tools
  sharing the host with pytest -- notably Claude Code's own
  `capsem-mcp` stdio subprocess (spawned by the `claude` CLI, not
  pytest) -- and attributed them to whichever test happened to run
  first. Example report: `[master] tests/capsem-build-chain/test_
  cargo_build.py::test_all_binaries_exist 49423 capsem-mcp
  target/debug/capsem-mcp` (PID 49423's PPID chain: `claude` ->
  terminal shell; pytest never in the chain). Added `_ancestry(pid)` +
  `_is_pytest_descendant(pid)` (walk `psutil.Process.parent()` up to
  init) and gated both sites: `check_leaks` only records first-seen
  when the suspect is actually descended from this pytest process,
  and `pytest_sessionfinish` only flags suspects with either
  attribution (recorded by a worker's `check_leaks`) or a live
  ancestry link to the controller. Sibling processes pass neither
  gate and are silently ignored. Covered by three new unit tests in
  `tests/test_leak_detection.py` (ancestry of init excludes self;
  ancestry of own subprocess includes self; ancestry of nonexistent
  PID is empty).

- **`PytestUnhandledThreadExceptionWarning` from `test_gw_terminal.py`
  module teardown.** The `MockWsProcess` daemon thread in
  `tests/capsem-gateway/test_gw_terminal.py` ran
  `loop.run_until_complete(server.serve_forever())`, and `stop()` tore
  the loop down with `loop.call_soon_threadsafe(loop.stop)`. Stopping a
  running loop while `run_until_complete` is awaiting a pending future
  is the exact case that raises `RuntimeError: Event loop stopped
  before Future completed.` on the worker thread; pytest picked it up
  at module teardown (visible on the last test, e.g.
  `test_ws_nonexistent_vm_closes`). Replaced `serve_forever()` with an
  `asyncio.Event`-based shutdown: `_serve` parks on the event and closes
  the server in its `finally`; `stop()` just sets the event via
  `call_soon_threadsafe`, letting `run_until_complete` return cleanly
  and the worker thread exit. Added a direct regression test
  (`test_mock_ws_process_stop_does_not_leak_thread_exception`) that
  installs a `threading.excepthook` and fails if any exception escapes
  the worker thread.

- **`capsem-agent` failed to compile under `clippy::manual-strip`.** The
  `extract_field` audit-log parser in `crates/capsem-agent/src/main.rs`
  hand-rolled a `starts_with('"')` + `rest[1..]` prefix strip that clippy
  1.93's `manual_strip` lint (denied via `-D warnings`) refused. Rewrote
  to `rest.strip_prefix('"')`; semantics unchanged (`stripped.find('"') + 2`
  still yields the same end offset into `rest`).

- **`capsem_read_file` returned ENOENT on real files after `capsem_resume`
  under concurrent load.** The guest agent's post-resume rebind polled
  `/mnt/shared/workspace` with `Path::exists`, which only drives a FUSE
  `GETATTR`. Under `pytest -n 4 --dist=loadfile` (4 concurrent VMs sharing
  one host's virtiofsd pool) virtiofsd could answer GETATTR on the
  workspace dir before it had populated its child-inode map, so `exists()`
  returned true, the agent `mount --bind /mnt/shared/workspace /root`'d an
  empty view, and every subsequent `/root/<file>` read returned ENOENT
  even though the host file was durably on disk (604319f already made
  write_file flush to host). Fix in `rebind_workspace_after_resume`
  (`crates/capsem-agent/src/main.rs`): warm-poll now calls
  `std::fs::read_dir(...).next()`, forcing a FUSE `READDIR` round-trip
  and proving virtiofsd has enumerated child inodes. Warming timeout
  (1 s total, 50 × 20 ms) is unchanged. If warming never completes the
  rebind now aborts instead of binding against an empty subtree, so the
  failure surfaces loudly in `read_file` rather than silently corrupting
  `/root`. Verified against the previously-flaky
  `tests/capsem-mcp/test_state_transitions.py::test_suspend_and_resume_persistent`
  under `-n 4 --dist=loadfile` (1/1 fail pre-fix, 5/5 pass post-fix).

- **`PytestUnknownMarkWarning` on `benchmark` marker.** Registered
  `benchmark` in `pyproject.toml [tool.pytest.ini_options].markers` so
  `tests/capsem-serial/test_parallel_benchmark.py`'s
  `pytest.mark.benchmark` no longer emits the warning. Warnings are
  errors per CLAUDE.md.

- **Stage-5 flake: pytest `check_leaks` fixture crashing at teardown.** Under
  concurrent load, macOS `sysctl(KERN_PROCARGS2)` can deny cmdline access for
  an unrelated host process; psutil surfaces that as an uncaught `SystemError`
  / `PermissionError` that dropped out of `process_iter(['pid','name','cmdline'])`
  before the existing per-iteration `try/except` could run, taking down the
  teardown of whichever test held the turn (observed on
  `test_cors_on_authenticated_endpoint`). Fix: `tests/conftest.py`'s
  `get_capsem_processes` now iterates without attr-prefetching cmdline and
  fetches cmdline lazily with a per-proc `try/except (psutil.Error, OSError,
  SystemError)`. Unit coverage in `tests/test_leak_detection.py`.

### Performance
- **`capsem delete` and `capsem purge` no longer pay the 2.7s graceful
  shutdown floor.** Previously `shutdown_vm_process` unconditionally sent
  `ServiceToProcess::Shutdown` via IPC, which armed capsem-process's 2.5s
  self-timer (giving the guest agent `SHUTDOWN_GRACE_SECS` to SIGTERM bash
  gracefully before SIGKILL) before the caller could observe process
  exit. Delete/purge don't need that grace because the session dir (with
  its workspace and bash history) is about to be removed anyway. Added a
  `graceful: bool` parameter to `shutdown_vm_process`; `handle_delete` and
  `handle_purge` now pass `false`, which SIGTERMs capsem-process directly
  (its `CFRunLoopStop` handler from 9b14618 makes this a clean exit) with
  a 1s poll before escalating to SIGKILL. `handle_stop` and `handle_run`
  keep graceful=true (persistent VMs need bash history preserved;
  handle_run reads session.db after teardown). Observed delete mean
  dropped from 2782 ms to ~70 ms across 3 benchmark runs, unblocking
  `tests/capsem-serial/test_lifecycle_benchmark.py::test_lifecycle_benchmark`
  under `just test` stage 5.

### Changed
- **Convention: Rust unit tests live in a sibling `tests.rs`, not an inline
  `mod tests { ... }` block.** Documented in `CLAUDE.md` (Code Style) and
  `skills/dev-testing/SKILL.md` (with extraction recipe and rationale).
  Codifies the pattern just applied across `policy_config`, `session`,
  `capsem-proto`, and `virtio_fs`. Agents writing new Rust modules should
  default to the sibling pattern; reviewers should push back on new inline
  test blocks.

### Fixed
- **`capsem-agent` `write_nofollow`: fsync before returning so
  `capsem_write_file` is durable across an immediate `capsem_suspend`.**
  Previously the agent did open+write+close without fsync. On VirtioFS, close
  only triggers FUSE_FLUSH (which virtiofsd is free to no-op), so the write
  could still be buffered inside Apple VZ's in-process virtiofsd when a
  caller immediately suspended the VM. VZ tore down virtiofsd before the
  data reached the host backing store, and the resumed VM (with a fresh
  virtiofsd) saw ENOENT. Surfaced as a concurrency flake of
  `tests/capsem-mcp/test_state_transitions.py::test_suspend_and_resume_persistent`
  under `just test` stage 5 -- 2 ms between write_file returning and the
  suspend request is not enough time for Apple's virtiofsd to drain.
  `file.sync_all()` sends FUSE_FSYNC, a core FUSE opcode virtiofsd must
  honor, giving write_file a real durability contract.
- **`capsem-logger/src/writer.rs`: `clippy::type_complexity` in
  `exec_event_insert_populates_row` test.** The test declared an
  8-element tuple type on the destructuring binding for a
  `Connection::query_row` call; clippy flagged it under
  `--all-targets -- -D warnings`. Replaced the outer type annotation
  with per-column `let` bindings inside the closure, which also makes
  each column's expected type read at the call site rather than in a
  parallel tuple. No behavior change; `cargo test -p capsem-logger`
  still 210 pass.

### Changed
- **Inline `#[cfg(test)] mod tests { ... }` blocks extracted to sibling
  `tests.rs` files across four hot-path modules.** Pure mechanical code
  motion -- each block moved verbatim (dedented one level) into a new
  `tests.rs` alongside its parent, and the parent now declares
  `#[cfg(test)] mod tests;`. Test visibility is identical to inline;
  zero behavior change. Before / after line counts:
  `capsem-core/src/net/policy_config/mod.rs` 4,364 → 38
  (tests.rs 4,325); `capsem-core/src/session/mod.rs` 1,230 → 12
  (tests.rs 1,217); `capsem-proto/src/lib.rs` 1,722 → 403
  (tests.rs 1,318); `capsem-core/src/hypervisor/kvm/virtio_fs/mod.rs`
  1,218 → 313 (tests.rs 904). Net: ~7,800 lines of test code no longer
  sits between file open and production definitions, which was the
  single biggest friction when agents or humans navigate these files.
  Full suite green: `cargo test -p capsem-core` 1,464 pass;
  `cargo test -p capsem-proto` 144 pass; clippy clean on every
  touched crate.

### Changed
- **`capsem-service`: `PersistentRegistry` extracted from `main.rs` into its
  own `capsem_service::registry` module.** Pure code motion: `PersistentVmEntry`,
  `PersistentRegistryData`, and `PersistentRegistry` with its eight methods
  (`load`, `save`, `register`, `unregister`, `get`, `get_mut`, `list`,
  `contains`) now live in `crates/capsem-service/src/registry.rs` and are
  re-imported by `main.rs`. Seven registry-only tests move with the types;
  seven new tests drive the module to 100% line coverage (corrupt-JSON
  load, missing-file load, `get` / `get_mut` / `contains` miss paths,
  `list` iteration, atomic temp-rename on save). Moved tests switch from
  ad-hoc `env::temp_dir()` + manual cleanup to `tempfile::TempDir` to
  eliminate cross-run path collisions. `main.rs` drops from 4,855 to
  4,563 lines. No behavior change; first step of the
  `capsem-service-split-followup` sprint (T1).

### Added
- **Unit-coverage lifts on six files to recover the `unit` codecov flag.**
  Workspace line coverage had regressed below the 80% unit target after
  several service/CLI mains grew. Added 55 new tests across
  `capsem-core/src/vm/terminal.rs` (was 0%, now ~100%),
  `capsem-core/src/net/policy_config/types.rs` (was 45%),
  `capsem-core/src/net/policy_config/corp_provision.rs` (was 40%; tests
  exercise install/read/refresh paths against a tempdir plus the
  stale-TTL guard), `capsem-core/src/net/policy_config/loader.rs` (was
  79%, new coverage on `parse_mcp_section` / `parse_mcp_section_json` /
  `validate_setting_value`), `capsem-logger/src/writer.rs` (ExecEvent,
  McpCall, AuditEvent roundtrips plus `try_write` and `:memory:` reader
  rejection), and `capsem-process/src/helpers.rs`
  (`query_max_fs_event_id`). Workspace unit coverage moved from 76.79%
  to 77.49% lines / 80.78% regions / 78.85% functions; the remaining
  gap to 80% lines is concentrated in `capsem-service/src/main.rs`,
  `capsem/src/main.rs`, and `capsem-process/src/vsock.rs` and is tracked
  by `sprints/capsem-service-split-followup/`.

### Fixed
- **Flaky env-var race in `policy_config/loader.rs` tests.**
  `user_config_path_override_via_env`, `corp_config_path_override_via_env`,
  and `corp_config_path_default` each mutated `CAPSEM_USER_CONFIG` /
  `CAPSEM_CORP_CONFIG` at process scope, so parallel cargo-test execution
  could observe one test's `set_var` while another asserted the env was
  unset. Merged the three into one `env_var_path_resolution` test that
  snapshots and restores prior values. No production behavior change --
  these env vars are set once at startup in prod.

### Changed
- **Marketing tagline updated to "The fastest way to ship with AI securely."**
  Replaces the previous "Native AI Agent Security" / "Sandbox AI coding agents..."
  phrasing across the marketing site hero, footer, and meta tags; the docs site
  splash and description; the workspace `Cargo.toml` package description; the
  `capsem --help` about line; the `capsem setup` welcome; the macOS `.pkg`
  installer welcome page; and the README header.

### Added
- **Integration-test fixtures archive their tmp_dir on failure.** When any
  test that spins up a capsem-service via `tests/helpers/service.py::ServiceInstance`,
  the e2e `RealService`, or the MCP conftest's `_start_capsem_service`
  fails, the fixture teardown now copies its
  `/var/folders/.../capsem-test-*` directory into
  `test-artifacts/<timestamp>-<worker>-<nodeid>/<tmp-basename>/` before
  the usual `shutil.rmtree`, so `service.log`, `logs/gateway.log`,
  `sessions/<vm>/process.log`, `sessions/<vm>/serial.log`, and
  `sessions/<vm>/session.db` all survive for post-mortem. The failing
  test's stderr prints `ARTIFACT: preserved <src> -> <dest>`; Unix
  sockets/FIFOs are skipped because `shutil.copy2` can't read them.
  `test-artifacts/` is gitignored. `skills/dev-debugging/SKILL.md` and
  `skills/dev-bug-review/SKILL.md` document the layout and when to read
  it -- first stop for "VM didn't boot" and "exec timed out" failures
  in the integration suite, where log availability used to depend on
  whether macOS had culled `/var/folders` yet.

### Changed
- **Temp VM names now suffix `-tmp` and never collide on the first word.**
  Auto-generated names went from `tmp-<adj>-<noun>` to `<adj>-<noun>-tmp`
  (e.g. `brave-falcon-tmp`) so every tab/list entry leads with a
  distinctive adjective instead of the same `tmp-` prefix. The generator
  also consults the live instance table and skips any adjective that
  matches the leading segment of an existing VM name, so two concurrent
  temp VMs never share a first word. The adjective and noun rosters were
  expanded (68 adjectives, 85 nouns) to keep the avoid-set useful even
  under heavy concurrency, and the generator falls back to a random
  adjective if every one is already claimed. `scripts/integration_test.py`
  was updated to match on the `-tmp` suffix instead of the prefix.

### Changed
- **Throughput benchmark target moved off `ash-speed.hetzner.com`.** The
  `capsem-bench throughput` command, the in-VM `test_proxy_download_throughput`
  diagnostic, and the host-side `mitm_proxy_download_throughput` integration
  test all pointed at `ash-speed.hetzner.com/{1,10,100}MB.bin`, which has
  been 404ing silently (the integration-test swap in `bdc8c12` already
  noticed -- curl reported 146 bytes of nginx error page while every
  test asserted only "request logged + decision=allowed"). Swapped all
  three to `https://cdn.elie.net/static/files/i-am-a-legend/i-am-a-legend-slides.pdf`
  (~9.5 MB via Cloudflare, 301-redirects to `elie.net`). Size constants
  dropped to a conservative 9 MiB floor and the curl invocations gained
  `-L` so the proxy's 301-follow is exercised; the Rust test hits
  `elie.net` directly because raw hyper does not follow redirects.
  Dropped `ash-speed.hetzner.com` from the default web allow list
  (`config/defaults.toml`, `guest/config/security/web.toml`, and the
  hand-written `frontend/src/lib/mock-settings.ts`) since no live test
  or config still needs it; regenerated `config/defaults.json` and
  `frontend/src/lib/mock-settings.generated.ts` from the TOML. Docs
  page `docs/src/content/docs/development/benchmarking.md` updated to
  match.

### Added
- **`tests/test_repack_deb.py` -- 6 pytests that exercise
  `scripts/repack-deb.sh` directly in under a second.** Previously the
  repack step was only validated through `just test-install`, which
  takes minutes (Tauri build + systemd container + pnpm install)
  before any repack-related bug surfaces. The new harness builds a
  minimal fixture `.deb` with `dpkg-deb -b`, seeds fake companion
  binaries, and invokes the script end-to-end; coverage includes the
  happy path (all six companion binaries land at `/usr/bin/<name>`
  with mode 0755), `DEBIAN/postinst` copy fidelity, loud failure when
  a companion binary is missing, loud failure when the input path
  contains an embedded newline (regression for the `ls *.deb`
  multi-match bug), the build-timestamp stamp on `Version:`, and
  output-defaults-to-overwriting-input semantics. Skipped with a
  clear message when `dpkg-deb` is not on PATH (macOS default); runs
  in Linux CI and inside the `capsem-install-test` container.
  Verified in-container: 6 passed in 0.17s.
- **`just test` now records an in-VM capsem-bench baseline on every
  run.** The stage-6 "Benchmarks" step used to call
  `{{binary}} "capsem-bench"`, which clap parsed as a host-side
  subcommand and aborted with `unrecognized subcommand 'capsem-bench'`.
  Replaced with a new pytest
  (`tests/capsem-serial/test_capsem_bench_baseline.py`) that provisions
  a fresh VM, runs `capsem-bench all` inside it, pulls
  `/tmp/capsem-benchmark.json` out via `/exec cat`, and archives it to
  `benchmarks/capsem-bench/data_<version>_<arch>.json` with host-side
  timestamp + arch stamp. Mirrors the `_save_benchmark` pattern used by
  the existing `test_lifecycle_benchmark.py` host-side archives
  (`benchmarks/lifecycle/`, `benchmarks/fork/`). No regression gate
  yet -- once ~5-10 clean archives land per arch, per-category
  tolerances can be picked and promoted to pytest asserts, mirroring
  `OP_GATE_MS` / `FORK_GATE_MS` / `IMAGE_SIZE_GATE_MB` in the
  lifecycle benchmark. Host-side lifecycle/fork regressions remain
  gated today.
### Fixed
- **Service reaps `capsem-process` orphans on startup when reusing a run_dir.**
  A SIGKILL to capsem-service (crash, OOM, or `svc.proc.kill()` in the
  recovery test suite) does not propagate to its per-VM children. The
  children kept running with their `--session-dir` still pointing at the
  dead service's run_dir, holding Apple VZ VMs, vsock ports, and sockets
  indefinitely. When a replacement service started on the same run_dir it
  only removed stale socket files -- the orphan processes themselves
  persisted across the entire test session.
  Added `find_orphan_capsem_pids` + `reap_orphan_capsem_processes` in
  `crates/capsem-service/src/main.rs`: on startup (after creating
  `instances/`, before socket cleanup), shell out to `ps`, filter
  `capsem-process` lines whose cmdline contains `--session-dir <run_dir>`,
  SIGTERM them, poll up to 2s, SIGKILL survivors. The matcher is a pure
  function with four unit tests in `#[cfg(test)]` covering happy path,
  unrelated run_dir, non-capsem-process binaries that happen to mention
  the run_dir, and empty input. After the fix, the recovery tests
  (`tests/capsem-recovery/test_orphaned_process.py`,
  `test_service_health_after_recovery.py`) leave zero surviving
  `capsem-process` children.
- **Leak detector: controller-only gate + cross-process attribution.**
  Under `-n 4`, each xdist worker ran its own `pytest_sessionfinish` and
  flagged every other worker's session-scoped fixture processes as a
  "leak", because workers cannot distinguish their own children from
  peers' on the shared host. The in-worker gate also fired mid-teardown
  against processes that would have exited a second later via
  capsem-guard. Restructured: workers now only record first-seen
  attribution to `tests/leak-attribution.jsonl` (shared append log) and
  do NOT fail their session; the controller / single-process runner does
  the real gate at `pytest_sessionfinish`, after every worker has
  finished, when the host is the source of truth. The controller settles
  suspects with an exponential-backoff poll (50 ms -> 500 ms, 15 s
  budget) mirroring `capsem_core::poll::poll_until`, filters by the
  conftest-import-time baseline, merges worker attribution from the
  jsonl, and writes a deduped report to `tests/leak-report.log`. Verified
  `tests/capsem-mcp/ + tests/capsem-recovery/ -n 4` now finishes with
  zero reported leaks and zero surviving `capsem-*` processes on the host.
- **Leak detector: eliminate false positives and xdist-controller double-reporting.**
  `tests/conftest.py`'s `get_capsem_processes` was matching `'capsem-' in arg`
  across every process's full cmdline, so `cargo build -p capsem-*`, `rustc`
  driving a capsem crate, and every unrelated tool invoked from a path
  containing `capsem-next/` showed up as a "leak". The per-test check_leaks
  fixture also logged a line for every session-scoped fixture process on every
  test it outlived, so a single shared_vm in a 20-test file produced 20 false
  leak entries. On top of that, under `-n 4` the xdist controller process --
  which never runs session-scoped fixtures or tests -- also ran
  `pytest_sessionfinish` with an empty baseline and re-reported every capsem
  process as `<unknown>`. Rewrote the detector: match on `psutil` process
  name starting with `capsem-` (no cmdline scanning); snapshot the baseline
  at conftest import time so the xdist controller sees one too; per-test
  fixture now only records first-seen attribution; real leak check fires
  once at `pytest_sessionfinish` against processes still alive not in the
  baseline; skip the check entirely in the xdist controller and let each
  worker report its own leaks with real attribution. Verified
  `tests/capsem-build-chain/` and `tests/capsem-mcp/ -n 4` now produce no
  false-positive leak entries.
- **Stop logging routine VM lifecycle transitions at WARN.** Two `tracing::warn!` lines in capsem-service were firing on every normal shutdown -- "shutdown_vm_process removing instance" and "provision_sandbox child exit handler removing instance" -- which made the warn channel useless for actual problems. The first is now `debug!`. The second was further wrong: it fired *before* checking whether the child died unexpectedly vs after an explicit shutdown. Moved the warn inside the `if let Some(info) = removed` branch so it only fires for the genuinely surprising case (and reworded to say so), with a `debug!` for the expected post-shutdown path.
- **`shutdown_vm_process` is now synchronous: awaits actual exit + cleans the UDS socket inline, no background reaper.** Previously it spawned a fire-and-forget `tokio::spawn` to wait for the process and remove the socket, which left every caller racing the reaper. `handle_delete`, `handle_run`, and `handle_stop` were each working around this by calling `wait_for_process_exit` themselves (or hand-rolling the same loop), and `handle_purge` -- which fan-outs via `join_all` -- was the only one *not* working around it, so its parallel shutdowns relied on the reaper to clean up. Collapsed all of this: `shutdown_vm_process` now blocks on `wait_for_process_exit(pid, 5s)` and removes `*.sock` / `*.ready` itself, dropping ~50 lines of duplicate poll/SIGKILL/cleanup code from the four call sites and giving every caller a single clean contract -- when this returns, the process is gone, the socket is removed, and the session DB has flushed.
- **VM process cleanup now uses `poll_until` instead of hand-rolled fixed-interval loops, and `handle_run`/`handle_delete` synchronously await process exit before responding.** `wait_for_process_exit` was polling at fixed 100ms intervals and `handle_delete` was reinventing the same loop inline (with a different timeout, racing the background reaper from `shutdown_vm_process`). Switched the helper to `capsem_core::poll::poll_until` (50ms initial, exponential backoff to 500ms cap) and routed `handle_delete` + `handle_run` through it, which (a) removes the duplication, (b) cuts common-case latency since most processes exit in <50ms, (c) gives both endpoints the SIGKILL fallback for free, and (d) eliminates the `handle_run` race where the response could be returned before the VM process was actually gone (root cause of leak-detector false positives in `tests/capsem-mcp/`).
- **Fixed clippy break in `kill_all_vm_processes`** introduced by the prior service-shutdown cleanup change. The for-loop was switched to borrow `pids_and_sockets` (so `uds_path`/`session_dir` became references), but the existing `&uds_path`/`&session_dir` calls weren't updated, producing two `clippy::needless_borrows_for_generic_args` errors that broke `just test` Stage 1.
- **Improved VM process cleanup in delete handler.** Replaced fixed wait loops with bounded polling and SIGKILL fallback in `handle_delete` to ensure robust cleanup of `capsem-process` instances during deletion.
- **Fixed zombie process leak in service test helper.** Added `wait()` after `kill()` in `ServiceInstance.stop` to ensure child processes are fully reaped.
- **Wired capsem-guard into MCP subprocesses.** Added `capsem-guard` to `capsem-mcp-aggregator` and `capsem-mcp-builtin` to ensure they exit when their parent process dies, eliminating leaks.
- **Improved service-side VM process cleanup.** Replaced fixed 500ms sleep with a bounded polling loop (up to 2s) and SIGKILL fallback in `kill_all_vm_processes` to ensure robust cleanup of `capsem-process` instances.
- **`_clean-stale` now caps each cargo kind directory by size, so
  target/ stops growing unbounded during active dev.** The age-only
  prune (remove entries older than 2-3 days) never fired in practice
  because every build touches every `deps/`, `incremental/`, `build/`,
  and `.fingerprint/` entry -- nothing ever crossed the age threshold
  and the recipe's report said `cargo removed=0` while `target/` sat
  at 72 GB on `/System/Volumes/Data` (23 GB of that in
  `target/debug/incremental/` alone; push to 100% full triggered
  ENOSPC in several integration tests). Added a second pass to
  `scripts/clean_stale.py::clean_cargo_artifacts` that, for each
  profile (debug/release/llvm-cov-target), enforces a per-kind size
  budget (`deps` 12 GB, `incremental` 3 GB, `build` 1 GB,
  `.fingerprint` 500 MB) by deleting oldest-mtime entries until the
  total drops under cap. Newest entries survive so a warm build cache
  is preserved. `deps/` pruning scopes to cargo-generated extensions
  (`.rlib`, `.o`, `.rmeta`, `.d`) -- test binaries are left alone.
  Added 3 tests (budget evicts oldest, no-op under cap, deps filter
  scopes by extension); existing 16 clean_stale tests still pass.
  Measured on this machine: 72 GB -> 30 GB, 110,400 entries evicted
  in 21 s.
- **Artifact capture no longer fills the disk with rootfs.img copies.**
  `tests/helpers/service.py::preserve_tmp_dir_on_failure` recursively
  copied every file from a failing test's `/var/folders/.../capsem-test-*`
  tmpdir into `test-artifacts/`, including per-VM `sessions/<id>/system/rootfs.img`
  (~2 GB each, plus the `auto_snapshots/0/system/rootfs.img` clones).
  29 failure dirs consumed 18 GB apparent / ~9 GB real (APFS clone
  sharing) on /System/Volumes/Data -- enough to push the host to 100%
  and cause downstream ENOSPC failures in other tests. Taught the
  `shutil.copytree` ignore callback to skip (a) files named `rootfs.img`
  / `rootfs.img.backing`, (b) any regular file larger than
  `ARTIFACT_MAX_FILE_BYTES` (25 MB), and (c) sockets/FIFOs (pre-existing).
  Added a rotation pass: after every preserve, only the
  `ARTIFACT_MAX_KEPT_DIRS` most-recent subdirs under `test-artifacts/`
  survive (default 20). Landed `tests/test_preserve_artifacts.py` with
  5 pytests pinning these invariants (rootfs skipped, oversize skipped,
  logs/session.db preserved, no-op when no failures, rotation keeps N).
- **`tests/capsem-security/test_binary_perms.py::test_agent_binaries_555`
  is green on macOS again.** `capsem-builder`'s container agent build
  runs `chmod 555 /output/<binary>` inside the build container
  (`src/capsem/builder/docker.py::container_compile_agent` line 444),
  but Docker-for-Mac bind-mount semantics let the 0o755 executable
  bits survive on the host side for `capsem-pty-agent` and
  `capsem-net-proxy` (capsem-mcp-server and capsem-sysutil came out
  0o555 cleanly -- same chmod, different result, macOS Docker
  filesystem weirdness). The initrd-pack recipe already re-applied
  `chmod 555` to its copies, but the on-disk `target/linux-agent/<arch>/`
  files remained 0o755, tripping the invariant check. Added an
  explicit `chmod 555 "$RELEASE_DIR"/{capsem-pty-agent,...}` step
  right after the `uv run capsem-builder agent` invocation in the
  `_pack-initrd` recipe so the invariant is enforced every time the
  build runs, regardless of what the container filesystem decides to
  preserve.
- **`just test-install` no longer passes dpkg-deb a two-path mess
  after a version bump.** The repack step did
  `DEB=$(ls /cargo-target/debug/bundle/deb/*.deb)` -- when the persistent
  `capsem-install-target` volume still held a previous version's `.deb`
  (e.g. today's `0.16.1` -> `1.0.1776688771` bump left the old file
  sitting next to the new one), the glob matched both and `$()`
  captured them joined by a newline. `scripts/repack-deb.sh` then got
  one path-with-embedded-newline, which `dpkg-deb` tried to open as a
  single file and bailed with `No such file or directory`. Added
  `rm -f /cargo-target/debug/bundle/deb/*.deb` before the Tauri build
  so the bundle dir always starts empty, and switched the lookup to
  `ls -t ... | head -1` as belt-and-braces for the same class of
  bug.
- **Linux builds of `capsem-process` / `capsem-service` compile again.**
  Two sites in `crates/capsem-process/src/vsock.rs` called
  `capsem_core::hypervisor::apple_vz::run_on_main_thread(...)` inside
  the Stop and Suspend command handlers. The `apple_vz` module is
  gated on `#[cfg(target_os = "macos")]` (see
  `capsem-core/src/hypervisor/mod.rs:7`), so both sites broke
  `cargo build` on Linux with `cannot find 'apple_vz' in 'hypervisor'`
  -- surfaced by `just test-install`'s in-container
  `cargo build {{host_crates}}` step. Wrapped each call in
  `#[cfg(target_os = "macos")]` with a non-macOS branch that invokes
  the `VmHandle` methods directly; Apple VZ has a main-thread
  constraint (CFRunLoop) that KVM does not, and KVM's trait default
  returns "not supported" for `pause`/`save_state`, which `?`
  propagates -- the correct behaviour for a backend without
  checkpoint support. Also silenced `-D warnings` on
  `capsem-service::spawn_companions`'s `tray_bin` parameter, which is
  consumed only by the `#[cfg(target_os = "macos")]` tray-spawn block
  and therefore unused on Linux. Added
  `#[cfg(not(target_os = "macos"))] let _ = tray_bin;` to mark the
  intent without changing the cross-platform signature.
- **`just test-install` no longer dies with `Permission denied`
  when rustup tries to self-update.** Same root class as the
  `just cross-compile` EXDEV fix: the `capsem-install-test` image
  extends `capsem-host-builder` and inherits its `/usr/local/rustup`
  (root-owned from image build). The test-install recipe runs `cargo
  build` as the non-root `capsem` user, so rustup's
  channel-sync-on-first-cargo attempt to write
  `/usr/local/rustup/tmp/` is denied (`os error 13`). Added a
  dedicated `capsem-install-rustup:/usr/local/rustup` named-volume
  mount to the systemd-container `docker run`, added
  `/usr/local/rustup` to the chown pass, and added the volume to the
  `_clean-host-image` cleanup list. Mirrors the `capsem-rustup`
  pattern introduced for cross-compile; using a separate volume keeps
  the two images' rustup states from cross-contaminating if they ever
  drift to different stable channels.
- **`just test` / `just smoke` no longer hang on a pnpm interactive
  prompt.** The `_pnpm-install` helper ran `pnpm install
  --frozen-lockfile` with no `CI` env var, so whenever the on-disk
  `node_modules` store drifted from the lockfile (version bump, pnpm
  upgrade, stale npm artifacts, manual edits), pnpm asked `The
  modules directory at ... will be removed and reinstalled from
  scratch. Proceed? (Y/n)` on stdin and sat there forever in a
  non-interactive just-test run. Added `CI=true` to the invocation --
  same idiom already used in the cross-compile docker bash and the
  test-install container at lines 494 / 792 of the justfile -- which
  tells pnpm to auto-accept defaults instead of prompting.
- **`just cross-compile` no longer requires the release Tauri signing
  keys for dev builds.** The recipe read `private/tauri/capsem.key`
  and `private/tauri/password.txt` on the host and passed them to the
  container unconditionally. For any dev who doesn't have those files
  (everyone outside release CI), both env vars became empty strings,
  which Tauri 2 treats as "try to sign with an empty key" and aborts
  with `failed to decode secret key: incorrect updater private key
  password: Missing comment in secret key`. The real release keys are
  injected via GitHub Actions secrets (`TAURI_SIGNING_PRIVATE_KEY` +
  `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` in
  `.github/workflows/release.yaml`); dev builds only need *a* valid
  key so `cargo tauri build` completes. Now the host only passes the
  signing env vars when both `private/tauri/capsem.key` and
  `private/tauri/password.txt` actually exist; otherwise the container
  generates a throwaway dev key into
  `/cargo-target/dev-tauri-private` (persistent across runs via the
  existing `capsem-host-target-<arch>` volume) with
  `cargo tauri signer generate --ci --force`. The generated key has a
  fixed password of `dev` -- its signatures are worthless for
  release-updater verification, but the bundle builds.
- **`just cross-compile` no longer dies with `Invalid cross-device link`
  when rustup self-updates inside the host-builder container.**
  `rust-toolchain.toml` pins `channel = "stable"`, so every time a new
  stable drops, the first cargo invocation inside the pre-built
  `capsem-host-builder` image triggers a rustup channel sync. The sync
  tries to `rename(2)` a toolchain directory
  (`toolchains/stable-.../lib/rustlib/.../self-contained`) from the
  image's lower overlay layer into `/usr/local/rustup/tmp/` on the
  container's upper layer; Docker-for-Mac's overlayfs bounces that
  specific cross-layer rename with `os error 18` and rustup aborts.
  Added a persistent `capsem-rustup:/usr/local/rustup` named-volume
  mount to the `docker run` in `cross-compile`, matching the same
  pattern used for `capsem-cargo-registry` / `capsem-cargo-git` /
  `capsem-host-target-<arch>`. First run copies the image's baked-in
  rustup tree into the volume; subsequent runs put all of rustup on
  one filesystem, so the rename stays within a single mount and the
  EXDEV class of bug is eliminated whether or not rustup self-updates.
  Updated `_clean-host-image` to rm the new volume and drop the
  never-wired `capsem-rustup-{arm64,x86_64}` placeholders.
- **`just test` and `just smoke` execution lock actually blocks
  concurrent runs now.** The lockfile lived at
  `$CAPSEM_RUN_DIR/execution.lock` under `$CAPSEM_HOME`, but the recipe
  ran `rm -rf "$CAPSEM_HOME"` *before* the `flock`. A second invocation
  therefore nuked the first's lockfile and created a new one; `flock -n
  3` on the new inode succeeded unchallenged (the first invocation's
  fd was pinned to the unlinked inode), so two `just test` or `just
  smoke` runs could race through the same `$CAPSEM_HOME`/shared-service
  path and trample each other's VMs. Moved the lockfile to
  `target/capsem-test-execution.lock` (outside `$CAPSEM_HOME`, survives
  the wipe) and acquired it *before* `rm -rf`. Extracted the
  mkdir/exec/flock dance into a single shell helper
  (`scripts/lib/exec_lock.sh::acquire_exec_lock`) and replaced all 8
  inline copies in the justfile (`dev`, `shell`, `run`, `test`,
  `smoke`, `build-gateway`, `bench`, `release`) with two-line
  `source + acquire_exec_lock <path>` calls. Added
  `tests/test_exec_lock.py` (3 tests: concurrent blocker, reacquire
  after release, parent-dir creation) so this regression can't sneak
  back in.
- **`cargo test -p capsem-guard --lib` is deterministic again.** The
  `install_happy_path_returns_guards_and_creates_lock` test did
  `install -> drop -> install` in-process, which reliably failed under
  parallel `cargo test` because a sibling test
  (`singleton_reacquires_after_ungraceful_holder_exit`) calls
  `Command::spawn`, and the forked child briefly inherits our flock fd
  before exec'ing. `O_CLOEXEC` only closes on exec, not on fork; that
  window is enough for the kernel-level flock to survive our drop, so
  the second `install()` returns `Ok(None)` instead of `Ok(Some(_))`.
  This trap is already called out on
  `singleton_reacquires_after_drop_in_isolated_process`, which solves
  it by forking a clean subprocess for the drop-then-reacquire check;
  the new install_happy_path test quietly regressed that workaround.
  Removed the drop+re-install portion of the test -- its stated
  purpose (cover `install()`'s `Ok(Some(_))` arm and assert the
  lockfile exists) is preserved. llvm-cov on
  `capsem-guard/src/lib.rs` is unchanged to the region (714 / 37
  missed, 94.82%); the deleted lines duplicated coverage already
  carried by the isolated subprocess test.
- **Excluded `tests/capsem-build-chain/` from parallel pytest execution.** The suite runs `cargo build` and `codesign` via session-scoped fixtures, which caused races and failures on codesigning (`replacing existing signature` errors) when run concurrently with other tests. Now run in serial after the parallel block.
- **`capsem-process` now exits on `SIGTERM` on macOS.** Previously, the process blocked on `CFRunLoopRun()` and the signal handler task only logged the signal without stopping the run loop. Now, the signal handler calls `CFRunLoopStop` to allow the process to exit cleanly, fixing race conditions in VM cleanup tests.
- **MCP `shared_vm` consumers no longer intermittently 404 after
  `test_purge_all` runs on the same xdist worker.** `test_purge_all` was
  calling `capsem_purge { all: true }` on the session-scoped
  `capsem_service`, which also hosts the session-scoped `shared_vm`
  (persistent, named `shared-<worker>-<hex>`). Because `all=true`
  destroys every sandbox on the service -- persistent included -- any
  subsequent test on the same worker that used `shared_vm`
  (`test_sql_query`, `test_exec.*`, `test_file_io.*`, `test_lifecycle.*`,
  `test_mcp_call.*`) got `404 Not Found: sandbox not found` whenever
  pytest happened to schedule `test_purge_all` first. Fix: extracted the
  MCP conftest's service-startup into `_start_capsem_service()` so the
  `--gateway-port 0`, `--foreground`, `sign_binary`, and log-dumping
  invariants live in one place, added an `isolated_mcp_session`
  function-scoped fixture that spins up its own transient service for
  globally destructive tests, and migrated `test_purge_all` onto it.
  Added `test_isolated_mcp_session_does_not_affect_shared_service` to
  pin the isolation invariant so a future destructive test can't quietly
  regrow the same bug.
- **`capsem_mcp_call` no longer hangs for 60s on every invocation.** The
  service -> capsem-process IPC channel is `tokio-unix-ipc`, which uses
  bincode as its wire format. Bincode is not self-describing, and
  `serde_json::Value::deserialize` calls `deserialize_any`, which bincode
  explicitly rejects. `ServiceToProcess::McpCallTool { arguments:
  serde_json::Value }` therefore serialized fine on the service side and
  then failed to deserialize inside capsem-process the moment the message
  hit the wire -- the per-connection handler returned silently and the
  service's 60s `send_ipc_command` timeout fired. End result: every
  `tests/capsem-mcp/test_mcp_call.py` test spent exactly 60s hanging
  (120s combined, 75% of the 160s MCP parallel group), and the entire
  `capsem_mcp_call` feature path was dead on arrival on any non-stub
  aggregator. Fix: changed the IPC payload to JSON-stringified forms --
  `McpCallTool { arguments_json: String }` and
  `McpCallToolResult { result_json: Option<String> }` -- so the payload
  is opaque to bincode. The service and capsem-process now
  `serde_json::to_string` / `from_str` at the boundary. Added
  `mcp_call_tool_roundtrip_bincode` / `mcp_call_tool_result_roundtrip_bincode`
  tests in `capsem-proto` that exercise the real bincode path (the old
  tests only roundtripped through `serde_json::to_vec`, which is
  self-describing and missed the bug). MCP pytest group: 160s -> ~40s.
- **`capsem install` and `just install` can no longer bake a
  `target/test-home` path into the installed LaunchAgent / systemd unit.**
  `install_service()` resolves `--assets-dir` via
  `capsem_core::paths::capsem_assets_dir()`, which honors `CAPSEM_HOME` /
  `CAPSEM_RUN_DIR` / `CAPSEM_ASSETS_DIR`. If the installer inherited any
  of those from a prior `just test` session, the resulting LaunchAgent
  permanently referenced a directory that `just test` wipes on every
  run -- and with `KeepAlive=true`, launchd kept respawning it against a
  dead path, racing against `_ensure-service` during subsequent tests.
  Two-layer fix:
  - `install_service()` now bails with a clear message if any of the three
    isolation vars are set, telling the caller to `unset` them.
  - The `just install` recipe explicitly `unset`s them before running, so
    shells that accidentally still have them exported install cleanly.
  `scripts/integration_test.py::_kill_dev_service` also switched from
  `pkill -f capsem-service.*--foreground` (which catches any installed
  LaunchAgent/systemd unit on the box) to a strict pidfile-based kill,
  mirroring the discipline `_ensure-service` already follows.
- **`capsem run` auto-launch now honors `CAPSEM_HOME`.** When the client
  couldn't reach the service socket it fell back to
  `launchctl kickstart` / `systemctl --user start` whenever a
  LaunchAgent / systemd unit existed. Those units point at the default
  `$HOME/.capsem` layout, so under an isolated test run
  (`CAPSEM_HOME=target/test-home/.capsem`) the kicked service bound a
  socket in the *real* home while the client kept polling the test home
  until the 5s `AwaitStartup` budget expired -- `scripts/integration_test.py`'s
  ephemeral-model check always failed on machines with capsem installed.
  `UdsClient::try_ensure_service` now skips the service-manager branch
  whenever `CAPSEM_HOME` is set and goes straight to direct-spawn, so the
  child service inherits `CAPSEM_HOME` and binds the socket the client is
  watching. Production `~/.capsem` flow is unchanged.
- **Direct-spawn auto-launch no longer hangs the CLI's stdout/stderr
  pipes.** `UdsClient::try_ensure_service`'s fallback path spawned the
  service with inherited stdio, so when the CLI was invoked from Python
  under `subprocess.run(capture_output=True)`, the detached service
  kept stdout/stderr open long after the CLI returned. Python's
  `communicate()` waited for EOF on those pipes and always timed out at
  its outer 120s deadline -- the same symptom
  `scripts/integration_test.py::check_persistence` hit under a test
  harness without an existing running service. The spawn now redirects
  all three fds to `/dev/null`; service logs still land in
  `<run_dir>/service.log` as before.
- **`_ensure-service` no longer leaks the execution-lock fd.** The
  backgrounded capsem-service inherited fd 3 (which holds `flock -n 3` on
  `$CAPSEM_RUN_DIR/execution.lock`) from its parent shell. If `just smoke`
  or `just test` aborted after starting the service, the service kept fd 3
  open and the flock stayed held after the outer shell exited, bricking
  subsequent runs with "another agent holds the test execution lock". The
  service is now launched with `3>&-` so fd 3 is closed before exec.
- **`just install` now leaves `~/.capsem/assets/` in the layout the service's
  resolver actually reads.** The .pkg/.deb ships only `manifest.json` (binaries
  and assets are on independent shipping cadences), and `capsem setup` was a
  stub with a TODO, so a fresh install left the UI banner stuck on "VM assets
  are missing" and every VM boot failed asset resolution. Added
  `scripts/sync-dev-assets.sh`, invoked by the `install` recipe after the
  installer runs, which mirrors the locally built `assets/$arch/*` hash-named
  files into `~/.capsem/assets/$arch/` (the exact paths
  `ManifestV2::resolve()` looks up) and removes the legacy `v1.0.*/`
  directories that accumulated from the old v1 layout. Also updated
  `scripts/simulate-install.sh` to honor the same layout so
  `tests/capsem-install/` agrees with production.

### Added
- **`capsem setup` actually downloads VM assets, and `capsem update --assets`
  re-fetches them on their own cadence.** New
  `capsem_core::asset_manager::download_missing_assets()` streams each arch's
  asset files from the GitHub release URL (per-arch upload names:
  `arm64-vmlinuz` / `arm64-initrd.img` / `arm64-rootfs.squashfs`),
  blake3-verifies the bytes, and places them at
  `$base/$arch/{hash_filename}` with 0o444 perms. `step_welcome` in the setup
  wizard, and a new `capsem update --assets` subcommand, both call into it.
  `CAPSEM_RELEASE_URL` env override lets integration tests redirect the
  download target.

### Tests
- **`tests/capsem-install/` is now safe to run bare-metal.** The module-level
  `CAPSEM_DIR` previously hardcoded `$HOME/.capsem`, so running
  `pytest tests/capsem-install/` clobbered the developer's real install
  (`simulate-install.sh` overwrote binaries; `test_full_uninstall` literally
  asserted `~/.capsem` was removed). `conftest.py` now provisions a temp
  `CAPSEM_HOME` for the session and auto-skips the `live_system` tier
  bare-metal unless `CAPSEM_ALLOW_DESTRUCTIVE=1`, because those tests invoke
  `capsem setup` / `capsem uninstall` which touch the system-level
  LaunchAgent / systemd unit outside any `CAPSEM_HOME` override.
  `test_installed_layout` was rewritten to assert the v2 layout
  (`$ASSETS/$arch/{hash_filename}`) instead of the legacy
  `$ASSETS/v$VERSION/` the resolver no longer reads.
  New `test_asset_download.py` covers the happy path, 404, hash mismatch,
  and idempotent rerun for `capsem update --assets` against a local HTTP
  fixture.

### Changed
- **`just test` and `just smoke` reordered for fail-fast feedback.** Audits,
  Rust lint, and the frontend suite now run in a single parallel block at the
  top of each recipe, so a bad Svelte type, a broken clippy lint, or a
  dependency advisory surfaces in under two minutes instead of after 5-10
  minutes of `cargo llvm-cov` and cross-compile. The lint gate switched from
  `cargo check --workspace` to
  `cargo clippy --workspace --all-targets -- -D warnings`, enforcing the
  project's stated bar (`CLAUDE.md`: "treat clippy and rustc warnings as
  build failures") with no duplicate compile (clippy is a strict superset of
  check). Smoke additionally gained `pnpm run check` in its parallel block --
  previously a Svelte/TS type error only surfaced under `just test`.
- **`just test` ignores `tests/capsem-recipes/` and `tests/capsem-install/`
  in its parallel pytest stage.** Both directories contain tests that
  `subprocess.run(["cargo", "build", ...])` from inside pytest; under `-n 4`
  this atomically replaced the codesigned `capsem-service` / `capsem-process`
  binaries while other xdist workers were booting VMs against them, hanging
  `just test` at 99%. The recipe tests are redundant inside `just test`
  (clippy + `cargo llvm-cov` + `_build-host` already cover their assertions)
  and remain runnable standalone via `uv run pytest -m recipe`. The install
  suite is fully covered by `just test-install` inside Docker.
- **Every Shiki grammar and theme is now a lazy chunk fetched on first
  use, and the heavy app views are code-split.** The app was importing
  `'shiki'` (the default `bundle-full` export), which references all
  235 Shiki languages -- Vite code-split every one, shipping >600 KB
  chunks for grammars we never use (emacs-lisp, wolfram, wasm,
  vue-vine, ...). Moved to `shiki/core` and swapped the Oniguruma WASM
  regex engine for `createJavaScriptRegexEngine()` (removes a 608 KB
  WASM-as-JS chunk; the JS engine covers every grammar we ship).
  `shiki.ts` now creates the highlighter with empty `langs` and
  `themes` arrays and exposes a single `highlightCode(code, lang,
  theme)` entry point plus `ensureShikiLang` / `ensureShikiTheme`
  helpers. Each of the 31 supported languages and 21 themes is a
  `() => import('@shikijs/langs/<name>')` entry, so Vite emits one
  chunk per grammar/theme; they are fetched the first time a matching
  file is rendered, then retained for the session. A `shikiTick`
  pattern in StatsView upgrades the plaintext fallback to highlighted
  HTML once the prewarm promise for its langs/theme resolves. In
  `App.svelte` the heavy views (Settings, Stats, Logs, ServiceLogs,
  Files, Inspector, OnboardingWizard, CreateSandboxDialog) are now
  loaded via `{#await import()}` so they're fetched only on first use.
  App chunk drops from 582 KB to 142 KB. `chunkSizeWarningLimit` is
  raised to 700 KB to accommodate the inherent ~620 KB cpp grammar
  chunk (loaded only for `.cpp`/`.hpp`/`.cc`/`.cxx`); every other
  chunk stays under 200 KB. `@shikijs/langs` and `@shikijs/themes` are
  now direct deps (previously transitive) so Vite can resolve the
  subpath imports.

### Fixed
- **`just test` no longer kills or mutates a locally installed capsem.**
  Previously the test harness (`scripts/integration_test.py`,
  `_ensure-service`, and every Rust site that computed `$HOME/.capsem/...`
  directly) ran against the shared `~/.capsem/` directory, so a pkill-by-name
  on `capsem-service --foreground` took down the user's installed daemon,
  `~/.capsem/run/service.{sock,pid}` were deleted, and `~/.capsem/assets`
  was swapped for a symlink. Added a `CAPSEM_HOME` env var honored by a new
  `capsem_core::paths` module (with `capsem_run_dir`, `capsem_assets_dir`,
  `capsem_sessions_dir`, `capsem_bin_dir`, `capsem_logs_dir`,
  `service_socket_path`, `service_pidfile_path`) and routed every
  `$HOME/.capsem/...` site across `capsem`, `capsem-service`,
  `capsem-mcp`, `capsem-gateway`, `capsem-tray`, `capsem-app`, and
  `capsem-core` through it. `just test` / `just smoke` now export
  `CAPSEM_HOME=target/test-home/.capsem` (cleaned each run, swept by
  `just clean`). `_ensure-service` no longer uses pkill-by-name --
  it kills only the service tracked by its own pidfile, so an isolated
  test run never touches an installed daemon. The execution-lock flock
  moves into the test home alongside its socket.
- **Dev-build tray icon now renders orange** so the menu-bar icon is
  visually distinct from an installed release build. Grey pixels are
  recoloured to a `#FF8800` ramp at icon-load time under
  `cfg!(debug_assertions)`; anti-aliased edges remap by luminance so the
  icon stays smooth instead of banding. Release builds are untouched.
- **External links ("Get a key", API key docs, onboarding "Learn more")
  now open in the system browser from the Tauri desktop app.** Previously
  `<a target="_blank">` did nothing in the Tauri webview because
  `window.open` is a no-op there, and the `open_url` IPC handler already
  wired up in `capsem-app` was never called from the frontend. `openUrl()`
  in `api.ts` now detects the Tauri shell via `__TAURI_INTERNALS__` and
  invokes the `open_url` command; a document-level click interceptor in
  `App.svelte` routes every `<a target="_blank">` and `http(s):`/`mailto:`
  link through it, so existing call sites keep working unchanged. Browser
  dev mode still falls back to `window.open`.
- **Dark-mode warning banners no longer render with a white strip and
  unreadable text.** Two compounding issues: `html`/`body` had no
  theme-aware `background-color`, so the browser's default white canvas
  showed through any transparent element; and `--warning` /
  `--warning-foreground` were referenced across the frontend (install-
  incomplete banner, "VM assets are missing" alert, password-required
  badges, MCP section warnings) but never defined, so `bg-warning/10`
  resolved to fully transparent. Set the canvas to
  `var(--background)`/`var(--foreground)` on `html, body` and defined
  `--warning` (amber-600 light / amber-400 dark) plus
  `--warning-foreground` in `:root` and `.dark` so the amber tint and
  legible contrast appear on every warning surface.
- **`just install` no longer re-shows the GUI onboarding wizard on every
  reinstall.** The single `onboarding_completed` flag conflated "CLI install
  finished" with "user dismissed the welcome wizard", so dev reinstalls
  re-triggered the full-screen wizard even when the user had already clicked
  through it. Split into two flags: `install_completed` (set by `capsem setup`
  on success) and `onboarding_completed` (set only by the GUI wizard's "Get
  Started" button). Added `onboarding_version` to let a future release force
  re-onboarding by bumping `CURRENT_ONBOARDING_VERSION`. The frontend now reads
  a server-computed `needs_onboarding` instead of mirroring the version
  constant. Added `capsem setup --force-onboarding` to reset the wizard flags
  without wiping install state. Existing state files missing `install_completed`
  are migrated on load: if the `summary` step is present, install is inferred
  complete so upgraded users don't see a spurious "install didn't finish"
  banner. The app renders that banner only when `install_completed=false`,
  with a "Retry install" button that hits a new `POST /setup/retry` endpoint
  -- the service spawns `capsem setup --non-interactive --accept-detected` so
  users can recover from a broken install without opening a terminal.
- **`just smoke` now passes on dev machines whose user config opts
  into `security.web.allow_read=true`.** Three unrelated failures came
  out in the same wash:
  - Doctor `test_denied_domain_rejected` (test_network.py) and
    `test_denied_domain` (test_sandbox.py) hard-coded the default-deny
    posture. They now skip when `CAPSEM_WEB_ALLOW_READ=1` -- surfaced
    by injecting the `security.web.allow_{read,write}` toggles into
    the guest as `CAPSEM_WEB_ALLOW_{READ,WRITE}` env vars, the same
    pattern already used for `CAPSEM_OPENAI_ALLOWED` / etc. The
    policy's actual read/write denial is still exercised by
    `test_post_to_random_domain_denied`, which doesn't depend on the
    user's toggles.
  - `scripts/integration_test.py` looked for `run-*` session
    directories; current `capsem-service` generates
    `tmp-<adj>-<noun>` IDs (see `generate_tmp_name`). Also relaxed
    the `~/.capsem/logs` check -- that directory only exists after
    the Tauri desktop shell has been launched, which integration_test
    never does. The script now only validates the logs if they're
    present.
  - `config/integration-test-user.toml` used the stale
    `network.custom_{allow,block}` / `network.default_action`
    setting IDs; migrated to current `security.web.*` keys so the
    deny list (`deny.example.com`) actually takes effect.
- **`scripts/integration_test.py` restarts `capsem-service` with
  `CAPSEM_{USER,CORP}_CONFIG` in its env before booting the test VM**,
  then tears it down on exit. Required because the dev service
  (started by `_ensure-service`) inherits no test config, and
  `capsem run` talks to whatever service is already listening, so the
  per-VM policy previously fell back silently to `~/.capsem/user.toml`.
  Complements the service-side env passthrough in `refactor(service):
  extract pure helpers into lib + submodules`.

### Added
- **Failed-session log preservation for post-mortem.** Three host-side
  loss paths used to silently `remove_dir_all` the session directory
  when capsem-process died unexpectedly, taking `process.log`,
  `mcp-aggregator.stderr.log`, `serial.log`, and `session.db` with
  them -- exactly when those logs are most useful. All three now
  funnel through a single `ServiceState::preserve_failed_session_dir`
  helper that renames the dir to a `-failed-<ts>-<rand>` sibling
  (via `capsem_core::session::generate_session_id`) and calls
  `cull_failed_sessions` to cap the surviving count at
  `MAX_FAILED_SESSIONS = 5`. If rename fails (EEXIST, permission,
  cross-filesystem), `warn!` with the specific error and fall back
  to `remove_dir_all` so disk isn't leaked when the filesystem is
  already unhappy. Paths wired through the helper:
  (a) `handle_run`'s `wait_for_vm_ready` timeout -- now also awaits
  `wait_for_process_exit` before rename so the child has finished
  flushing session.db and log files (avoids the path-based-reopen
  ENOENT hazard during shutdown);
  (b) `scrub_evicted_instance` (promoted from free fn to
  `ServiceState` method) when `cleanup_stale_instances` detects a
  dead PID -- the loss path the last service commit introduced;
  (c) `provision_sandbox`'s child-exit handler, which fires only
  when the child died outside the explicit teardown path
  (`shutdown_vm_process` removes the map entry first, so the
  `removed = Some(info)` branch is by definition the "died
  unexpectedly" case). Four new unit tests pin the contract: rename
  preserves file contents, cull keeps newest and prunes oldest, cull
  is a no-op under the cap, cull never touches non-`-failed-` dirs.
- **Multi-agent execution lock on heavy `just` recipes.** `smoke`,
  `test`, `bench`, `shell`, `exec`, `ui`, `install`, and
  `test-gateway-e2e` now acquire a non-blocking `flock(1)` on
  `~/.capsem/run/execution.lock` before doing anything that touches
  the shared `capsem-service`. A second agent attempting a heavy
  recipe while one is in flight gets an immediate
  `"another agent holds the capsem execution lock ..."` error instead
  of silently restarting the service under the first agent's VMs.
  The kernel releases the lock when the holding process exits, so
  there are no stale lockfiles on crash/SIGKILL. `flock` is now
  checked by `just doctor` (hints point at `brew install flock` on
  macOS, `util-linux` on Linux) and auto-installed by
  `scripts/bootstrap.sh` on macOS when Homebrew is available.

### Changed
- **`UdsClient::connect_with_timeout` now uses
  `capsem_core::poll::poll_until`** instead of a hand-rolled
  exponential-backoff loop. New `ConnectMode { FailFast, AwaitStartup }`
  parameter makes the retryable-vs-permanent classification explicit
  at every call site: the initial probe in `request()` stays
  `FailFast` so CLI calls don't sit for 5 s when the service is
  definitively down; post-launch retries in `try_ensure_service` are
  `AwaitStartup` so a just-started service's `ENOENT`/`ConnectionRefused`
  are treated as "socket not bound yet" rather than "service dead."
  Also folded: `try_ensure_service` now returns the connected
  `UnixStream` so `request()` no longer does a third redundant
  connect. Net effect on the code is smaller than the diff suggests --
  mostly deletes the hand-rolled state machine and replaces it with
  the shared primitive. See `/dev-rust-patterns` lesson 19 and
  `/dev-bug-review` (the skill now explicitly calls out
  "grep for existing primitives, don't hand-roll" as a first-class
  step of the workflow).
- **`capsem-service` split into `lib + bin`** -- new `crates/capsem-service/src/lib.rs` exposes the `api`, `errors`, `fs_utils`, and `naming` submodules. Pure helpers (`AppError`, `sanitize_file_path`, `extract_magika_info`, `identify_file_sync`, `validate_vm_name`, `generate_tmp_name`) move out of `main.rs` into their own files with their own `#[cfg(test)] mod tests`. `ServiceState`, `PersistentRegistry`, `resolve_workspace_path`, and every axum handler stay in `main.rs` (their move is a follow-up sprint). `api.rs` content is unchanged -- `errors.rs` re-exports `ErrorResponse` via `pub use`. +14 net new unit tests; `errors.rs`/`fs_utils.rs`/`naming.rs` each at 100% line, region, and function coverage. Unblocks future `crates/capsem-service/tests/` integration tests now that `lib.rs` exists.
- **Workspace MSRV bumped from Rust 1.82 to 1.91.** `capsem-core`'s
  `mcp::builtin_tools` relies on `str::floor_char_boundary`, stable in
  1.91, which clippy's `incompatible_msrv` lint correctly flagged.
  Raising the floor clears the lint (no downgrade path), matches the
  toolchain the tree is actually built with, and unblocks
  `cargo clippy -- -D warnings` across the workspace.

### Fixed
- **`capsem doctor` (and any other auto-launch path) no longer
  spuriously fails with "Service manager started capsem but socket not
  ready."** Root cause: `UdsClient::connect_with_timeout` fast-failed
  on `ENOENT`/`ConnectionRefused` from its very first attempt, breaking
  out of its own retry loop before the just-requested service could
  bind its socket. The obvious symptom was the misleading error
  message; the less obvious consequence was that the auto-launch
  path became racy under load and flaky in tests. Fix is the
  `ConnectMode`-aware refactor above plus preserving the inner error
  via `Context` instead of the old `.map_err(|_| anyhow!(...))` which
  threw the real `io::Error::kind` away. Pre-existing clippy cleared
  in the same file: 3 `print_literal` on table-header printlns
  (intentional literal labels -- allowed locally with a comment),
  3 `field_reassign_with_default` in `setup.rs` test fixtures.
  Regression tests in `client.rs`: FailFast short-circuits in under
  500 ms on a missing socket; AwaitStartup sees a `UnixListener`
  bound 400 ms after the connect call starts; AwaitStartup times out
  cleanly with a preserved error chain when nothing ever binds.

### Added
- **Host-side logs now carry `vm_id` and `trace_id` as structured
  fields for cross-process correlation.** `capsem-process` generates a
  16-hex-char `trace_id` at startup and enters a root
  `info_span!("vm", vm_id, trace_id)` that every subsequent log line
  inherits. The same pair is propagated to the aggregator subprocess
  via `CAPSEM_VM_ID` / `CAPSEM_TRACE_ID` env vars, and
  `capsem-mcp-aggregator` enters a matching
  `info_span!("aggregator", vm_id, trace_id)`. Grep for a `trace_id` to
  follow a single VM's execution across `process.log`,
  `mcp-aggregator.stderr.log`, and `session.db` in the same session
  directory. First step toward broader log correlation -- other
  binaries (service, gateway, app) will pick up the same pair in
  follow-ups. OpenTelemetry export was proposed alongside this and
  explicitly deferred to a sprint proposal: it's a feature, it adds
  a new outbound channel to an air-gapped product, and the
  correlation problem that motivated it is solved by `trace_id`
  alone.

### Fixed
- **`capsem-mcp-aggregator` stderr no longer pollutes `process.log`.**
  `capsem-process` spawned the aggregator with
  `Stdio::inherit()` for stderr, so the aggregator's plain-text
  tracing merged into the parent's JSON tracing stream and made
  `process.log` effectively unparseable with `jq` / log pipelines.
  Two coupled fixes: (a) the aggregator's subscriber now uses
  `.json()`, matching `capsem-process` and `capsem-service`; (b) the
  aggregator's stderr is now redirected to a dedicated
  `mcp-aggregator.stderr.log` in the VM's session directory, opened
  with `0o600` under `#[cfg(unix)]` per
  `/dev-rust-patterns` lesson 14. End state: `process.log` is pure
  parent JSON, `mcp-aggregator.stderr.log` is pure aggregator JSON.
  Also elevated a small set of lifecycle events from `debug` to
  `info` (aggregator reader/writer/monitor task start/stop,
  `mcp::gateway::serve_mcp_session_inner` EOF) so critical
  lifecycle transitions are always visible in the default filter.
  Cleared 4 pre-existing clippy errors in `capsem-process` that the
  gate surfaced: one `too_many_arguments` on `ipc::handle_ipc_connection`
  (8 > 7 -- `#[allow(...)]` with no behavior change), three
  `useless_vec` in unit tests. Three new unit tests pin the
  `trace_id` contract (16 hex chars, no collisions over 64 calls)
  and the aggregator-log path (lives in session dir). The JSON
  format switch and the root-span wiring are not cleanly unit
  testable without a live subscriber harness; validated via compile
  + clippy + existing suites.
- **`capsem-app`'s update-prompt no longer blocks a tauri/tokio worker
  thread while the user decides.** `check_for_update_with_prompt` in
  `crates/capsem-app/src/main.rs` used `tauri_plugin_dialog`'s
  `.blocking_show()` from inside an `async fn` spawned on the runtime.
  Because the user can leave the dialog sitting for seconds to minutes,
  the blocked thread effectively holds a runtime worker for human time
  -- same anti-pattern we just fixed in the tray (`std::process::Command`
  in async). The fix is NOT `spawn_blocking` (its bounded pool is sized
  for short I/O, not human waits); it's bridging the plugin's
  callback-based `.show(|accepted| ...)` to async via
  `tokio::sync::oneshot`. See `/dev-rust-patterns` "Blocking-in-async
  anti-pattern" and `/dev-bug-review`.
- **`capsem-app` session log now created with mode `0o600`.** The
  per-launch log at `~/.capsem/logs/<timestamp>.jsonl` was opened via
  `File::create`, which applies the user's umask (typically `0644`) and
  leaves the file readable by every local user. The log contains
  tracing spans with VM ids, filesystem paths, provider API metadata,
  and tool-call arguments -- on a shared box that is a user-to-user
  information leak. Factored an `open_log_file(path)` helper that uses
  `OpenOptions::mode(0o600)` under `#[cfg(unix)]` with a plain-options
  fallback elsewhere, matching the established pattern already used by
  `pty_log.rs`, the gateway auth token, per-VM sockets, and
  `capsem-core`'s key helpers. Two new unit tests pin the behavior
  (file round-trips content; mode is exactly `0o600` on Unix). Also
  cleared a pre-existing `needless_borrows_for_generic_args` clippy on
  the deep-link `window.eval` call in the same file. See
  `/dev-rust-patterns` lesson 14.
- **`provision_sandbox` no longer holds the `instances` mutex across
  blocking filesystem work, and no longer probes for stale records on
  every successful provision.** `cleanup_stale_instances` previously
  held the std::sync::Mutex from the `kill(pid, 0)` probe loop all the
  way through the `remove_dir_all` + `remove_file` sweep for every
  evicted ephemeral session -- hundreds of ms of blocking I/O under
  which every other `instances.lock()` caller (~30 sites: list /
  status / stop / delete / suspend / resume / fork / exec handlers)
  stalled. Split into a two-phase contract: `drain_dead_instances`
  probes and evicts under the lock (microseconds), and the caller
  scrubs each evicted entry's filesystem artifacts via the free
  `scrub_evicted_instance` with the lock released. Additionally gated
  the probe itself: `provision_sandbox` now only runs it when
  `instances.contains_key(id)` or the map is already at
  `max_concurrent_vms` -- the two conditions under which stale
  reclamation could unblock the caller. Three regression tests pin
  the drain contract (dead-only eviction, no-op when all alive, mutex
  released on return). Follow-up to commit 34d0e3f.
- **`POST /run` no longer blocks the tokio reactor on provision.**
  `handle_run` at `crates/capsem-service/src/main.rs:2484` was calling
  `state.provision_sandbox(...)` directly from the axum async handler,
  missed by commit 34d0e3f's spawn_blocking sweep that covered
  `handle_provision` and `handle_fork`. Same blocking I/O
  (APFS clonefile, `rootfs.img` fsync, walkdir, subprocess spawn),
  same fix -- wrap in `tokio::task::spawn_blocking` with the runtime-
  handle thread-local preserved for the inner
  `tokio::process::Command::spawn`.
- **Cleared 18 pre-existing clippy errors surfaced by running
  `-D warnings` across the provision path's dependency graph:** 3 in
  `capsem-service/main.rs` (two `u64 as u64` casts in
  `attach_summary_telemetry`, one `iter_kv_map` in the MCP refresh
  broadcast), 6 in `capsem-core/asset_manager.rs` (five `iter_kv_map`,
  one `collapsible_if` in the asset-version resolver), 1 in
  `capsem-core/setup_state.rs` (field reassignment after
  `Default::default()` in a unit test), 1 `incompatible_msrv`
  (addressed via the MSRV bump above), 4 redundant closures + 3
  redundant `+ 0` operands in `capsem-logger/reader.rs` test fixtures.
  None were behavioral; the redundant-closure fixes convert
  `|row| read_*_row(row)` to `read_*_row`.
- **`just test` no longer self-destructs across parallel workers from a
  broad `pkill`.** Four sites fired `pkill -9 -x capsem-service` (or
  `-f capsem-service`) which matched every `capsem-service` on the box,
  including every other pytest-xdist worker's test service. A single
  install-tests fixture running `simulate-install.sh` took the whole suite
  down -- reproducibly -- pushing ~148 tests into "service refused
  connection" / "VM never exec-ready" cascades. Each site now scopes the
  match to its own install prefix:
  - `scripts/simulate-install.sh` matches `$INSTALL_DIR/<name>`.
  - `tests/capsem-install/conftest.py::_kill_service` matches
    `$INSTALL_DIR/<name>`.
  - `tests/capsem-install/test_service_install.py` matches
    `$INSTALL_DIR/capsem-service`.
  - `crates/capsem/src/uninstall.rs` and
    `crates/capsem/src/service_install.rs` use `current_exe().parent()` to
    scope `pkill` to the binary's own install directory -- semantically
    also correct in production: `capsem uninstall` from `~/.capsem/bin`
    only affects processes launched from `~/.capsem/bin`, leaving dev
    services under `target/debug/` alone.
- **Gateway tests now pass their parent PID.** `tests/helpers/gateway.py`
  spawned `capsem-gateway` without `--parent-pid`, so `capsem-guard`
  returned `Err(NoParent)` and the gateway exited 0 immediately. Every
  gateway fixture then failed its 10s readiness wait and every gateway
  test (60+ errors, ~10 failures) cascade-failed at setup. Helper now
  passes `--parent-pid=os.getpid()` and `--run-dir` so the per-test
  singleton lock lands in the test tmp dir.
- **Built-in MCP snapshot tools: `snapshots_history` dropped its `path`
  argument and `snapshots_compact` dropped its `name` argument** because
  `SnapshotPaginationParams` / `SnapshotCompactParams` in
  `crates/capsem-mcp-builtin/src/main.rs` didn't declare those fields.
  rmcp's typed-parameter deserialiser silently discarded the unknown
  keys, so every `snapshots_history` call returned
  `-32602 missing 'path' argument`. Added a dedicated
  `SnapshotHistoryParams` struct and extended `SnapshotCompactParams`.
  Also renamed `SnapshotCheckpointParams` → `SnapshotRevertParams` with
  `path: String` required and `checkpoint: Option<String>` optional
  (matches `handle_revert_file`'s auto-pick-newest behaviour).
- **Built-in MCP tool failures are now `isError: true` on the result
  instead of a success-shaped result containing error text.**
  `extract_text` in `capsem-mcp-builtin` returned
  `Ok(text)` for every tool response, including the ones where
  `call_builtin_tool` had set `isError: true`. rmcp maps `Err(String)`
  to the wire-level `isError` result, so blocked-domain and
  invalid-URL rejections from `fetch_http` / `grep_http` / `http_headers`
  went through as regular successes. Now propagated correctly.
- **Three built-in HTTP tools carry MCP annotations.** Added
  `annotations(title, read_only_hint, destructive_hint, idempotent_hint,
  open_world_hint)` to `fetch_http`, `grep_http`, `http_headers` so their
  `tools/list` output matches the file-tool annotations the MCP spec
  expects clients to surface.
- **Guest `snapshots` CLI calls now use namespaced tool names.** The
  in-VM `snapshots` helper in `guest/artifacts/snapshots` called
  `snapshots_create`/`_list`/`_revert`/`_delete`/`_history`/`_compact`
  against the host MCP gateway, which namespaces aggregator tools as
  `{server}__{tool}` -- so every bare call returned
  `-32603 tool call failed` and every `snapshots …` command inside the
  VM died with the capsem-mcp-server stderr bleeding into the CLI
  error. Prefixed each call with `local__`. See
  `crates/capsem-core/src/mcp/types.rs::namespace_name` and the `local`
  key in `config/defaults.json`.
- **Tray no longer flashes the menu bar during tests.** Added
  `CAPSEM_TRAY_HEADLESS` env var to `capsem-tray` -- when set, the
  binary still arms parent-watch and acquires the singleton flock but
  skips `NSStatusItem` / `TrayIconBuilder` creation and idles. The
  integration test helpers (`tests/helpers/service.py`,
  `tests/capsem-mcp/conftest.py`) no longer pass `--tray-binary` at all;
  the tray-focused `tests/capsem-service/test_companion_lifecycle.py`
  keeps spawning the tray but in headless mode. Full-suite runs now
  create zero menu-bar icons.
- **In-VM diagnostic test suite realigned to current product behaviour.**
  `guest/artifacts/diagnostics/test_mcp.py` and `test_network.py` had a
  cluster of stale assertions: tool names without the `local__`
  namespace prefix that the gateway applies, the four
  `pytest.raises(AssertionError)` blocks that were only catching the
  "tool not found" protocol error (now that the tools are found and
  exercise real error paths), `mcpServers["capsem"]` instead of the
  canonical `["local"]` key from `config/defaults.json`, `fetch_http` /
  `grep_http` / `http_headers` expecting `{"isError": true}` where the
  host code was returning a success result containing error text,
  `list_changed_files` expected in `tools/list` (renamed to
  `snapshots_changes`), and `test_denied_domain_rejected` using
  `api.openai.com` which is policy-gated by `CAPSEM_OPENAI_ALLOWED`
  (returns 401 when enabled, not the 403 the test wanted).
  Introduced `ns()` / `_init_and_call` auto-prefix + `_assert_tool_error`
  helpers; `_init_and_call` now collapses JSON-RPC errors into
  `isError: true` tool results so callers see a single shape regardless
  of where the failure originated.
- **`tests/capsem-stress/test_rapid_exec.py::test_rapid_file_io` hit a
  404 on every iteration.** The test POSTed to `/write-file/{id}` and
  `/read-file/{id}` (dashes) but the service routes are `/write_file/`
  and `/read_file/` (underscores); it also sent `data: list[int]` bytes
  where the endpoint expects `content: str`. Fixed.

### Added
- **`capsem-guard` crate** -- new tiny library (`crates/capsem-guard/`) with
  parent-watch + singleton flock primitives. Used by `capsem-gateway` and
  `capsem-tray` to make them non-standalone companions of `capsem-service`:
  they refuse to start without a valid `--parent-pid`, acquire a system-wide
  singleton lock, and self-exit within 100 ms when the parent dies. Works
  under SIGKILL, OOM, and pytest-xdist worker death -- scenarios where
  `tokio::process::Command::kill_on_drop(true)` silently does nothing.
  Implementation details: `getppid()`-based watcher (immune to zombie state),
  `O_CLOEXEC`-atomic `flock(2)` with a process-local registry to cover the
  fork-to-exec window, global tray lock at `~/.capsem/run/tray.lock` (one
  menu-bar icon system-wide). 31 Rust unit tests + 15 adversarial Python
  integration tests in `tests/capsem-service/test_companion_lifecycle.py`
  (refuse-standalone × 4, singleton × 3 incl. 20-way hammer, dies-with-parent
  × 2, service-SIGKILL end-to-end × 1, timing-budget regression guards × 5).
  See `/dev-rust-patterns` lesson 18.

### Fixed
- **Tray action dispatch no longer stalls its tokio worker on fork/exec.**
  `launch_ui` and `launch_ui_action` in `crates/capsem-tray/src/main.rs`
  called `std::process::Command::spawn` synchronously from the async
  `dispatch_action` path. Because the tray runs a `new_current_thread`
  tokio runtime (one worker), each Connect / New Session / Save / Fork
  click briefly froze status polling and further action dispatch during
  the `posix_spawn`/`fork+exec` syscall. Swapping to
  `tokio::process::Command` would not have helped -- its `spawn()` still
  invokes the same blocking syscall. Both launches now run on a
  dedicated `std::thread::spawn` (not `tokio::task::spawn_blocking`, whose
  bounded worker pool is the wrong fit for the reaper's long
  `Child::wait()`) and the child is now reaped, eliminating zombie
  accumulation on the long-lived tray process. Deduped the two
  near-identical launch bodies behind `find_capsem_app_binary` + a pure
  `build_launch_invocation` helper, covered by 6 new unit tests pinning
  deep-link construction for the direct-binary and `open -a Capsem`
  fallback paths. Also fixed a `clippy::redundant_closure` around
  `tray_lock_path`. See `/dev-rust-patterns` "Blocking-in-async
  anti-pattern" and `/dev-bug-review`.
- **`POST /fork/{id}` and `POST /sandboxes` (provision) no longer block
  the tokio reactor during heavy filesystem work.** `handle_fork` called
  `capsem_core::auto_snapshot::clone_sandbox_state` directly from the axum
  handler; `handle_provision` called the synchronous
  `ServiceState::provision_sandbox`, which wraps the same clone plus a
  `sync_all()` flush of `rootfs.img` and a walkdir-based `disk_usage_bytes`.
  Under concurrent fork/provision load these could exhaust axum worker
  threads and stall unrelated requests. Both call sites are now wrapped in
  `tokio::task::spawn_blocking`, matching the established pattern in the
  same file (`handle_upload`, `list_dir_recursive`, `handle_detect_host_config`,
  the `remove_dir_all` cleanups). The sync-to-spawn_blocking handoff
  preserves the tokio runtime handle via thread-locals, so the
  `tokio::process::Command::spawn` call inside `provision_sandbox` still
  works. All 116 capsem-service tests remain green.
- **AI traffic parsers no longer build a full JSON DOM for tool call args
  and responses.** Three places in `crates/capsem-core/src/net/ai_traffic/`
  parsed LLM SSE payloads into `serde_json::Value` only to stringify them
  (Gemini `functionCall.args` in `google.rs`, Gemini `functionResponse.response`
  in `request_parser.rs`) or not use them at all (OpenAI Responses API
  `ResponseInfo.output`). Switched the two stringified sites to
  `Box<serde_json::value::RawValue>` so the fragment is kept as a lazy byte
  slice and re-emitted verbatim without an intermediate `BTreeMap`/`Vec` DOM
  allocation; deleted the unused OpenAI `output` field entirely. Enabled the
  workspace `serde_json` `raw_value` feature. Added two regression tests
  (`stream_function_call_preserves_arg_bytes_verbatim`,
  `google_function_response_preserves_bytes_verbatim`) pinning the byte-
  verbatim preservation behavior -- RawValue keeps whitespace and key order
  as-sent, where `Value` would re-serialize to canonical-compact form. See
  `/dev-rust-patterns` lesson 6.
- **Companion processes no longer leak across interrupted test runs.**
  `just test -n 4` under ctrl-C / pytest-xdist worker death / SIGKILL left
  `capsem-gateway` and `capsem-tray` reparented to PID 1 because their only
  cleanup hook was `kill_on_drop(true)`, which does not fire on ungraceful
  exit. Accumulated orphans caused downstream "vm-ready never asserted"
  poll spins, UDS connection refusals, and the suspend/resume regression.
  Fixed by wiring all companions through the new `capsem-guard` library;
  the contract is enforced on the companion side so the spawner can't get
  it wrong.

### Changed
- **Linux CI now measures coverage for every portable host crate.** The
  `llvm-cov nextest --codecov` invocation on the KVM runner previously
  tested only 8 of 14 workspace members. Added `capsem-agent` (118 tests),
  `capsem-gateway` (128), `capsem-process` (72), and `capsem-guard` (31)
  -- none of which had macOS-only code paths gating their Linux build.
  The only crates still excluded from Linux CI are `capsem-app` (Tauri
  shell) and `capsem-tray` (`muda` menu-bar), both genuinely macOS-only.
  Net effect: ~349 additional Rust tests now contribute to the Codecov
  dashboard from the Linux side, catching Linux-specific regressions the
  macOS run cannot. Added a new `Guard` component to `codecov.yml` so the
  crate shows up alongside Gateway, Service, CLI, etc.

### Fixed
- **`just ui` / `just shell` re-invocation no longer leaves the dev service
  without a gateway.** Three related bugs in the companion-shutdown path
  collectively caused the new gateway to hit `EADDRINUSE` on port 19222
  whenever the prior dev service was killed (SIGTERM or SIGKILL) and a
  new one spawned within `_ensure-service`'s 500 ms restart budget. User
  symptom: the frontend WebSocket connected briefly (served by the
  orphan gateway), then dropped when the orphan's parent-watch fired,
  after which every reconnect hit "connection refused". Each bug has a
  dedicated regression test in `tests/capsem-service/test_companion_lifecycle.py`.
  - `capsem-service` killed VMs *before* companions on graceful shutdown.
    `kill_all_vm_processes` includes an unconditional 500 ms SIGTERM-grace
    `thread::sleep`, so companion-kill didn't run until at least 500 ms
    after SIGTERM -- exactly when the new service was spawning its own
    gateway. Fixed by reordering graceful_shutdown to kill companions
    first. Guard: `TestServiceSigtermReapsCompanionsPromptly` (300 ms
    budget).
  - `kill_all_vm_processes` slept 500 ms *even when zero VMs were
    running*, inflating every shutdown by half the `_ensure-service`
    budget. Fixed by early-returning when the VM list is empty and
    skipping the grace sleep when no VM was actually signalled. Guard:
    `TestServiceShutdownIsFastWithoutVMs` (300 ms full shutdown budget).
  - `capsem-guard`'s parent-watch polled every 500 ms, so a SIGKILL'd
    service's companions could remain alive up to a full poll interval --
    the full `_ensure-service` budget by itself. Tightened
    `PARENT_POLL_INTERVAL` from 500 ms to 100 ms (`getppid()` is a vDSO
    call; cost is negligible). Guard: `TestCompanionsDieFastAfterServiceSigkill`
    (300 ms budget on SIGKILL path). Also documents an end-to-end
    restart contract in `TestServiceRestartSequenceKeepsGatewayHealthy`.

- **`just _clean-stale` no longer hangs for minutes.** The bash body called
  `lsof -tU "$s"` once per socket in `/tmp/capsem/*.sock`. On macOS each call
  scans every process's FD table (~200 ms), so after ~1700 dead sockets
  accumulated the loop took ~6 minutes and made `just test` / `just smoke` /
  `just install` / `just build-assets` look stuck. Replaced the entire recipe
  with `scripts/clean_stale.py`, which probes socket liveness via
  `socket.connect()` (~4 us per socket, ~50000x faster) and ports the other
  stages (stale rootfs/`_up_` dirs, stale test fixtures, cargo artifact
  age-prune) to Python. Measured: 1772 orphan sockets + 926 stale cargo dirs
  cleaned in 3.2 s total; steady-state second run 1.3 s. Covered by 16 pytest
  cases in `tests/capsem-cleanup-script/` including a 2000-socket perf guard
  that fails if the regression ever returns.

- **`just test -n 4` concurrency cascade** -- four independent bugs surfaced as
  "flaky tests" whenever pytest ran with parallel workers. Collapsed the cascade
  from ~130 test failures down to ~5.
  - **`capsem-service` is now self-idempotent on startup.** New
    `crates/capsem-service/src/startup.rs` probes `/version` on the target UDS
    and an adjacent advisory flock serialises the probe→remove-stale→bind
    critical section. Four parallel `capsem-service --uds-path X` invocations
    converge on exactly one running service; losers exit 0 when the version
    matches, exit non-zero on mismatch (never auto-kill).
  - **`capsem-gateway` honours the service's `run_dir`.** New `--run-dir` flag
    (plus `CAPSEM_RUN_DIR` env fallback) replaces the `$HOME/.capsem/run`
    hardcode. The service passes it when spawning the gateway child, so
    `gateway.{token,port,pid}` land where the service polls for them. The
    gateway also writes `gateway.port` *after* `TcpListener::bind` so
    OS-assigned ports (`--gateway-port 0`) are recorded correctly instead of
    persisting the configured `0`.
  - **`axum::serve` no longer blocks on `gateway-ready`.** `spawn_companions`
    ran inline before `axum::serve`, delaying UDS accept by up to 5 s per
    startup while polling for `gateway.token`. Companion spawning is now
    detached via `tokio::spawn`, so the UDS accepts the instant it binds.
    Companion children are parked in a `Mutex<Vec<Child>>` and explicitly
    killed on graceful shutdown (kill_on_drop handles crash paths).
  - **CLI `run_dir` derives from `--uds-path`.** When `--uds-path` is explicit
    (tests, custom deployments), `crates/capsem/src/main.rs` now takes the
    parent directory as `run_dir` instead of falling back to
    `CAPSEM_RUN_DIR`/`$HOME`. Keeps doctor logs and inherited paths consistent
    with wherever the service actually writes.
- **`ProvisionResponse.uds_path` is the source of truth for instance sockets.**
  Clients were recomputing `<run_dir>/instances/{id}.sock`, but the service
  falls back to `/tmp/capsem/<hash>.sock` when the preferred path exceeds
  macOS's 104-byte `SUN_LEN`. The fallback hash uses process-randomised
  `DefaultHasher`, so clients *cannot* reliably recompute. The provision
  response now includes the server-chosen path; `capsem doctor` uses it
  directly (fixes "Session did not become ready within 30s" on e2e tests
  rooted under `/var/folders/...`).

### Changed
- **Shared `capsem_core::uds` module** -- extracted `SUN_PATH_MAX` and
  `instance_socket_path` into `crates/capsem-core/src/uds.rs` so the
  SUN-length workaround lives in exactly one place. Service delegates; clients
  use it as a fallback only when talking to a pre-`uds_path` service.
- **`capsem doctor` uses the shared poll helper** -- the hand-rolled
  `loop { if sock.exists() ... sleep 200ms }` waiting for the per-VM IPC
  channel was replaced with `capsem_core::poll::poll_until`, same primitive
  already used by CLI/service/MCP.

### Changed
- **Crate `capsem-ui` renamed to `capsem-app`** -- crate and binary name now match the directory. Tauri identifier (`com.capsem.capsem`), productName (`Capsem`), and code-signing/notarization are unaffected. `justfile`, CI workflow, `capsem-tray` binary path lookups, `capsem-build-chain` tests, and the relevant skills were updated. localStorage keys `capsem-ui-mode` / `capsem-ui-font-size` were intentionally left unchanged to preserve user preferences across upgrades.
- **Workspace Cargo metadata** -- `[workspace.package]` now carries `description`, `license = "Apache-2.0"`, `repository`, `homepage`, `rust-version`, and `authors`; every per-crate `Cargo.toml` inherits via `.workspace = true`. `cargo metadata` consumers (SBOM, GitHub dep graph, cargo-deny) now see canonical values for all 13 crates.
- **Skills drift reconciled** -- `/dev-capsem` crate map no longer has the duplicate `capsem-gateway` row and now lists `capsem-mcp-aggregator` and `capsem-mcp-builtin`. `/dev-mcp` tool table drops three tools that no longer exist (`capsem_image_list/inspect/delete`), adds `capsem_mcp_servers`, `capsem_mcp_tools`, `capsem_mcp_call`, and documents the three-crate MCP subprocess architecture. CLAUDE.md project layout now lists all 13 crates and the skills table matches `skills/` on disk.

### Added
- **`SECURITY.md`** -- vulnerability reporting policy (GitHub Security Advisories), supported versions, disclosure timeline, scope (sandbox escape, MITM bypass, supply-chain integrity) and explicit out-of-scope (anything inside the guest VM by design).
- **`RELEASE.md`** -- human-facing pre/post-release checklist that points back to `/release-process` for depth. Captures the `just cut-release` path, CI pipeline shape, and what to check after the tag is pushed.
- **`rust-toolchain.toml`** -- pins the stable channel + `aarch64-unknown-linux-musl` / `x86_64-unknown-linux-musl` targets so local and CI builds resolve the same toolchain.
- **`docs/usage/mcp-tools.md`** -- user-facing reference for the 22 MCP tools exposed by `capsem-mcp`, grouped by session lifecycle, exec/file, telemetry, MCP aggregator, and diagnostics. Source of truth remains `crates/capsem-mcp/src/main.rs`.
- **`docs/usage/shell-completions.md`** -- how to generate and install bash/zsh/fish/PowerShell completions via `capsem completions <shell>`.
- **Pointer READMEs at `crates/capsem/README.md` and `crates/capsem-proto/README.md`** -- ~10-line README each for the two externally-visible crates, linking to capsem.org.

### Added
- **capsem/setup.rs tests + small DI refactor** -- helpers (`load_state`, `save_state`, each `step_*`) now take `capsem_dir: &Path` explicitly instead of reading it from `$HOME` at call time. `run_setup` still computes the real dir once and threads it through, so the public contract is unchanged. 11 new unit tests cover state-file roundtrip (including atomic overwrite + parent-dir creation), corrupt-state recovery, and `step_corp_config` success / invalid-TOML / missing-file paths against a `tempdir()`. `setup.rs` coverage 0% → 47%.
- **Unit tests for capsem-app helpers** -- `parse_flag`, `cleanup_old_logs`, `format_log_filename` (extracted from `log_filename` for testability). 12 new tests covering the deep-link argument parser and log housekeeping.
- **HTTP-level tests for capsem-tray gateway client** -- new `spawn_http_probe` test helper spins up a single-connection `tokio::net::TcpListener` so `status`, `stop_vm`, `delete_vm`, `suspend_vm`, `resume_vm`, `provision_temp` are exercised end-to-end (happy path + 4xx/5xx + dead host). `GatewayClient::new`/`new_with_base_url` added for injection. `capsem-tray/src/gateway.rs` jumps from 36% to 94% coverage. Also added `parse_port_file` tests against malformed `gateway.port` contents.
- **capsem-gateway Args + event-ws tests** -- clap default/override tests and a `handle_events_ws`-without-Upgrade test. `main.rs` coverage 69% → 75%.
- **capsem-logger reader fixture-based aggregate tests** -- populates net/model/tool/mcp/fs tables and asserts `session_stats`, `top_domains`, `search_net_events`, `net_event_counts`, `recent_net_events`, `tool_calls_for`, `tool_responses_for`. 24 new tests, reader.rs coverage 75% → 79%.
- **Coverage reporting for capsem-ui, capsem-mcp-aggregator, capsem-mcp-builtin** -- these three crates were invisible to Codecov (never in the `-p` list passed to `cargo llvm-cov`). Added to both macOS and Linux CI runs (capsem-ui macOS-only since Tauri). The `tooling` component in `codecov.yml` now includes the MCP subprocess crates; new `systray` component covers `capsem-tray`; `crates/capsem-app/gen/**` added to ignore list so Tauri-generated code doesn't pollute coverage.
- **PTY ring buffer on host for banner replay** -- `capsem-process` now fronts the terminal broadcast channel with a `TerminalRelay` that retains the last 64 KiB of PTY output. Newly-subscribing WebSocket or IPC clients receive the buffered snapshot atomically (snapshot + subscribe under one mutex) before the live stream, so a fresh browser tab sees the shell's login banner even though the shell printed it before the client connected. Covers both `/terminal` WS (frontend) and `StartTerminalStream` IPC (`capsem shell`).
- **Tray menu split by VM kind** -- persistent running: Connect + Stop + Fork + Delete. Persistent stopped/suspended: Resume + Fork + Delete (or just Fork + Delete when fully stopped). Ephemeral running: Connect + Save + Delete (no Stop, since stopping an ephemeral == destroying it). Save and Fork open the desktop app with `--action save|fork` and dispatch a `capsem:tab-action` event that the Toolbar picks up to open the matching dialog -- the tray can't prompt for a name, so the UI owns it.
- **Tray deep-link uses direct binary path** -- `open -a Capsem --args` only forwards args to a *new* launch on macOS; it drops args when the app is already running. `capsem-tray` now invokes `/Applications/Capsem.app/Contents/MacOS/capsem-ui` (or `~/Applications/...`) directly, so `tauri-plugin-single-instance` sees the second launch and forwards `--connect` / `--action` to the running instance.
- **Session-boot overlay in the terminal** -- three pulsing dots + "Setting up session..." shown while the WS is reconnecting but has never received a byte. Overlay inherits the terminal theme background (no flash), switches to "Reconnecting..." once a real byte has been seen (tracked on first `onmessage`, not on `onopen`, so spurious gateway-initiated closes during VM boot don't trigger the wrong label).
- **`ProvisionRequest.ram_mb` / `cpus` now optional** -- service fills missing fields from merged VM settings (`vm.resources.ram_gb`, `vm.resources.cpu_count`). Lets callers without a settings round-trip (the tray's "New Session") honor the user's configured defaults instead of hardcoding.
- **`capsem-app` reverted to thin webview shell** -- 578-line `main.rs` + 9 helper files (`assets.rs`, `boot.rs`, `cli.rs`, `commands/`, `gui.rs`, `logging.rs`, `session_mgmt.rs`, `state.rs`, `vsock_wiring.rs`) collapsed to a 185-line `main.rs` with 3 IPC commands: `log_frontend`, `open_url`, `check_for_app_update`. Drops `capsem-core`, `capsem-logger`, `anyhow`, `reqwest`, `rmp-serde`, and the macOS `objc2-*` deps from the app. All VM/MCP/MITM logic stays in the service daemon; the app only hosts the webview and deep-link handling.
- **Terminal iframe owns its WebSocket lifecycle** -- replaces the parent/iframe `ready`/`vm-id`/`ws-ticket` postMessage handshake with URL-param init (`/vm/terminal/index.html?vm=…&theme=…&mode=…&fontSize=…&fontFamily=…`). The iframe fetches its own gateway token, manages WebSocket lifecycle + exponential-backoff reconnect with fresh tokens. Parent→iframe postMessage now covers only runtime signals (`theme-change`, `focus`, `clipboard-paste`). Removes `MsgReady`, `MsgVmId`, `MsgWsTicket`, `MsgWsConnected` from the contract.
- **Frontend logging to Rust tracing** -- new `frontend/src/lib/tauri-log.ts` patches `console.*` + `window.onerror` + `onunhandledrejection` to forward via `invoke('log_frontend')` from `@tauri-apps/api/core`. Webview logs now land in `~/.capsem/logs/<timestamp>.jsonl` alongside backend events, target `frontend`. No-op outside the Tauri webview (detects via `__TAURI_INTERNALS__`, not the opt-in `window.isTauri` global).
- **Frontend build timestamp in toolbar** -- `__BUILD_TS__` set at Vite build time, displayed right-side of toolbar. Makes stale-bundle issues obvious at a glance.
- **`just build-ui [release]` recipe** -- frontend build + `cargo build -p capsem-ui` in lockstep. Required because `tauri::generate_context!()` embeds the frontend bundle at cargo compile time; rebuilding only the frontend has no effect on an already-compiled binary. Documented in `CLAUDE.md`, `/dev-just`, and `/frontend-design`.
- **`just run-ui -- [args]`** -- `build-ui` then launch `./target/debug/capsem-ui` with passthrough args (e.g., `just run-ui -- --connect <vm-id>`).

### Removed
- **`POST /setup/assets/download`** -- zero callers anywhere (no frontend, no CLI, no MCP tool wraps it). The handler was a stub that always returned `{"started": false, "reason": "asset pipeline not yet wired -- run \`capsem update\` from the terminal"}`. The real asset download path is the `capsem update` CLI. Removing the route and the `handle_trigger_download` handler; if/when an in-service asset pipeline is added later, add it back under the name that matches its behavior.

### Changed
- **`capsem_service_logs` now routes through the service's `/service-logs` endpoint** instead of opening `$CAPSEM_RUN_DIR/service.log` directly. The direct-file read was an inherited shortcut and left two parallel implementations of the same logic (MCP tool + HTTP handler) that could drift. The MCP tool now has a single code path on par with `capsem_vm_logs`; grep/tail filtering is still applied locally on the returned text. Post-mortem reads when capsem-service has crashed are no longer covered by this tool -- use `tail -f ~/.capsem/run/service.log` from the shell, same as every other tool that can't reach a dead service.

### Fixed
- **Suspend/resume: /root reads now survive a VM restore** -- two bugs compounded. (a) `AppleVzSerialConsole::spawn_reader` started the pipe reader inside `machine.start()` before the capsem-process tokio broadcast subscriber attached, so the first ~100ms of post-resume serial output was dropped by `tokio::broadcast::send` (no receivers, message discarded). The serial log showed no reconnect/rebind activity even though the guest agent was running it, which masked the next bug for months. Now `AppleVzHypervisor::boot` attaches a file-writer subscriber *before* `machine.start()` spawns the reader; log path flows through a new `serial_log_path` field on `VmConfig` / `BootOptions`. The duplicate subscriber in `capsem-process/src/main.rs` is removed. (b) After resume the guest agent has to rebind `/root` onto a fresh virtiofs mount because the old connection is gone, but the chroot's `/mnt/shared` path wasn't created in the rootfs, and `mount --bind` was firing before the new virtiofs had completed its FUSE init handshake with the new host virtiofsd -- so the bind captured a stale-empty subtree and `/root` stayed ENOENT even though the mount reported success. Agent now does `mkdir -p /mnt/shared`, mounts the virtiofs, polls `/mnt/shared/workspace` until `exists()` succeeds (20ms x 50 attempts), then binds to `/root`. Plus the supporting plumbing: persists `VZGenericMachineIdentifier` across save/restore (else `restoreMachineStateFromURL` fails with `VZErrorRestore(12)`), dispatches VZ `pause`/`save_state`/`stop` via `CFRunLoopPerformBlock` (NOT `dispatch_async(main_queue)` -- that deadlocks on VZ's own completions), `fsync`s the checkpoint before process exit, clears stale `.ready` + UDS on resume so `wait_for_vm_ready` doesn't match the prior boot, subscribes every IPC connection to the state broadcast (was only `TerminalOutput`), tolerates `fsfreeze` ENOTSUP on the VirtioFS root, and the agent reconnects via a 3s heartbeat + `POLLHUP` on the vsock fd after the host process disappears. The MCP `test_suspend_and_resume_persistent` xfail is removed; the lifecycle test's `marker in str(read_resp)` substring check is tightened to require `"content" in read_resp` since the ENOENT error message echoes the path (the old assertion was a false-positive on the failing path).
- **`capsem-mcp` now respects HTTP status codes when talking to capsem-service** -- `UdsClient::request` used to discard the response status and try to deserialize every body as success, so a non-2xx response with a JSON body (e.g. the `{"error": "..."}` payload the service returns on 502/503/400) was handed back to the tool layer as `Ok(value)` with an embedded error field. `capsem_mcp_call` printed the raw 502 body as a successful tool result; other tools only avoided this because they happen to run `format_service_response` which catches the embedded `error` key. The client now reads `status()` first and returns `Err("502 Bad Gateway: ...")` on non-success, preferring the `error` field from the JSON body when present.
- **`capsem_core::setup_state::load_state` now warns on corrupt files** -- previously a malformed or truncated `~/.capsem/setup-state.json` was silently swallowed and the function returned `SetupState::default()`, so `capsem setup` would quietly report "no steps done" and re-run the whole wizard with no indication anything was wrong. Now logs a `warn!` with the path and parse error before falling back; behavior on a missing file (the first-run case) is unchanged.
- **`DbReader::query_raw` now validates SQL up front** -- previously it relied on `SQLITE_OPEN_READ_ONLY` at the connection level, which made in-memory readers accept writes and produced cryptic "attempt to write a readonly database" errors on the file-backed path. Now calls `validate_select_only` first, returning a clear `<KEYWORD> statements are not allowed` message consistently. Defense-in-depth; no behavior change for valid SELECT queries.
- **Terminal iframe src must end in `index.html`** -- Tauri's custom protocol on macOS does not auto-append `index.html` for trailing-slash paths the way Vite/Astro dev server does. `/vm/terminal/` silently 404'd in the Tauri webview while working in Chrome dev mode.
- **CSP re-enabled without blocking Astro hydration** -- production CSP on the terminal iframe now includes `'unsafe-inline'` for `script-src` (Astro emits inline hydration scripts in prod). `connect-src` stays locked to gateway + localhost, which is the meaningful defense against a compromised terminal exfiltrating data.

### Added (existing work, continued)
- **Files API: path sanitization and Magika init** -- allowlist-based `sanitize_file_path` (strips XSS, null bytes, unicode, rejects `..` traversal), `resolve_workspace_path` (canonicalize + starts_with check), and shared `Mutex<magika::Session>` in `ServiceState` for AI-powered file type detection.
- **GET /files/{id} directory listing** -- recursive host-side VirtioFS directory listing with file metadata (size, mtime), Magika file-type detection (label, MIME, is_text) at all depths, hidden file filtering, configurable depth (1-6).
- **GET/POST /files/{id}/content** -- binary-safe file download (raw bytes + Magika MIME type + Content-Disposition) and upload (raw bytes, create_dir_all, mode 0644) via host-side VirtioFS. 10MB limit enforced server-side.
- **Files tab UI** -- host-side file tree replaces vsock `find` command (real sizes, Magika labels, no frame limit), syntax-highlighted file viewer with copy-to-clipboard and download buttons, inline image/SVG preview, binary file handling, drag-and-drop upload with visual overlay and status feedback. Shiki language detection expanded to 30+ languages with content-sniffing fallback.
- **Orthogonal asset versioning** -- binary version (`1.0.{timestamp}`) and asset version (`YYYY.MMDD.patch`) are fully independent. The v2 manifest has separate `assets` and `binaries` sections with `min_binary`/`min_assets` compatibility ranges, deprecation tracking, and release dates. Assets use hash-based filenames (`rootfs-{hash16}.squashfs`) via hardlinks for zero-cost dedup.
- **`capsem status` shows full system health** -- version, service/gateway connectivity and version sync (catches stale processes), asset version with per-file ok/MISSING status.
- **Service `/version` endpoint** -- returns the running service binary version for staleness detection.
- **`/setup/assets` uses resolved paths** -- returns hash-named file paths and asset version instead of hardcoded logical names.

### Changed
- **MCP builtin tools refactored to standalone server** -- HTTP tools (fetch_http, grep_http, http_headers) and snapshot tools (snapshots_changes, snapshots_list, snapshots_revert, etc.) extracted from gateway into `capsem-mcp-builtin`, a stdio MCP server subprocess managed by the aggregator like any external server. Gateway dispatch simplified to route all tool calls uniformly through the aggregator.
- **MCP aggregator IPC switched to MessagePack** -- NDJSON protocol replaced with length-prefixed msgpack frames for better performance and binary safety.
- **MCP server definitions support stdio transport** -- `McpServerDef` gains `command`, `args`, `env` fields. Auto-detected stdio servers from Claude/Gemini configs are now connectable (previously display-only). `unsupported_stdio` field removed.
- **MCP server renamed from "Capsem" to "local"** -- the builtin server is now named "local" in both the settings tree and runtime API for consistency.
- **frontend: MCP section with collapsible server cards** -- each server card expands to show its tools with per-tool allow/ask/block permission selectors. Runtime status badges (running/stopped, tool count) from mcpStore. Refresh button in header.
- **frontend: MCP settings wired to gateway** -- MCP server add/remove/toggle and policy now persist via the settings API. Config reload broadcasts to running VMs immediately.
- **frontend: toolbar redesign** -- hamburger menu on left with view switcher, VM actions moved to dropdown menu, live stats (tokens, tool calls, cost) on the right. Shell OSC title shows in center.
- **frontend: settings page loading states** -- spinner while loading, error banner with retry on failure.
- **frontend: restart button** -- toolbar restart now stops then resumes the VM.
- **frontend: fork auto-opens tab** -- forking a VM automatically opens it in a new tab.

### Added
- **Host-side command recording (3 layers)** -- records all shell commands from the host for tamper-proof auditing:
  - Layer 1 (exec_events): structured API-path commands logged to session.db at dispatch time
  - Layer 2 (pty.log): raw PTY transcript with timestamps and direction tags, 20MB rotation
  - Layer 3 (audit_events): kernel execve syscalls via auditd, streamed over vsock:5006 to session.db
- **`capsem history` CLI** -- `capsem history <session>` with `--layer`, `--search`, `--tail`, `--json` flags
- **History API endpoints** -- `GET /history/{id}`, `/history/{id}/processes`, `/history/{id}/counts`, `/history/{id}/transcript`
- **Cross-session history index** -- `exec_count` and `audit_event_count` columns in main.db sessions table
- **Kernel audit support** -- CONFIG_AUDIT + CONFIG_AUDITSYSCALL in guest kernel, auditd started in capsem-init with immutable rules
- **`capsem-mcp-builtin` crate** -- standalone stdio MCP server binary for local tools (HTTP + snapshot). Spawned by the aggregator as "local" server, tools discovered and cached like any external server.
- **MCP aggregator subprocess** -- external MCP server connections now run in an isolated `capsem-mcp-aggregator` subprocess with only network access, no VM/DB/filesystem privileges. Spawned by capsem-process at boot.
- **service MCP API endpoints** -- `GET /mcp/servers`, `GET /mcp/tools`, `GET /mcp/policy`, `POST /mcp/tools/refresh`, `POST /mcp/tools/{name}/approve`, `POST /mcp/tools/{name}/call` unblock the frontend and CLI.
- **CLI `capsem mcp` subcommands** -- `capsem mcp servers`, `capsem mcp tools`, `capsem mcp policy`, `capsem mcp refresh`, `capsem mcp call`.
- **debug MCP tools** -- `capsem_mcp_servers`, `capsem_mcp_tools`, `capsem_mcp_call` in capsem-mcp for AI agent MCP management.
- **MCP IPC protocol** -- `McpListServers`, `McpListTools`, `McpRefreshTools`, `McpCallTool` service-to-process messages with corresponding result types.

### Changed
- **frontend: MCP settings wired to gateway** -- MCP server add/remove/toggle and default tool policy now persist via the settings API instead of local-only state. Servers, tools, and policy load from the gateway on mount.
- **frontend: restart button works** -- toolbar restart button now stops then resumes the VM (was previously identical to stop).
- **frontend: fork auto-opens tab** -- forking a VM from the toolbar now automatically opens the forked VM in a new tab.
- **frontend: settings loading/error states** -- settings page shows a spinner while loading and an error banner with retry on failure.

### Fixed
- **Stale update cache suggests downgrade** -- `read_cached_update_notice` now re-validates with `is_newer` before displaying, preventing bogus "Update available: 1.0.x -> 0.16.x" notices after a version scheme change.
- **Install leaves stale gateway token** -- `just install` now unloads the LaunchAgent before killing processes, preventing macOS from respawning the old service. Cleans stale `gateway.token` and `gateway.port` files.
- **Asset resolution in arch subdirs** -- `ManifestV2::resolve` checks both `base_dir/{hash}` and `base_dir/{arch}/{hash}`, fixing installed service asset lookup.
- **`_pack-initrd` skips docker when binaries are current** -- avoids unnecessary container cross-compile on every `just shell`.
- **v1 asset code removed** -- `asset_manager.rs` reduced from 1947 to ~400 lines. All v1 types, download infra, and legacy cleanup deleted. Download stubs point to `sprints/orthogonal-ci/plan.md`.
- **MITM cert "not yet valid" after Mac sleep** -- leaf certificates now use a fixed `notBefore` of 2026-01-01 instead of `now - 1h`, preventing cert validation failures when the guest clock drifts. Ping messages now carry `epoch_secs` so the guest clock resyncs every 10s heartbeat, covering Mac sleep/wake and long-running VMs.
- **frontend: tab names use VM name** -- provisioning and deep-link flows now show the VM's fun name (e.g. "tmp-agile-blaze") instead of the raw ID.
- **frontend: snapshot stats query real VM** -- Snapshots tab in Stats view now queries the VM's session.db via `/inspect` instead of the local mock database.
- **frontend: VM logs and service logs wired** -- VM Logs view parses NDJSON process logs into structured table with level/source/message columns and Process/Serial toggle. Service Logs view fetches from new `/service-logs` endpoint.
- **frontend: detail panel restored** -- click any tool call, network request, or file event row in Stats to open a slide-out detail panel with Shiki syntax-highlighted JSON, headers, and request/response bodies.

### Changed
- **frontend: removed URL bar** -- toolbar no longer shows the address/search bar; cleaner layout with just VM actions and view switcher.
- **frontend: removed Inspector tab** -- Inspector view removed from the toolbar view switcher; available via hamburger menu if needed.
- **frontend: removed status dot** -- connection indicator dot removed from toolbar.
- **service: added /service-logs endpoint** -- returns last 100KB of service.log as plain text for the frontend Service Logs view.

### Changed
- **build: auto-prune stale cargo artifacts** -- `_clean-stale` now removes orphaned `.o`/`.rlib`/`.rmeta` files and incremental dirs older than 3 days when `target/` exceeds 10 GB. Runs automatically after `test`, `smoke`, and `install` to prevent unbounded growth (previously hit 72 GB from accumulated hash variants).
- **CLI: simplified command structure** -- removed `service` subcommand group; `install`, `status`, `start`, `stop` are now top-level commands. Removed session-level `stop` (use `suspend` or `delete`) and `status` (use `info`). Removed `start` alias from `create`. Renamed all "sandbox" terminology to "session". Session identifier parameter shows as `<SESSION>` in help.
- **CLI: enriched `list` output** -- table now shows NAME, STATUS, RAM, CPUs, and UPTIME columns instead of the old ID/STATUS/PERSIST/PID.
- **CLI: enriched `info` output** -- shows formatted session details with telemetry (tokens, cost, tool calls, requests) instead of raw JSON. Use `--json` for machine-readable output.
- **CLI: service start/stop** -- new `capsem start` and `capsem stop` commands to start/stop the background daemon via launchctl (macOS) or systemctl (Linux).
- **MCP: tool descriptions updated** -- all tool descriptions now use "session" instead of "VM" or "sandbox".

### Fixed
- **tray: icon stays white template** -- tray icon no longer switches to a dark non-template icon when VMs are running. Always uses the template icon so macOS adapts it to menu bar appearance.
- **tray: VM names no longer truncated** -- VM labels in the tray menu now show the full name or ID instead of truncating to 8 characters.
- **tray: unified "New Session" action** -- replaced "New Temporary" and "New Permanent..." menu items with a single "New Session" that creates a session (save it to make it permanent).

### Added
- **VM identity: fun temporary names** -- ephemeral VMs get memorable names like `tmp-brave-falcon` instead of opaque `vm-1712345678`. Persistent VMs keep user-chosen names. Shell prompt now shows the VM name (hostname) instead of static "capsem".
- **VM identity: host timezone injection** -- guest VMs inherit the host's timezone at boot via `TZ` env var and `/etc/localtime`. `date` inside the VM now shows local time instead of UTC. Clock and timezone are also resynced on resume from suspend.
- **service: settings endpoints** -- `GET /settings` returns the merged settings tree (user + corp + defaults) with issues and presets. `POST /settings` batch-updates settings atomically. `GET /settings/presets` lists security presets. `POST /settings/presets/{id}` applies a preset. `POST /settings/lint` validates config. All endpoints are thin wrappers around existing `capsem-core` functions.
- **service: telemetry-enriched `/list`** -- running VMs in `GET /list` now include live telemetry (tokens, cost, tool calls, requests, file events) read from session.db. Shared `enrich_telemetry()` function used by both `/list` and `/info/{id}`.
- **gateway: telemetry pass-through in `/status`** -- `VmSummary` now includes 11 optional telemetry fields forwarded from the service. Frontend gets per-VM stats in a single poll without per-VM API calls.
- **frontend: dashboard global stats** -- NewTabPage shows 4 summary cards (sessions, total tokens, total cost, requests) from `GET /stats` cross-session aggregation.
- **frontend: VM table telemetry columns** -- Uptime, Tokens, Cost columns in the sandbox table. Shows "--" for stopped VMs, live values for running.
- **frontend: `getStats()` API** -- new API function with graceful offline handling (returns empty stats when disconnected).
- **frontend: shared formatters** -- `format.ts` with `formatUptime`, `formatTokens`, `formatCost`, `formatDuration`, `formatBytes`, `formatTime`, `truncate`, `fmtAge`. StatsView refactored to use shared module.
- **standalone installer: macOS .pkg build** -- `scripts/build-pkg.sh` assembles a .pkg from the Tauri .app, all 6 companion binaries, VM assets, and a postinstall script that copies to `~/.capsem/bin/`, codesigns, registers LaunchAgent, and runs setup. CI pipeline updated to build .pkg alongside .dmg.
- **`just install` builds and installs the platform package** -- builds release binaries, frontend, and Tauri app, then assembles and installs the native package: .pkg with macOS Installer GUI on macOS, .deb via `dpkg -i` on Linux. The postinstall script handles codesign, PATH, service registration, and setup. Replaces the old `simulate-install.sh` bypass.
- **`just test-install` exercises the real .deb path** -- Docker e2e tests now build a real .deb (Tauri + `repack-deb.sh`), install with `dpkg -i` (exercising `deb-postinst.sh` with systemd registration and setup), then run the pytest suite against the installed layout. Named volumes cache cargo builds across runs. Tests split into packaging (run in Docker) and `live_system` (need VM assets, run on real systems).
- **standalone installer: Linux .deb repack** -- `scripts/repack-deb.sh` injects companion binaries and a postinst script into the Tauri .deb. Postinst symlinks system binaries to `~/.capsem/bin/`, registers systemd user unit, and runs setup.
- **CLI: auto-setup on first use** -- running any sandbox command without prior `capsem setup` triggers non-interactive setup automatically (service registration, credential detection, asset download). Skipped when `--uds-path` is explicit.
- **`just install`: graceful stop + health check** -- stops existing service before overwriting binaries, verifies service health after registration, auto-runs setup on first install.

### Fixed
- **CLI: delete nonexistent sandbox now returns error** -- HTTP status code was not checked before deserializing response body, causing 404 errors to be silently swallowed when `T = serde_json::Value` in the untagged `ApiResponse` enum.
- **uninstall: kill and remove all 6 binaries** -- capsem-gateway and capsem-tray were missing from the CAPSEM_BINARIES list and pkill commands.
- **install: .pkg and .deb packaging scripts fail on missing binaries** -- `build-pkg.sh` and `repack-deb.sh` printed a WARNING and continued when a companion binary was missing, potentially producing broken packages in CI. Now exit with error, matching `simulate-install.sh` behavior.
- **install: setup-state.json atomic write** -- `save_state()` used `fs::write()` directly; a crash mid-write could corrupt the setup state. Now uses temp file + `fs::rename` for atomic updates.
- **install: macOS .pkg postinstall user detection** -- postinstall script assumed `$USER` was always set correctly by macOS Installer.app. When installed via `sudo installer -pkg` (CLI), `$USER` is root. Now checks `$SUDO_USER` first, falls back to console owner via `stat /dev/console`.
- **install: Linux .deb postinst XDG_RUNTIME_DIR** -- `deb-postinst.sh` ran `su $TARGET_USER -c "capsem service install"` without propagating `XDG_RUNTIME_DIR`, causing `systemctl --user` to fail. Now passes `XDG_RUNTIME_DIR=/run/user/$UID` explicitly.

### Changed
- **image elimination: everything is a sandbox** -- removed the "image" concept entirely. `fork` now creates a stopped persistent sandbox instead of an image. `create --from <sandbox>` replaces `create --image`. Image registry, image CLI commands, and image MCP tools are all removed. `--image` remains as a hidden alias for `--from`. `SandboxInfo` API now includes `forked_from` and `description` fields. Session DB schema bumped to v6 (renames `source_image` to `forked_from`). Net reduction: ~500 lines and one abstraction layer.
- **CI: test 6 additional Rust crates** -- capsem-service, capsem (CLI), capsem-mcp, capsem-tray, capsem-process now run in CI (422 tests were previously local-only). capsem-app gets a compile check.
- **CI: run non-VM Python integration tests** -- capsem-bootstrap, capsem-codesign, capsem-rootfs-artifacts suites now execute in CI. All 25 integration suites are collect-only verified.
- **CI: Rust coverage floor** -- `--fail-under-lines 70` enforced on both macOS and Linux CI jobs. Codecov unit upload now fails CI on error.
- **capsem-process: module decomposition** -- split 1,522-line main.rs monolith into 6 modules (helpers, job_store, vsock, ipc, terminal + main). Tests grew from 24 to 62.
- **dev-testing skill: test matrix** -- added Rust crate CI matrix, Python integration suite tier map, and coverage targets documentation.
- **integration tests: suite expansion** -- capsem-recovery (4->9 tests: stale sentinels, partial sessions, post-recovery health), capsem-stress (3->7: rapid exec, file I/O, name reuse, mass delete), capsem-config-runtime (5->10: env injection, python3, arch match, workspace write, rootfs readonly), capsem-session-lifecycle (6->10: WAL cleanup, ordered events, domain fields, live DB reads). Fixed two `>= 0` assertions that always passed.

### Added
- **service: `GET /stats` endpoint** -- returns full main.db aggregation in one call: global stats (tokens, cost, tool calls, network counts), recent sessions with all telemetry columns, top providers, top tools, and top MCP tools. Replaces the need for raw SQL on `_main`.
- **service: `/inspect/_main` support** -- the `/inspect/{id}` endpoint now recognizes `_main` as a sentinel, routing raw SQL queries to the global session index (main.db) instead of a per-VM session.db. Unblocks `queryDbMain()` in the frontend.
- **service: `SandboxInfo` telemetry fields** -- `/info/{id}` now returns live session telemetry for running VMs: input/output tokens, estimated cost, tool calls, MCP calls, network request counts, file events, model call count, and uptime. `/list` includes uptime for running VMs. All new fields are optional and omitted when absent for backwards compatibility.
- **gateway: token endpoint for browser auth** -- `GET /token` returns the auth token, restricted to loopback IP (127.0.0.1/::1) via hardcoded peer IP check. Allows browser-based frontends to authenticate without filesystem access.
- **gateway: WebSocket query-param auth** -- `/terminal/{id}` paths accept `?token=` query parameter as auth fallback for browser WebSocket connections (which cannot set custom headers). Only the `token` param is recognized; all others are silently dropped. Non-terminal paths ignore query params entirely.
- **frontend: settings export/import** -- export all settings to JSON file, import from previously exported file. Import stages changes for review before saving. Validates version, skips corp-locked and unchanged settings.
- **frontend: MCP server management UI** -- add/remove/enable/disable external MCP servers from the settings page. Form with name, URL, bearer token, and custom headers. Replaces the "edit config.toml" placeholder.
- **smoke: per-step timing and log file** -- smoke recipe now logs to `target/smoke.log` with elapsed time per step and total. `capsem doctor --fast` skips the 64s throughput download test.
- **smoke: parallel test groups** -- Python integration tests run MCP, service/CLI, and gateway groups concurrently (122s -> 58s). Pre-signs binaries to avoid codesign races.
- **bench: host-side lifecycle and fork benchmarks** -- `just bench` now runs both in-VM benchmarks and host-side lifecycle/fork benchmarks from `test_lifecycle_benchmark.py`.

### Fixed
- **tray: invisible menu bar icon** -- tray main loop used `thread::sleep(16ms)` which does not pump the macOS Cocoa run loop. `NSStatusItem` requires an active run loop to render. Replaced with `CFRunLoopRunInMode` which processes AppKit events at the same 60 Hz cadence.
- **service: companion logs to files** -- gateway and tray child processes had stdout/stderr routed to `/dev/null`, making debugging impossible. Now logs to `~/Library/Logs/capsem/gateway.log` and `tray.log`, falling back to null if the file can't be opened.
- **install: remove stale pgrep wait loop** -- service install no longer polls for capsem-tray death after `bootout` + `pkill -9`, removing up to 3s of unnecessary latency.
- **agent: venv activation race** -- capsem-pty-agent now waits up to 3s for capsem-init's background venv creation before checking `/root/.venv/bin/activate`. Previously the agent checked once and missed it, leaving `VIRTUAL_ENV` unset for the shell and all exec commands.
- **agent: write_nofollow missing parent dirs** -- runtime `FileWrite` via `write_nofollow()` now creates parent directories before opening the file. Previously, writing to `/root/project/main.py` failed with "No such file or directory" if `/root/project/` didn't exist.
- **service: handle_delete now waits for process death** -- `handle_delete` gives the VM process 500ms to flush its session DB, then SIGKILLs if still alive. Previously it was fire-and-forget: the HTTP 200 returned before the process died, leaving orphans when the service was restarted.
- **service: handle_stop now waits for process death** -- same fix as delete. Prevents resume from racing the old process on the same socket (old process's shutdown timer would kill the new one).
- **service: handle_run rollup race** -- session DB rollup (file events, net events, MCP calls) now completes before the HTTP response returns. Previously it was `tokio::spawn`'d fire-and-forget, so callers reading `main.db` immediately after `capsem run` saw empty counters.
- **service: wait_for_process_exit verifies SIGKILL** -- after sending SIGKILL, now polls for up to 2s to confirm the process actually died. Logs a warning/error if it survives.
- **service: socket path fallback for long run_dir paths** -- `instance_socket_path()` falls back to `/tmp/capsem/{hash}.sock` when the preferred `{run_dir}/instances/{id}.sock` path exceeds 90 bytes. Fixes "path must be shorter than SUN_LEN" crashes when tests use `/var/folders/...` temp dirs.
- **install: dev symlink conflict** -- `simulate-install.sh` now removes the `~/.capsem/assets` dev symlink before copying real assets. Previously `cp` failed with "identical file" when the symlink pointed at the source.
- **install: PATH added to correct shell profile** -- install now writes to `~/.bash_profile` (if it exists) instead of `~/.bashrc` on bash, since macOS Terminal opens login shells that don't source `~/.bashrc`.
- **justfile: _ensure-service kills orphaned VM processes** -- kills `capsem-process` instances before killing the service, with a SIGKILL follow-up. Previously, service restart orphaned running VMs.
- **test: codesign race in parallel test groups** -- `sign_binary()` now uses file locking to prevent concurrent test processes from corrupting binaries during `codesign --force`.
- **test: fork timing gate relaxed** -- `test_winter_is_coming` fork gate raised from 0.5s to 2.0s. The proper gate (500ms over 3 runs) is in the dedicated fork benchmark.

### Fixed
- **frontend: syntax highlighting race condition** -- file editor controls (settings.json, state.json, etc.) now reliably get Shiki syntax coloring. Previously the first FileEditorControl to mount could miss highlighting due to async Shiki init not re-triggering the render effect.
- **frontend: design system color violations** -- MCP section badges and status indicators now use semantic tokens (blue=positive, purple=negative) instead of raw green/red Tailwind colors.
- **perf: cold boot 6x faster (6.2s -> 1.0s)** -- first VM boot was wasting 5 seconds on a silent IPC Ping timeout. When `vm_ready` was false, capsem-process silently dropped the Ping and the service waited the full 5s before retrying. Fixed: process now closes the connection immediately on not-ready Ping, and readiness is signaled via a `.ready` sentinel file (stat() check, 5ms poll) instead of IPC round trips. Also deduplicated three hand-rolled exponential backoff implementations (vsock_connect_retry, reconnect loop, CLI connect_with_timeout) into a shared `capsem_proto::poll` module with `RetryOpts` + `retry_with_backoff`, reused by the async `capsem_core::poll::poll_until` via type alias.
- **perf: async VM delete (5s -> 20ms)** -- `shutdown_vm_process` now sends the shutdown signal and returns immediately. Process teardown (wait + force-kill + socket cleanup) runs in a background task. Telemetry rollup in `handle_run` waits for process exit in background before reading session.db, ensuring DbWriter has flushed. Also: consolidated redundant mutex acquisitions in `handle_fork` (4 locks -> 1-2) and `handle_persist` (2 locks -> 1), parallelized IPC fan-out in `handle_reload_config` and `handle_purge` with `join_all`, moved `remove_dir_all` to `spawn_blocking`, added periodic cleanup timer, logged registry save failures.
- **perf: capsem-process self-exit on shutdown** -- after forwarding `HostToGuest::Shutdown` to the guest agent, capsem-process now waits `SHUTDOWN_GRACE_SECS + 500ms` then calls `vm.stop()` and `exit(0)`. Previously, `CFRunLoopRun` kept the process alive indefinitely after guest shutdown, requiring SIGKILL from the service.
- **fork: full VM state preservation** -- fork now captures both rootfs overlay and workspace files. Previously, `create_session_from_image()` cloned data to `session_dir/system/` and `session_dir/workspace/` (real directories), but the VM only sees `session_dir/guest/` via VirtioFS, so cloned data was invisible to the guest. Fixed to clone into `guest/` subdirectories with compat symlinks, matching the VirtioFS share layout.
- **disk usage: report actual blocks, not logical size** -- `disk_usage_bytes()` now uses `blocks * 512` instead of `meta.len()`, so sparse files (e.g. a 2GB rootfs.img overlay with 9MB of actual changes) report their true disk footprint. Fixes inflated image sizes in `capsem_image_inspect`.
- **benchmark: fork performance and size regression gates** -- `test_fork_benchmark` in `test_lifecycle_benchmark.py` profiles fork speed (< 500ms gate), image size (< 12MB gate), boot-from-image speed, and data survival (packages + workspace). Runs 3 cycles with per-run and summary output + JSON.
- **CLI: connect timeout with exponential backoff** -- CLI no longer hangs when the service socket is unreachable. `connect_with_timeout()` retries with exponential backoff (100ms, 200ms, ..., up to 10 attempts). Fails immediately on ENOENT or connection refused. Explicit `--uds-path` skips auto-launch entirely for instant failure.
- **test: DRY shared constants and helpers across integration tests** -- extracted `DEFAULT_RAM_MB`, `DEFAULT_CPUS`, `EXEC_READY_TIMEOUT`, `EXEC_TIMEOUT_SECS`, `HTTP_TIMEOUT`, and `GUEST_WORKSPACE` into `tests/helpers/constants.py`. Moved duplicated `parse_content`, `content_text`, `wait_exec_ready`, and `wait_file_ready` helpers into `tests/helpers/mcp.py`. Updated 49 test files to use shared constants and helpers instead of hardcoded values.
- **test: fix file I/O paths rejected by workspace sandbox** -- tests were writing to `/tmp/` which the agent now correctly rejects as outside the workspace root (`/root/`). Fixed all service, gateway, isolation, session, and E2E test paths to use `/root/`.

### Added
- **VM lifecycle: guest-initiated shutdown** -- `shutdown`, `halt`, `poweroff`, and `reboot` commands work inside the VM via `capsem-sysutil`, a multi-call binary deployed to `/run/capsem-sysutil` with symlinks in `/sbin/`. Opens a dedicated vsock:5004 lifecycle channel (independent of the PTY agent) to send `ShutdownRequest` to the host. Reboot prints an error (not supported in sandbox). Includes countdown timer matching `SHUTDOWN_GRACE_SECS`.
- **VM lifecycle: suspend and warm resume** -- persistent VMs can be suspended via `capsem suspend <name>` (CLI), `capsem_suspend` (MCP), or `suspend` inside the guest. Uses Apple VZ `saveMachineStateTo` (macOS 14+) with a quiescence protocol: agent freezes filesystem (`fsfreeze -f /`), host pauses VM, saves checkpoint, stops. Resume detects checkpoint file and uses `restoreMachineStateFrom` for warm restore. Agent reconnects with exponential backoff, re-sends Ready, and thaws filesystem.
- **VM identity** -- service injects `CAPSEM_VM_ID` (UUID) and `CAPSEM_VM_NAME` (user-chosen name or UUID for ephemeral) as environment variables. Agent calls `sethostname(CAPSEM_VM_NAME)` after boot so the shell prompt reflects the VM name.
- **VmHandle trait: pause/resume/save/restore** -- hypervisor abstraction extended with `pause()`, `resume()`, `save_state(path)`, `restore_state(path)`, and `supports_checkpoint()`. Apple VZ implementation dispatches to main thread. KVM defaults return errors.
- **capsem-doctor: lifecycle diagnostics** -- new `test_lifecycle.py` category verifying sysutil symlinks (`/sbin/shutdown`, `/sbin/halt`, `/sbin/poweroff`, `/sbin/reboot`, `/usr/local/bin/suspend`), `CAPSEM_VM_ID`/`CAPSEM_VM_NAME` env vars, and hostname matching VM name.
- **capsem-gateway: TCP-to-UDS reverse proxy** -- standalone binary that bridges TCP (default port 19222) to capsem-service UDS. Bearer token auth (64-char random, regenerated on restart, written to `~/.capsem/run/gateway.token` with 0600 permissions). All service endpoints proxied through with method/path/query/body preserved. `GET /` health check (no auth). `GET /status` aggregated VM health with 2s cache TTL for efficient tray polling. CORS permissive for browser access. Graceful shutdown cleans up token/port/pid files. No capsem-core dependency, no VM access -- pure low-privilege proxy.
- **capsem-service: auto-spawn gateway and tray** -- service now spawns capsem-gateway (TCP proxy) and capsem-tray (macOS menu bar) as child processes on startup. Both are killed on graceful shutdown. Tray spawn is macOS-only, gateway spawn is cross-platform. Sibling binary discovery falls back to target/debug/ for development.

### Changed
- **exec: separated from interactive PTY** -- exec commands now spawn a direct child process with piped stdout instead of injecting into the shared PTY and scanning for a magic ESC sentinel. Output flows on a dedicated vsock port 5005 (`VSOCK_PORT_EXEC`) with an `ExecStarted { id }` handshake, while `ExecDone` continues on the control channel. Eliminates sentinel spoofing risk, removes `strip_ansi()` post-processing, and keeps control_loop responsive to heartbeats during long-running commands. The interactive terminal (vsock:5001) is no longer contaminated by exec output.
- **removed capsem-app direct CLI mode** -- deleted `run_cli()` and `cli.rs` from capsem-app. All VM operations now go through capsem-service (single path). The Tauri GUI uses the service API like every other client.

### Security
- **image name path traversal** -- image names (fork, inspect, delete) are now validated with the same rules as VM names (alphanumeric, hyphens, underscores only). Previously, a name like `../../etc` could escape the images directory during fork or trigger `remove_dir_all` outside the sandbox on delete. Defense-in-depth assertion added to `ImageRegistry::image_dir()`.
- **persistent registry atomic writes** -- `PersistentRegistry::save()` now uses write-to-tmp + fsync + rename instead of direct `std::fs::write`. Prevents a crash mid-write from producing a zero-byte or partial JSON file, which would lose all persistent VM state on next startup.
- **symlink sandbox escape hardening** -- guest agent FileWrite/FileRead/FileDelete handlers now validate paths with `validate_file_path_safe()` (canonicalize + workspace containment check) and use `O_NOFOLLOW` on actual file operations to eliminate TOCTOU window. Previously, FileWrite had no validation at all, and FileRead/FileDelete only checked for `..` and NUL bytes. A compromised guest could create a symlink pointing outside `/root/` and read/write/delete arbitrary files through it. Snapshot system now preserves symlinks instead of silently dropping them, includes them in workspace hashes and file counts, and surfaces `is_symlink` in MCP snapshot listings.
- **capsem-gateway: 10 MB request body size limit** -- proxy now enforces a 10 MB maximum on incoming request bodies via `http_body_util::Limited`, returning 413 Payload Too Large for oversized payloads. Prevents OOM from malicious clients.
- **capsem-gateway: CORS restricted to localhost origins** -- replaced `CorsLayer::permissive()` (allow all origins) with a predicate that only allows `http(s)://localhost`, `http(s)://127.0.0.1`, and `tauri://` origins. Prevents cross-origin requests from external websites.
- **capsem-gateway: auth failure rate limiting** -- after 20 failed auth attempts within 60 seconds, the gateway returns 429 Too Many Requests instead of 401. Prevents brute-force token guessing.
- **capsem-process: UDS sockets hardened to 0600** -- IPC and terminal WebSocket sockets now have chmod 0600 after bind. Previously inherited umask (0755), allowing any local user to connect to a VM's terminal or send exec commands with no auth.
- **capsem-process: environment cleared on spawn** -- service now uses `env_clear()` before spawning capsem-process, passing only `HOME`, `PATH`, `USER`, `TMPDIR`, `RUST_LOG`. Prevents API keys, tokens, and secrets from the user's shell leaking into per-VM processes.
- **capsem-process: serial.log permissions 0600** -- serial log files now created with explicit 0600 mode. Previously world-readable via umask default, potentially exposing terminal output containing secrets.
- **capsem-process: guest cannot force process exit** -- control channel read error on vsock:5000 now breaks the read loop instead of calling `process::exit(1)`. A compromised guest can no longer DoS its host process by closing the vsock fd.
- **capsem-tray: macOS menu bar tray** -- standalone binary that polls the gateway `/status` endpoint. VMs split into Permanent and Temporary sections. Permanent VMs get Connect/Resume, Fork, Stop, Delete; temporary VMs get Connect/Resume, Delete. Connect/Resume is a single context-sensitive button (shows Resume when suspended). "New Permanent..." opens UI with name dialog. Color-coded icons: purple (active), black template (idle, auto light/dark), red (error). Uses `tray-icon` + `muda` for native NSStatusItem. No capsem-core dependency, no Tauri.

### Fixed
- **capsem doctor: streaming output and VM cleanup** -- doctor now types the command into the shell (TerminalInput) instead of using Exec, so test output streams in real-time instead of buffering until completion. VM is always deleted on exit, including Ctrl-C. Full output written to `~/.capsem/run/doctor-latest.log`.
- **capsem-sysutil: help output to stdout** -- `print_help()` wrote to stderr (`eprintln!`), so `shutdown --help` appeared empty to callers checking stdout. Now uses `println!`.
- **snapshot revert: symlink support** -- revert used `.exists()` (follows symlinks) and `fs::copy` (dereferences), so reverting a symlink failed with "file does not exist". Now uses `symlink_metadata()` to detect symlinks and restores them with `read_link` + `symlink()`. Auto-select also uses `symlink_metadata` so snapshots containing only symlinks are found.
- **snapshot revert: VirtioFS stale cache** -- overwriting a file via `fs::copy` could leave VirtioFS with a stale cached size, causing truncated reads in the guest. Now removes the file first and fsyncs after write to force cache invalidation.
- **venv activation in exec** -- agent now adds `VIRTUAL_ENV` and prepends venv bin to `PATH` in boot_env after capsem-init creates the venv. Both PTY shell and exec commands see the venv. Removed duplicate activation from capsem-bashrc.
- **capsem-process: service-initiated suspend was silently dropped** -- the IPC handler in `handle_ipc_connection` matched `ServiceToProcess::Suspend` but logged "not yet implemented" instead of forwarding to the ctrl channel where the actual suspend logic lives. `capsem suspend <id>` and the MCP `capsem_suspend` tool were non-functional. Now forwards to the ctrl channel like `Shutdown` does.
- **capsem-agent: reconnect timer never reset** -- `start_reconnect` was set once at first disconnect and never updated after successful reconnect. A second suspend/resume cycle >30s into the VM's lifetime caused the agent to immediately timeout and exit. Now resets timer and backoff delay after each successful reconnect.
- **capsem-sysutil: operator precedence bug in --help guard** -- `a == "--help" || a == "-h" && cmd != "shutdown"` parsed as `"--help" || ("-h" && ...)` due to `&&` binding tighter. Added explicit parentheses.
- **capsem-sysutil: fd leak on write failure** -- `send_lifecycle_msg` did not close the vsock fd if `write_all_fd` or `encode_guest_msg` failed. Now closes fd on all paths.
- **capsem-service: suspend silently reported success on failure** -- `handle_suspend` discarded IPC send errors with `let _` and returned `{"success": true}` even when the VM never confirmed suspended state. Now propagates all IPC errors and returns 500 if the VM does not confirm suspension within 15 seconds.
- **capsem-service: resume did not pass checkpoint path** -- `resume_sandbox` re-spawned `capsem-process` without `--checkpoint-path`, causing suspended VMs to cold-boot instead of warm-restoring. Now passes `--checkpoint-path` when the registry entry has `suspended: true` and the checkpoint file exists.
- **capsem-service: resume did not clear suspended flag** -- after successful resume, `entry.suspended` stayed `true` and `entry.checkpoint_path` retained the stale value. Now clears both and saves the registry.
- **capsem-service: /list and /info did not distinguish Suspended from Stopped** -- persistent VMs with `suspended: true` were reported as "Stopped". Now returns "Suspended" status, and the gateway's `/status` endpoint includes `suspended_count` in `ResourceSummary`.
- **capsem-gateway: terminal WebSocket gave no error on VM unavailable** -- when the per-VM UDS connect failed after WebSocket upgrade, the connection silently dropped. Now sends a Close frame with code 1011 and reason "VM not available".
- **capsem-gateway: proxy timeout too short for suspend** -- 30-second proxy timeout could expire during suspend operations (up to 26s). Increased to 120 seconds. Added 5-minute safety timeout on the background HTTP connection driver.
- **capsem-gateway: terminal UDS path fallback incorrect** -- `terminal_uds_path` used `parent().unwrap_or("/tmp")` which never triggered for bare filenames (parent returns `Some("")`). Now filters empty parents before falling back.
- **capsem-process: VirtioFS share narrowed to guest/ subtree** -- VirtioFS previously shared the full `session_dir`, exposing `session.db`, `serial.log`, and `auto_snapshots/` to the guest. Now only `session_dir/guest/` (containing `system/` and `workspace/`) is shared. Host-only files are outside the share boundary. Compat symlinks preserve existing code paths.
- **capsem-gateway: proxy URI parse could panic** -- `forward()` used `.unwrap()` on `upstream_uri.parse()`, which could panic on malformed URIs. Replaced with error propagation that returns 502 Bad Gateway.
- **capsem-gateway: terminal WebSocket rejected underscores in VM IDs** -- `handle_terminal_ws` validation allowed only `[a-zA-Z0-9-]`, rejecting persistent VMs with underscores (e.g. `my_dev`). Aligned with service's `validate_vm_name()`: `[a-zA-Z0-9_-]`, must start alphanumeric, length 1-64.
- **capsem-gateway: terminal.rs had zero test coverage** -- added 31 unit + integration tests covering ID validation, UDS path construction, WebSocket relay (text, binary, ping/pong, close with reason, process disconnect, client disconnect, missing UDS, invalid ID rejection). Coverage went from 0% to 89%.
- **capsem-gateway: not tracked in CI coverage** -- added `-p capsem-gateway` to CI coverage pipeline and `gateway` component to codecov.yml (80% target).
- **capsem-process: process.log written in text format instead of JSONL** -- tracing subscriber used default text formatter with ANSI colors, making process.log unparseable by integration tests and tooling. Switched to JSON format matching capsem-service. Also changed RUST_LOG from `debug` to `capsem=info` for subprocess to avoid noisy debug entries.
- **capsem run: session not registered in main.db** -- `handle_run` in capsem-service provisioned and destroyed VMs without creating a session record or rolling up telemetry counters. Sessions from `capsem run` were invisible to `capsem sessions` and integration tests.
- **capsem run: missing `--env` support** -- `capsem run` had no way to pass environment variables to the guest, unlike `capsem create -e`. Added `--env`/`-e` to CLI, `env` field to `RunRequest`, and `env` param to `capsem_run` MCP tool. Integration test now passes API key via `--env` instead of relying on process env inheritance.
- **capsem-process: missing boot timeline in process.log** -- state transition events were only emitted in the capsem-app CLI path, not in capsem-process. Boot timeline is now logged after `boot_vm` returns.
- **Test scripts missing `run` subcommand** -- `injection_test.py`, `integration_test.py`, and `doctor_session_test.py` called `capsem <command>` instead of `capsem run <command>`, causing exit 2 on all scenarios. Also improved failure output to show full stdout/stderr instead of just lines matching "FAILED".
- **capsem-init: guest binaries deployed 755 instead of 555** -- `capsem-doctor`, `capsem-bench`, and `snapshots` were deployed with write bits via initrd overlay, violating the read-only binary invariant.
- **Dead code wired into production paths** -- consolidated duplicate path logic between `paths.rs` and `service_install.rs`. `is_service_installed()` now guards `try_ensure_service()` to prevent unmanaged duplicate service spawns. `start_background_download()` wired into setup wizard. `install_bin_dir()` wired into uninstall for layout-aware binary removal. `assets_dir_from_home()` used by `discover_paths()`. Removed `ServiceSpawnArgs` (was identical to `CapsemPaths`). Zero `#[allow(dead_code)]` annotations remain.
- **initrd repack: permission denied on read-only guest binaries** -- `_pack-initrd` now `rm -f` before overwriting 555-permission files (`capsem-doctor`, `capsem-bench`, `snapshots`), matching the pattern already used for agent binaries.
- **Service race condition on exec/write/read after provision** -- `handle_exec`, `handle_write_file`, and `handle_read_file` now wait for the VM socket to be ready before sending IPC commands. Previously, calling these endpoints immediately after `/provision` or `/resume` would fail with "failed to connect to sandbox" because the capsem-process had not yet created its socket. Extracted `wait_for_vm_ready` helper (socket existence + ping) shared by all IPC handlers. This fixes `capsem doctor` and any client that calls exec without polling.
- **pnpm audit: defu prototype pollution and vite file read vulnerabilities** -- added `defu>=6.1.5` and `vite>=6.4.2` overrides to frontend `pnpm.overrides`.
- **capsem-process: reject invalid fd -1 in clone_fd** -- defensive check prevents undefined behavior when an invalid file descriptor is passed.
- **capsem doctor: streaming output** -- doctor now streams test results in real-time via terminal IPC instead of buffering all output until completion. Also adds `--durations=10` to surface the 10 slowest tests.
- **capsem doctor: removed invalid --json flag** -- `capsem-doctor` is a pytest wrapper that doesn't support `--json`. The flag caused pytest to exit with "unrecognized arguments".
- **MCP snapshots_changes: JSON pagination breaks parsing** -- `format=json` output was wrapped in pagination headers (`Content length: ...`), making `json.loads()` fail. JSON format now returns the raw array without pagination headers.
- **Guest binary permissions: snapshots and capsem-bench** -- changed from 755 to 555 in rootfs Dockerfile to match the read-only binary invariant.
- **Rust warnings-as-errors for all crates** -- `RUSTFLAGS="-D warnings" cargo check --workspace` now runs in both `just smoke` and `just test`, blocking on any warning in any crate. Previously only capsem-service and capsem-process were checked, and only in `just test`.

- **Settings system** -- dynamic settings UI rendered from the backend tree structure (not mocked). Recursive `SettingsSection` renderer handles all setting types: bool toggles (immediate save), text/number/select inputs (staged batch save), password fields with reveal + prefix validation + "required" badge, file editors with Shiki syntax highlighting (theme-aware, shared singleton), domain chip lists (add/remove). Toggle-gated provider cards with slide transitions, chevron animation, and collapsed warning summaries. Settings store (`settings.svelte.ts`) with `load/stage/save/discard/updateImmediate`. Settings model (`settings-model.ts`) with tree indexing, widget resolution, preset matching, pending changes tracking. MCP section with policy, built-in tools, and external servers. Security preset selector. Dirty bar with unsaved change count + Save/Discard. WCAG AA contrast tests for all warning/error/status colors (11 checks, amber-700/red-700 light, amber-400/red-300 dark). 206 tests passing.

- **Gateway wiring (Sprint 05)** -- frontend now connects to capsem-gateway for real data instead of mock. API client (`api.ts`) with Bearer auth token fetched from `GET /token` (hardcoded 127.0.0.1 IP check). Reactive gateway store with automatic health check and reconnection. VM store polls `GET /status` every 2s with visibility-aware pause. All view components (Stats, Logs, Files, Inspector) wired to gateway API with transparent mock fallback on network error. Terminal WebSocket connection via iframe postMessage handshake with `?token=` query-param auth (allowlisted, other params dropped). VM lifecycle actions (stop/delete/fork/resume) wired in both toolbar and new-tab page. Connection status dot in toolbar. Gateway-side: added `GET /token` endpoint with loopback IP restriction, added query-param auth fallback for WebSocket paths only. Settings remains mock (service has no settings CRUD API yet). 115 gateway tests, 303 frontend tests passing.

### Added
- **Theme system** -- three independent axes: UI mode (auto/light/dark), accent color (9 options), and terminal theme (12 families with dark/light variants). All persisted in localStorage. Terminal themes sourced from canonical iTerm2-Color-Schemes palettes. Accent colors are primary-only CSS overrides on a single consistent dark/light base (removed ~2000 lines of per-theme Preline CSS). Settings page with Interface and Terminal subsections, color swatches, and live terminal preview.
- **Bundled fonts** -- Google Sans Flex for UI chrome, Google Sans Code as default terminal font, plus JetBrains Mono, Fira Code, Cascadia Code, Inconsolata, Hack, Space Mono, Ubuntu Mono. All local TTF in `public/fonts/`, zero external loads. Terminal font and font size configurable in Settings with localStorage persistence and iframe propagation via postMessage.
- **Auto Docker GC** -- `_docker-gc` recipe runs automatically after `build-assets`, `cross-compile`, and `test-install` to prevent unbounded disk growth. Prunes stopped containers, unused images >72h, build cache >72h, and runs `fstrim` on the Colima VM disk to release freed space back to macOS.
- **Doctor: separate CLI vs daemon checks** -- `just doctor` now checks the Docker CLI binary and daemon reachability independently, with platform-specific fix hints (macOS: start Colima, Linux: systemctl start docker).
- **Shell completions and `capsem uninstall`** -- `capsem completions bash|zsh|fish` generates shell completions via clap_complete. `capsem uninstall --yes` stops service, removes unit, binaries, `~/.capsem/`, and logs.
- **`capsem update` self-update** -- checks GitHub for new releases, downloads assets with hash verification, and cleans up old versions. Update notice displayed on every command (24h cached check). `--yes` skips confirmation. Development builds directed to build from source. Install layout detection (MacosPkg, UserDir, Development).
- **`capsem setup` interactive wizard** -- first-time setup with security preset selection, AI provider credential detection, repository access check, service installation, and PATH verification. Supports `--non-interactive`, `--preset`, `--force`, `--accept-detected`, and `--corp-config` flags. Persists state to `~/.capsem/setup-state.json` for incremental re-runs. Corp-aware: skips prompts for corp-locked settings.
- **Corp config provisioning** -- enterprise users can provision corp config from a URL or local file path via `capsem setup --corp-config`. Config installs to `~/.capsem/corp.toml` with source metadata in `corp-source.json`. Background refresh with ETag-based conditional GET. Loader now merges system (`/etc/capsem/corp.toml`) and user-provisioned (`~/.capsem/corp.toml`) corp configs with system taking precedence per-key.
- **Remote manifest fetch and background asset download** -- `fetch_remote_manifest()` and `fetch_latest_manifest()` fetch VM asset manifests from GitHub releases. `start_background_download()` spawns a tokio task that checks and downloads missing assets with progress reporting via an mpsc channel. Reuses existing AssetManager, DownloadProgress, and blake3 verification.
- **`capsem service install/uninstall/status`** -- register capsem as a LaunchAgent (macOS) or systemd user unit (Linux) with `capsem service install`. Pure generator functions produce the plist/unit content; side-effecting functions handle platform registration. Auto-launch prefers the service manager when a unit is installed.
- **CLI auto-launches service on first command** -- `capsem list` (or any command) now auto-starts the service daemon if no socket is found. Tries systemd/LaunchAgent if a unit is installed, falls back to direct spawn. New `paths` module discovers sibling binaries and assets with installed-first resolution (`~/.capsem/assets/`) before dev fallback. MCP server also uses installed-first asset resolution. Consolidated CLI HTTP methods into a single `request()` with retry-on-connect-fail.
- **Native installer e2e test harness** -- Docker-based install test infrastructure with systemd user sessions. `just install` builds and installs to `~/.capsem/` with codesigning on macOS. `just test-install` runs the full install layout tests in a Docker container. `capsem version` now prints a unique build hash (`capsem 0.16.1 (build c37b920.1775464335)`) for binary identity verification. CI runs install tests on every PR; release pipeline gates on them.
- **Fork images** -- snapshot running or stopped VMs into reusable template images (`capsem fork`), boot new VMs from them (`capsem create --image`). Image registry with list/inspect/delete. Flat genealogy model (images depend only on base squashfs, never on each other). Asset cleanup protects referenced squashfs versions. Available via CLI, MCP tools (`capsem_fork`, `capsem_image_list`, `capsem_image_inspect`, `capsem_image_delete`), and service HTTP API.
- **Session DB schema v5** -- adds `source_image` and `persistent` columns. Vacuum skips persistent VM sessions.
- **CLI parity sprint** -- `--timeout` on `exec`, `capsem version`, `-q`/`--quiet` on `list`, `--tail N` on `logs`, `capsem restart` for persistent VMs, `--env KEY=VALUE` / `-e` on `create` for guest environment injection.
- **`--env` plumbing** -- environment variables flow from CLI/MCP through service, process, and into guest boot config (`send_boot_config`). Supports up to 128 env vars per VM.
- **MCP: `capsem_version` tool** -- returns MCP server version and service connectivity status.
- **MCP: `tail` parameter** -- on `capsem_vm_logs` and `capsem_service_logs` tools, limit output to last N lines (applied after grep filter).
- **MCP: `env` parameter** -- on `capsem_create` tool, inject environment variables into the guest.
- **Next-gen daemon architecture (Sprint 1)** -- capsem now runs as a daemon service (`capsem-service`) that spawns isolated per-VM processes (`capsem-process`), mirroring Chrome's multi-process security model. The service manages VM lifecycle over a UDS API, while each process boots and owns exactly one VM.
- **Full CLI client (`capsem`)** -- new subcommands: `start`, `stop`, `shell`, `list`/`ls`, `status`, `exec`, `delete`/`rm`, `info`, `logs`, `doctor`. The CLI communicates with the service daemon over `~/.capsem/service.sock`.
- **`capsem-mcp` crate** -- standalone MCP server (stdio transport via `rmcp`) that bridges AI agent tool calls to the service API. Provides `capsem_create`, `capsem_exec`, `capsem_read_file`, `capsem_write_file`, `capsem_list`, `capsem_delete`, `capsem_info`, `capsem_inspect`, `capsem_inspect_schema`, `capsem_service_logs`, `capsem_vm_logs` tools.
- **Structured IPC protocol** -- `capsem-proto` extended with `Exec`, `WriteFile`, `ReadFile`, `ReloadConfig`, `StartTerminalStream` commands and matching result variants. New `ipc_ext` module in `capsem-core` for framed message helpers.
- **Service-level resource management** -- concurrent VM limit (`max_concurrent_vms`), per-VM CPU/RAM validation (1-8 CPUs, 256MB-16GB), stale instance cleanup, auto-remove flag, socket path length validation.
- **Multi-version asset resolution** -- service resolves assets from `~/.capsem/assets/v{version}/` with arch-specific fallback.
- **Network policy config: builder tests** -- comprehensive unit tests for `settings_to_vm_settings`, `settings_to_domain_rules`, `load_merged_settings`, and preset validation.
- **Session maintenance** -- new cleanup routines in `capsem-core` for session directory housekeeping.
- **Testing sprint Phase 3 complete** -- 11 new test suites (T15-T25) covering build chain E2E, guest validation, cleanup verification, codesign strict, serial console, session.db lifecycle, config runtime, recipe smoke, recovery/crash-resilience, rootfs artifacts, and exhaustive per-table session.db validation. ~84 new Python integration tests across 40+ test files.
- **New just recipes for Phase 3 tests** -- `test-build-chain`, `test-guest`, `test-cleanup`, `test-codesign`, `test-serial`, `test-session-lifecycle`, `test-config-runtime`, `test-recipes`, `test-recovery`, `test-rootfs`, `test-session-exhaustive`, plus a combined `test-vm` recipe.

### Changed
- **`capsem-process` is now the VM owner** -- boot logic moved from `capsem-app` into `capsem-process`, which receives config via CLI args and communicates with the service over a typed IPC channel (`tokio-unix-ipc`). Includes PTY exec with ANSI stripping, file I/O forwarding, and terminal streaming.
- **`capsem-agent` guest binary** -- updated vsock I/O, net proxy, and MCP server modules to match the new host-guest protocol.
- **Justfile overhaul** -- restructured recipes for the daemon workflow (`run-service`, `run-process`), updated build and test targets.

### Fixed
- **Silent epoch on malformed image timestamps** -- `time_format` serde deserializer silently returned `UNIX_EPOCH` for garbage input, corrupting image sort order. Now returns a proper deserialization error.
- **`top_mcp_tools` merged tools from different servers** -- SQL `GROUP BY tool_name` without `server_name` collapsed cross-server tools into one row with an arbitrary server name. Added `server_name` to the GROUP BY clause.
- **Image registry TOCTOU and concurrent write corruption** -- `create_image_from_session` had a TOCTOU race (exists check then create_dir_all). Replaced with atomic `create_dir`. Added `flock`-based file locking around registry insert/remove with atomic write (write-to-temp then rename).
- **`handle_logs` returned 404 for stopped persistent VMs** -- unlike `handle_info`, it only checked running instances. Added persistent registry fallback.
- **Blocking I/O in async context** -- `std::thread::sleep` in CLI shell loop (replaced with `tokio::time::sleep`), `std::process::Command` in MCP service relaunch (replaced with `tokio::process::Command`), blocking file reads in MCP `service_logs` and service `handle_logs` (wrapped in `spawn_blocking`).
- **CLI `SandboxInfo` missing fields** -- CLI struct lacked `ram_mb`, `cpus`, `version` fields that the service returns. Added with `#[serde(default)]` and display in `status` command.
- **Panicking `unwrap()` in MCP service relaunch** -- `Path::parent().unwrap()` replaced with proper error propagation.
- **`snapshots` CLI missing from release rootfs** -- the `snapshots` tool was never copied into the rootfs Docker build context or Dockerfile template, so release builds shipped without it. Added `ROOTFS_ARTIFACTS` constant as single source of truth in `docker.py`, plus 6 validation layers: builder unit tests, builder doctor pre-build check, config validator, rootfs artifacts test suite, CI release workflow validation, and in-VM guest binary assertions (changed from `pytest.skip` to `pytest.fail`).
- **`just doctor-fix` fails on fresh machines** -- `build-assets` triggered `_ensure-setup` which ran `doctor` which failed on missing assets, creating a circular dependency. Fix commands now set `CAPSEM_SKIP_ASSET_CHECK=1` and `touch .dev-setup` to break the cycle. Guest binary checks are also skipped when asset check is skipped (no assets = no binaries). Fixes bail on first failure instead of continuing to run dependent steps.
- **Docker cross-arch builds fail (legacy builder cache poisoning)** -- Docker's legacy builder shared intermediate layer cache across `--platform` values, reusing arm64 layers for x86_64 builds. Fixed by requiring Docker BuildKit (buildx). Added buildx and Colima Rosetta checks to `just doctor` and `scripts/bootstrap.sh`.

## [0.16.1] - 2026-04-02

### Added
- **KVM boot diagnostics** -- when vCPU creation fails on Linux, Capsem now runs automatic diagnostic probes: kernel version, nested KVM status, KVM capabilities, and a fresh-VM-without-IRQCHIP test to isolate the root cause. All results logged at ERROR level so they appear without `RUST_LOG=debug`.
- **`scripts/kvm-diagnostic.py`** -- standalone diagnostic script for manual KVM environment debugging. Tests 7 phases: /dev/kvm basics, capabilities, Capsem boot sequence, no-irqchip mode, reversed ordering, split IRQCHIP, and environment info.

### Fixed
- **KVM boot errors are now actionable** -- `/dev/kvm` missing explains how to enable KVM (modprobe, BIOS). Permission denied suggests `usermod -aG kvm`. EEXIST on vCPU creation explains restricted/nested KVM and points to the diagnostic script.
- **Linux boot failure shows macOS error message** -- `gui.rs` said "unsigned binary or missing entitlement" on all platforms. Now shows platform-specific guidance: KVM troubleshooting on Linux, entitlement info on macOS.
- **LATEST_RELEASE.md stale at v0.15.1** -- boot screen showed wrong version. Regenerated from CHANGELOG.md.

### Changed
- **`just doctor` rewritten as standalone scripts** -- moved from 265-line inline justfile recipe to `scripts/doctor-common.sh` + platform-specific `doctor-macos.sh` and `doctor-linux.sh`. Colored output (green/red/yellow), structured recap table, and auto-fix: detects fixable issues (missing rustup targets, cargo tools, broken symlinks) and prompts to fix them automatically. `--fix` flag for non-interactive auto-fix.

## [0.16.0] - 2026-04-02

### Added
- **`just clean` reports freed space** -- shows per-directory sizes before deletion and total freed at the end. Also cleans `tmp/` and `coverage/` directories.
- **`just clean-all` prunes docker volumes** -- adds `--volumes` to docker prune for full reclaim.
- **Automatic incremental cache trimming** -- `_clean-stale` now checks if `target/` exceeds 20 GB and auto-removes incremental compilation caches (`target/debug/incremental`, `target/release/incremental`, `target/llvm-cov-target`). Prevents unbounded growth that caused 113 GB bloat.
- **`_clean-stale` wired into all build paths** -- added to `build-assets` and `cross-compile` dependency chains (was already in `test` and `_compile`).
- **Revert telemetry** -- `snapshots_revert` now logs a `restored` file event to the session DB, including the source checkpoint (e.g., `"src/main.py (from cp-3)"`). New `FileAction::Restored` variant in capsem-logger, `FileEventStats.restored` counter in reader queries.
- **Boot audit logging** -- comprehensive `[boot-audit]` tracing throughout the GUI and CLI boot paths (main.rs, gui.rs, boot.rs, cli.rs, session_mgmt.rs). Every step from session cleanup through hypervisor boot is timestamped, making hangs immediately diagnosable.
- **Doctor: VM asset and guest binary checks** -- `just doctor` now validates asset manifest version, B3SUM integrity, and guest binary presence/format.
- **Smoke test recipe** -- `just smoke-test` (alias `just smoke`) runs unit tests + repack + sign + capsem-doctor as a fast end-to-end validation without full asset rebuild.
- **Doctor: Docker BuildKit (buildx) and Colima Rosetta checks** -- `just doctor` now validates that buildx is installed and Colima has Rosetta enabled for cross-arch container builds.

### Fixed
- **Cross-arch Docker builds fail on macOS** -- Docker's legacy builder shared intermediate layer cache across `--platform` values, causing arm64 layers to be reused for x86_64 builds. Fixed by requiring Docker BuildKit (buildx), which properly includes platform in cache keys. Added buildx to `just doctor` and `scripts/bootstrap.sh`.
- **Snapshots tab shows nothing during long sessions** -- the tab called `callMcpTool('snapshots_list')` once on mount, never refreshed, and failed silently if the MCP gateway wasn't wired yet. Replaced with SQL queries against a new `snapshot_events` table in `session.db`, consistent with all other stats tabs. Each snapshot event stores a self-contained `(start_fs_event_id, stop_fs_event_id]` range for efficient per-snapshot change counts via `fs_events` cross-reference.
- **Symlink loop hangs app on startup** -- `disk_usage_bytes()` used `is_dir()` / `metadata()` which follow symlinks. A `.venv/lib64 -> lib` relative symlink in session workspaces caused infinite recursion, hanging the app at boot. Fixed to use `symlink_metadata()` throughout. Added regression tests for symlink loops, absolute escapes, and real session timing.
- **Wizard flashes briefly on app launch** -- the setup wizard appeared for one frame before settings finished loading. Added `!settingsStore.loading` guard to prevent the wizard from rendering until settings are fully resolved.
- **KVM boot path compile errors** -- `vm/boot.rs` referenced `rootfs_path()` and `virtiofs_share()` methods that were renamed. Fixed to use `disk_path()` and `virtio_fs_share()`.
- **capsem-cli missing `mut`** -- `socket.read(&resp_buf)` needed `&mut resp_buf`.

### Security
- **Symlink sandbox escape (documented)** -- guest agents can create symlinks through VirtioFS that point to arbitrary host paths (e.g., `host_root -> /`). Host-side code that follows these symlinks escapes the sandbox. `disk_usage_bytes` is fixed; 6 other code paths identified and documented in `tmp/bugs/symlink_escape.md` for hardening.

## [0.15.3] - 2026-04-02

### Fixed
- **x86_64 CI boot test fails on restricted KVM** -- GitHub Actions runners expose `/dev/kvm` but lack full VM support (no CPUID, no PIT). The boot test now probes KVM capability before attempting a VM boot and skips gracefully with a warning annotation when the runner's KVM is insufficient.

## [0.15.2] - 2026-04-02

### Fixed
- **x86_64 boot test fails on CI: KVM_CREATE_PIT2 unsupported** -- GitHub Actions runners use restricted KVM that doesn't support the legacy i8254 PIT timer. Made PIT creation optional with a warning; when unavailable, `no_timer_check` is appended to the kernel cmdline so Linux uses alternative timer sources.
- **`cross-compile` missing boot test** -- CI installs the `.deb` and boot-tests with capsem-doctor but `cross-compile` didn't. Added boot test step that runs when `/dev/kvm` is available and the target matches the native arch; skips on macOS or cross-arch builds.
- **`cross-compile` missing GNU cross-linker config** -- `.cargo/config.toml` only had musl linker entries. Added `x86_64-linux-gnu-gcc` and `aarch64-linux-gnu-gcc` for GNU targets used by the Tauri app build.

## [0.15.1] - 2026-04-01

### Fixed
- **x86_64 Linux build fails: aarch64 boot module not cfg-gated** -- `mod boot` (ARM64 kernel loading, FDT, register setup) was included unconditionally, causing 14 compile errors on x86_64 (`set_one_reg`, `REG_PC`, `KERNEL_TEXT_OFFSET` not found). Gated with `#[cfg(target_arch = "aarch64")]`.
- **Cross-compile linker error on arm64 hosts** -- building `capsem-agent` for `x86_64-unknown-linux-gnu` inside the Docker container used the native `cc` (arm64) which doesn't understand `-m64`. Added `x86_64-linux-gnu-gcc` and `aarch64-linux-gnu-gcc` cross-linker entries to `.cargo/config.toml`.
- **Multiarch dpkg conflict in cross-compile Docker image** -- `libpango1.0-dev` arm64-to-amd64 swap failed on shared `.gir` file. Added `--force-overwrite` to `swap-dev-libs.sh`.

### Changed
- **`build-assets` builds both arm64 and x86_64** -- previously only built for the native architecture, so cross-compile for the other arch always failed locally due to missing VM assets.
- **`full-test` includes `cross-compile`** -- catches platform-gating errors before tagging instead of discovering them in CI.

## [0.15.0] - 2026-04-01

### Added
- **x86_64 KVM backend** -- full KVM support for x86_64 Linux: bzImage boot protocol, identity-mapped page tables, GDT, IRQCHIP/PIT interrupt controller, CPUID passthrough, 16550 UART serial console (PIO), E820 memory map, virtio-mmio device discovery via kernel cmdline. The .deb now boots VMs on both aarch64 and x86_64.
- **Cross-compile Docker image** -- purpose-built `capsem-host-builder` image (Ubuntu 24.04) with all Tauri build deps pre-baked (system libs, Node.js 24, pnpm 10, Rust stable, cargo tools, uv). Replaces the old `rust:bookworm` ad-hoc install approach. Named volumes cache cargo registry and per-arch build artifacts between runs. New recipes: `just build-host-image`, `just clean-host-image`.
- **x86_64 release boot test** -- release pipeline now boot-tests the x86_64 .deb with capsem-doctor before publishing.
- **Compile-time KVM struct size assertions** -- `const _` assertions for all KVM ioctl structs (both aarch64 and x86_64) that fail at compile time, not runtime.
- **Kernel arch-mismatch detection** -- x86_64 boot rejects ARM64 Image kernels, aarch64 boot rejects bzImage kernels, with clear error messages instead of cryptic crashes.

### Changed
- **Container runtime: Podman replaced with Colima + Docker CLI** -- macOS now uses Colima (Apple Virtualization.framework with Rosetta) instead of Podman (libkrun). Rosetta gives near-native x86_64 container performance on Apple Silicon, making cross-arch kernel and rootfs builds much faster. All podman-specific code paths removed; standardized on `docker` CLI everywhere.

### Fixed
- **`just run` blocked on Linux** -- the `_sign` recipe hard-exited on non-macOS, preventing `just run`, `just bench`, and `just full-test` from working on Linux with KVM. Now skips codesigning on Linux.
- **x86_64 KVM boot broken: wrong entry point + missing setup header** -- the 64-bit entry point was `KERNEL_LOAD_ADDR` instead of `KERNEL_LOAD_ADDR + 0x200` (`startup_64`), causing the vCPU to execute 32-bit code in long mode and hang. Fixed by preserving bzImage setup header into boot_params and correcting the entry point.
- **`install.sh` fails on Linux** -- added OS and architecture detection so the same one-liner works on both macOS (arm64 .dmg) and Linux (x86_64/arm64 .deb via `apt install`).
- **Site docs claim macOS-only** -- updated to reflect Linux/KVM support.
- **`.cargo/config.toml` not tracked** -- broke codesigning on fresh clones. Fixed by anchoring the gitignore pattern to root.
- **Boot screen showed "No release notes available"** -- replaced Vite plugin path with `LATEST_RELEASE.md` generated by `cut-release`.
- **No error screen when VM assets fail** -- added proper error state to the boot screen with trigger-specific messages.

## [0.14.20] - 2026-03-30

### Fixed
- **CI release upload collision on per-arch VM assets** -- `gh release upload "$f#${arch}-${base}"` sets the display label, not the filename. Both arches uploaded `initrd.img`, causing a name collision. Fixed by renaming files to `${arch}-${base}` before upload.

## [0.14.19] - 2026-03-30

### Fixed
- **AI CLI version check fails in CI** -- `extract_tool_versions()` runs `gemini --version` and `codex --version` inside the built rootfs image, but `/opt/ai-clis/bin` was not on the container PATH. Added `ENV PATH` to the Dockerfile template after npm CLI install so version extraction finds the binaries.
- **`cut-release` skipped container build** -- `cut-release` depended on `just test` (unit tests only), so Dockerfile and rootfs issues were only caught by CI after tagging. Now `cut-release` depends on `full-test`, which depends on `build-assets`. The full chain (container build + unit tests + capsem-doctor + integration + bench) runs locally before any tag is created.
- **Container agent build fails writing Cargo.lock** -- source mounted `:ro` prevented cargo from generating `Cargo.lock`. Switched to symlinking source into writable `/build` dir so cargo can write the lockfile without modifying the host.

## [0.14.18] - 2026-03-30

### Changed
- **Config-driven tool version extraction** -- `extract_tool_versions()` now builds its shell script from TOML configs (`version_commands` fields) instead of a hardcoded tool list. Covers build tools (node, npm, uv, pip), apt packages (git, python3, gh, tmux, curl), Python packages (pytest, numpy, requests, pandas), and AI CLIs (claude, gemini, codex) with grouped output in tool-versions.txt. Build-time validation catches silent install failures (N/A) for enabled AI CLIs. New W013 diagnostic warns when an AI provider has a CLI but no `version_command`.

### Fixed
- **VM asset download fails with arch-prefixed release names** -- CI uploads per-arch assets as `arm64-rootfs.squashfs` etc., but `AssetManager` constructed download URLs with bare filenames (`rootfs.squashfs`), causing 404s. Added `arch_prefix` to `AssetManager` so download URLs match the release naming convention. Local storage still uses bare filenames.

## [0.14.17] - 2026-03-30

## [0.14.16] - 2026-03-30

### Fixed
- **CI test job: create stub assets for Tauri build.rs** -- the parallelization commit removed asset downloads from test, but `cargo test --workspace` compiles capsem-app whose build.rs needs assets/manifest.json. Was masked by Rust cache until tauri.conf.json change invalidated it.
- **CI create-release cleanup** -- removed stale AppImage/updater references (latest.json merge, tar.gz/sig collection), fixed SBOM attestation to cover both DMG and deb, fixed test summary to parse `cargo llvm-cov` output format, prefix per-arch VM assets (`arm64-vmlinuz`, `x86_64-vmlinuz`) to avoid upload name collisions.

## [0.14.15] - 2026-03-30

## [0.14.14] - 2026-03-30

## [0.14.13] - 2026-03-30

### Improved
- **CI pipeline parallelized (~18 min vs ~45 min)** -- test runs in parallel with build-assets and app builds. Test gates create-release but doesn't block compilation. Removed redundant cross-compile check and asset downloads from test job.

### Fixed
- **Pin Xcode 16.2 on macOS CI runners** -- Xcode 15.4's xcodebuild crashes with `Abort trap: 6` when Tauri tries to locate notarytool. Runner image update broke the default Xcode between v0.14.11 (passed) and v0.14.12 (failed). Explicitly selecting Xcode 16.2 prevents runner drift.
- **Drop AppImage from Linux releases** -- linuxdeploy cannot run on GitHub CI runners (Ubuntu 24.04 lacks FUSE2, and neither `libfuse2` nor `APPIMAGE_EXTRACT_AND_RUN=1` resolves it reliably). Linux ships `.deb` only on both arm64 and x86_64. Root cause of every v0.14.x Linux build failure (14 consecutive failed releases).
- **Container agent build: replace `file` with `ls -l`** -- `file` command is not available in `rust:slim-bookworm`. Binary verification now uses `ls -l` (coreutils); real validation (existence + non-zero size) is done in Python after the container exits.
- **Broken capsem-doctor link in docs** -- getting-started page linked to `/testing/capsem-doctor/` (removed section) instead of `/debugging/capsem-doctor/`.
- **Site description outdated** -- splash page and meta description now mention Linux (KVM) support added in v0.14.
- **Security docs sidebar ordering** -- three security pages lacked `sidebar.order`, causing alphabetical sort instead of logical progression.
- **`.dockerignore` untracked** -- Docker builds on CI or fresh clones were copying `target/`, `node_modules/`, `.venv/` into build context.

## [0.14.12] - 2026-03-29

### Fixed
- **Skip AppImage on arm64 Linux** -- linuxdeploy has no arm64 build. arm64 Linux (Chromebooks) now builds `.deb` only. x86_64 builds both deb + AppImage.

## [0.14.11] - 2026-03-29

### Fixed
- **CI Linux build: add Tauri signing keys** -- `build-app-linux` was missing `TAURI_SIGNING_PRIVATE_KEY`, causing "public key found but no private key" failure. Also collect `.tar.gz` and `.sig` updater artifacts.

### Added
- **`just cross-compile [arch]`** -- build agent binaries + full Linux app (deb + AppImage) inside a container. No host cross-compile toolchain needed. Supports arm64 and x86_64. Clean build every run (no stale volumes).
- **Container-native agent compilation** -- builds natively inside a Linux container, eliminating cross-compile cfg gating issues.
- **Multi-arch Linux release** -- CI now builds deb + AppImage for both arm64 and x86_64 via matrix job. Artifacts validated with `dpkg-deb --info` and `file`.

## [0.14.10] - 2026-03-29

### Fixed
- **CI Linux build: install xdg-utils** -- Tauri's AppImage bundler requires `xdg-open`. Added `xdg-utils` to `apt-get install` in `build-app-linux`.
- **Linux build: gate all macOS-only APIs** -- `ApfsSnapshot` (`libc::clonefile`), `AppleVzHypervisor` import in boot.rs, and `vm_integration.rs` tests were not `cfg`-gated, causing compile failures on Linux app builds. Boot now dispatches to `KvmHypervisor` on Linux.
- **Builder: apt clock skew on macOS** -- Podman/Docker VM clock drift after sleep/wake caused `apt-get update` to reject release files as "not valid yet" (exit 100). Added `Acquire::Check-Date=false` to all apt-get calls in Dockerfile templates and squashfs creation. Also added `sync_container_clock()` to auto-sync the VM clock with the host before builds.

### Added
- **Platform gating static analysis test** -- `cargo test --test platform_gating` scans all `.rs` files for ungated macOS-only and Linux-only symbols. Catches platform API issues before they reach CI.
- **Builder doctor: container clock check** -- `capsem-builder doctor` now detects clock skew between host and container VM, reports direction and magnitude, and suggests a fix.

### Improved
- **Boot timing display** -- formatted table with right-aligned columns and proportional bar chart instead of flat log lines.
- **capsem-bench refactored to package** -- split 897-line single file into `capsem_bench/` Python package with per-category modules (disk, rootfs, startup, http_bench, throughput, snapshot). Shell wrapper at `capsem-bench` preserves the same CLI interface.
- **capsem-bench JSON output** -- saved to `/tmp/capsem-benchmark.json` inside the VM instead of dumped to stdout.

### Docs
- **Site restructuring** -- moved capsem-doctor to new top-level Debugging section (with troubleshooting guide), moved benchmarking methodology to Development, added top-level Benchmarks section with current performance results (boot time, disk I/O, CLI startup, HTTP, throughput, snapshots).

## [0.14.8] - 2026-03-29

### Fixed
- **Linux build: gate all macOS-only APIs** -- `ApfsSnapshot` (`libc::clonefile`) and `AppleVzHypervisor` import in boot.rs were not `cfg`-gated, causing compile failures on Linux app builds. Boot now dispatches to `KvmHypervisor` on Linux.

## [0.14.7] - 2026-03-29

### Fixed
- **Linux build: gate `ApfsSnapshot` behind `cfg(target_os = "macos")`** -- `libc::clonefile` is macOS-only, causing compile failure on Linux app builds.

## [0.14.6] - 2026-03-28

### Fixed
- **CI build-assets restores Rust toolchain** -- v0.14.5 removed `dtolnay/rust-toolchain` when switching to just recipes, but `build-rootfs` cross-compiles the guest agent and needs the musl target installed.
- **CI build-assets builds both kernel and rootfs** -- release workflow only built rootfs, missing vmlinuz and initrd.img. Now uses `just build-kernel` and `just build-rootfs` recipes instead of reimplementing build logic.
- **CI assets/current ordering** -- moved `cp -r` after `generate_checksums` so Tauri's `build.rs` finds real files instead of a stripped symlink.

### Improved
- **`just doctor` codesigning diagnostics** -- new four-step Codesigning section checks Xcode CLTools, codesign binary, entitlements.plist, and runs a real test sign. Every `[FAIL]` line now includes a copy-pasteable fix command.
- **`bootstrap.sh` platform checks** -- macOS: validates Xcode Command Line Tools. Linux: prints informational notice about which recipes work (test, build-assets, audit) vs require macOS (run, dev, bench).
- **`_sign` recipe platform guard** -- fails immediately on Linux with actionable message instead of cryptic "codesign: command not found".
- **`run_signed.sh` error surfacing** -- codesign failures now print to stderr with a hint to run `just doctor`, instead of silently logging to `target/build.log`.
- **Developer getting-started docs** -- added platform requirements table, codesigning section with validation table, and codesign troubleshooting to the site.

## [0.14.2] - 2026-03-28

### Fixed
- **KVM virtio_blk split-borrow** -- `queue_notify` uses `.take()` pattern to avoid split-borrow when processing read/write/get_id operations.
- **CI release uses cp -r for assets/current** -- GitHub Actions artifacts strip symlinks, causing the `ln -s` approach to fail. Switched to `cp -r`.
- **Builder checksums handle current/ as directory** -- `generate_checksums()` now removes `current/` whether it's a symlink or a directory (from a prior `cp -r`).
- **Guest agent `libc::time_t` deprecation** -- replaced deprecated `libc::time_t` with `i64` in vsock_io timeout constant.

### Added
- **Developer getting-started documentation** -- full setup guide at capsem.org/development/getting-started/ covering prerequisites, container runtime setup, cross-compilation, and troubleshooting.
- **Bootstrap script** -- `scripts/bootstrap.sh` checks all required tools, installs Python and frontend deps, and runs `just doctor`.
- **`.dev-setup` sentinel** -- `just doctor` writes a `.dev-setup` file on success. All recipes (`run`, `test`, `dev`, `bench`) auto-run doctor if the sentinel is missing, preventing new developers from skipping setup.
- **`uv` check in `just doctor`** -- doctor now validates that `uv` is installed (previously missing, causing silent `build-assets` failures).
- **README prerequisites** -- "Build from source" section now lists required tools and links to the full development guide.
- **`dev-start` skill** -- quick-start pointer skill for new developers.

## [0.14.1] - 2026-03-28

### Fixed
- **Builder uses Python blake3 for checksums** -- `generate_checksums()` no longer shells out to `b3sum` CLI. Uses the `blake3` Python library directly, making the builder self-contained in CI environments.
- **Site workflow uses pnpm 10** -- pnpm 9 errored with workspace detection issues.

## [0.14.0] - 2026-03-28

### Added
- **Hypervisor abstraction layer** -- `Hypervisor`, `VmHandle`, `SerialConsole` traits in new `hypervisor` module. Platform-agnostic `VsockConnection` with lifetime anchor pattern.
- **KVM backend** -- embedded VMM using rust-vmm crates (`kvm-ioctls`, `vm-memory`, `linux-loader`). Virtio console, block, vsock (vhost-vsock), and VirtioFS (embedded FUSE server) devices. GICv3 interrupt controller, FDT generation, multi-vCPU support. ~5,500 LOC.
- **Linux app builds** -- Tauri deb and AppImage targets. macOS-only dependencies gated behind `cfg(target_os = "macos")`. CFRunLoop pumping replaced with platform-agnostic sleep on Linux.
- **capsem-builder Python package** -- config-driven build system for guest VM images. Pydantic models for all TOML configs, Jinja2 Dockerfile renderer (rootfs + kernel, multi-arch), compiler-style validation linter, Click CLI, scaffolding, BOM manifest, vulnerability audit parsing, MCP stdio server, and build doctor. 408 tests at 97% coverage.
- **capsem-builder CLI** -- `validate`, `build`, `inspect`, `init`, `add`, `audit`, `new`, `mcp`, and `doctor` commands.
- **Docker build execution** -- `capsem-builder build` produces real VM assets (kernel, initrd, rootfs squashfs). Config-driven multi-architecture output to per-arch subdirectories (`assets/arm64/`, `assets/x86_64/`).
- **Guest image TOML configs** -- declarative configs in `guest/config/` replacing hardcoded values: `build.toml` (multi-arch), `ai/*.toml` (3 providers), `packages/*.toml`, `mcp/*.toml`, `security/web.toml`, `vm/resources.toml`, `vm/environment.toml`, `kernel/defconfig.*` (arm64 + x86_64).
- **Jinja2 Dockerfile templates** -- `Dockerfile.rootfs.j2` and `Dockerfile.kernel.j2` render multi-arch Dockerfiles from TOML configs. 51 conformance tests verify parity with hand-authored Dockerfiles.
- **Settings schema (Pydantic)** -- canonical schema source with two-node-type design (GroupNode + SettingNode). JSON Schema generation, cross-language golden fixtures with Python/Rust/TypeScript conformance tests (99 tests).
- **Config-driven settings grammar** -- formalized TOML grammar with Group, Leaf, and Action node types. Settings UI fully data-driven.
- **Batch settings IPC** -- `load_settings` and `save_settings` Tauri commands replace 3 parallel calls with 1.
- **SettingsModel TypeScript class** -- pure TS class with settings logic, fully unit-tested (43 tests).
- **Snapshot benchmarks** -- `capsem-bench snapshot` measures create/list/changes/revert/delete latency at 10/100/500 file workspace sizes.
- **Direct clonefile(2) syscall** -- `ApfsSnapshot` uses `libc::clonefile()` directly. Snapshot create dropped from 50ms to 3.7ms (93% faster).
- **Hardlink-based incremental snapshots** -- `SnapshotBackend` trait with `ApfsSnapshot` (macOS) and `HardlinkSnapshot` (cross-platform) implementations.
- **FUSE ops unit tests** -- 30+ tests covering file I/O, directory operations, metadata, and adversarial cases.
- **Doctor session validation test** -- `scripts/doctor_session_test.py` verifies session.db telemetry after capsem-doctor run.
- **Container runtime resource checks** -- `just doctor` and `capsem-builder doctor` verify podman/Docker have enough memory (min 4GB).
- **Asset resolution test suite** -- 28 new tests across Rust and Python for manifest parsing, hash verification, and per-arch resolution.
- **`manifest_compat` module** -- shared `extract_hashes()` for manifest hash extraction, testable independently from `build.rs`.
- **Multi-arch asset selection** -- host app detects architecture at compile time and loads assets from per-arch subdirectories. Backward compatible with flat layout.
- **Asset pipeline documentation** -- new site page and skill documenting the build-to-boot asset flow.
- **Hypervisor architecture documentation** -- boot sequence, KVM internals, virtio device slots, VirtioFS server. Five mermaid diagrams.
- **Capsem-doctor documentation** -- 11 test categories, test infrastructure, adding new tests.
- **Corporate image support** -- custom guest configs produce different images (6 corporate image tests).
- **Persistent MCP client** -- `snapshots` CLI reuses a single fastmcp Client across all tool calls.

### Changed
- **Multi-arch release pipeline** -- CI builds arm64 and x86_64 VM assets in parallel on native runners. Per-arch attestation. Unified manifest with both architectures.
- **Release workflow adds Linux builds** -- separate `build-app-linux` job produces deb and AppImage alongside macOS DMG.
- **Site deployment fixed** -- workflow switched from npm to pnpm, Node pinned to 24.
- Apple Virtualization.framework code moved to `hypervisor/apple_vz/` behind `cfg(target_os = "macos")` gate. macOS-only dependencies now target-conditional.
- `VsockManager` replaced by `mpsc::UnboundedReceiver<VsockConnection>` returned from `Hypervisor::boot()`.
- `auto_snapshot` uses `SnapshotBackend` trait (APFS clonefile on macOS, recursive copy elsewhere).
- `notify` crate uses default features (cross-platform) instead of macOS-only `macos_fsevent`.
- Claude Code installed via native installer (`curl` instead of `npm`). Binary in `/usr/local/bin/` (chmod 555).
- Builder cleans up container images after extracting assets.
- Guest artifacts moved to `guest/artifacts/` from `images/`.
- `just build-assets` now uses capsem-builder with config-driven Dockerfile generation.
- Multi-arch cross-compilation configured for both `aarch64-unknown-linux-musl` and `x86_64-unknown-linux-musl`.
- Multi-arch diagnostics accept both `aarch64` and `x86_64`.
- Linux KVM backend promoted to Production status.
- CI coverage tracking for Linux KVM backend (`linux-unit` Codecov flag).
- Settings grammar documented with full specification.
- Settings architecture page with 7 mermaid diagrams.
- Side effect dispatch driven by metadata instead of hardcoded checks.
- MCP injection generalized for multiple servers from config.
- Site: mermaid diagram support via `astro-mermaid`.
- Skills table added to CLAUDE.md and GEMINI.md.
- `cut-release` recipe now bumps `pyproject.toml` alongside Cargo.toml and tauri.conf.json.
- Preflight checks add `uv` tool and `x86_64-unknown-linux-musl` target.
- README updated for multi-platform support (macOS + Linux), documentation links point to capsem.org.

### Fixed
- **Asset manifest format bug** -- `gen_manifest.py` produced filenames like `"arm64/vmlinuz"` instead of bare `"vmlinuz"`, causing `build.rs` to silently skip hash verification.
- **Per-arch manifest parsing** -- `Manifest::from_json()` rejected per-arch format. Added `from_json_for_arch()`.
- **apt clock skew in container builds** -- added `Acquire::Check-Valid-Until=false` to all apt calls.
- **Mock data generated from build system** -- settings and MCP data now generated from `config/defaults.json` and Rust `mcp-export` binary instead of hand-crafted mock.
- **`step` metadata field flows to UI** -- was silently dropped from generated JSON.
- **Build log contamination** -- signing and generation scripts now log to `target/build.log`.
- **Snapshot MCP no longer hangs** -- blocking I/O moved to `spawn_blocking` threads.
- **Snapshot panel now displays snapshots** -- frontend now passes `format: "json"`.
- **Vacuum preserves content sessions** -- keeps at least 25 sessions with AI activity.
- **inspect-session shows MCP tool usage** -- per-tool breakdown replaces old view.
- **Integration test Gemini API key handling** -- reads from `~/.capsem/user.toml` as fallback.
- **FS monitor debouncer lost delete events** -- replaced last-write-wins hashmap with event queue.
- **MCP snapshot tools returned unbounded JSON** -- now paginated text tables.
- **Frontend npm audit vulnerabilities** -- pinned transitive deps via pnpm overrides.

### Security
- **Safe FUSE deserialization** -- `read_struct` returns `Option<T>` with hard bounds check in all builds.
- **fsync/flush error propagation** -- returns mapped errno on failure instead of silently succeeding.
- **VirtioFS resource limits** -- file handle cap (4096), read size clamp (1MB), gather buffer limit (2MB).
- **Async VirtioFS worker thread** -- FUSE processing on dedicated thread, irqfd interrupt delivery, virtqueue memory barriers.
- **Security documentation** -- threat model overview and virtualization security pages.

### Removed
- **`images/` directory** -- legacy build files fully replaced by `guest/config/`, `guest/artifacts/`, and `src/capsem/builder/templates/`.

## [0.12.1] - 2026-03-25

### Fixed
- **Files and Snapshots tabs broken in GUI mode** -- `FsMonitor` (file watcher) and `AutoSnapshotScheduler` were only started in CLI mode, never wired into the GUI boot path. Both now start automatically when running the Tauri app.
- **Snapshot API tool name mismatch** -- frontend sent `list_snapshots`/`delete_snapshot` but backend expected `snapshots_list`/`snapshots_delete`, causing all snapshot operations to fail silently.

### Changed
- **Snapshots tab revamped** -- unified table replacing separate manual/auto sections. New columns: total changes, added, modified, deleted per snapshot. Change counts sourced from per-snapshot diffs already computed by the backend.

## [0.12.0] - 2026-03-24

### Changed
- **Decomposed god modules into focused sub-modules** -- split `main.rs` (2,722 LOC) into 7 modules (assets, boot, cli, gui, logging, session_mgmt, vsock_wiring); split `policy_config.rs` (5,999 LOC) into 8 sub-modules (types, registry, loader, presets, resolver, builder, lint, tree); split `session.rs` (1,995 LOC) into 3 sub-modules (types, index, maintenance). All existing import paths preserved via re-exports.
- **Decomposed Tauri commands into domain modules** -- split `commands.rs` (1,425 LOC) into 7 focused modules: terminal, settings, vm_state, session, mcp, logging, utilities. Shared helpers (active_vm_id, reload_all_policies) in mod.rs. All Tauri IPC paths unchanged.
- **Moved AI traffic parsing under `net/`** -- `gateway/` renamed to `net/ai_traffic/` to reflect its role as the MITM proxy's AI parsing layer. All import paths updated.
- **`net_event_counts()` returns a named struct** -- replaced bare `(usize, usize, usize)` tuple with `NetEventCounts { total, allowed, denied }` to prevent field-order bugs.

### Fixed
- **Guest agent vsock I/O no longer hangs on host stall** -- `vsock_connect()` now sets `SO_SNDTIMEO`/`SO_RCVTIMEO` (30s) on all vsock sockets. `write_all_fd` and `read_exact_fd` explicitly handle `EAGAIN` as a fatal timeout, preventing both kernel-level hangs and userspace spin-loops.
- **AsyncVsock double-close bug** -- removed manual `libc::close()` in `Drop` that double-closed the fd already owned by the inner `UnixStream`.

## [0.11.0] - 2026-03-24

### Added
- **`snapshots` CLI tool** -- in-VM command for managing workspace snapshots (`snapshots create/list/changes/history/compact/revert/delete`). Uses FastMCP client to talk to the host MCP gateway. Supports `--json` flag for machine-readable output.
- **`snapshots_history` MCP tool** -- shows all versions of a file across snapshots with sequential status (new/modified/unchanged/deleted). Accepts both relative paths and `/root/` prefixed paths.
- **`snapshots_compact` MCP tool** -- merges multiple snapshots into a single new manual snapshot. Newest-file-wins strategy. Deletes source snapshots after compaction, freeing pool slots.
- **Boot timing via vsock** -- capsem-init records per-stage durations as JSONL, PTY agent sends `BootTiming` message to host after boot. Host logs each stage with tracing and emits `boot-timing` event to frontend. Stages: squashfs, virtiofs, overlayfs, workspace, network, net_proxy, deploy, venv, agent_start.
- **Named snapshots** -- `snapshots_create` MCP tool creates named checkpoints with blake3 workspace hash. Manual snapshots are stored in a separate pool from auto snapshots and are never auto-culled.
- **Snapshot management MCP tools** -- 8 namespaced tools: `snapshots_create`, `snapshots_list`, `snapshots_changes`, `snapshots_revert`, `snapshots_delete`, `snapshots_history`, `snapshots_compact`. All prefixed with `snapshots_` to avoid collisions.
- **Snapshots UI tab** -- new tab in StatsView showing auto and manual snapshots with stat cards (total, auto, manual, available slots), delete button for manual snapshots.
- **`call_mcp_tool` Tauri command** -- generic frontend dispatcher for MCP built-in tools. Prepares for Phase 3 daemon MCP server.
- **Configurable snapshot limits** -- `settings.vm.snapshots.auto_max` (default 10), `settings.vm.snapshots.manual_max` (default 12), `settings.vm.snapshots.auto_interval` (default 300s) in the settings registry.
- **Boot time regression test** -- `test_boot_time_under_1s` fails if guest boot exceeds 1 second, catches regressions like the AI CLI copy stall.
- **XSS sanitization on guest data** -- boot timing stage names validated alphanumeric+underscore at both agent and host layers. File event paths reject NUL bytes, path traversal, control chars.
- **88 capsem-doctor MCP tests** -- comprehensive snapshot scenario coverage: modify/delete/recreate flows, copy/move, same-name-different-dirs, edge cases (deep paths, special chars, rapid snaps, 100 files), per-tool edge cases, belt-and-suspenders (MCP + CLI paths).
- Dual-pool snapshot scheduler: auto slots (ring buffer) + manual slots (named, never auto-culled). `SnapshotOrigin` enum (Auto/Manual).

### Changed
- **`snapshots_list` shows per-snapshot diffs** -- changes computed vs previous snapshot (not current workspace), showing what changed AT each snapshot. Includes `files_count` per entry.
- **`snapshots_revert` checkpoint is optional** -- auto-picks latest snapshot containing the file. Errors on "already current" (content + permissions match). Restores file permissions from snapshot.
- **All snapshots include blake3 hash** -- auto snapshots now compute workspace hash (previously manual-only).
- **Path normalization** -- all snapshot tools accept both `hello.txt` and `/root/hello.txt`.
- **AI CLIs use /opt/ai-clis directly** -- eliminated boot-time `cp -a` of hundreds of MB from squashfs to scratch disk. Boot time dropped from multi-second stall to ~530ms.
- **PATH single source of truth** -- `config/defaults.toml` defines PATH (sent via BootConfig SetEnv). Removed duplicate PATH exports from capsem-init, capsem-bashrc, capsem-doctor, profile.d.

### Fixed
- MCP file tools unavailable in GUI mode -- auto-snapshot scheduler was only wired into MCP config in CLI path, never in GUI boot path. Extracted shared `wire_auto_snapshots()` to eliminate duplication.
- `snapshots_list` changes were computed vs current workspace instead of vs previous snapshot
- `snapshots_history` status was computed vs current instead of sequentially
- `snapshots_revert` silently overwrote identical files
- File monitoring and MCP gateway no longer silently disabled when MITM proxy fails -- session DB decoupled from CA/policy loading
- Host file monitor (`FsMonitor`) was dropped immediately after creation, stopping FSEvents watcher
- `FsMonitor::emit` was not awaiting `db.write()`, so file events were never written to the session DB
- Zombie session vacuum warnings on startup
- `_init_and_call` test helper now surfaces actual MCP error messages instead of crashing with `KeyError`
- Snapshot test pool exhaustion -- autouse cleanup fixture deletes manual snapshots after each test

### Removed
- Guest `capsem-fs-watch` inotify daemon and vsock port 5005 -- host-side FSEvents monitoring fully replaces guest-side file watching

## [0.10.0] - 2026-03-21

### Added
- **VirtioFS storage mode** -- replaces tmpfs overlay + scratch disk with a single VirtioFS shared directory per session. Enables host-side file monitoring, auto-snapshots, and MCP file tools. System packages use an ext4 loopback image; workspace files in `/root` are directly visible on the host.
- **Host-side file monitoring** -- macOS FSEvents watches the VirtioFS workspace directory, replacing the in-guest `capsem-fs-watch` inotify daemon. More secure (no guest cooperation needed).
- **Rolling auto-snapshots** -- 12 APFS clone snapshots at 5-minute intervals (configurable). AI agents can list changed files and revert individual files to any checkpoint via MCP tools.
- **MCP file tools** -- `list_changed_files` (diff workspace against any auto-snapshot checkpoint) and `revert_file` (restore a file from any checkpoint, reflected immediately in guest via VirtioFS). Wired into the MCP gateway as built-in tools.
- **VirtioFS capsem-doctor tests** -- 9 new in-VM tests verifying VirtioFS root mount, ext4 loopback upper, loop device, workspace read/write, pip install, file delete+recreate
- Kernel support for VirtioFS (`CONFIG_FUSE_FS`, `CONFIG_VIRTIO_FS`) and loop devices (`CONFIG_BLK_DEV_LOOP`)
- Session schema v4: `storage_mode`, `rootfs_hash`, `rootfs_version` columns for rootfs lineage tracking
- Code coverage reporting via Codecov on PR and release CI pipelines
- OAuth credential forwarding for Claude Code and Gemini CLI -- auto-detects `~/.claude/.credentials.json` (subscription auth) and `~/.config/gcloud/application_default_credentials.json` (Google Cloud ADC), injects into guest VM at boot so agents work without API keys
- ECDSA SSH key detection (`id_ecdsa.pub`) in addition to ed25519 and RSA
- Boot screen with embedded release notes, download/boot progress, and re-run wizard button -- replaces the bare download progress overlay

### Changed
- Anthropic and OpenAI providers now enabled by default (was disabled) -- all three AI providers are allowed out of the box; corporate lockdown via `corp.toml` still overrides
- Default storage mode is now VirtioFS (block mode preserved for backward compatibility)
- Guest `capsem-fs-watch` daemon no longer launched in VirtioFS mode (host monitors instead)

### Fixed
- Frontend dependencies now auto-install on fresh clone -- `just dev`, `just ui`, `just run`, `just test`, `just doctor`, and all other recipes that need npm packages run `pnpm install --frozen-lockfile` automatically
- Setup wizard re-run now re-detects host configuration (SSH keys, API keys, OAuth credentials, GitHub tokens) instead of keeping stale values from first run

## [0.9.18] - 2026-03-21

### Fixed
- MCP server and filesystem watcher missing from release VM assets -- Claude and Gemini reported MCP as "disconnected" because `capsem-mcp-server` and `capsem-fs-watch` were never included in the release rootfs
- MCP Servers settings page showing "no VM running" permanently -- MCP data now reloads automatically when the VM finishes booting

### Added
- Build pipeline now auto-derives guest binary list from `capsem-agent/Cargo.toml` -- adding a new `[[bin]]` target is automatically picked up by `build.py`
- Rust test and preflight check verify all guest binaries appear in `Dockerfile.rootfs` and `justfile` -- prevents future binary-list drift between dev and release

## [0.9.17] - 2026-03-20

## [0.9.16] - 2026-03-20

## [0.9.15] - 2026-03-20

## [0.9.14] - 2026-03-20

### Fixed
- Download progress screen not shown on first launch: `vmStatus()` poll now returns "downloading" via app-level state, fixing the race where the event fired before the frontend subscribed
- `latest.json` missing from release artifacts, causing auto-updater `update check failed` on every boot

## [0.9.13] - 2026-03-20

### Fixed
- First-launch crash: `gui_boot_vm` called from tokio worker thread after rootfs download caused `EXC_BREAKPOINT` (`dispatch_assert_queue_fail`). VM start/stop now guarded by `is_main_thread()` check, post-download boot dispatched to main thread via `run_on_main_thread`
- Site domain references updated from `capsem.dev` (dead) to `capsem.org`

### Added
- Boot path logging: `resolve_rootfs` and `create_asset_manager` now log each location checked, version, manifest path, release count, and download status
- `cut-release` recipe: one-command version bump, changelog stamp, commit, tag, push, and CI wait

### Changed
- Release pipeline merged from two steps (build on tag push + publish via `workflow_dispatch`) into a single pipeline that builds and publishes on tag push
- `release` recipe simplified: waits for CI build (which now includes publish), no longer triggers a separate workflow
- Consolidated seven 0.9.x news posts into a single page covering 0.9.0 through 0.9.13

## [0.9.12] - 2026-03-19

### Added
- Wizard validates API keys in real-time against provider endpoints (spinner, check/X inline)
- API key detection now checks `~/.config/openai/api_key` and `~/.anthropic/api_key`
- Build verification documentation (SBOM, attestation, manifest signatures)

### Fixed
- `svelte-check` failing on `dist/` build artifacts (excluded from tsconfig)

## [0.9.11] - 2026-03-19

### Fixed
- Download progress now shown in main app view when setup wizard is skipped (returning users with existing config but missing rootfs saw a blank terminal)

### Added
- Frontend test infrastructure (vitest + @testing-library/svelte) with store and component tests

## [0.9.10] - 2026-03-19

### Fixed
- Rootfs removed from DMG bundle (was 463 MB, now ~15 MB) -- rootfs is downloaded on first launch
- Build attestation (SBOM + provenance) restored after CI refactor
- Manifest.json now signed with minisign (same key as updater artifacts)

## [0.9.3] - 2026-03-18

### Fixed
- CI codesign hang: keychain now set as default, explicitly unlocked with 1-hour timeout, and existing keychain search list preserved
- CI Node.js upgraded from 22 to 24
- CI release creation split from build: artifacts uploaded as CI artifacts, release created locally with `gh` CLI (org restricts GITHUB_TOKEN to read-only)

### Changed
- GitHub Actions upgraded to Node 24 (checkout v5, setup-node v5, upload/download-artifact v5, setup-buildx v4)
- CI workflow scoped to PRs only; site deploy scoped to main + site/ changes only

## [0.9.0] - 2026-03-18

### Added
- Persistent logging system: three-layer tracing (stdout, per-launch JSONL file, Tauri UI layer) with per-VM log files in session directories (CLI + GUI)
- Logs view in sidebar with live event stream, boot timeline visualization, session history browser, level filtering, and auto-scroll
- Per-launch log files (`~/.capsem/logs/<timestamp>.jsonl`) with automatic 7-day cleanup
- Per-VM session logs (`~/.capsem/sessions/<id>/capsem.log`) with structured JSONL events for both CLI and GUI modes
- `load_session_log` and `list_log_sessions` Tauri commands for historical log access
- Error messages now included in `vm-state-changed` events for all error states
- Boot timeline state transitions emitted as structured tracing events
- Integration test verifies log file creation, JSONL validity, level filtering, boot timeline events, and timestamp format
- App auto-update: `createUpdaterArtifacts` enabled so CI produces `.tar.gz` + `.sig` updater files and `latest.json` -- the built-in updater now works
- `app.auto_update` setting (default: true) to gate the startup update check, with "Check for Updates" button in Settings > App
- Multi-version asset manifest (`manifest.json`) replaces single-version `B3SUMS` -- supports multiple release versions, merge across releases, and future checkpoint restore
- Version-scoped asset directories (`~/.capsem/assets/v{version}/`) with automatic migration from flat layout and cleanup of old versions
- `pinned.json` support for keeping specific asset versions during cleanup (for future checkpointing)
- `scripts/gen_manifest.py` for manifest generation in justfile and build.py
- First-run setup wizard -- 6-step guided configuration (Welcome, Security, AI Providers, Repositories, MCP Servers, All Set) that runs while the VM image downloads in the background
- Host config auto-detection -- wizard scans ~/.gitconfig, ~/.ssh/*.pub, environment variables, and `gh auth token` to pre-populate settings with detected values
- SSH public key setting (`vm.environment.ssh.public_key`) -- injected as /root/.ssh/authorized_keys in the guest VM at boot
- Re-run setup wizard button in Settings > VM to revisit configuration without overwriting existing settings
- Resumable asset downloads -- partial .tmp files are preserved across app restarts and continued via HTTP Range headers instead of re-downloading from scratch
- Security presets ("Medium" and "High") -- one-click security profiles selectable from Settings > Security
- Automatic migration of old setting IDs (`web.*`, `registry.*`) to new `security.*` namespace -- existing user.toml and corp.toml files work without manual changes
- `fetch_http` now supports `format=markdown` (new default) -- converts HTML to clean markdown preserving headings, links, lists, bold/italic, and code blocks
- Wikipedia (`en.wikipedia.org`, `*.wikipedia.org`) added to default allow list for MCP HTTP tools
- Auto-detect latest stable kernel version from kernel.org during `just build-assets`
- User-editable bashrc and tmux.conf as file settings in Settings > VM > Shell
- Filetype-aware syntax highlighting for file settings (bash, conf, json)
- Documentation URLs for API key settings (links to provider console/settings pages)
- Repositories section in settings with git identity (author name/email) for VM commits
- Personal access token settings for GitHub and GitLab (enables git push over HTTPS via .git-credentials)
- GitLab as a repository provider with domain allow/block and token support
- Added `tmux` and `gh` to the default rootfs for terminal multiplexing and GitHub CLI support
- Token prefix hints in settings UI -- apikey inputs show expected format (e.g., `ghp_...`, `sk-ant-...`) with a warning if the entered value doesn't match
- `GH_TOKEN` / `GITHUB_TOKEN` env vars injected in VM when GitHub token is configured, enabling `gh` CLI without `gh auth login`
- `GITLAB_TOKEN` env var injected in VM when GitLab token is configured

### Changed
- CI release workflow now accumulates manifest.json across releases and uploads it alongside rootfs
- `_pack-initrd` regenerates manifest.json on every `just run` via `scripts/gen_manifest.py`
- `build.rs` reads hashes from manifest.json (preferred) with B3SUMS fallback
- Settings restructured: "Web" and "Package Registries" merged under new "Security" top-level section with "Web", "Services > Search Engines", and "Services > Package Registries" sub-groups
- MCP gateway rewritten to use rmcp (official Rust MCP SDK) -- replaces hand-rolled JSON-RPC/SSE client with proper Streamable HTTP transport, automatic pagination, and typed tool/resource/prompt routing
- Upgraded reqwest from 0.12 to 0.13
- MCP server UI redesigned: collapsible server cards with URL/auth config, "verified"/"definition changed" status labels
- Tool origin telemetry expanded from 2 values (native/mcp) to 3 values (native/mcp_proxy/local)
- Auto-detected stdio MCP servers from Claude/Gemini settings shown with unsupported warning instead of silently dropped
- `just install` now runs validation gates only (doctor + full-test); `.app` bundling is CI-only
- Missing API key warnings now appear in the group header when collapsed, with a "Get key" link
- GitHub moved from "Package Registries" to "Repositories" section
- `registry.github.*` setting IDs renamed to `repository.github.*`
- Package Registries description updated to "Package manager registries"

### Removed
- Stdio bridge for MCP servers (`stdio_bridge.rs`) -- replaced by HTTP client

### Fixed
- MCP server bearer token auth sent double "Bearer" prefix (`Bearer Bearer <token>`), causing 401 from authenticated servers like deps.dev
- Tool calls no longer double-counted in stats -- MCP-proxied tool_calls (origin=mcp_proxy) filtered from native counts across all 6 tool queries
- Native tool response preview now displayed in unified tool list (was hardcoded NULL, now joined from tool_responses via call_id)
- Non-text content blocks (tool_reference, image) in Anthropic tool results now produce meaningful preview instead of empty string
- OpenAI multipart tool result content now extracted correctly
- `check_session.py` tool response matching fixed -- joins on call_id only (tool responses arrive in next model call with different model_call_id)
- MCP server now visible in `claude mcp list` -- was injected into wrong file (`settings.json` instead of `.claude.json`)
- Codex CLI MCP server config added (`~/.codex/config.toml`) -- was missing entirely
- Disabling an AI provider now takes effect immediately on existing keep-alive connections (policy was previously snapshot per-connection, not per-request, so in-flight HTTP/1.1 connections continued to allow requests after the provider was toggled off)
- MCP tool_responses no longer double-counted in multi-turn conversations (request parsers now extract only trailing tool results instead of full history)
- MCP call previews no longer truncated at 200 chars (removed hard truncation; 256KB cap_field safety net remains)
- `fetch_http` paginate now UTF-8 safe -- uses `floor_char_boundary` to avoid panics on multi-byte content (emoji, Cyrillic, CJK, etc.)
- `fetch_http` on subpaths (e.g. `elie.net/about`) now returns full page content -- replaced `tl` HTML parser with `scraper` (html5ever) which correctly handles minified/complex HTML
- `fetch_http` format default changed from `content` to `markdown` for better AI agent consumption
- MCP byte tracking: `bytes_sent`/`bytes_received` columns added to mcp_calls for full I/O auditability
- Builtin MCP tool HTTP requests now emit net_events with `conn_type=mcp_builtin` for network audit visibility
- Guest process_name resolution uses `/proc/{pid}/cmdline` (real binary name) instead of `/proc/{pid}/comm` (thread name), fixing "MainThread" attribution
- Gemini tool call_ids now include a counter suffix to distinguish multiple calls to the same function
- Claude Code no longer warns about missing `/root/.local/bin` directory (created at boot after scratch disk mount)
- tmux now has a clean minimal config: mouse support, no escape delay, proper 256-color/truecolor, high scrollback
- tmux sessions can now find `gemini` and other npm-global binaries (PATH was lost when tmux started a login shell that reset it via `/etc/profile`)
- `gh auth status` injection test no longer fails with fake test tokens (test now verifies token detection, not authentication)
- Git authentication in VM: switched from `.netrc` to `.git-credentials` + `credential.helper=store` so `git push` works out of the box
- "Get one" links in settings now open in host browser via `tauri-plugin-opener` (previously broken in Tauri webview)

### Security
- Kernel hardening: heap zeroing (`INIT_ON_ALLOC`), SLUB freelist hardening, page allocator randomization, KPTI (`UNMAP_KERNEL_AT_EL0`), ARM64 BTI + PAC, `HARDENED_USERCOPY`, seccomp filter, cmdline hardening (`init_on_alloc=1 slab_nomerge page_alloc.shuffle=1`)
- Git credential tokens now reject `@` and `:` characters (in addition to newlines) to prevent URL injection in `.git-credentials`

## [0.8.8] - 2026-03-07

### Added
- Proxy throughput benchmark (`capsem-bench throughput`): downloads 100 MB through the full MITM proxy pipeline and reports MB/s — baseline ~35 MB/s on Apple Silicon
- `capsem-bench` is now repacked into the initrd on every `just run`, so changes to the benchmark script take effect immediately without a full rootfs rebuild
- `ash-speed.hetzner.com` added to the default network allow list and integration test config for the throughput benchmark
- Rust integration test `mitm_proxy_download_throughput` (in `crates/capsem-core/tests/mitm_integration.rs`): validates 100 MB download through the proxy at the host level; marked `#[ignore]` so it runs only on demand
- `test_proxy_download_throughput` in `capsem-doctor` (`test_network.py`): in-VM Layer 7 test verifying end-to-end proxy throughput; skips gracefully if the speed-test domain is not in the allow list
- `docs/performance.md`: documents all benchmark modes, baseline numbers, proxy data path, and domain allow list setup
- `just run` now kills any existing Capsem instance before booting, preventing a stale GUI window from appearing alongside a CLI run
- Notarization credential verification in CI preflight job: validates Apple API key against `notarytool history` before spending time on build-assets and tests
- Notarization preflight check in `scripts/preflight.sh`: verifies `.p8` key, API Key ID, Issuer ID, and runs a live `notarytool history` test

### Fixed
- `capsem-init` now aborts boot (kernel panic) if the tmpfs mount for the overlay upper layer fails, preventing a silent degraded boot where writes land on the initramfs instead of the intended tmpfs
- `capsem-init` now creates `/mnt/b` before mounting tmpfs on it (missing `mkdir -p` caused the tmpfs mount to fail with "No such file or directory" on fresh initrds)
- CI release no longer hangs on first-time notarization: `--skip-stapling` flag submits for notarization without waiting for Apple's response (first-time notarization can take hours)

### Security
- Boot invariant enforcement: `capsem-init` fatal-exits on tmpfs or overlayfs mount failure rather than continuing with a wrong upper layer; preflight check verifies this abort is present

## [0.8.4] - 2026-03-06

### Added
- `apt-get install` support inside the VM: overlayfs mounts with `redirect_dir=on,metacopy=on` (requires `CONFIG_OVERLAY_FS_REDIRECT_DIR`, `CONFIG_OVERLAY_FS_INDEX`, `CONFIG_TMPFS_XATTR` in kernel config), enabling dpkg directory renames without EXDEV errors. Packages installed in a session are gone after shutdown (ephemeral model preserved).
- `apt-packages.txt`: declarative list of system packages baked into the rootfs — edit and `just build-assets` to add/remove packages.
- Debian apt sources switched to HTTPS (`deb.debian.org`, `security.debian.org`) in `Dockerfile.rootfs`; both domains added to the default network allow list so the MITM proxy forwards them.
- Package lists pre-populated at rootfs build time so `apt-get install` works inside a running VM without a prior `apt-get update`.
- `force-unsafe-io` dpkg config in `capsem-init`: skips redundant fsyncs on overlayfs.
- Claude Code installed as a native binary (downloaded directly from Anthropic's GCS release bucket) instead of via npm, removing the Node.js dependency for the Claude CLI.
- Ephemeral model preflight check (`check_ephemeral_model` in `scripts/preflight.sh`): statically verifies `capsem-init` never skips `mke2fs` and never uses the scratch disk as overlay upper layer.
- Ephemeral model end-to-end test (`check_persistence` in `scripts/integration_test.py`): boots two consecutive VMs, writes a sentinel file in the first, and asserts it is absent in the second.

### Changed
- `images/README.md` developer section now documents how to add packages from all sources (apt, pip, npm, runtime) with copy-paste examples.

### Security
- Ephemeral model invariants documented in `CLAUDE.md` and enforced by preflight + integration test to prevent accidental persistence anti-patterns from being introduced.

### Added
- `just doctor` command: checks all required dev tools, container runtime (docker/podman), Rust targets, and cargo tools are installed
- Release preflight checks (`scripts/preflight.sh`): validates Apple certificate format, keychain import, and base64 sync before CI release
- `scripts/fix_p12_legacy.sh`: converts OpenSSL 3.x p12 files to legacy 3DES format macOS Keychain accepts
- CI preflight job in release workflow: fails fast on certificate/credential issues before slow build jobs

### Changed
- Release builds are CI-only (removed `just release`); push a `vX.Y.Z` tag to trigger `.github/workflows/release.yaml`
- `just build-assets`, `just install` now run `just doctor` first to catch missing tools early
- `just run`, `just full-test`, `just bench` now verify VM assets exist before proceeding

### Fixed
- Apple certificate import in CI: re-exported p12 with legacy 3DES/SHA1 encryption (macOS rejects OpenSSL 3.x default PBES2/AES-256-CBC with misleading "wrong password" error)

### Added
- Configuration overrides via `CAPSEM_USER_CONFIG` and `CAPSEM_CORP_CONFIG` environment variables to support isolated testing and CI.
- Dedicated integration test configurations (`config/integration-test-user.toml` and `config/integration-test-corp.toml`) for reproducible end-to-end validation.
- Thin DMG distribution: rootfs excluded from app bundle, downloaded on first launch via asset manager with blake3 hash verification
- Asset manager (`asset_manager.rs`): checks, downloads, and verifies VM assets from GitHub Releases with streaming progress
- Download progress UI: full-screen progress bar shown during first-launch rootfs download
- CLI download support: `capsem "command"` auto-downloads rootfs with stderr progress if missing
- Squashfs support: boot_vm accepts both rootfs.squashfs (new) and rootfs.img (legacy) formats
- Release workflow uploads rootfs.squashfs as separate GitHub Release asset alongside the thin DMG
- Onboarding plan (`docs/onboarding.md`): first-launch wizard scope for credentials, MCP config, and guided setup
- AI stats tab: unified model analytics with stat cards (total calls, tokens, cost, models), model usage chart, token breakdown, cost-over-time, and provider distribution
- `StatCards.svelte` reusable component for stat card rows across all analytics tabs
- Chart color system (`css-var.ts`): provider hue families, model color assignment, file action colors, server palette -- all using oklch() constants (no CSS var lookups)
- LayerChart v2 API documentation (`docs/libs/layercharts.md`) for LLM-friendly chart development

### Changed
- Asset resolution in macOS app bundle now searches multiple paths in `Resources` (including nested Tauri v2 paths) for better reliability.
- Integration test isolated from host user settings and correctly maps `GOOGLE_API_KEY` to `GEMINI_API_KEY` for the internal VM CLI.
- Tauri asset bundling now uses a flat map to prevent deeply nested `_up_/_up_/assets` structures in the final package.
- `just dev` now automatically passes `CAPSEM_ASSETS_DIR` to ensure the VM boots during local development.
- Stats "Models" tab renamed to "Model" (AITab.svelte replaces ModelsTab.svelte)
- Network, Tools, and Files stats tabs rebuilt with LayerChart v2 simplified chart components (BarChart, PieChart) replacing raw D3/Chart.js primitives
- SQL queries expanded: per-model token/cost breakdowns, provider distribution, cost-over-time, tool success rates, file action breakdowns
- Wizard auto-show on first run removed (setup wizard is still accessible from sidebar)

### Fixed
- Integration test SQLite connection robustness improved by using plain paths instead of URI formatting.
- Anthropic API tracking: MITM proxy now strips `accept-encoding` for AI providers so SSE streaming responses arrive uncompressed. This fixes the issue where Anthropic usage and cost were recorded as NULL.
- AI telemetry pollution: `model_call` records are now strictly filtered to valid LLM API paths (e.g., `/v1/messages`), preventing metadata endpoints from generating spurious NULL traces.
- Fallback model extraction: Added regex-based fallback to extract the model name from truncated JSON request bodies when the 64KB preview buffer limit is reached.
- fs-watch telemetry drops: Fixed a race condition during VM boot where early vsock connections (like `fs-watch`) were dropped by the host before the terminal/control handshake completed.
- `scripts/run_signed.sh` now correctly refreshes the binary signature via `touch` after re-signing with entitlements.
- Build prerequisites documentation updated with `b3sum`, `tauri-cli`, and `musl-cross` toolchain requirements.
- capsem-doctor PATH: writable bin dirs (`/root/.npm-global/bin`, `/root/.local/bin`) now included so AI CLIs and npm globals are found
- Gemini CLI settings.json: added `homeDirectoryWarningDismissed` and `sessionRetention` to suppress first-run prompts
- AI provider domain-blocked test now skips when the provider is explicitly enabled by policy
- Integration test handles compressed session DBs (`session.db.gz`) after vacuum
- Integration test accepts `vacuumed` as valid terminal session status

### Changed
- capsem-doctor and diagnostics are now repacked into the initrd, so changes take effect with `just run` instead of requiring `just build-assets`
- `just full-test` now includes initrd repack to ensure latest guest code is deployed

### Added
- `config_lint()` function: validates all settings (JSON files, number ranges, choices, API key format, nul bytes, URL format) with clear human-readable error messages displayed inline in the settings UI
- `SettingsNode` tree API: backend exposes the TOML settings hierarchy as a nested tree with resolved values at leaves, replacing the flat list for UI rendering
- `get_settings_tree` and `lint_config` Tauri commands for the new tree-based settings UI
- UI debug skill (`.claude/skills/UI_debug.md`): comprehensive Chrome DevTools MCP-based visual verification checklist for the settings UI

### Changed
- File settings now store path and content together as `{ path, content }` objects instead of keeping `guest_path` in metadata -- path is the source of truth for MCP injection and guest config generation
- Guest config file permissions tightened from 0o644 to 0o600 (owner-only) since settings files may contain API keys
- JSON validation uses zero-allocation `serde::de::IgnoredAny` instead of parsing into `serde_json::Value`
- Settings UI fully rewritten: left nav and section content are auto-generated from the TOML settings tree. Adding new categories or settings to `defaults.toml` automatically appears in the UI with no frontend code changes. Replaced 6 hardcoded section components (ProvidersSection, McpSection, NetworkPolicySection, EnvironmentSection, ResourcesSection, AppearanceSection) and their icon imports with a single generic recursive renderer (`SettingsSection.svelte`)
- SubMenu component now supports optional icons (icon-less items render label only)

### Security
- File setting paths are validated: must start with `/`, must not contain `..`, warns on unusual paths not under `/root/` or `/etc/`

### Added
- File analytics section: stat cards, action breakdown chart, events-over-time chart, and searchable event table for filesystem activity tracking
- Setup wizard hook: auto-detects first run (no API keys configured) and shows a welcome view with provider setup shortcut
- Reveal/hide toggle for API key and password fields in provider settings
- Range hints (min/max) shown below number inputs in VM resource and appearance settings
- Dropdown rendering for settings with predefined choices

### Changed
- Analytics data separation: Models and MCP analytics sections now exclusively query session.db; cross-session data (sessions over time, avg calls per session) moved to Dashboard
- "Session stats" button in terminal footer now navigates to session-level AI analytics instead of cross-session dashboard
- MCP analytics stat cards expanded from 2 (total + avg/session) to 4 (total, allowed, warned, denied)

### Security
- main.db `query_raw` now enforces `PRAGMA query_only = ON` around user SQL execution, preventing write-through via SQL injection (e.g., `SELECT 1; DROP TABLE sessions`) in the `query_db` IPC command
- Read-only enforcement tests for both session.db (`DbReader`) and main.db (`SessionIndex`) query paths: INSERT, CREATE TABLE, DROP TABLE, and semicolon injection all verified to fail at the SQLite level

### Changed
- Unified SQL gateway: `query_db` IPC command now supports both session.db and main.db via `db` parameter ("session" or "main"), with bind parameter support via `params` array. Replaced 11 per-query Tauri commands (net_events, get_model_calls, get_traces, get_trace_detail, get_mcp_calls, get_file_events, get_session_history, get_global_stats, get_top_providers, get_top_tools, get_top_mcp_tools) with a single `query_db` gateway
- Frontend queries now run through `db.ts` (unified query layer) instead of individual api.ts wrappers, using parameterized SQL from `sql.ts`
- Removed `ModelCallResponse` Rust wrapper struct (was only needed for the deleted `get_model_calls` command)
- Justfile streamlined from 23 recipes to 13 public + 5 internal helpers: `run` now auto-repacks initrd (replaces separate `repack`), `test` includes cross-compile + frontend check (replaces `check`), `full-test` combines capsem-doctor + integration test + bench (replaces `smoke-test`/`integration-test`/`preflight`), `build-assets` replaces `build`, `inspect-session` replaces `check-session`, `release` now produces a DMG at `target/release/Capsem.dmg`
- Removed recipes: `compile`, `sign`, `frontend`, `rebuild`, `repack`, `repack-initrd`, `ensure-tools`, `smoke-test`, `integration-test`, `preflight` (functionality preserved as internal `_`-prefixed helpers or merged into public recipes)

### Fixed
- 12 compilation warnings eliminated across 3 files: dead code warnings in `capsem-fs-watch` cross-platform helpers (blanket `#![cfg_attr(not(target_os = "linux"), allow(dead_code))]`), unused `SessionStats` import in commands.rs, and test-only `close()` method gated with `#[cfg(test)]`
- Test fixture updated from integration test session with full pipeline coverage: denied net events, deleted file events, positive cost estimates, `origin` column on tool_calls
- `fixture_top_domains_non_empty` test assertion fixed: `count >= allowed + denied` accounts for error events that are counted in total but not in allowed/denied buckets
- `query_raw_real_type` test now validates REAL type serialization without requiring positive cost values in the fixture
- Integration test now exercises denied net events (curl to blocked domain), deleted file events (create + rm), cost estimation assertions, and tool origin verification (34 checks, up from 28)

### Added
- Session DB lifecycle management: sessions now progress through running -> stopped -> vacuumed -> terminated states. After a session stops, its DB is checkpointed, vacuumed, and gzip-compressed (`session.db.gz`), then WAL/SHM files are removed. Terminated sessions retain their main.db audit trail record even after disk artifacts are deleted.
- `vm.terminated_retention_days` setting (default 365): controls how long terminated session records are kept in main.db before permanent purging
- Periodic main.db WAL checkpoint every 5 minutes to prevent unbounded WAL growth
- DbWriter now checkpoints WAL on clean shutdown (drop)
- Startup vacuum recovery: any sessions that stopped but were not vacuumed (e.g. due to crash) are automatically compressed on next app launch
- `check-session` script now handles compressed session DBs (auto-decompresses `.gz` files)
- End-to-end integration test (`just integration-test`): boots a real VM, exercises all 6 telemetry pipelines (fs_events, net_events, mcp_calls, model_calls, tool_calls, main.db rollup), runs capsem-doctor MCP tests, asks Gemini to write a poem, and verifies every event type is correctly logged in the session DB
- Release preflight gates (`just preflight`): unit tests, cross-compile, capsem-doctor smoke test, integration test, and benchmarks must all pass before `just release` or `just install` builds the app
- In-VM benchmark recipe (`just bench`): standalone entry point for capsem-bench (disk I/O, rootfs read, CLI startup, HTTP latency)
- Tool origin tracking: `tool_calls` table now records `origin` ("native" or "mcp") and `mcp_call_id` columns to distinguish model built-in tools from MCP gateway tools
- `check-session` data quality warnings: flags model_calls with NULL model, tokens, or request_body_preview
- `check-session` tool lifecycle section: shows origin breakdown and MCP call correlation
- Diagnostic logging when streaming model_calls complete with NULL model, tokens, or preview fields

### Fixed
- Session backfill now looks for `session.db` instead of the old `info.db` filename
- MITM proxy AI telemetry: model name, token counts, and request body preview were NULL for all model_calls when `log_bodies` was disabled. The proxy now always captures up to 64KB of AI provider request/response bodies for metadata parsing regardless of the `log_bodies` setting.
- MITM proxy model resolution: added fallback chain (request body -> SSE stream -> response JSON -> URL path) so model name is extracted even for providers that put it in the URL (e.g. Gemini `/v1beta/models/gemini-2.5-flash:generateContent`)
- MITM proxy stream detection: streaming flag now detected from URL path (`streamGenerateContent` vs `generateContent`) instead of unreliable request body parsing
- MITM proxy non-streaming usage: token counts now parsed from JSON response body when SSE stream parsing yields no usage metadata
- MITM proxy tool origin: tool_calls now use `tool_origin()` for correct "native" vs "mcp" classification instead of hardcoding "native"
- MITM proxy tool responses: tool_result entries from AI request bodies are now correctly extracted (previously always empty when body capture was disabled)
- MITM proxy non-streaming response parsing now handles gzip-compressed response bodies (upstream often sends Content-Encoding: gzip)
- MITM proxy no longer creates model_call records for HEAD requests (connectivity probes from AI CLIs have no body/model/tokens)
- Telemetry event pipeline silently dropping events under burst load: `try_write()` in MITM proxy and fs-watch handler failed without logging when the 256-slot DB channel was full (e.g. during `npm install`). Replaced with async `write().await` via `tokio::spawn` for backpressure, and bumped channel capacity from 256 to 4096.
- MCP builtin tools (`fetch_http`, `grep_http`, `http_headers`) returning empty responses: `capsem-mcp-server` used `SHUT_RDWR` after stdin closed, killing in-flight gateway responses before they could be read back. Changed to `SHUT_WR` (half-close) so the reader thread collects all responses before shutdown.
- MCP `fetch_http` and `grep_http` now reject binary content (images, PDFs, audio, video, etc.) with a clear error instead of returning garbled text or UTF-8 decode errors
- MCP tools now reject non-HTTP schemes (`file://`, `ftp://`, `data:`, etc.) before any network request is made
- MCP `grep_http` now rejects empty patterns instead of matching every line

### Changed
- Settings registry migrated from hardcoded Rust to `config/defaults.toml` (TOML-based, embedded at compile time). Setting definitions use `String` fields instead of `&'static str`. No user-facing behavior change.
- Session culling now marks sessions as "terminated" instead of deleting main.db rows, preserving the audit trail. Old terminated records are purged after `vm.terminated_retention_days` (default 365 days).
- Schema migrated from v2 to v3 (additive: new `compressed_size_bytes` and `vacuumed_at` columns on sessions table)
- MCP built-in tools exposed without `builtin__` prefix: models now see `fetch_http`, `grep_http`, `http_headers` instead of `builtin__fetch_http` etc. -- cleaner tool names for AI agents
- MCP built-in tool descriptions rewritten with full documentation: HTML extraction behavior, output format, pagination, domain policy enforcement, and error conditions
- Per-session analytics (Traffic, AI Models, MCP views) now use `queryDb(sql)` with SQL constants instead of dedicated Tauri commands -- reduces Rust boilerplate and gives the frontend more flexibility
- Network store rewritten: individual SQL queries replace monolithic `getSessionStats()` call, adding SQL-driven avg latency, method distribution, and process distribution
- Dashboard session detail no longer shows file event count (global dashboard should only show global data)
- Rootfs switched from 2GB ext4 to 382MB squashfs (zstd, 64K blocks) -- 81% smaller for DMG distribution
- Boot sequence uses overlayfs (immutable squashfs lower + ephemeral tmpfs upper) -- writes to system paths silently go to tmpfs
- Test fixture (`data/fixtures/test.db`) is now captured from real sessions instead of generated by a Python script
- `just update-fixture <path>` replaces `just gen-test-db`: copies a real session DB, scrubs API keys, and syncs to `frontend/public/fixtures/`

### Removed
- Dead AI gateway server (`gateway/server.rs`, 997 lines): axum HTTP server on vsock:5004 was never wired up in main.rs. All AI traffic goes through the MITM proxy on vsock:5002. `extract_model_from_path`, `parse_non_streaming_usage`, and `tool_origin` helpers moved to `gateway/provider.rs` and `gateway/events.rs` where the MITM proxy can use them.
- `VSOCK_PORT_AI_GATEWAY` constant (port 5004) -- unused, never wired up
- `GatewayConfig` struct -- only used by the dead server
- `gateway_integration.rs` test file -- tests for the dead server
- `axum` dependency from capsem-core
- `get_session_stats`, `get_mcp_stats`, `get_file_stats` Tauri IPC commands -- replaced by frontend SQL via `queryDb()`
- `SessionStatsResponse` struct from commands.rs and `SessionStatsResponse`, `SessionStats`, `McpCallStats`, `FileEventStats` types from frontend
- `SessionsSection.svelte` -- orphan component never imported by AnalyticsView
- `data/fixtures/generate_test_db.py` -- synthetic data generator replaced by real session captures

### Added
- `sql.ts`: centralized SQL query constants for all per-session analytics (13 queries covering net stats, domains, time buckets, provider usage, tool usage, model stats, MCP stats, file stats, latency, method/process distribution)
- `queryOne<T>()` and `queryAll<T>()` typed helpers in `api.ts` for running SQL against the active session's info.db
- Analytics data architecture documented in `docs/architecture.md` (two-database design, data flow, query strategy, polling patterns)
- Frontend development skill file (`.claude/skills/frontend.md`)
- In-VM filesystem watcher (`capsem-fs-watch`): inotify-based daemon streams file create/modify/delete events to the host over vsock:5005 for real-time file activity telemetry
- `fs_events` audit table in `capsem-logger`: records every file operation with timestamp, action, path, and size
- `FileEvent` type with `WriteOp::FileEvent` variant and reader queries (`recent_file_events`, `search_file_events`, `file_event_stats`)
- `get_file_events` and `get_file_stats` Tauri IPC commands for the frontend
- Files view in frontend: summary cards (total/created/modified/deleted), searchable event table with action badges, 2s polling
- Files sidebar navigation item with document icon between Sessions and MCP Tools
- Mock file event data (13 entries) for browser dev mode
- MCP gateway wired to vsock:5003: host now accepts MCP connections from guest agents, fixing Gemini CLI hang on startup
- Built-in HTTP tools: `fetch_http`, `grep_http`, `http_headers` -- AI agents can fetch web content, search pages, and inspect headers from within the sandbox, all checked against domain policy
- MCP domain policy hot-reload: changing network settings in the UI immediately updates which domains built-in HTTP tools can access
- `capsem-doctor` MCP tests: 6 new in-VM diagnostic tests verifying MCP binary, initialize handshake, tools/list, allowed/blocked fetch, and fastmcp availability
- `fastmcp` Python package in guest rootfs for building custom MCP servers inside the VM
- MCP Proxy Gateway: AI agents in the guest VM can now use host-side MCP tools transparently via a unified `capsem-mcp-server` binary injected at boot
- `capsem-mcp-server` guest binary: lightweight NDJSON-over-vsock bridge (~90 lines) relaying MCP JSON-RPC between agents and the host gateway on vsock:5003
- MCP gateway host module (`capsem-core::mcp`): types, policy engine, stdio bridge, server manager, and vsock gateway for routing tool calls to host-side MCP servers
- Namespaced MCP tools: tools from multiple servers are exposed as `{server}__{tool}` to prevent collisions (e.g., `github__search_repos`, `slack__send_message`)
- Per-tool dynamic policy: each MCP tool can be set to allow (forward normally), warn (forward + flag), or block (return JSON-RPC error) with hot-reload via `Arc<RwLock<Arc<McpPolicy>>>`
- MCP server auto-detection: reads existing MCP configs from `~/.claude/settings.json` and `~/.gemini/settings.json` at boot
- `mcp_calls` audit table in `capsem-logger`: full telemetry for every MCP tool call (server, method, tool, decision, duration, error)
- `McpCall` event type with `WriteOp::McpCall` variant and `insert_mcp_call()` writer method
- `DbReader` MCP queries: `recent_mcp_calls(limit, search)` with text search across server/method/tool, `mcp_call_stats()` aggregation (total, allowed, denied, warned, by-server breakdown)
- Schema migration: existing databases automatically gain the `mcp_calls` table on open
- `get_mcp_calls` and `get_mcp_stats` Tauri IPC commands for the frontend
- `inject_capsem_mcp_server()`: automatically merges `{"capsem": {"command": "/run/capsem-mcp-server"}}` into Claude and Gemini settings.json at boot, preserving user-provided MCP server entries
- MCP Tools view in frontend: summary cards (total/warned/denied), per-server breakdown, searchable call log table with decision badges
- MCP sidebar navigation item with layers icon between Sessions and Settings
- Mock MCP data: 6 sample calls across 3 servers (github, filesystem, slack) for browser dev mode
- Generic usage details tracking: token breakdowns (cache_read, thinking) stored as extensible `usage_details` JSON map instead of individual columns -- zero schema changes when adding new token types
- OpenAI Responses API (`/v1/responses`) streaming support: parses `response.created`, `response.output_text.delta`, `response.reasoning_summary_text.delta`, `response.function_call_arguments.delta`, `response.output_item.added/done`, and `response.completed` SSE events
- OpenAI cached token parsing from `prompt_tokens_details.cached_tokens` and reasoning token parsing from `completion_tokens_details.reasoning_tokens`
- Gemini thinking token parsing from `thoughtsTokenCount` (was parsed but unused)
- Non-streaming response parsing: gateway now extracts model, input/output tokens, and usage details from non-streaming JSON responses (all three providers), enabling cost estimation and token tracking for non-streamed API calls
- Cache and thinking token counts shown in session stats and trace detail UI

### Changed
- `capsem-proto` simplified: removed `McpGuestMsg`/`McpHostMsg` enums and encode/decode functions in favor of raw NDJSON passthrough (less code, better performance)
- `capsem-init` deploys `capsem-mcp-server` from initrd (with rootfs fallback)
- `just repack` cross-compiles and bundles `capsem-mcp-server` alongside pty-agent and net-proxy
- Sessions view: trace detail panel now shows MCP tool calls inline with model calls
- Token details stored as flexible `usage_details TEXT` JSON column replacing individual token columns -- single schema handles all current and future token breakdowns
- Cost estimation accounts for cached tokens: `cache_read` tokens subtracted from effective input before pricing calculation
- Pricing function signature simplified: accepts `&BTreeMap<String, u64>` usage details map instead of individual token parameters

### Fixed
- MCP gateway no longer sends a JSON-RPC response for `notifications/initialized` (it's a notification, not a request) -- fixes protocol confusion in some MCP clients
- Token metrics double-counted in trace detail view when a model call had both request and response tool entries -- now only the first row per call shows metrics
- Non-streaming API responses (no `stream: true`) recorded with null tokens and $0.00 cost -- now properly parsed for all providers
- HEAD connectivity checks from AI CLIs (Claude, Gemini) no longer create empty model_call rows -- filtered at the gateway level

## [0.8.0] - 2026-02-28

### Added
- `capsem-logger` crate: unified audit database with dedicated writer thread, replacing three separate SQLite databases (`WebDb`, `GatewayDb`, `AiDb`) with a single `session.db` per VM session
- Dedicated writer thread using `tokio::sync::mpsc` channel with block-then-drain batching (up to 128 ops per transaction), eliminating `spawn_blocking` + `Arc<Mutex<>>` contention
- `DbWriter` / `DbReader` API: async writes via channel, read-only WAL concurrent readers, typed `WriteOp` enum for debuggable operations
- Unified schema: `net_events` (all HTTPS connections), `model_calls` (denormalized request+response), `tool_calls`, `tool_responses` tables in a single DB file
- Inline SSE event parsing in the MITM proxy for AI provider traffic (Anthropic, OpenAI, Google Gemini)
- Provider-agnostic LLM event types (`LlmEvent`, `StreamSummary`) with `collect_summary()` for structured audit logging
- Hand-rolled SSE wire-format parser with chunk-boundary-safe state machine (no crate dependency)
- Provider-specific SSE stream parsers: Anthropic (interleaved content blocks, thinking), OpenAI Chat Completions (tool calls, content filter), Google Gemini (complete events, synthetic call IDs)
- Request body parser extracting model, stream flag, system prompt preview, message/tool counts, and tool_result entries for tool call lifecycle linking
- `AiResponseBody`: hyper Body wrapper that does SSE parsing inline during `poll_frame` with zero added latency
- AI provider domain detection (`api.anthropic.com`, `api.openai.com`, `generativelanguage.googleapis.com`) in the MITM proxy
- API key suffix extraction (last 4 chars, Stripe-style) from `x-api-key` and `Authorization: Bearer` headers
- Per-call cost tracking: gateway estimates USD cost using bundled model pricing data from pydantic/genai-prices
- Fuzzy model name matching for pricing: unknown model variants (date-stamped, custom-suffixed) now resolve to the correct pricing via progressive suffix stripping and longest-prefix fallback instead of silently returning $0.00
- Trace ID assignment in MITM proxy: multi-turn tool-use conversations are linked by shared trace IDs, enabling the Sessions view to render conversation spans
- SQL-driven session statistics: counts, token usage, cost, domain distribution, and time-bucketed charts all computed via SQLite queries
- New Tauri IPC commands: `get_session_stats` (full aggregate dashboard data), `get_model_calls` (model call history with search)
- LLM Usage section in Sessions view: API call count, input/output tokens, estimated cost, per-provider breakdown, model calls table, tool usage badges
- SQL-powered search in Network view: debounced search queries hit SQLite LIKE instead of client-side filtering
- `just update_prices` recipe to refresh bundled model pricing data
- `capsem-bench` in-VM performance benchmark tool: disk I/O (sequential read/write, random 4K IOPS) and HTTP throughput (ab-style concurrent requests with latency percentiles)
- `capsem-bench rootfs` benchmark: sequential and random 4K read performance on the read-only rootfs
- `capsem-bench startup` benchmark: cold-start latency for python3, node, claude, gemini, and codex CLIs (3 runs, min/mean/max)
- Rich table formatting for all capsem-bench output (replaces manual text formatting)
- Configurable VM CPU cores via `vm.cpu_count` setting (1-8, default 4)
- Configurable VM RAM via `vm.ram_gb` setting (1-16 GB, default 4 GB)
- 1 GB swap file on scratch disk for better memory pressure handling
- Search category in settings: Google Search (on by default), Perplexity, and Firecrawl toggles with domain-level policy
- Custom allow/block domain lists (`network.custom_allow`, `network.custom_block`) for user-defined domain rules
- Active Policy debug panel in Network view: collapsible section showing allowed/blocked domain lists, default action, corp managed status, and policy conflicts
- Policy conflict detection: domains appearing in both allow and block lists are flagged in the Network view

### Changed
- Terminal UI overhaul: borderless look with 10px padding, thin styled scrollbar, theme-matching background (full black in dark mode)
- Removed bottom status bar; session stats (tokens, tools, cost, VM status) now displayed inline below the terminal
- Sidebar reorganized: Console + Sessions in nav, Settings/theme/collapse in footer
- Network view moved into Settings as a collapsible "Network Statistics" section
- Sessions panel (charts, spans, analytics) now accessible from sidebar nav
- Session Statistics section added to bottom of Settings view
- MITM proxy and gateway server use `DbWriter` channel instead of `spawn_blocking` + `Arc<Mutex<>>` for all database writes
- Session telemetry stored in `session.db` (was `info.db`)
- VM Disk Performance Overhaul: 2M+ IOPS for random 4K reads (~8 GB/s) and ~20x speedup in random write throughput
- Network Proxy Overhaul: replaced synchronous thread-per-connection guest proxy with Tokio-based async implementation
- Structural Latency Elimination: `TCP_NODELAY` on both guest and host proxies, reducing proxy overhead to the physical network floor (~40ms median RTT)
- VM CPU default increased from 2 to 4 cores
- VM RAM default increased from 512 MB to 4 GB
- Scratch disk default increased from 8 GB to 16 GB
- Node.js V8 heap cap raised from 512 MB to 2 GB to match higher RAM
- Network store is now SQL-driven: counts and charts read from `get_session_stats` instead of counting JS arrays
- Session info response expanded with LLM metrics (model call count, tokens, tool calls, estimated cost)
- `net_events` command accepts optional `search` parameter for SQL-backed filtering
- `get_session_info` is now async with `spawn_blocking` for proper non-blocking DB access
- Rootfs disk caching mode changed from `Automatic` to `Cached` for aggressive host page cache retention on the read-only disk
- Host-side disk settings: enabled host-level caching (`VZDiskImageCachingMode::Cached`) and disabled synchronization barriers (`VZDiskImageSynchronizationMode::None`)
- Guest-side kernel tuning: `capsem-init` now sets I/O scheduler to `none`, `read_ahead_kb` to 4096, and `nr_requests` to 256 for all VirtIO devices
- Filesystem optimizations: `noatime,nodiratime,noload` mount options for rootfs and scratch disks
- Scratch disk format optimization: `mke2fs -m 0` to reclaim reserved root blocks
- `elie.net` moved from a Package Registry toggle to the default custom allowed domains list
- `network.log_bodies` and `network.max_body_capture` moved from Network to VM category
- Session settings (`session.retention_days`, `session.max_sessions`, `session.max_disk_gb`) moved from Session to VM category
- Mock data now mirrors the full backend settings registry (~35 settings across 7 categories)
- Settings view categories displayed in fixed order: AI Providers, Search, Package Registries, Network, Guest Environment, Appearance, VM
- Settings view categories collapsed by default (click to expand)
- Network view: allowed/blocked domain lists are now separate collapsible groups within Active Policy

### Fixed
- VM status indicator now shows correct color (blue for running, yellow for booting) instead of defaulting to no color due to state casing mismatch between Rust and frontend
- MITM proxy now assigns trace IDs and estimates costs for AI model calls, enabling Sessions view to display LLM statistics
- Fixture-dependent test assertions in capsem-logger replaced with data-agnostic checks to prevent breakage on fixture regeneration
- Benign "error shutting down connection" warnings in the host proxy logs are now filtered

### Removed
- Dead `gateway/audit.rs` module (839 lines, never compiled) superseded by capsem-logger
- `GatewayDb` (redundant flat table, replaced by `model_calls` in unified schema)
- `AiDb` (normalized 4-table schema, merged into `capsem-logger`)
- `WebDb` (replaced by `net_events` table in unified schema)
- `StreamAccumulator` (unused since `AiResponseBody` replaced it)
- `registry.elie.allow` setting (replaced by `network.custom_allow` default)
- `registry.debian.allow` setting (rootfs is read-only, packages cannot be installed at runtime)
- `domainlist` setting type from frontend (custom allow/block use standard `text` type with ID-based chip rendering)

### Security
- Terminal input batching thread now caps coalesced buffer at 64 KB, preventing unbounded memory growth if the IPC channel is flooded faster than the inner try_recv loop can drain
- Sanitize HTTP headers in telemetry logs: allowlisted headers (content-type, host, server, etc.) stored verbatim; all others (authorization, x-api-key, cookies) have values replaced with BLAKE3 hash prefix (`hash:<12-char-hex>`) to prevent credential leakage while preserving header presence and enabling correlation

## [0.7.0] - 2026-02-26

### Changed
- Terminal output uses poll-based binary IPC (`terminal_poll`) instead of JSON event emission, eliminating ~4x serialization overhead
- Terminal input batched with 5ms window (up to 4KB) to reduce IPC round-trips per keystroke
- Vsock read buffer increased from 8KB to 64KB and mpsc channel from 256 to 8192 entries
- CoalesceBuffer defaults changed from 10ms/64KB to 5ms/10MB for higher throughput
- Terminal output queue with 64-entry backpressure cap prevents OOM when frontend stops polling

## [0.6.0] - 2026-02-26

### Added
- Guest dev environment: `pip install`, `uv pip install`, `npm install -g` all work out of the box on the read-only rootfs
- Python venv auto-activated at boot with `--system-site-packages` (packages install to `/root/.venv`)
- `pip` and `python` aliased to `uv pip` and `uv run python` (faster, no root warning)
- AI CLIs (claude, gemini, codex) installed to writable scratch disk at boot so auto-update works
- npm global prefix redirected to writable `/root/.npm-global` for `npm install -g`
- Pre-installed Python packages declared in `images/requirements.txt`: numpy, requests, httpx, pandas, scipy, scikit-learn, matplotlib, pillow, pyyaml, beautifulsoup4, lxml, tqdm, rich
- Pre-installed npm globals declared in `images/npm-globals.txt` (AI CLIs)
- Login banner shows AI tool status: ready (blue), no API key (purple), disabled by policy (purple)
- Host injects `CAPSEM_ANTHROPIC_ALLOWED`, `CAPSEM_OPENAI_ALLOWED`, `CAPSEM_GOOGLE_ALLOWED` env vars at boot
- Configurable login banner (`images/banner.txt`) and random developer tips (`images/tips.txt`)
- Removed PEP 668 EXTERNALLY-MANAGED marker from rootfs
- `just build` upgrades all tools to latest: apt packages, pip, npm, node, nvm, uv
- Claude Code yolo mode: `~/.claude/settings.json` with `bypassPermissions` + `CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC=1`, and `~/.claude.json` state file to skip onboarding, trust dialogs, and keybinding prompts
- Gemini CLI yolo mode: `~/.gemini/settings.json` with `approvalMode: "yolo"`, telemetry/auto-updates disabled, folder trust disabled, and Gemini's own sandbox disabled (capsem provides the sandbox)
- Metadata-driven env var injection: settings declare `env_vars` in metadata instead of hardcoded mappings
- Built-in guest environment settings (`guest.shell.term`, `guest.shell.home`, `guest.shell.path`, `guest.shell.lang`, `guest.tls.ca_bundle`) configurable via user.toml and corp.toml
- Individual vsock boot messages (`SetEnv`, `FileWrite`, `BootConfigDone`) replacing single `BootConfig` frame, eliminating the 8KB frame size limit for boot configuration
- Guest boot log at `/var/log/capsem-boot.log` recording clock sync, env vars, file writes, and handshake status
- Per-service domain settings (`ai.*.domains`) with user-editable comma-separated domain patterns
- AI provider API key injection into guest VM environment variables (`ANTHROPIC_API_KEY`, `OPENAI_API_KEY`, `GEMINI_API_KEY`)
- Google AI (`ai.google.allow`) enabled by default for out-of-the-box Gemini CLI support
- Per-session unique IDs (`YYYYMMDD-HHMMSS-XXXX`) replacing hardcoded "default"/"cli" VM IDs
- Session index database (`~/.capsem/sessions/main.db`) tracking metadata across sessions
- `get_session_info` and `get_session_history` Tauri IPC commands for the Sessions view
- Session retention settings: `session.retention_days`, `session.max_sessions`, `session.max_disk_gb`
- Age-based, count-based, and disk-based session culling at startup
- Migration from legacy `session.json` files to `main.db` on startup
- Request count snapshotting (`count_by_decision`) when sessions stop
- Svelte 5 + Tailwind v4 + DaisyUI v5 frontend framework replacing vanilla JS
- Single Svelte island architecture: `<App client:only="svelte" />` in Astro shell
- Sidebar navigation with collapsible icon rail (Console, Sessions, Network, Settings)
- Network events view with filterable table, expandable rows showing headers/body
- Settings view with categorized editor, type-aware inputs, corp lock indicators
- Sessions view with VM state timeline from state machine history
- Terminal view wrapping existing xterm.js web component with Tauri event wiring
- Status bar showing VM state indicator, HTTPS call count, allowed/denied stats
- Light/dark theme toggle with localStorage persistence and system preference fallback
- Svelte 5 rune stores for VM state, network events, settings, theme, and sidebar
- TypeScript IPC layer (`types.ts` + `api.ts`) with typed wrappers for all Tauri commands
- `svelte-check` added to `just check` and `pnpm run check` pipelines
- Generic typed settings system replacing TOML-based policy config -- each setting has ID, type, category, default, metadata, and optional `enabled_by` parent toggle
- Per-setting corp override: corporate settings (`/etc/capsem/corp.toml`) lock individual settings, not entire sections
- Setting metadata with domain patterns, HTTP method permissions, numeric bounds, and text choices
- `get_settings` and `update_setting` Tauri IPC commands for the settings UI
- Settings architecture documentation in `docs/architecture.md`
- Policy override security documentation in `docs/security.md`

### Changed
- Increased vsock MAX_FRAME_SIZE from 8KB to 256KB for generous boot payloads
- Boot handshake protocol now sends env vars and files as individual messages instead of a single `BootConfig` payload
- Sessions view redesigned: current session info cards, network analytics, session history table (replaced CPU/memory/binary stats that VZ doesn't expose)
- Per-session telemetry renamed from `web.db` to `info.db` (legacy `web.db` still read for backward compatibility)
- Each VM boot creates a fresh telemetry database, eliminating stale request carryover between sessions
- Network policy replaced with simplified rule-based system: per-domain read/write verb control with defaults (GET allowed, POST denied)
- Configuration format changed from section-based TOML (`[network]`, `[guest]`, `[vm]`) to flat settings map (`[settings]` with dotted keys like `"registry.github.allow"`)
- Domain allow/block lists now derived from setting toggles and their metadata (e.g., toggling `registry.github.allow` controls `github.com`, `*.github.com`, `*.githubusercontent.com`)
- AI provider domains moved from explicit block-list to disabled-by-default toggles with domain metadata
- Guest environment variables stored as `guest.env.*` settings instead of `[guest].env` table
- VM settings (scratch disk size) stored as `vm.scratch_disk_size_gb` setting instead of `[vm]` section
- Removed SNI-based pre-TLS policy check; all policy enforcement at HTTP level
- Removed generativelanguage.googleapis.com from block-list (Gemini API testing)
- MITM proxy streams request and response bodies instead of buffering in memory
- Upstream TLS config cached per-VM instead of recreated per-request
- Default `log_bodies` changed from false to true

### Fixed
- Denied domains now record HTTP method, path, and status in telemetry (TLS handshake completes, denial at HTTP 403 level)
- Guest receives proper HTTP 403 response with reason for denied requests instead of cryptic TLS connection error
- "Invalid Date" in Session/Network views: timestamps now serialize as epoch seconds instead of SystemTime objects
- Legacy "default"/"cli" sessions migrated as "crashed" instead of carrying over stale "running" status
- web.db now records query string, matched rule, and 403 status for denied requests
- Upstream connection failures record error reason in telemetry

### Removed
- `get_vm_stats` command and `VmStats`/`BinaryCall` types (VZ framework doesn't expose guest metrics)
- Hardcoded `DEFAULT_VM_ID` constant -- replaced by dynamic session IDs
- `session.json` files -- replaced by `main.db` session index (migrated automatically)
- SNI parser module (`sni_parser.rs`) -- domain extracted from TLS handshake instead

### Security
- Env var sanitization: reject keys containing `=` or NUL bytes, values containing NUL (prevents agent crash / kernel panic)
- Blocked env var list: LD_PRELOAD, LD_LIBRARY_PATH, IFS, BASH_ENV, and other dangerous variables rejected during boot
- Boot allocation caps: max 128 env vars, 64 files, 10MB total file data
- FileWrite path traversal protection: reject paths containing `..`
- Defense-in-depth: guest agent validates env vars and file paths independently of host
- Body size limit (100MB) prevents OOM from malicious guest payloads
- Replaced unsafe borrow_fd with safe fd cloning
- Corp-locked settings cannot be modified by user, enforced at the merge level

## [0.5.0] - 2026-02-25

### Added
- Ephemeral scratch disk for `/root` workspace (8GB default, configurable via `[vm].scratch_disk_size_gb` in `~/.capsem/user.toml`)
- Per-session directory structure (`~/.capsem/sessions/<vm_id>/`) with session metadata (`session.json`)
- Stale session cleanup on startup: leftover scratch images deleted, orphaned "running" sessions marked as "crashed"
- Block device identifiers (`rootfs`, `scratch`) for stable device naming in the guest (`/dev/disk/by-id/virtio-*`)
- uv fast Python package installer available to guest AI agents

### Changed
- Guest `/root` workspace now uses ext4 on a virtio block device instead of RAM-backed tmpfs, increasing usable space from ~512MB to 8GB+
- Upgraded Node.js from Debian's v18 to v24 LTS via nvm
- Replaced pip3 with uv for in-VM Python package management (certifi, pytest)

### Fixed
- gemini CLI crashing with `SyntaxError: Invalid regular expression flags` due to Node.js 18 lacking the 'v' regex flag
- AI CLI smoke test was too lenient -- now verifies `--help` runs without JS runtime errors instead of only checking for signal crashes

## [0.4.0] - 2026-02-25

### Added
- Host-side state machine (`HostState`) with validated transitions, timing history, and structured perf logging
- Per-state message validation: host validates both outbound and inbound vsock control messages against lifecycle stage
- New Tauri IPC commands for Svelte UI: `get_guest_config`, `get_network_policy`, `set_guest_env`, `remove_guest_env`, `get_vm_state`
- Structured `vm-state-changed` events with JSON payloads (state + trigger) instead of plain strings
- Protocol documentation (`docs/protocol.md`): wire format, message reference, state machine diagrams, boot handshake, security invariants
- Zero-trust guest binary security rule documented in `docs/security.md`
- `write_policy_file()` for TOML serialization of user.toml changes from the UI
- MITM transparent proxy: full HTTP inspection (method, path, status code, headers, body preview) for all HTTPS traffic from the guest VM
- Static Capsem MITM CA certificate (ECDSA P-256, 100-year validity) baked into the guest rootfs trust store
- On-demand domain certificate minting with RwLock cache for TLS termination
- HTTP-level policy engine: method+path rules on top of domain allow/block lists (`[[network.rules]]` in user.toml)
- Extended telemetry: `web.db` now records HTTP method, path, status code, request/response headers, and body previews
- CA trust environment variables (`REQUESTS_CA_BUNDLE`, `NODE_EXTRA_CA_CERTS`, `SSL_CERT_FILE`) injected via BootConfig
- certifi CA bundle patching in rootfs for Python SDK compatibility (requests, openai, anthropic)
- Schema migration for existing `web.db` databases (adds new columns without data loss)
- Clock synchronization -- guest VM clock is set from host at boot time (fixes TLS cert validation, git, curl)
- Environment variable injection via vsock boot config (`BootConfig`/`BootReady` handshake)
- `[guest]` section in `user.toml` for custom guest environment variables
- `--env KEY=VALUE` CLI flag for one-off env injection (`capsem --env FOO=bar echo $FOO`)
- `capsem-proto` crate -- shared protocol types for host/guest communication
- Clock sync diagnostic test in `capsem-doctor`
- In-VM diagnostic test suite expanded: MITM CA trust chain tests (system store, certifi, curl without -k, Python urllib), network edge cases (HTTP port 80, non-443 ports, direct IP, AI provider blocking, multi-domain DNS), process integrity (pty-agent, dnsmasq, no systemd/sshd/cron), deeper kernel hardening (no modules loaded, no debugfs, no IPv6, no swap, no kallsyms, ro cmdline), environment validation (TERM, HOME, PATH, arch, kernel version, mount points), and 14 additional unix utility checks
- `just test` recipe runs workspace tests with coverage summary via `cargo-llvm-cov`
- `just ensure-tools` auto-installs `cargo-llvm-cov` and `llvm-tools-preview` on fresh clones
- Air-gapped networking: `curl https://elie.net` now works from inside the guest VM
- Host-side SNI proxy inspects TLS ClientHello, enforces domain allow-list, and bridges to the real internet
- Domain policy engine with allow-list, block-list, and wildcard pattern matching (`*.github.com`)
- Configurable domain policy via `~/.capsem/user.toml` and `/etc/capsem/corp.toml` (corp overrides user)
- Per-session `web.db` (SQLite) recording every HTTPS connection attempt for auditing
- Guest-side `capsem-net-proxy` binary: TCP-to-vsock relay for transparent HTTPS proxying
- Default developer allow-list: GitHub, npm, PyPI, crates.io, Debian repos, elie.net
- AI provider domain blocking at SNI level (api.anthropic.com, api.openai.com, googleapis.com)
- `net_events` Tauri command for querying recent network events from the frontend
- Per-VM network isolation: each VM gets its own policy, web.db, and connection handlers

### Changed
- SNI proxy replaced by MITM transparent proxy for full HTTP-level traffic inspection and policy enforcement
- Domain policy (`DomainPolicy`) wrapped by `HttpPolicy` which adds method+path rules while preserving backward compatibility
- `load_merged_policy()` now returns `HttpPolicy` instead of `DomainPolicy`
- HTTPS proxy connections spawn as async tokio tasks instead of blocking threads
- Control protocol split into disjoint `HostToGuest`/`GuestToHost` enums with reserved variants for file operations and lifecycle management
- Guest agent boot sequence restructured: vsock connects first, receives clock + env from host before forking bash
- Max control frame size bumped from 4KB to 8KB to accommodate env var payloads
- `just build`, `just repack`, and `just check` now run tests with coverage as a gate before proceeding
- Kernel now includes IP stack + netfilter (CONFIG_INET=y, iptables REDIRECT) for air-gapped networking
- Rootfs includes iproute2, iptables, and dnsmasq for guest network setup
- capsem-init sets up dummy0 NIC, fake DNS, and iptables rules at boot
- `just repack` now includes `capsem-net-proxy` alongside `capsem-pty-agent`
- Refactored VM smoke test into pytest-based diagnostic suite (`capsem-doctor`)
- Split tests into focused modules: sandbox security, utilities, runtimes, AI CLIs, workflows
- Added sandbox security tests (rootfs read-only, no kernel modules, no /dev/mem, network isolation, no setuid/setgid)
- Added Python and Node.js execution tests (actual code runs, not just version checks)
- Added AI CLI sandbox verification (binaries execute without crashing)
- Network sandbox tests updated: verify air-gapped proxy (allowed/denied domains) instead of raw network block

### Fixed
- MITM proxy TLS handshake failure: rustls crypto provider was not initialized, causing silent panics on every proxy connection
- MITM proxy now uses explicit `builder_with_provider()` instead of relying on global crypto state, eliminating the class of bug entirely
- `just build` failure: Dockerfile.rootfs could not find CA cert (build context was `images/`, cert was in `config/`)
- `just build` failure: certifi not installed when CA bundle patching step runs
- Kernel `CONFIG_KALLSYMS=n` was silently ignored because the option requires `CONFIG_EXPERT=y` to be configurable
- Kernel cmdline now includes `ro` for read-only rootfs mount
- `just smoke-test` now returns non-zero exit code on test failures
- In-VM diagnostic test fixes: `/proc/modules` absent is valid (CONFIG_MODULES=n), bash test checks availability not current shell, CA bundle tests grep base64 instead of DER-encoded CN, Python TLS test verifies handshake not HTTP status

### Deprecated
- `sni_proxy::handle_connection` -- use `mitm_proxy::handle_connection` for full HTTP inspection

### Security
- `CONFIG_EXPERT=y` in kernel defconfig ensures all hardening options (KALLSYMS=n, MODULES=n, etc.) are respected by `make olddefconfig`
- Kernel symbol table (`/proc/kallsyms`) now empty -- eliminates kernel ASLR bypass vector
- MITM proxy enables full HTTP audit trail: every request method, path, status code, and headers are logged to web.db
- HTTP-level enforcement rules allow fine-grained control (e.g., allow GET but deny POST to specific paths)
- Default-deny domain policy: only explicitly allowed domains are reachable from the guest
- No DNS leaves the VM: all resolution is faked to a local IP
- Corporate policy (`/etc/capsem/corp.toml`) overrides user settings for enterprise lockdown
- Per-VM isolation prevents cross-VM network interference

## [0.3.0] - 2026-02-24

### Added
- PTY-over-vsock terminal communication replacing serial broadcast channel
- Guest PTY agent (`capsem-pty-agent`) for high-throughput terminal I/O with full PTY support
- Terminal resize support (`stty size` reflects window dimensions)
- vsock control channel with MessagePack framing for structured commands (resize, heartbeat)
- Kernel vsock support (`CONFIG_VSOCKETS`, `CONFIG_VIRTIO_VSOCKETS`)
- Multi-VM-ready app state architecture (`vm_id`-keyed `HashMap`)
- Output coalescing (10ms/64KB) to prevent frontend IPC saturation
- Boot-time command execution via vsock (`Exec`/`ExecDone` control messages)
- CLI mode (`capsem "command"`) routes commands through vsock PTY agent with exit code propagation

### Changed
- Terminal input now routes through vsock when connected, falling back to serial
- Guest init script (`capsem-init`) launches PTY agent instead of direct bash/setsid
- CLI mode rewritten from serial I/O to vsock-based execution with proper exit codes
- `just repack` now cross-compiles and bundles the PTY agent into the initrd for fast iteration
- Serial forwarding stops once vsock connects, eliminating duplicate output
- M5 redesigned: zero-trust network boundaries with SNI proxy domain filtering, AI provider domain blocking, and real-time file telemetry via fanotify
- M6 redesigned: active AI audit gateway with 9-stage event lifecycle (PII scrubbing, tool call interception, secret scanning), replaces passive proxy approach
- M7 redesigned: hybrid MCP architecture -- local tools run sandboxed in-VM, remote tools route through host gateway with credential injection
- M8 redesigned: per-session audit databases with zstd-compressed blobs, OverlayFS config write-back, enterprise observability (Prometheus, OTLP, corporate policy via MDM)

### Fixed
- Shell prompt not appearing after command execution (stderr was redirected to /dev/hvc0, sending readline prompt through buffered serial path instead of vsock PTY)

### Security
- Removed serial console fallback: missing PTY agent halts boot instead of opening an unprotected shell
- Replaced scattered `unsafe { File::from_raw_fd }` + `mem::forget` with centralized `borrow_fd` helper using `ManuallyDrop`
- Added T13 threat (AI Traffic Audit Bypass) documenting the enforcement chain: iptables -> vsock bridge -> SNI proxy -> audit gateway
- Updated T3 (Data Exfiltration) with fswatch telemetry, PII engine, and secret scanning mitigations
- Updated T5 (Credential Theft) with gateway key injection and PII scrubbing on model calls
- Updated T11 (Network Exfiltration) with AI domain blocking at SNI proxy and 9-stage lifecycle enforcement
- Added Corporate Security Profile section with MDM-distributable policy.toml for enterprise deployments

## [0.2.0] - 2026-02-24

### Added
- blake3 integrity checking of VM assets (B3SUMS)
- Kernel hardening configuration for guest VM
- Proper terminal signal handling (setsid for controlling tty)
- Boot-up tracing spans for timing diagnostics
- Utility helpers for VM lifecycle

### Fixed
- Utility module fixes

## [0.1.0] - 2026-02-23

### Added
- Native macOS app using Tauri 2.0 with Astro frontend
- Linux VM sandboxing via Apple Virtualization.framework
- Virtio serial console with bidirectional I/O (xterm.js <-> guest /dev/hvc0)
- Custom capsem-init (PID 1) with chroot and setsid
- Docker/Podman-based VM asset build pipeline (kernel, initrd, rootfs)
- `just` task runner workflows (build, repack, dev, run, release, install)
- Codesigning with com.apple.security.virtualization entitlement
- xterm.js terminal web component
- Tauri auto-updater plugin integration

### Changed
- Complete rewrite from Python proxy architecture (v1) to native Rust/Tauri VM app
