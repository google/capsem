---
name: dev-benchmark
description: Measuring Capsem with capsem-bench. Use when running benchmarks, adding a dimension or a collector, reading the store, or judging whether a slowdown is real.
---

# Benchmarking

One binary computes every number. A collector prints raw samples and computes
nothing. That split is the whole design, and it exists because the thing it
replaced had ten producers each computing its own `p99`.

## Quick start

```bash
just bench-quick                 # dev loop: no guest, bounded, records like any run
just bench                       # every dimension that has a collector
just bench routes criterion      # only these
just bench-slow routes           # same harness, rebuilding intentionally stale VM assets
just bench-report                # what each subject reads, and how it has moved
```

`capsem-bench-rs doctor` first if a number surprises you. Every measurement
this repo took before it existed was taken on an unqualified machine.

## The binary

`crates/capsem-bench`, shipped as `capsem-bench-rs` (host and guest).

| subcommand | does |
|---|---|
| `list` | every dimension and whether the quick lane covers it |
| `run [dim...]` | measure, then record into the store |
| `doctor` | is this machine fit to measure -- load, governor, KVM, strays |
| `compare A B <dim>` | two stores, metric by metric; combines the latest two authoritative runs by default |
| `verify` | ratchet a run against evidence; refuses on an unfit machine |
| `report` | every subject and its trend |
| `protocol`, `protocol-delta`, `delta` | the scenario engine: host-direct vs guest-through-Capsem |
| `http-load` | duration-driven async HTTP load with pooled keep-alive connections and cold/warm latency |
| `dns-load` | duration-driven async DNS load with one persistent UDP socket per logical client |

Modules: `schema.rs` (the record), `stats.rs` (**all** statistics),
`store.rs` (SQLite), `collector.rs` (the subprocess protocol), `machine.rs`
(`doctor`), `scenarios.rs` and `protocol.rs` (the scenario engine).

## The record

`capsem.bench.v2`: one envelope, one clock, explicit identity, a flat metric
list with stable dotted keys (`gateway./vms/list.cpu_s`). Per metric: `n`,
`min`, `max`, `mean`, `median`, `p90`, `p95`, `p99`, `p999`, `stddev`, `cv`,
`mad`, rounded to two decimals.

It lives in SQLite -- `cache/target/tests/benchmarks/benchmarks.db`, two tables,
keyed by dimension and architecture. The scheme before it was a file per
dimension per release per architecture in ten incompatible shapes, one of them 80 KB of captured stdout;
asking "is `/vms/list` slower than three releases ago" meant globbing filenames
and knowing which shape each match used. It is now a query.

`quick` runs are recorded and never selected as evidence, so a dev-loop
measurement is visible without becoming a baseline. Dimension and architecture
must match before two numbers are compared -- otherwise you have measured the
difference between two machines.

## Collectors

`benchmarks/collectors/<dimension>`, executable, no suffix. Print one JSON document
of raw samples on stdout and nothing else:

```json
{"metrics": {"cpu_s": {"unit": "seconds", "samples": [0.14, 0.15]}}}
```

Rules the collector protocol enforces, each because it was violated:

- a second document is refused -- the collector ran twice and only the first
  would ever be read
- an empty `metrics` map is refused -- it passes every ratchet
- a metric with no samples is refused
- leading noise before the document is tolerated, so a collector can keep its
  own diagnostics
- every collector is bounded; one that never exits holds the machine lock the
  whole gate runs under

`[boundary.bench]` caps a collector at 300 lines. A collector growing past that
means statistics have started living in it again.

The eight guest collectors are one file, `_guest`, symlinked per mode: they
read their mode from `argv[0]` and drive the modules in
`guest/artifacts/capsem_bench/`. `criterion` derives its targets from the Cargo
manifests and reads raw per-iteration nanoseconds out of Criterion's
`sample.json`. A target is measured only when it produces fresh raw samples;
running a `[[bench]]` target alone does not establish coverage.

Two current targets, `ipc_framing_micro` and `db_read_write_micro`, only print
manual summary tables. They produce no Criterion samples and are diagnostic-only;
do not count them as stored measurements. Their migration and per-target sample
freshness checks are tracked in [#373](https://github.com/google/capsem/issues/373).
The separate `db_external_reader_poll`, `db_writer_pressure`, and
`session_stats_poll` targets do produce Criterion samples.

Load generation is part of the measurement system. Do not use one blocking
thread per client: scheduler starvation can make a high-concurrency run issue
fewer requests than its requested concurrency. The guest MITM lane delegates
to `capsem-bench-rs http-load`; every client loop must attempt at least one
request, and fewer total attempts than clients invalidates the run. Keep cold
and warm latency separate so connection admission and TLS setup cannot be
mistaken for steady-state request cost. Each logical client owns one HTTP
client and reuses its keep-alive connection: a shared pool may open multiple
connections per logical client under load and invalidate the requested
concurrency. DNS load follows the same rule: the guest wrapper delegates to
`capsem-bench-rs dns-load`, and each logical client reuses one UDP socket for
the entire level. A Python thread per client or a fresh socket per query
measures scheduler and socket churn rather than the DNS path. The host
collector also reads the
fully flushed service log and rejects proxy traffic-descriptor refusals that
the HTTP client retried internally; zero client errors alone is insufficient.
The DNS lane likewise rejects audit refusals and SERVFAIL responses. When DNS
RPS flattens or errors rise with concurrency, correlate the failed level with
the proxy log before raising a socket cap: synchronous ledger commitment and
admission exchanges can make the audit queue the real bottleneck. Changes to
that path belong in the shared benchmark lane and must keep its strict error
gate; do not replace it with a one-off load script.

Guest load collectors sample the complete Capsem process tree. Report CPU,
file descriptors, RSS, and, where the platform exposes them, PSS and USS for
the total tree and each named component. Use PSS when estimating many-VM host
capacity because summing RSS counts shared pages once per process; use USS to
show the memory that disappears with that process. Compare two unchanged runs
before and two changed runs after any performance work.

`just bench mitm-scale` is the aggregate HTTPS capacity proof. Its
`[benchmark.mitm_scale]` config creates a fresh identical cohort at each VM
count, fixes one keep-alive concurrency level per VM, samples the Capsem tree
and mock upstream separately, and rejects request errors or hidden proxy
refusals. It also records physical cores and logical CPUs. Judge scaling below
physical-core saturation first; once total CPU reaches the host budget, a flat
or falling aggregate RPS is host pressure rather than evidence of a shared
Capsem lock. Report aggregate RPS, RPS per consumed CPU, PSS per loaded VM and
per-VM latency together. Run the lane twice after changing VM ownership,
network workers, proxy concurrency, ledger admission or their limits.

`just bench content-filter` compares a disabled baseline with independently enabled
PII, secret and UTF-8 filters through the same native product dev image and Rust
protocol engine. `[benchmark.content_filter]` owns response sizes, one/two-client
cohorts, request counts and deliberate SSE tail delay. Both first-body-byte and
completion distributions retain every raw observation. This makes complete-body
buffering visible instead of reporting only total throughput. The collector refuses
request failures, retries, wrong byte counts, missing samples and hidden proxy
refusals, and separates upstream fixture pressure from Capsem CPU/RSS/PSS/USS.
Matching/blocking, permitted broker headers and durable archive non-disclosure are
proved by `tests/ironbank/test_content_filter_ledger.py`, not inferred from speed.
Enabled content inspection deliberately buffers bounded SSE bodies; do not claim
streaming-token latency or run authoritative comparisons on an unfit machine.

## Is the move real?

`compare` and `verify` report `delta_abs`, `delta_pct`, `ratio`, and
`significant`. A bare `ratio > 1.1` cannot tell a regression from a jittery
machine, which is exactly what happened: 0.6.0 qualification failed on
`gateway /vms/list CPU=0.160s > 0.140s`, and a rerun showed it was a one-off.
`significant` requires the breach to also exceed the baseline's own `cv` and
the config-owned minimum time resolution. A percentage on nanoseconds is not
product performance: those deltas remain visible as `tiny`, but never hold a
release.

Treat low-single-digit route-latency movement, including 3-5 ms, as host and
scheduler noise rather than a product speedup or regression. Route-health
ceilings are rounded gross-regression guards with a config-owned 20% operating
margin; they are not comparative performance claims. A real comparative claim
comes from repeated stored evidence and must clear its observed variance.

`[benchmark_regression] maximum_factor` is the ratio, config-owned and
relative to checked-in evidence. `minimum_time_resolution_ms` is a resolution
floor, not a performance budget: the ratio still catches meaningful drift
without pretending sub-millisecond product precision exists. Comparative
performance gets a 10% envelope; separately authored health ceilings retain
20% operating headroom and are rounded to human millisecond/second increments.

## Route coverage

53 of the 101 service routes had no timing signal, and nothing noticed, because
coverage was a hand-maintained list inside a test file. It is derived from the
router now: every registered route must be in `[benchmark.routes]` as
`measured`, `unmeasured`, or in the reasoned `internal` map. The fast Citadel guard at
`tests/citadel/test_bench_route_coverage.py` fails before build work on a route
in no class, a duplicate classification, an unexplained internal route, or
service/gateway topology drift. `unmeasured` is debt that may only shrink;
`internal` is a one-entry ratchet, not an easy exemption for new routes.

For every route in `measured`, the `routes` collector records sequential warm
and cold latency plus a 16-client kept-alive load window for both the service
UDS and authenticated gateway. The load window emits raw concurrent latency,
aggregate RPS, and CPU samples. Mutation and external-state routes remain in
the explicit debt inventory until their owning fixture can replay them safely.

## Benchmarks that are still pytest

Not yet migrated; they remain the gate for their paths.

| what | where |
|---|---|
| VM lifecycle: provision, exec-ready, exec, delete | `tests/capsem-serial/test_lifecycle_benchmark.py` |
| fork, image size, boot-from-image, data survival | same file, `test_fork_benchmark` |
| route latency gate | `tests/ironbank/test_route_latency.py` |
| route latency artifact | `tests/capsem-serial/test_route_latency_benchmark.py` |

Each guards against the latest checked-in evidence by the same config-owned
relative factor. Do not author a second absolute ceiling beside one.

Read [references/guest-benchmarks.md](references/guest-benchmarks.md) before
changing guest categories, storage/rootfs attribution, or protocol deltas.

## When to run

- `just bench-quick` while working on the service or gateway hot paths
- `just bench` before claiming anything about performance
- `just bench criterion` after touching MITM, SSE, JSON-RPC, DNS, the
  interpreters, or the logger's write path
- the lifecycle and fork suites after boot, teardown, image or VirtioFS changes
