# Hotspot Report

## Artifacts

- VM/MITM network matrix:
  `benchmarks/mitm-local/data_1.0.1780763638_arm64.json`
- Host direct control baseline:
  `benchmarks/mitm-local/control_host_direct_1.0.1780763638_arm64.json`
- Lifecycle timing:
  `benchmarks/lifecycle/data_1.0.1780763638.json`
- DB writer pressure:
  `benchmarks/db-writer/data_1.0.1780763638_arm64.json`

## VM/MITM Matrix

10 requests, concurrency 1, through guest -> net-proxy -> vsock -> MITM ->
local mock server. The gated test also queried `session.db` before teardown
and proved expected paths, WebSocket `101`, all `allowed`, and no raw
`capsem_test_` marker in audited text columns.

| Case | p50 | p95 | p99 | Rate |
| --- | ---: | ---: | ---: | ---: |
| tiny HTTP | 1.5 ms | 3.3 ms | 4.3 ms | 541.3 rps |
| 1 MB HTTP | 14.6 ms | 15.9 ms | 16.1 ms | 68.5 rps / 71.8 MB/s |
| gzip 1 MB | 34.7 ms | 37.7 ms | 37.9 ms | 28.6 rps / 29.9 MB/s |
| SSE model stream | 1.4 ms | 2.6 ms | 2.7 ms | 576.8 rps |
| denied-target fixture | 1.3 ms | 2.0 ms | 2.2 ms | 677.0 rps |
| credential response | 1.2 ms | 2.1 ms | 2.1 ms | 699.5 rps |
| WebSocket echo | 0.2 ms | 0.2 ms | 0.2 ms | 2456.0 fps |
| WebSocket close | 1.7 ms | 1.7 ms | 1.7 ms | 528.5 fps |

## DB Writer

| Burst | p50 | p95 | p99 | Mean | Throughput |
| --- | ---: | ---: | ---: | ---: | ---: |
| 128 events | 1.5188 ms | 1.5538 ms | 1.5588 ms | 1.5250 ms | 83.934K events/s |
| 1024 events | 6.8931 ms | 7.0277 ms | 7.0382 ms | 6.9160 ms | 148.063K events/s |
| 4096 events | 27.0200 ms | 27.8743 ms | 28.0951 ms | 27.1623 ms | 150.797K events/s |

The logger-owned writer is not the release bottleneck at this scale. Keep the
single-writer design.

## Launch

3 runs from `test_lifecycle_benchmark`.

| Operation | p50 | p95 | p99 |
| --- | ---: | ---: | ---: |
| provision | 973.2 ms | 982.1 ms | 982.9 ms |
| exec ready | 11.6 ms | 11.6 ms | 11.6 ms |
| exec | 11.3 ms | 11.4 ms | 11.4 ms |
| delete | 60.0 ms | 61.0 ms | 61.1 ms |
| total | 1057.0 ms | 1065.1 ms | 1065.8 ms |

## Findings

- The first real VM benchmark exposed three correctness gaps before it produced
  trustworthy numbers: stale initrd, dynamic local debug ports not represented
  in policy, and WebSocket client proxy semantics. All three are now fixed in
  the benchmark rail.
- HTTP/SSE/credential small responses are low single-digit milliseconds through
  the full VM/MITM path.
- 1 MB plain HTTP is roughly 16 ms p99, which is acceptable for the current
  release.
- 1 MB gzip is the slowest measured network case at roughly 38 ms p99. This is
  a candidate for a small follow-up only if gzip-heavy workloads matter.
- WebSocket relay is fast after the upgrade path is established.
- Launch is just over 1 second p99 for provision -> ready -> exec -> delete in
  this local run.

## Recommendation

Do not start a broad speed sprint for 1.3. The measured bottlenecks do not
justify destabilizing the security engine or DB writer before release.

Recommended follow-up is narrow:

- If gzip-heavy model/provider traffic becomes important, profile the gzip body
  path specifically before changing decompression behavior.
- Keep Linux/KVM/EROFS/DAX validation with the Linux team; this Mac run only
  proves the Apple VZ path and local network security rail.
- Add live debug metric export only in the planned local OTEL/debug endpoint
  sprint. Do not route these counters through `/status`.
