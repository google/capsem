# H01 - Safety And Queue Contracts

## Goal

Make the KVM virtio foundation stricter before adding more machinery. Firecracker
is fast partly because it validates aggressively before raw host pointers and
queue state enter hot paths.

## Scope

- Validate full `gpa + len` ranges for every guest-memory iovec before host I/O.
- Add descriptor index, chain length, alignment, and queue-size invariants where
  Capsem is weaker than Firecracker.
- Add adversarial virtio-blk and virtqueue tests for malformed ranges and queue
  wrap behavior.
- Keep the zero-copy scatter/gather `preadv`/`pwritev` path.

## Non-Goals

- Do not turn io_uring on by default.
- Do not change rootfs format.
- Do not add product status UI in this slice.

## Done

- Bad guest descriptors fail closed without passing invalid pointers to host
  syscalls.
- Existing block performance shape is preserved unless a benchmark proves a
  deliberate tradeoff.

## Proof

- Focused virtio queue/block unit tests.
- Adversarial malformed descriptor tests.
- Focused VM smoke if touched code crosses activation/runtime paths.

