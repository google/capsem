---
title: Performance Results
description: Reference benchmark results for Capsem VM boot time, disk I/O, CLI startup, network, and snapshot operations.
sidebar:
  order: 1
---

Reference results from the latest 1.3 benchmark ledgers. Numbers vary with host
load, cache state, architecture, and network path. Before cutting a release,
rerun the benchmark gates and commit the updated `benchmarks/**/data_*.json`
artifacts.

## 1.3 Rootfs Decision

Capsem 1.3 uses EROFS as the primary rootfs asset and keeps squashfs as a
legacy fallback. The release default is EROFS `lz4hc` level `12`.

| Lane | Rootfs size | Fresh run | Sequential rootfs read | Random rootfs read | `node --version` | `codex --version` |
|---|---:|---:|---:|---:|---:|---:|
| squashfs zstd | 458.5 MiB | 9.10s | 599.3 MB/s | 7,757 IOPS | 130.6ms | 305.2ms |
| EROFS zstd-15 | 562.7 MiB | 6.58s | 1,567.2 MB/s | 19,857 IOPS | 36.4ms | 131.7ms |
| EROFS lz4hc-12 | 720.5 MiB | 6.05s | 4,316.7 MB/s | 28,235 IOPS | 18.5ms | 78.1ms |

Zstd was tested on macOS and Linux and was not worth it for this release's
speed-first workload. It remains an experimental build option for future size
or distribution experiments; it is not the default.

## Mac DAX Probe

Linux/KVM DAX remains valuable for the Linux lane. On macOS/VZ, the EROFS DAX
probe currently fails over the existing virtio-blk path with `dax options not
supported`, so Mac keeps non-DAX EROFS `lz4hc` level `12`.

| Lane | Fresh run | Sequential rootfs read | `codex --version` |
|---|---:|---:|---:|
| EROFS lz4hc-12 non-DAX | 6.00s | 4,117.1 MB/s | 77.8ms |
| EROFS lz4hc-12 DAX probe | mount rejected | n/a | n/a |

## Boot Time

The diagnostic suite enforces boot time below 1 second for the core guest boot
path. The heavier end-to-end benchmark rows above include release assets and
CLI startup checks, so use them for rootfs comparisons and use doctor output
for boot-regression gates.

Historically, the two heaviest boot stages were network rule setup and Python
virtualenv creation. The 1.3 network lane moved NAT setup to `iptables-nft`; a
fresh network benchmark must be rerun on the final nft lane before publishing
network-grade numbers.

## Disk I/O

Scratch disk performance on the VirtioFS-backed workspace from the previous
host benchmark artifact:

| Test | Throughput | IOPS | Duration |
|------|-----------:|-----:|---------:|
| Sequential write (1MB blocks) | 1,854 MB/s | - | 138ms |
| Sequential read (1MB blocks) | 3,754 MB/s | - | 68ms |
| Random 4K write (fdatasync) | 33 MB/s | 8,353 | 1,197ms |
| Random 4K read | 279 MB/s | 71,440 | 140ms |

Sequential I/O benefits from VirtioFS pass-through to APFS. Random write IOPS
are limited by per-write `fdatasync`, which reflects worst-case
database-style writes.

## VM Lifecycle

Host-side latency for individual VM operations. Measured over 3
provision/exec/delete cycles on the same service instance.

| Operation | Min | Mean | Max | Description |
|-----------|----:|-----:|----:|-------------|
| provision | 895ms | 931ms | 951ms | Create and boot a temporary VM |
| exec_ready | 11.5ms | 12.1ms | 12.9ms | First ready check after provisioning |
| exec | 10.7ms | 10.9ms | 11.3ms | Simple `echo ok` on running VM |
| delete | 60.1ms | 60.6ms | 61.5ms | VM teardown request |
| total | 980ms | 1,015ms | 1,033ms | Full lifecycle loop |

Run:

```bash
uv run pytest tests/capsem-serial/test_lifecycle_benchmark.py::test_lifecycle_benchmark -xvs
```

## Fork

Host-side latency for fork and boot-from-image over 3 cycles.

| Metric | Min | Mean | Max | Gate | Description |
|--------|----:|-----:|----:|-----:|-------------|
| fork | 83ms | 88ms | 93ms | 500ms | APFS clonefile of rootfs overlay and workspace |
| image_size | 7.5MB | 7.5MB | 7.5MB | 12MB | Actual allocated blocks |
| boot_provision | 744ms | 747ms | 752ms | 1,200ms | Clone image into new session and boot |
| boot_ready | 11ms | 11ms | 12ms | 1,200ms | First ready check after provisioning |

Run:

```bash
uv run pytest tests/capsem-serial/test_lifecycle_benchmark.py::test_fork_benchmark -xvs
```

## Reproducing

```bash
# Generate benchmarks/fork/data_{version}.json and lifecycle data.
uv run pytest tests/capsem-serial/test_lifecycle_benchmark.py -xvs

# Run guest benchmarks.
just bench
```

The guest benchmark writes JSON output to `/tmp/capsem-benchmark.json` inside
the VM. Release prep must copy current benchmark evidence into the docs page
and commit versioned benchmark artifacts before tagging.
