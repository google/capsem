---
name: dev-benchmark
description: Benchmarking with capsem-bench and capsem-bench-rs. Use when running benchmarks, adding a category, interpreting results, or investigating a performance regression.
---

# Benchmarking

## Quick start

```bash
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

Read [references/guest-benchmarks.md](references/guest-benchmarks.md) before
changing or diagnosing guest categories, storage/rootfs attribution, snapshots,
protocol abstraction deltas, cross-platform comparisons, or environment knobs.

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

The test runs three cycles and refuses any operation mean more than the
config-owned relative factor above the latest checked-in lifecycle evidence.
No duration is independently authored in the test.

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

| Metric | What it measures |
|--------|------------------|
| fork | `POST /fork/{id}` — clone rootfs overlay + workspace |
| image_size | Actual allocated blocks of the forked image |
| boot_provision | `POST /provision` with the forked image |
| boot_ready | First exec succeeds on the image-booted VM |
| pkg_survived | Installed package survives fork (must pass) |
| ws_survived | `/root` file survives fork (must pass) |

The four numeric metrics use the same config-owned relative factor and latest
checked-in fork evidence as their guard. Do not add a second milliseconds or
MiB ceiling; update baseline evidence only after reviewing an intentional run.

### Output

- Per-run breakdown with timing + survival status
- Summary table with min/mean/max; failures name current, baseline, and ratio
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
Intentional Criterion publication archives both harnesses from
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
- Full release run: `just test`

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
