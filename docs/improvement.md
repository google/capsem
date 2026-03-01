# Performance Improvements

## Disk I/O Optimization Results
- **Scratch Disk (4K Random Write):** Improved from 1.9 MB/s to ~37 MB/s (~19x speedup, ~9,500 IOPS).
- **Scratch Disk (4K Random Read):** Improved from 1,654.0 MB/s to 7,989.1 MB/s (nearly 5x speedup, jumping from 423K IOPS to over 2.04 Million IOPS).
- **Rootfs (4K Random Read):** Improved from 4.3 MB/s to 112.2 MB/s (~26x speedup).
- **Strategy (Host):** Enabled host-level caching (`VZDiskImageCachingMode::Cached`) and disabled strict synchronization barriers (`VZDiskImageSynchronizationMode::None`) for both rootfs and ephemeral scratch disks in `crates/capsem-core/src/vm/machine.rs`.
- **Strategy (Guest):** Added `sysfs` tuning to `images/capsem-init` setting the I/O scheduler to `none` (delegating to macOS) and increasing `read_ahead_kb` to `4096` and `nr_requests` to `256` for all VirtIO block devices. Added `noatime` and `nodiratime` to ext4 mount options.

## Squashfs + Overlayfs Migration (Milestone 3)

Switched rootfs from 2GB ext4 (mounted read-only) to squashfs with zstd compression + overlayfs (tmpfs upper). Tested three squashfs configurations to find the best trade-off between image size, random I/O, and cold start latency.

### Image Size
| Config | Size | vs ext4 |
|--------|------|---------|
| ext4 (baseline) | 2,048 MB | -- |
| squashfs 128K / zstd-19 | 363 MB | -82% |
| squashfs 16K / zstd-default | 420 MB | -79% |
| **squashfs 64K / zstd-15 (chosen)** | **382 MB** | **-81%** |

### Rootfs Read I/O
| Metric | ext4 (ro) | 128K/zstd-19 | 16K/zstd-default | **64K/zstd-15** |
|--------|-----------|-------------|-----------------|----------------|
| Seq read (1MB) | N/A | 725 MB/s | 347 MB/s | **558 MB/s** |
| Rand read (4K) MB/s | 112.2 | 19.7 | 21.7 | **20.6** |
| Rand read (4K) IOPS | 28,723 | 5,040 | 5,562 | **5,284** |

Random 4K reads regressed vs ext4 because squashfs decompresses a full block for each 4K read. Smaller blocks (16K) help IOPS marginally but destroy sequential throughput. 64K is the sweet spot: sequential reads recover to 558 MB/s while random IOPS are comparable to 128K.

### CLI Cold Start Latency (mean, 3 runs)
| Command | 128K/zstd-19 | 16K/zstd-default | **64K/zstd-15** |
|---------|-------------|-----------------|----------------|
| python3 | 7.5 ms | 12.6 ms | **8.4 ms** |
| node | 131 ms | 242 ms | **136 ms** |
| claude | 289 ms | 450 ms | **290 ms** |
| codex | 134 ms | 252 ms | **130 ms** |
| gemini | 1,211 ms | 1,418 ms | **1,211 ms** |

16K blocks nearly doubled cold start times due to higher metadata overhead. 64K matches the 128K config within noise.

### Scratch Disk (unchanged, still ext4)

Scratch disk numbers vary run-to-run due to host page cache state. No meaningful change from the rootfs migration.

### Trade-off Summary

64K blocks with zstd level 15 is the chosen configuration:
- **Image size:** 382 MB (-81% vs ext4) -- only 19 MB larger than the most aggressive 128K/zstd-19 config
- **Cold starts:** Identical to 128K config (python3 8ms, node 136ms, claude 290ms)
- **Random 4K IOPS:** 5,284 (-82% vs ext4) -- the inherent squashfs cost, unavoidable at any block size
- **Sequential reads:** 558 MB/s -- good middle ground between 128K (725) and 16K (347)

The random 4K regression primarily affects `import` storms in Python/Node (many small `.py`/`.js` files). For the distribution use case (shipping a DMG), the 81% size reduction is the priority.

## Network Proxy Performance Results
- **Async Implementation:** Replaced the synchronous, thread-per-connection `net-proxy` with a Tokio-based async implementation in `capsem-agent`. This resolves potential deadlocks and improves concurrency handling.
- **Latency Analysis:** Identified that the ~180ms latency floor for Google is primarily due to geographical network distance. Verified with `elie.net` (Cloudflare) which shows a raw latency floor of ~85ms through the proxy stack.
- **Keep-alive:** Verified that HTTP keep-alive and connection reuse are working correctly across the proxy, maintaining persistent connections from guest-to-host and host-to-upstream.

## Build and Hashing
- **BLAKE3 Integration:** Switched from SHA256 to BLAKE3 for rootfs integrity checking.
- **Results:**
    - SHA256 (debug): 78,635 ms
    - BLAKE3 (opt): 41 ms
    - **Speedup:** ~1,895x faster configuration building.

## Kernel Hardening
- **Custom Kernel:** Implemented a hardened ARM64 kernel (`v6.6.127`) with `CONFIG_MODULES=n`, `CONFIG_INET=n`, and high-CVE subsystems disabled.
- **Size Reduction:** Kernel reduced from 30MB to 7MB; Initrd reduced from 30MB to 966KB.
- **Security Invariants:** KASLR, stack protection, and strict RWX enabled.
