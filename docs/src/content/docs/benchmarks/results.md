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

Capsem 1.3 uses EROFS `lz4hc` level `12` as the release rootfs asset. The
squashfs row below is historical comparison data only, not a release fallback.

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

## Local Network And Model Fixtures

Release network proof uses the shared `mock_server`, not public internet. The
current VM artifact is
`benchmarks/capsem-bench/data_1.3.1781205836_arm64.json` and was recorded
through the profile-selected VM path against local HTTP, JSON model,
credential-shaped, and WebSocket control fixtures.

| Scenario | Success | Requests/sec | p50 | p99 |
|---|---:|---:|---:|---:|
| HTTP tiny response | 50/50 | 1,886.9 | 1.9ms | 8.3ms |
| JSON model response | 1,000/1,000 | 2,810.4 | 8.8ms | 27.5ms |
| credential-shaped response | 1,000/1,000 | 1,524.9 | 11.0ms | 64.9ms |

WebSocket control fixture: echo `10` frames at `1,454.6` frames/sec with
`0.2ms` p50 and `2.6ms` p99 latency; close control frame completed in `5.9ms`
p50/p99.

Historical release-scale local fixture artifact:
`benchmarks/mock-server-protocol/data_1.3.1781205836_arm64.json`.

| Scenario | Success | Requests/sec | p50 | p99 |
|---|---:|---:|---:|---:|
| JSON model response | 50,000/50,000 | 3,000.9 | 18.8ms | 58.0ms |
| credential-shaped response | 50,000/50,000 | 3,029.0 | 18.8ms | 55.9ms |

The full protocol fixture corpus is still exercised by doctor and unit
contract tests; the release-scale benchmark intentionally selects
`model_json_response,credential_response` so it measures hot model/credential
traffic without turning the 1 MiB body fixtures into a 100+ GiB transfer.

Host-direct control smoke after adding the JSON model fixture proved only that
`/model/response` is routable and returns model-shaped JSON. Do not use its
localhost latency or requests/sec as release performance evidence; the release
gate must rerun `capsem-bench protocol` with `CAPSEM_MOCK_SERVER_BASE_URL`
from inside a profile-selected VM so the request crosses guest redirect, vsock,
MITM parsing, CEL/security evaluation, logging, and the local mock server.

Corrected host-direct calibration with meaningful sample size:
`50,000` requests per selected scenario at concurrency `64` completed with zero
errors. `model_json_response`: `4,321.8` requests/sec, `13.9ms` p50,
`30.7ms` p99. `credential_response`: `4,361.8` requests/sec, `13.8ms` p50,
`30.2ms` p99, and the JSON artifact confirmed no raw synthetic credential was
stored. This remains a host-control fixture only, archived as
`benchmarks/mock-server-protocol/control_host_direct_c64_model_credential_1.0.1780954707_arm64.json`.

## DNS Load

DNS release proof must run `capsem-bench dns-load` inside a VM so traffic goes
through the guest redirect, DNS proxy, host DNS handler, and
`SecurityRuleSet`. Current baseline artifact:

| Concurrency | Requests/sec | p50 | p99 | Errors |
|---:|---:|---:|---:|---:|
| 1 | 3,556.5 | 0.264ms | 0.497ms | 0 |
| 10 | 12,928.5 | 0.744ms | 1.142ms | 0 |
| 50 | 12,425.0 | 3.971ms | 4.915ms | 0 |
| 200 | 11,482.1 | 16.464ms | 26.734ms | 0 |

Focused VM-path `c=64` check from this release branch:
`CAPSEM_BENCH_CONCURRENCY=64 CAPSEM_BENCH_DURATION_S=5 capsem-bench dns-load`
completed `21,669` DNS requests in 5s, `4,333.8` requests/sec, `13.13ms` p50,
`33.82ms` p99, `0` errors, decision distribution `allowed=21669`.

## MCP Load

Focused VM-path `c=64` check from this release branch:
`CAPSEM_BENCH_CONCURRENCY=64 CAPSEM_BENCH_DURATION_S=5 capsem-bench mcp-load`
completed `37,775` `local__echo` calls in 5s, `7,555.0` requests/sec,
`7.52ms` p50, `20.92ms` p99, `24.66ms` p999, `0` errors.

MCP brokered OAuth credential resolution is measured in
`cargo bench -p capsem-core --bench security_actions` as
`mcp_brokered_oauth_resolve`: `10.10µs` median with the brokered secret stored
behind a `credential:blake3` reference.

## VM Lifecycle

Host-side latency for individual VM operations. Measured over 3
provision/exec/delete cycles on the same service instance.

| Operation | Min | Mean | Max | Description |
|-----------|----:|-----:|----:|-------------|
| provision | 1,032.6ms | 1,034.3ms | 1,035.9ms | Create and boot a temporary VM |
| exec_ready | 12.6ms | 12.8ms | 13.0ms | First ready check after provisioning |
| exec | 10.3ms | 11.5ms | 12.3ms | Simple `echo ok` on running VM |
| delete | 59.5ms | 60.8ms | 62.0ms | VM teardown request |
| total | 1,115.1ms | 1,119.4ms | 1,121.8ms | Full lifecycle loop |

Run:

```bash
uv run pytest tests/capsem-serial/test_lifecycle_benchmark.py::test_lifecycle_benchmark -xvs
```

## Fork

Host-side latency for fork and boot-from-image over 3 cycles.

| Metric | Min | Mean | Max | Gate | Description |
|--------|----:|-----:|----:|-----:|-------------|
| fork | 38.0ms | 40.5ms | 43.3ms | 500ms | APFS clonefile of rootfs overlay and workspace |
| image_size | 11.8MB | 11.8MB | 11.8MB | 12MB | Actual allocated blocks |
| boot_provision | 930.6ms | 948.6ms | 983.8ms | 1,200ms | Clone image into new session and boot |
| boot_ready | 12.3ms | 12.6ms | 13.1ms | 1,200ms | First ready check after provisioning |

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
