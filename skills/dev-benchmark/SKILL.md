---
name: dev-benchmark
description: Capsem benchmarking with capsem-bench and capsem-bench-rs. Use when running benchmarks, adding new benchmark categories, interpreting results, or investigating performance regressions. Covers benchmark categories (disk, rootfs, storage, startup, protocol/http, snapshot, all), the JSON output format, and how to add new benchmarks.
---

# Benchmarking

## Quick start

```bash
just bench                          # Run the standard artifact-recording benchmark suite
just exec "capsem-bench snapshot"    # Snapshot benchmarks only
just exec "capsem-bench disk"        # Disk I/O only
just exec "capsem-bench storage"     # Storage split diagnostics
just exec "capsem-bench-rs protocol" # Rust HTTP/model/MCP/DNS protocol benchmark
just test                           # Full validation including benchmarks
```

## capsem-bench

`capsem-bench all` is the guest benchmark contract. Hot protocol benchmarks are
implemented only by the Rust binary `capsem-bench-rs`; the Python wrapper may
orchestrate them for `all`, but must not generate HTTP/protocol/throughput
numbers itself. Do not add Python load generation as release truth for HTTP,
DNS, MCP/tools, model/SSE, or credential-broker paths. Python guest modules
still cover legacy disk/rootfs, storage, startup, and snapshot modes until
those modes are ported.

Structured JSON is saved to `/tmp/capsem-benchmark.json` for machine
consumption. Hot protocol artifacts must include lane metadata so the same
scenario can be compared as `host_direct` and `guest_capsem`.

**Locations:**
- Rust hot benchmark binary: `crates/capsem-bench`
- Legacy guest modules: `guest/artifacts/capsem_bench/`

### Benchmark categories

| Category | Command | What it measures |
|----------|---------|-----------------|
| disk | `capsem-bench disk` | Sequential/random I/O on scratch disk (write/read throughput, IOPS) |
| rootfs | `capsem-bench rootfs` | Read-only rootfs performance: largest-file sequential read, random 4K reads, large-binary sequential reads, small JS/package reads, and metadata stat-walk throughput |
| storage | `capsem-bench storage` | Diagnostic split across rootfs reads and writable paths such as `/root`, `/tmp`, `/var/tmp`, `/var/log`, and `/run` |
| startup | `capsem-bench startup` | Cold-start latency for python3, node, claude, gemini, codex |
| protocol/http | `capsem-bench-rs protocol` | HTTP, model/SSE, credential, MCP/tools, and DNS protocol scenarios against `capsem-mock-server` |
| snapshot | `capsem-bench snapshot` | Snapshot create/list/changes/revert/delete via MCP (ms per op at 10/100/500 files) |
| all | `capsem-bench all` | Default production suite including storage split diagnostics; with `CAPSEM_MOCK_SERVER_BASE_URL`, calls `capsem-bench-rs protocol` and merges the Rust network sections |

### Abstraction delta

Every hot protocol benchmark needs paired lanes:

- `host_direct`: host Rust `capsem-bench-rs` directly against `capsem-mock-server`
- `guest_capsem`: guest Rust `capsem-bench-rs` through the real Capsem network path

The delta artifact must report the cost of Capsem's abstraction for each
scenario: RPS ratio, throughput ratio, p50/p95/p99 latency delta, and error
delta. If a benchmark cannot produce both lanes, it is diagnostic only and not
a release performance gate.

`just bench` also records host-side benchmark artifacts under
`benchmarks/lifecycle/`, `benchmarks/fork/`, and `benchmarks/route-latency/`.
The route-latency artifact measures `/stats` reads while public profile
mutation routes are writing through the DB boundary. Treat it as the control
plane contention baseline for UI/TUI responsiveness.

Historical host-native benchmark runs may also exist under
`benchmarks/host-native/` with local disk I/O, CLI startup, synthetic small-file
reads, metadata-stat throughput, filesystem context, UTC timestamp, host
hardware/OS metadata, and git state. Use this when comparing VM performance
against the hardware that produced the run. The default host I/O directory is
`target/host-native-benchmark`, not `/tmp`, so Linux tmpfs does not become the
accidental baseline. Override with `CAPSEM_HOST_NATIVE_BENCH_DIR` for a specific
disk.

`just bench` runs `scripts/archive_superseded_benchmark_artifacts.py` for
retention. Before recording new artifacts, it copies the current host
architecture's active generated artifacts into `benchmarks/archive/` so
same-version reruns do not silently overwrite the prior evidence. After
recording artifacts, active benchmark directories keep only the newest generated
`data_*.json` per category, architecture, and lane. Superseded generated
artifacts are zipped under `benchmarks/archive/` with a manifest including path,
hash, project version, architecture, lane, timestamp, and source commit. Treat
archives as historical provenance, not current marketing or development
baselines.

`capsem-bench all` includes the `storage` section. Keep that in the canonical
path so Linux and macOS artifacts both capture rootfs/workspace/tmpfs
attribution data; only the long-running load diagnostics stay opt-in.

### Cross-platform comparison

`just benchmark-compare` reads committed artifacts under `benchmarks/`, compares
Linux `x86_64` against macOS `arm64`, prints ratios and percentage deltas for
shared lanes, and lists missing lanes. Use it after both platforms rerun
`just benchmark`; do not create platform-specific benchmark shortcuts.

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
- `CAPSEM_STORAGE_BENCH_PATHS`: Colon-separated writable paths for storage split diagnostics (default: `/root:/tmp:/var/tmp:/var/log:/run`)
- `CAPSEM_STORAGE_BENCH_SIZE_MB`: Write test size in MB for each storage split writable path (default: 64)
- `CAPSEM_STORAGE_IO_PROFILE_SIZE_MB`: File size in MB for detailed sequential/random storage IOPS profiling (default: 64)
- `CAPSEM_STORAGE_IO_PROFILE_RANDOM_OPS`: Random read/write operation count for storage IOPS profiling (default: 2000)

## Investigating slowness

### Snapshot performance

1. Run snapshot benchmark: `just exec "capsem-bench snapshot"`
2. Check per-stage timing: `RUST_LOG=capsem=debug just exec "capsem-bench snapshot"` -- look for `snapshot_into_slot timing` log lines showing `clone_ws_ms`, `clone_sys_ms`, `hash_ms`
3. Check session data: `just inspect-session` -- MCP tool usage section shows avg duration per snapshot operation
4. Query detailed durations: `just query-session "SELECT tool_name, duration_ms FROM tool_calls WHERE origin = 'mcp' AND tool_name LIKE 'snapshot%' ORDER BY duration_ms DESC LIMIT 20"`

Common causes:
- **clone_ws_ms high**: Large workspace, or APFS clonefile falling back to byte copy
- **hash_ms high**: Many files in workspace (walkdir overhead), or slow filesystem
- **compact slow**: Merging many snapshots with overlapping files

### Disk I/O regression

1. Run: `just exec "capsem-bench disk"`
2. Compare sequential write/read throughput against baseline
3. Check if VirtioFS mode changed (block mode has different I/O characteristics)

### Storage split regression

1. Run: `just exec "capsem-bench storage"`
2. Compare `/root` against `/tmp`, `/var/tmp`, `/var/log`, and `/run` to separate VirtioFS workspace costs from tmpfs, overlay, and rootfs read costs
3. Check `storage.kernel` for `/proc/cmdline`, virtio block queue settings, FUSE connection backpressure knobs, and known host-side KVM queue sizes
4. Check `storage.rootfs.backing.erofs_mounts` for the booted EROFS rootfs before comparing Linux/macOS rootfs reads; SquashFS fields are historical diagnostics only, not the 1.3 release gate
5. Compare the detailed I/O profile: sequential 4K/64K/1M IOPS/MB/s, random 4K read IOPS, and random 4K sync-write IOPS with p95 latency
6. Use the reported mount table to confirm which filesystem backs each path before assigning blame to KVM, VirtioFS, overlayfs, or the host filesystem

### Rootfs read regression

1. Run: `just exec "capsem-bench rootfs"`
2. Compare `rootfs.seq_read` for the historical largest-file sequential read gate
3. Compare `rootfs.large_binary_seq_read` to isolate large CLI binary reads
4. Compare `rootfs.small_js_read` for loader-style reads across many small JS/JSON/package files
5. Compare `rootfs.metadata_stat` for thousands of `lstat` calls across the rootfs tree
6. Keep `rootfs.rand_read_4k` as the broad mixed-file random-read signal

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

## Host-side route latency benchmark

Profiles hot service/gateway read endpoints and DB contention using persistent
HTTP clients instead of curl helpers so process startup does not pollute timing.
This is the TUI/control-plane hot-path gate.

```bash
uv run pytest tests/ironbank/test_route_latency.py -q -s
uv run pytest tests/capsem-serial/test_route_latency_benchmark.py -q -s
```

**Gate location:** `tests/ironbank/test_route_latency.py`
**Artifact location:** `tests/capsem-serial/test_route_latency_benchmark.py`
**Benchmark output:** `benchmarks/route-latency/data_<version>.json`

### Endpoint groups

| Group | What it covers | Default gate |
|-------|----------------|--------------|
| service_hot | `/status`, `/vms/list`, `/stats`, profile assets/plugins/enforcement/detection/MCP/security routes | route-specific p95 <= 2-3ms, max <= 5-8ms |
| gateway_hot | Gateway proxy for the same hot control routes | route-specific p95 <= 3-4ms, max <= 8-10ms |
| db_contention | `/stats` reads while `PATCH /profiles/code/mcp/default/edit` writes profile mutation ledger rows | Ironbank gate: p95 <= 15ms, max <= 40ms. Release artifact gate: p95 <= 15ms, p99 <= 40ms, max archived for visibility |

### When to run

- After changes to `/list`, `/status`, `/info`, history, files, settings,
  profile, rule, detection, enforcement, setup, skills, or gateway proxy paths
- After adding TUI polling, dashboard, tray, or gateway aggregation behavior
- Before release when claiming local control-plane responsiveness

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

## Host-side Security Engine benchmark

Profiles Security Engine hot-path costs with Rust Criterion and VM-originated
enforcement through real service, process, and network transport paths.

```bash
cargo bench -p capsem-security-engine --bench security_engine_cel
cargo bench -p capsem-core --bench security_packs
```

The `capsem-security-engine` harness measures canonical CEL compile/evaluate,
detection evaluation, backtest evidence dedupe, runtime registry projection,
compiled-plan rebuilds, policy-context projection/materialization, 100-rule
last-match paths, and native lookup comparators. The `capsem-core` security-pack
harness measures Detection IR V1 JSON parse/validate, Detection IR to CEL
detection-rule lowering, and lower-plus-compile costs.
`just benchmark` archives both Criterion harnesses from
`target/criterion/**/new/{benchmark,estimates}.json` into
`benchmarks/security-engine/data_{version}_{arch}_cel_microbench.json` and
`benchmarks/security-engine/data_{version}_{arch}_security_packs_microbench.json`;
do not rely on terminal output as the durable record.

Profiles VM-originated Security Engine enforcement through real service,
process, and network transport paths. This is outside the guest via pytest, not
via `capsem-bench`.

```bash
uv run pytest tests/capsem-serial/test_security_engine_benchmark.py -xvs
```

**Location:** `tests/capsem-serial/test_security_engine_benchmark.py`

### Operations measured

| Operation | What it times |
|-----------|---------------|
| blocked_process_exec | Service API exec request -> capsem-process IPC -> process `SecurityEvent` projection -> CEL enforcement block -> response |
| blocked_http_request | Guest curl -> network transport/MITM -> HTTP `SecurityEvent` projection -> CEL enforcement block -> response |
| keepalive_blocked_http_request | Guest Python TLS client -> one persistent MITM TLS connection -> repeated HTTP `SecurityEvent` projection -> CEL enforcement block -> response |
| blocked_dns_request | Guest resolver -> capsem DNS proxy -> DNS `SecurityEvent` projection -> CEL enforcement block -> NXDOMAIN response |
| blocked_mcp_request | Guest `/run/capsem-mcp-server` -> framed vsock MCP endpoint -> MCP `SecurityEvent` projection -> CEL enforcement block -> JSON-RPC denial |

### Output

- Per-run blocked exec latencies
- Per-run blocked HTTP request latencies
- Per-run blocked DNS request latencies
- Per-run blocked MCP request latencies
- JSON saved to
  `benchmarks/security-engine/data_{version}_{arch}_{workload}.json`
  with command, commit, host, rule, assertion, and latency metadata

### Regression gates

The first gross-regression gates assert mean blocked process exec latency stays
under 750ms and mean blocked HTTP request latency stays under 1,000ms. The
artifacts also verify runtime match counters, canonical `session.db` security
rows, and `logs` attribution. HTTP artifacts include guest wall-clock timing,
curl phase timing/deltas, and a persistent keep-alive lane. Use the
post-pretransfer first-byte delta and keep-alive first-byte timing to reason
about MITM/Security Engine response cost instead of raw guest curl wall time.
The keep-alive lane also guards against bursty same-millisecond logging
collapsing `security_events` rows. DNS artifacts additionally verify
`dns_events` policy fields and security-log qname projection. MCP artifacts
verify `tool_calls` policy fields and request-id-matched server/tool log
projection.

### When to run

- After changes to `capsem-security-engine`
- After changes to Detection IR parsing/lowering in `capsem-core`
- After changes to process security event projection or exec dispatch
- After changes to DNS proxy runtime enforcement or `dns_events` logging
- After changes to runtime enforcement rule propagation/counters
- After changes to `security_events` logging or `capsem logs`
- Before making release or marketing claims about Security Engine latency

## Tests

- In-VM benchmark test: `just exec "capsem-bench all"`
- In-VM availability: `test_utilities.py::test_utility_available[capsem-bench]`
- Host-side lifecycle: `uv run pytest tests/capsem-serial/test_lifecycle_benchmark.py::test_lifecycle_benchmark -xvs`
- Host-side fork: `uv run pytest tests/capsem-serial/test_lifecycle_benchmark.py::test_fork_benchmark -xvs`
- Host-side endpoint latency: `uv run pytest tests/capsem-serial/test_endpoint_latency_benchmark.py -xvs`
- Host-side Security Engine: `uv run pytest tests/capsem-serial/test_security_engine_benchmark.py -xvs`
- Both host-side: `uv run pytest tests/capsem-serial/test_lifecycle_benchmark.py -xvs`
- Full run: `just benchmark` (or alias `just bench`) or `just test`

## Benchmark data directory

Host-side benchmarks save arch-scoped JSON to `benchmarks/` (committed to git
for performance baselines). Set `CAPSEM_BENCHMARK_RUN_ID` for an
intentional named run and `CAPSEM_BENCHMARK_OUTPUT_DIR` for exploratory runs
that should not dirty the checkout:

```
benchmarks/
  fork/data_1.2.3_x86_64_linux-rc1.json          # Fork speed, image size, data survival
  lifecycle/data_1.2.3_x86_64_linux-rc1.json     # Provision, exec-ready, exec, delete
  endpoint-latency/data_*.json   # Service/gateway read latency across 8 live VMs
  security-engine/data_*.json    # CEL microbench and VM-originated enforcement
```

These data files feed the documentation benchmark page at `docs/src/content/docs/benchmarks/results.md`. Before a release, run both benchmarks and update the results page with the new numbers. See `/release-process` for the full checklist.
