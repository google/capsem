# H06 - Benchmark And Product Proof

## Goal

Keep the science strict and make the product surface honest.

## Scope

- Maintain one canonical `just benchmark` path.
- Commit accepted artifacts only for accepted states.
- Compare Linux and macOS with percentage deltas, host-native baselines, and
  hardware context.
- Ensure benchmark artifacts include relevant hypervisor settings:
  - CPU count, RAM, architecture, hypervisor backend;
  - block backend/engine, queue size, event-index/ioeventfd state;
  - rootfs format/compression/block size;
  - kernel cmdline and storage/FUSE limits;
  - git/source dirty state.
- Add product-facing status proof for resource counters and hypervisor health.

## Done

- A user can run status/info and understand how much CPU, memory, and I/O a VM
  is using.
- Engineers can compare benchmarks without guessing which path or hardware
  produced a number.

## Proof

- `just benchmark`
- `just benchmark-compare`
- status/info functional proof
- docs updated with artifact interpretation

