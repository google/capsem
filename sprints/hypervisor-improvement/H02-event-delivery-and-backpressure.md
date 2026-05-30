# H02 - Event Delivery And Backpressure

## Goal

Move Capsem device scheduling toward a coherent Firecracker-like event shape
without collapsing all blocking work onto one loop.

## Scope

- Define a small shared worker/event-loop pattern for KVM device workers.
- Keep heavy file/FUSE work off the vCPU thread.
- Extend `KVM_IOEVENTFD` only where device semantics are understood.
- Use event-index and deferred used-ring batching beyond block where safe.
- Replace io_uring sync fallback on queue saturation with explicit backpressure
  or `undo_pop`-style retry semantics.
- Add metrics for queue-full, deferred work, wake counts, and interrupt
  suppression.

## Risks

- `ioeventfd` bypasses MMIO write side effects. Each device needs a precise
  wake path before conversion.
- event-index bugs can wedge queues. Every device conversion needs race tests.
- A central loop can regress latency if blocking operations run in it.

## Done

- Block async saturation behavior is explicit and test-covered.
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
