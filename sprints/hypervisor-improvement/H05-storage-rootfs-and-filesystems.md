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
- Separate EROFS DAX pmem backing experiments from virtio-blk Direct I/O:
  direct file-backed pmem is about avoiding the anonymous rootfs copy, while
  `O_DIRECT` still needs a clean revisit for writable scratch and fallback
  rootfs-over-blk behavior.
- Keep `/root` host-visible unless an explicit product replacement exists.
- Keep Apple VZ reruns part of any accepted shared/rootfs change.

## Current Evidence

- EROFS DAX works through opt-in virtio-pmem and mounts `/run/capsem-lower`
  with `dax=always`.
- File-backed DAX now works through `CAPSEM_KVM_ROOTFS_PMEM_FILE_BACKED=1` and
  the benchmark harness pads generated EROFS images to KVM's 128 MiB pmem
  alignment before mapping them directly.
- File-backed DAX is mixed, not a default yet. Against anonymous-copy DAX,
  uncompressed EROFS lost 2.9% sequential read, 3.4% random IOPS, and 5.6%
  cold large-binary throughput, but gained 5.6% lower-rootfs metadata.
- Compressed `erofs-lz4hc-c65536` file-backed DAX lost 3.1% sequential read and
  4.5% cold large-binary throughput, but gained 12.5% random IOPS and 10.1%
  small-JS reads.

## Done

- We know whether slow rootfs reads, metadata IOPS, small JS reads, and large
  binary launch are block backend issues, filesystem format issues, compression
  issues, or VirtioFS/workspace issues.

## Proof

- `capsem-bench rootfs`
- `capsem-bench storage`
- `just benchmark`
- macOS artifact rerun for accepted shared/rootfs changes
