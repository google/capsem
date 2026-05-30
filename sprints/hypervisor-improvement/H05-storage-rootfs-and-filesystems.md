# H05 - Storage Rootfs And Filesystems

## Goal

Close Linux/macOS storage gaps without breaking Capsem's product storage model.

## Scope

- Benchmark rootfs alternatives:
  - current squashfs zstd level/block size;
  - smaller/larger squashfs block sizes;
  - lower zstd levels;
  - lz4/uncompressed if supported by the build and kernel;
  - read-only ext4 image as a comparison baseline.
- Record rootfs format, compression, block size, and host filesystem context in
  artifacts.
- Evaluate cache/flush policy knobs for rootfs and system overlay.
- Keep `/root` host-visible unless an explicit product replacement exists.
- Keep Apple VZ reruns part of any accepted shared/rootfs change.

## Done

- We know whether slow rootfs reads, metadata IOPS, small JS reads, and large
  binary launch are block backend issues, filesystem format issues, compression
  issues, or VirtioFS/workspace issues.

## Proof

- `capsem-bench rootfs`
- `capsem-bench storage`
- `just benchmark`
- macOS artifact rerun for accepted shared/rootfs changes

