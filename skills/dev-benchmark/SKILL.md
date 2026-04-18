---
name: dev-benchmark
description: Capsem benchmarking with capsem-bench. Use when running benchmarks, adding new benchmark categories, interpreting results, or investigating performance regressions. Covers all 7 benchmark categories (disk, rootfs, startup, http, throughput, snapshot, all), the JSON output format, and how to add new benchmarks.
---

# Benchmarking

## Quick start

```bash
just bench                          # Run all benchmarks in VM (~2 min)
just run "capsem-bench snapshot"    # Snapshot benchmarks only
just run "capsem-bench disk"        # Disk I/O only
just test                           # Full validation including benchmarks
```

## capsem-bench

Python tool that runs inside the VM. Rich tables to stderr (human), structured JSON saved to `/tmp/capsem-benchmark.json` (machine).

**Location:** `guest/artifacts/capsem_bench/` (Python package, invoked via `capsem-bench` shell wrapper)

### Benchmark categories

| Category | Command | What it measures |
|----------|---------|-----------------|
| disk | `capsem-bench disk` | Sequential/random I/O on scratch disk (write/read throughput, IOPS) |
| rootfs | `capsem-bench rootfs` | Read-only rootfs performance (sequential + random 4K reads) |
| startup | `capsem-bench startup` | Cold-start latency for python3, node, claude, gemini, codex |
| http | `capsem-bench http [URL] [N] [C]` | HTTP throughput through MITM proxy (requests/sec, latency percentiles) |
| throughput | `capsem-bench throughput` | 100MB download through MITM proxy (end-to-end MB/s) |
| snapshot | `capsem-bench snapshot` | Snapshot create/list/changes/revert/delete via MCP (ms per op at 10/100/500 files) |
| all | `capsem-bench` | All of the above |

### Snapshot benchmarks

Tests the full MCP snapshot pipeline end-to-end (guest CLI -> MCP server -> vsock -> host gateway -> filesystem). Measures at 3 workspace sizes (10, 100, 500 files):

- **create**: Populate workspace, create named snapshot via MCP
- **list**: List all snapshots with change diffs
- **changes**: List changed files since checkpoint
- **revert**: Revert a single file from snapshot
- **delete**: Delete the snapshot

Key metrics: per-operation latency in ms. Regressions in `create` usually mean the clone or hash stage got slower. Use `RUST_LOG=capsem=debug` to see per-stage breakdown (clone_ws_ms, clone_sys_ms, hash_ms).

### JSON output format

```json
{
  "version": "0.3.0",
  "timestamp": 1711561234.5,
  "hostname": "capsem",
  "disk": { "seq_write_mbps": 450, ... },
  "rootfs": { ... },
  "startup": { "python3": { "min_ms": 45, "mean_ms": 48, "max_ms": 52 }, ... },
  "http": { "rps": 120, "p50_ms": 42, ... },
  "throughput": { "throughput_mbps": 85, ... },
  "snapshot": {
    "10_files": { "create_ms": 120, "list_ms": 50, ... },
    "100_files": { "create_ms": 250, ... },
    "500_files": { "create_ms": 800, ... }
  }
}
```

### Environment variables

- `CAPSEM_BENCH_DIR`: Test directory for disk benchmarks (default: `/root`)
- `CAPSEM_BENCH_SIZE_MB`: Write test size in MB (default: 256)

## Investigating slowness

### Snapshot performance

1. Run snapshot benchmark: `just run "capsem-bench snapshot"`
2. Check per-stage timing: `RUST_LOG=capsem=debug just run "capsem-bench snapshot"` -- look for `snapshot_into_slot timing` log lines showing `clone_ws_ms`, `clone_sys_ms`, `hash_ms`
3. Check session data: `just inspect-session` -- MCP tool usage section shows avg duration per snapshot operation
4. Query detailed durations: `just query-session "SELECT tool_name, duration_ms FROM mcp_calls WHERE tool_name LIKE 'snapshot%' ORDER BY duration_ms DESC LIMIT 20"`

Common causes:
- **clone_ws_ms high**: Large workspace, or APFS clonefile falling back to byte copy
- **hash_ms high**: Many files in workspace (walkdir overhead), or slow filesystem
- **compact slow**: Merging many snapshots with overlapping files

### Disk I/O regression

1. Run: `just run "capsem-bench disk"`
2. Compare sequential write/read throughput against baseline
3. Check if VirtioFS mode changed (block mode has different I/O characteristics)

### Adding a new benchmark

1. Create a new module in `guest/artifacts/capsem_bench/` (e.g., `mytest.py`) with a `mytest_bench()` function that returns a dict and prints a Rich table
2. Add the mode name to `VALID_MODES` in `__main__.py`
3. Wire it into `main()` with the `if mode in ("name", "all"):` pattern (lazy import)
4. Update this skill and the benchmarking doc page

## Host-side lifecycle benchmark

Profiles individual VM lifecycle operations from the host. Runs outside the guest via pytest, not via `capsem-bench`.

```bash
uv run pytest tests/capsem-serial/test_lifecycle_benchmark.py -xvs
```

**Location:** `tests/capsem-serial/test_lifecycle_benchmark.py`

### Operations measured

| Operation | What it times |
|-----------|--------------|
| provision | HTTP POST `/provision` to service (VM creation + process spawn) |
| exec_ready | First `echo ready` exec succeeds (VM boot + vsock handshake) |
| exec | Simple `echo ok` on a running VM |
| delete | HTTP DELETE `/delete/{name}` (VM teardown + cleanup) |

### Output

- Per-run breakdown printed to stdout
- Summary table with min/mean/max per operation
- JSON saved to `benchmarks/lifecycle/data_{version}.json` (committed to git for historical tracking)

### Regression gates

Every operation must complete in under 1.2 seconds. The test runs 3 cycles and asserts each individual operation stays under the gate.

## Host-side fork benchmark

Profiles fork (image creation) and boot-from-image. Same test file, separate test function.

```bash
uv run pytest tests/capsem-serial/test_lifecycle_benchmark.py::test_fork_benchmark -xvs
```

### Operations measured

| Metric | What it measures | Gate |
|--------|-----------------|------|
| fork | `POST /fork/{id}` — APFS clonefile of rootfs overlay + workspace | < 500ms |
| image_size | Actual disk usage of forked image (blocks, not logical size) | < 12MB |
| boot_provision | `POST /provision` with `image` param — clone image into new session | < 1200ms |
| boot_ready | First exec succeeds on the image-booted VM | < 1200ms |
| pkg_survived | Packages installed via apt survive fork (rootfs overlay) | must pass |
| ws_survived | Files written to /root/ survive fork (VirtioFS workspace) | must pass |

### Output

- Per-run breakdown with timing + survival status
- Summary table with min/mean/max + gate thresholds
- JSON saved to `benchmarks/fork/data_{version}.json` (committed to git for historical tracking)

### When to run

- After changes to fork/image code (`capsem-core/src/image.rs`)
- After changes to VirtioFS session layout (`capsem-core/src/lib.rs`)
- After changes to disk usage reporting (`session/maintenance.rs`)
- After changes to boot-from-image path in `capsem-service` or `capsem-process`
- Before cutting a release

### When to run (lifecycle)

- After changes to boot path (`capsem-process`, `capsem-init`, `capsem-core/vm/boot.rs`)
- After changes to VM teardown / delete path
- After changes to the service daemon (`capsem-service`)
- Before cutting a release

## Tests

- In-VM benchmark test: `just run "capsem-bench all"`
- In-VM availability: `test_utilities.py::test_utility_available[capsem-bench]`
- Host-side lifecycle: `uv run pytest tests/capsem-serial/test_lifecycle_benchmark.py::test_lifecycle_benchmark -xvs`
- Host-side fork: `uv run pytest tests/capsem-serial/test_lifecycle_benchmark.py::test_fork_benchmark -xvs`
- Both host-side: `uv run pytest tests/capsem-serial/test_lifecycle_benchmark.py -xvs`
- Full run: `just bench` or `just test`

## Benchmark data directory

Host-side benchmarks save versioned JSON to `benchmarks/` (committed to git):

```
benchmarks/
  fork/data_0.16.1.json          # Fork speed, image size, data survival
  lifecycle/data_0.16.1.json     # Provision, exec-ready, exec, delete
```

These data files feed the documentation benchmark page at `docs/src/content/docs/benchmarks/results.md`. Before a release, run both benchmarks and update the results page with the new numbers. See `/release-process` for the full checklist.
