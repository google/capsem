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
| `compare A B <dim>` | two stores, metric by metric |
| `verify` | ratchet a run against evidence; refuses on an unfit machine |
| `report` | every subject and its trend |
| `protocol`, `protocol-delta`, `delta` | the scenario engine: host-direct vs guest-through-Capsem |

Modules: `schema.rs` (the record), `stats.rs` (**all** statistics),
`store.rs` (SQLite), `collector.rs` (the subprocess protocol), `machine.rs`
(`doctor`), `scenarios.rs` and `protocol.rs` (the scenario engine).

## The record

`capsem.bench.v1`: one envelope, one clock, explicit identity, a flat metric
list with stable dotted keys (`gateway./vms/list.cpu_s`). Per metric: `n`,
`min`, `max`, `mean`, `median`, `p90`, `p95`, `p99`, `p999`, `stddev`, `cv`,
`mad`, rounded to two decimals.

It lives in SQLite -- `target/test-benchmarks/benchmarks.db`, two tables. The
scheme before it was a file per dimension per release per architecture per
profile in ten incompatible shapes, one of them 80 KB of captured stdout;
asking "is `/vms/list` slower than three releases ago" meant globbing filenames
and knowing which shape each match used. It is now a query.

`quick` runs are recorded and never selected as evidence, so a dev-loop
measurement is visible without becoming a baseline. Architecture and profile
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
`sample.json`, so a new `[[bench]]` target is measured without editing it.

## Is the move real?

`compare` and `verify` report `delta_abs`, `delta_pct`, `ratio`, and
`significant`. A bare `ratio > 1.2` cannot tell a regression from a jittery
machine, which is exactly what happened: 0.6.0 qualification failed on
`gateway /vms/list CPU=0.160s > 0.140s`, and a rerun showed it was a one-off.
`significant` requires the breach to also exceed the baseline's own `cv`.

`[benchmark_regression] maximum_factor` is the ratio, config-owned and
relative to checked-in evidence. There is deliberately no absolute
seconds-or-megabytes cap: one would be a number somebody authored rather than
measured.

## Route coverage

53 of the 101 service routes had no timing signal, and nothing noticed, because
coverage was a hand-maintained list inside a test file. It is derived from the
router now: every registered route must be in `[benchmark.routes]` as
`measured`, `unmeasured`, or in the reasoned `internal` map. The fast Citadel guard at
`tests/citadel/test_bench_route_coverage.py` fails before build work on a route
in no class, a duplicate classification, an unexplained internal route, or
service/gateway topology drift. `unmeasured` is debt that may only shrink;
`internal` is a one-entry ratchet, not an easy exemption for new routes.

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
