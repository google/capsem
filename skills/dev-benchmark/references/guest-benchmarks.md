# Guest benchmark reference

## Categories and authority

| Category | Command | Measures |
|---|---|---|
| disk | `capsem-bench disk` | Sequential/random scratch I/O and IOPS |
| rootfs | `capsem-bench rootfs` | Large/small reads, random 4K, and stat walks |
| storage | `capsem-bench storage` | Rootfs plus `/root`, tmp, log, and run paths |
| startup | `capsem-bench startup` | Python, Node, Claude, Gemini, Codex startup |
| protocol | `capsem-bench-rs protocol` | HTTP/model/SSE/credential/MCP/DNS scenarios |
| snapshot | `capsem-bench snapshot` | MCP create/list/changes/revert/delete |
| all | `capsem-bench all` | Canonical suite; merges Rust protocol results |

Hot protocol release evidence requires paired `host_direct` and
`guest_capsem` lanes against `capsem-mock-server`. Report RPS and throughput
ratios, p50/p95/p99 deltas, and error delta. A single lane is diagnostic only.
The Python wrapper may orchestrate but never generate protocol numbers.

Intentional host evidence lives under
`benchmarks/baselines/{lifecycle,fork,route-latency}`.
Historical host-native evidence includes filesystem and machine metadata under
`benchmarks/baselines/host-native`; use `target/host-native-benchmark`, not `/tmp`, unless
`CAPSEM_HOST_NATIVE_BENCH_DIR` explicitly selects another disk. There is no Just
history command: run the owner, review JSON, and commit a new file without
overwriting prior evidence. Compare Linux x86_64 and macOS arm64 only after the
same owner reruns both; `build_system/scripts/build/benchmark_report.py` validates/renders them.

## Snapshot diagnosis

Snapshot covers 10/100/500-file workspaces through guest CLI → MCP → vsock →
gateway → filesystem. Run `just exec "capsem-bench snapshot"`; add
`RUST_LOG=capsem=debug` and inspect `clone_ws_ms`, `clone_sys_ms`, and `hash_ms`.
Use `build_system/scripts/doctor/check_session.py` or query `tool_calls` in the session database for
per-operation durations. High clone time means workspace size or CoW fallback;
high hash time means walk overhead; slow compact means overlapping snapshots.

## Storage and rootfs diagnosis

- Run `disk` for sequential throughput and confirm VirtioFS/block mode.
- Run `storage`; compare `/root` with tmp/log/run, inspect mount attribution,
  kernel queue/backpressure fields, EROFS mounts, detailed 4K/64K/1M sequential
  and random IOPS, and p95 sync-write latency. SquashFS is historical only.
- Run `rootfs`; compare `seq_read`, `large_binary_seq_read`, `small_js_read`,
  `metadata_stat`, and the broad `rand_read_4k` signal.

`capsem-bench all` must keep storage attribution so Linux and macOS artifacts
both identify rootfs/workspace/tmpfs costs; only long load diagnostics are
opt-in. Structured output is `/tmp/capsem-benchmark.json` and includes version,
timestamp, host, per-category results, and size-scoped snapshot results.

## Environment

- `CAPSEM_BENCH_DIR` (default `/root`), `CAPSEM_BENCH_SIZE_MB` (256)
- `CAPSEM_STORAGE_BENCH_PATHS` (default `/root:/tmp:/var/tmp:/var/log:/run`)
- `CAPSEM_STORAGE_BENCH_SIZE_MB` and `CAPSEM_STORAGE_IO_PROFILE_SIZE_MB` (64)
- `CAPSEM_STORAGE_IO_PROFILE_RANDOM_OPS` (2000)

## Adding a guest benchmark

Add a lazy module under `guest/artifacts/capsem_bench/`, return a dict and Rich
table, register the mode in `VALID_MODES`, include it in `all` when canonical,
and update this reference plus the benchmark documentation.
