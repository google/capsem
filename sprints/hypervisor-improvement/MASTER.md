# Hypervisor Improvement Meta Sprint

Last updated: 2026-06-02

## Mission

Turn the Firecracker source audit into a durable Capsem hypervisor improvement
program. The goal is not to copy Firecracker blindly. The goal is to adopt the
parts of its shape that make a VMM reliable and fast over years:

- explicit CPU and device lifecycle control;
- validated guest-memory and virtqueue contracts;
- coherent event delivery and backpressure;
- storage/rootfs choices measured across Linux and Apple;
- first-class user-visible status, resource usage, metrics, and OpenTelemetry;
- benchmark artifacts that explain both speed and hardware context.

This sprint owns Linux/KVM implementation. Shared benchmark, rootfs, telemetry,
status, and product-surface changes must remain useful to Apple VZ and future
Android/ARM work.

## Fundamental Advance Rule

Benchmarks are proof gates, not the sprint engine. The next performance work
must start from source-level understanding and architectural hypotheses:

- trace the full lifecycle in Capsem and at least one proven VMM before
  changing code;
- make one coherent implementation slice at a time, not isolated knob tweaks;
- prefer changes that improve the core VMM shape even before a microbenchmark
  exposes the full value;
- run focused smoke measurements only to catch obvious regressions during
  development;
- reserve full `just benchmark` for accepted milestones and cross-platform
  comparison.

If a task cannot name the mechanism it is improving, it is not ready for more
benchmark time.

## Firecracker Lessons

Source basis: `private/firecracker` at
`c1eab585c9a9db6463ae29c9f6c5cee5155f03ce`.

Firecracker's relevant strengths:

- vCPU threads start through a paused state machine, use a startup barrier, and
  can be controlled with channel events plus `KVM_RUN.immediate_exit`.
- devices are driven by a central epoll/event-manager surface instead of ad hoc
  one-off loops;
- virtio queues use event-index notification suppression, deferred used-ring
  publication, and explicit interrupt decisions;
- queue notifications use `KVM_IOEVENTFD` broadly;
- block I/O has an explicit sync/async engine contract, fixed-fd io_uring,
  opcode restrictions, kernel feature probing, completion eventfds, and
  backpressure when queues are full;
- request parsing and guest-memory access are validated aggressively before
  host syscalls receive raw pointers;
- runtime configuration exposes performance-relevant choices such as block
  engine, cache mode, and rate limiters;
- metrics exist close to hot paths, so performance behavior can be explained.

Capsem strengths we should preserve:

- product storage model: immutable squashfs rootfs, ext4 system overlay,
  host-visible workspace through VirtioFS, snapshots, MCP file APIs;
- cross-platform benchmark path through `just benchmark`;
- KVM block already has vectored zero-copy `preadv`/`pwritev`, ioeventfd worker,
  used-ring batching, event-index support, and OTel-ready block counters;
- Apple VZ parity matters for rootfs, benchmark, telemetry, and product
  surfaces even when Linux/KVM uses deeper low-level primitives.

## Execution Order

| # | Sub-sprint | Status | Purpose | Depends On |
| --- | --- | --- | --- | --- |
| H00 | [Reality And Wrap-Up](H00-reality-and-wrap-up.md) | Done | Close current KVM/block context, preserve benchmark truth, identify the 2-3 pre-flight items before deeper work. | none |
| H01 | [Safety And Queue Contracts](H01-safety-and-queue-contracts.md) | Done | Fix guest-memory range validation, descriptor validation, queue invariants, and adversarial tests. | H00 |
| H02 | [Event Delivery And Backpressure](H02-event-delivery-and-backpressure.md) | Not Started | Generalize worker/event-loop patterns, widen ioeventfd/event_idx where safe, add queue-full backpressure. | H01 |
| H03 | [Observability Status And OTel](H03-observability-status-and-otel.md) | Active | Make CPU, memory, IO, queue, backend, and lifecycle counters visible in status and exportable to OTel. | H00 |
| H04 | [CPU SMP And Lifecycle](H04-cpu-smp-and-lifecycle.md) | Not Started | Improve vCPU lifecycle, `immediate_exit`, SMP topology, exit metrics, timer confidence, and pause/resume control. | H01, H03 |
| H05 | [Storage Rootfs And Filesystems](H05-storage-rootfs-and-filesystems.md) | Not Started | Compare rootfs formats/chunks/compression/cache policies and preserve Apple/Linux product semantics. | H03 |
| H06 | [Benchmark And Product Proof](H06-benchmark-and-product-proof.md) | Not Started | Keep performance science strict: artifacts, host-native baselines, status visibility, doctor gates, macOS reruns. | H01-H05 |
| H07 | [Docs Changelog And Release Gate](H07-docs-changelog-and-release-gate.md) | Not Started | Update docs, skills, bootstrap/doctor expectations, changelog, and final validation. | H06 |
| H08 | [Disk Throughput Attribution](H08-disk-throughput-attribution.md) | Active | Trace DAX, virtio-blk, scratch, VirtioFS, and RPS-adjacent I/O before landing code-path speedups. | H03, H05 |
| H09 | [Network And RPS Attribution](H09-network-rps-attribution.md) | Not Started | Split guest network, vsock, MITM, DNS, security-engine, endpoint-latency, TUI/status polling, and workspace/disk effects before landing RPS speedups. | H03, H06, H08 |

## Global Acceptance Gates

- Every functional milestone updates this sprint tracker and `CHANGELOG.md`.
- Every performance claim cites committed `just benchmark` artifacts and
  percentage deltas against the previous accepted artifact.
- Every performance implementation cites the source-path mechanism it changes
  before citing benchmark output.
- Every telemetry/status claim proves that a real session exposes the counter or
  resource value through the user-facing path and the OTel-ready metrics path.
- Every KVM-only change compiles cleanly on non-Linux or is cfg-isolated.
- Every shared rootfs/benchmark/status change is suitable for Apple VZ rerun.
- Hot-path metrics use stable, low-cardinality names.
- Safety fixes include adversarial tests, not just happy-path benchmarks.
- Disk throughput work starts from code-path attribution. DAX rootfs, fallback
  virtio-blk rootfs, writable scratch, VirtioFS, and RPS-adjacent paths must be
  measured as different lanes before conclusions are generalized.
- Network/RPS work starts from attribution too. Guest network, vsock bridge,
  MITM/proxy, DNS, security engine, service/gateway endpoints, TUI/status
  polling, and workspace/disk dependencies must be measured as separate lanes.

## Proof Matrix

- Unit/contract:
  - `cargo test -p capsem-core hypervisor::kvm --lib`
  - focused queue/block/vCPU/status tests for each modified module
- Functional:
  - `just run "echo ok"`
  - `just run "capsem-doctor"`
  - status/info command that shows new resource and hypervisor counters
- Adversarial:
  - malformed descriptors, bad GPA ranges, queue wrap, event-index races,
    io_uring queue-full, quiesce with pending work, pause/stop while blocked in
    `KVM_RUN`
- E2E/VM:
  - full `capsem-doctor` after device/lifecycle changes
  - suspend/resume persistence checks when lifecycle changes
- Telemetry:
  - inspect metrics recorder output in tests
  - inspect real session/status output after VM run
  - OTel/export path where the exporter exists, or explicit no-op recorder
    proof with stable metric names when exporter wiring is a later slice
- Performance:
  - `just benchmark`
  - `just benchmark-compare`
  - focused `capsem-bench disk`, `rootfs`, `storage`, and `startup` during
    experiments, with accepted results committed only through canonical
    artifacts

## Pre-Flight Wrap-Up Candidates

Before starting H01-H07 implementation, close these:

1. Reconcile the existing `virtio-block-firecracker-path` sprint with the final
   default-off io_uring decision and benchmark numbers.
2. Keep current KVM doctor/test status explicit so the next sprint starts from a
   known-green or known-red baseline.
3. Decide the first execution slice: safety/range validation first, or
   observability/status first. The recommended order is H01 then H03 because
   unsafe counters over weak contracts can make bad states look official.
