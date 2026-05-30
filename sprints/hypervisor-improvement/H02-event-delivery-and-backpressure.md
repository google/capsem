# H02 - Event Delivery And Backpressure

## Goal

Move Capsem block/device scheduling to a coherent Firecracker-like async
engine shape, then measure the whole shape before ablation. The mistake to avoid
is optimizing one tiny switch at a time and rejecting pieces that only pay off
when the full event/backpressure/io_uring design is present.

## Scope

- Define the block async engine as a complete profile:
  - ioeventfd wake into a dedicated iothread;
  - io_uring for both read-only rootfs and writable overlay/block devices;
  - fixed registered backing fd for submitted I/O;
  - kernel opcode probing before enabling the engine;
  - ring restrictions while created disabled, then explicit enable;
  - queue-full backpressure with `undo_pop()` and completion-triggered retry;
  - deferred used-ring publication and event-index interrupt decisions;
  - quiesce drains both submitted and completed work before checkpoint.
- Keep heavy file/FUSE work off the vCPU thread.
- Extend `KVM_IOEVENTFD` only where device semantics are understood.
- Use event-index and deferred used-ring batching beyond block only after the
  full block profile has a benchmarked result.
- Add metrics for engine selection, queue-full, deferred work, wake counts,
  completion counts, fixed-fd/probe failures, and interrupt suppression.
- After the full profile benchmark, run ablation in meaningful groups:
  sync vs async engine, fixed fd/restrictions on/off, queue depth, and
  completion retry/backpressure behavior.

## Risks

- `ioeventfd` bypasses MMIO write side effects. Each device needs a precise
  wake path before conversion.
- event-index bugs can wedge queues. Every device conversion needs race tests.
- A central loop can regress latency if blocking operations run in it.

## Done

- KVM block has one coherent async profile that matches the relevant
  Firecracker shape instead of a collection of tiny toggles.
- The full profile has a canonical benchmark result with deltas against the
  accepted Linux baseline.
- Any ablation is done after that full-profile benchmark and is recorded as
  grouped evidence, not a replacement for the coherent profile.
- Any new device using event-index/ioeventfd has functional and adversarial
  proof.

## Proof

- Queue race tests.
- io_uring full-queue tests.
- VM smoke and focused benchmarks for each converted device path.

## Progress

- First slice complete: KVM virtio-blk io_uring submission-queue saturation no
  longer falls back to synchronous I/O. The worker rewinds the popped
  descriptor with `VirtQueue::undo_pop()`, records
  `async_queue_full_total`, leaves the request uncompleted, and retries it on a
  later drain. The counter flows through VM metrics, `capsem info`, and the
  OTel metric-point contract.
- Second slice complete: io_uring completions now immediately retry a
  backpressured descriptor when capacity is freed. The completion branch reaps
  completions, then performs a completion-triggered queue drain so a descriptor
  rewound by SQ-full backpressure does not wait for another guest notify.
- Direction change, 2026-05-30: stop evaluating tiny isolated changes as the
  main landing strategy. Next implementation is the full block async engine
  profile: default async block engine, fixed registered fd, opcode probe,
  ring restrictions, existing backpressure/completion retry, then benchmark the
  profile as a whole before ablation.
- Full KVM block async profile implemented locally: io_uring is selected for
  read-only and writable block devices unless `CAPSEM_KVM_BLK_IO_URING=sync`
  is set; the ring is created disabled, probed for readv/writev support,
  restricted to fixed-fd readv/writev SQEs, enabled explicitly, and kicked once
  per queue drain or completion retry batch.
- First full-profile measurements are mixed. Against same-run sync, the 128-SQE
  full profile is +8.0% on cold large-binary reads, +2.7% on small-JS reads,
  +4.2% on metadata stats, +4.2% on node startup, but -8.2% on random rootfs
  reads and -9.6% on codex startup. A 256-SQE ablation did not justify keeping
  the larger ring: it helped random reads (+3.4% vs 128) and metadata (+4.5%)
  but regressed cold binary reads (-5.6%) and small-JS reads (-5.7%).
