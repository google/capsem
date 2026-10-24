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
just bench-quick routes          # diagnostic dev loop; includes guest fixtures
just bench-report                # what each subject reads, and how it has moved
source_commit=$(git rev-parse HEAD)
uv run --project build_system --frozen capsem-gate bench --commit "$source_commit" "routes criterion"
```

Quick routes reduces sample and cycle counts; lifecycle, network membership,
credential and MCP cases still boot owned guest fixtures. It requires their
matching host/guest/image inputs and is not a host-only or fixture-free shortcut.

Authoritative runs need an explicit exact source SHA and matching host/guest/image
receipts. The gate's ordinary `just bench` defaults to `unknown`; use its explicit
`--commit` interface above for evidence, adding `--slow` only when stale assets
intentionally need rebuilding. An omitted dimension list requests ALL declarations
and fails if any collector is missing; it never silently skips unfinished work.
Use an explicit complete set. Keep frozen-cohort binary invocations under the
bounded command owner and retain source/config/asset identity.

## The binary

`crates/capsem-bench`, shipped as `capsem-bench-rs` (host and guest).

| subcommand | does |
|---|---|
| `list` | every dimension and whether the quick lane covers it |
| `run [dim...]` | measure, then record into the store |
| `doctor` | is this machine fit to measure -- load, governor, KVM, strays |
| `compare A B <dim>` | requires complete fit compatible repeats on both sides; two by default |
| `verify` | ratchet compatible repeats; refuses diagnostic or missing-fitness observations |
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

It lives in SQLite -- `cache/target/tests/benchmarks/benchmarks.db`,
keyed by dimension and architecture. The scheme before it was a file per
dimension per release per architecture in ten incompatible shapes, one of them 80 KB of captured stdout;
asking "is `/vms/list` slower than three releases ago" meant globbing filenames
and knowing which shape each match used. It is now a query.

Store schema v4 retains original samples as little-endian f64 blobs beside each
summary in the same transaction. Record JSON deliberately excludes these arrays;
do not expand raw samples into JSON for storage. Pre/post machine-fitness receipts
and exact config text are MessagePack metadata. Historical v2/v3 summaries remain
readable observations without fabricated fitness or raw data. Their already
computed summaries are not invalidated, but they cannot become new evidence.

`quick` runs are recorded and never selected as evidence, so a dev-loop
measurement is visible without becoming a baseline. Dimension and architecture
must match before two numbers are compared -- otherwise you have measured the
difference between two machines. Each dimension takes fresh fitness checks before
launch and after descendant cleanup. An unfit preflight launches nothing; post-run
contamination is stored diagnostically and returns nonzero. Comparison requires
the requested repeat count, identical metric keys/units and compatible machine/
configuration cohorts. Repeats on one side must share source; baseline and
candidate may intentionally use different source commits. Direct protocol
`--record` lacks pre-workload fitness and is diagnostic only.

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
- `[benchmark.run]` caps stdout during drain and samples during deserialization;
  exceeding a per-metric or total budget fails rather than silently thinning
  evidence. The store enforces the same bounds and prunes raw data with its run.

`[boundary.bench]` caps a collector at 300 lines. A collector growing past that
means statistics have started living in it again.

The eight guest collectors are one file, `_guest`, symlinked per mode: they
read their mode from `argv[0]` and drive the modules in
`guest/artifacts/capsem_bench/`. `criterion` derives its targets from the Cargo
manifests and reads raw per-iteration nanoseconds out of Criterion's
`sample.json`. Its empty invocation-owned output directory must cover the exact
pinned Criterion `--list` case set, with matching `benchmark.json` identities.
Missing, extra, malformed, partial and historical `--reuse` inputs fail.
Original upstream input text and invocation identity remain in the bounded
sidecar; normalized samples use the existing Rust statistics/store owner.
Cargo's shared build cache is unchanged. Reading, parsing and conversion are
collector work, not established per-child workload CPU/resource receipts.

Criterion prepares with `cargo bench --no-run --message-format=json`, admitting
one exact local executable bound to package, target, source and profile from Cargo metadata and
the canonical shared target. It then invokes that same executable with `--bench
--list` and `--bench` (without `--bench`, pinned Criterion enters test mode).
On macOS the existing signed-copy runner prepares the actual executable; a
private bounded typed receipt preserves original argv0/assets/signature ownership
and rejects replacement between phases. Original bounded Cargo/list/sample inputs
and distinct preparation/declaration/runtime wall clocks remain in the existing
sidecar. Those clocks are not workload CPU, exact RSS peaks or statistical results.

Criterion's measured child also uses the existing stamped process-tree sampler,
excluding prepare/sign/list and collector parsing. A fresh child has zero CPU
origin, so its observed startup CPU is retained. Resource keys explicitly say
`observed_cpu_lower_bound_s` and `sampled_rss_high_water_bytes`; they are one
job-level observation for each target invocation, not per-case resource distributions.
RSS sums come from sequential reads and can double-count shared pages. Optional
PSS/USS remain absent when unsupported. No continuous or atomic peak is claimed.
Terminal/zombie and unseen short-lived descendant CPU remain uncertified;
successful original-child/descendant retirement is a separate lifecycle receipt.
Sampling interval, process count, identity churn, observations and stop budgets
belong to `[benchmark.criterion.resources]`. Exceeded budgets/read failures fail
the run; they never clip data or fabricate a zero. Original f64 metric samples
still use the sole Rust statistics/raw store, with bounded identity/completeness
metadata in the existing sidecar (not a retained per-tick memory trace).

Four current targets, `ipc_framing_micro`, `db_read_write_micro`,
`vsock_framing_micro` and `ledger_codec`, print only manual summary tables.
Their canonical diagnostic classification is `[benchmark.criterion]` in
`config/gate.toml`: default selection names and excludes them; explicit
selection fails. They never inherit another target's measured coverage.
Their genuine Criterion migration remains tracked in
[#373](https://github.com/google/capsem/issues/373).
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

Disk/storage retain original unrounded per-operation latency arrays in the
existing guest diagnostic JSON artifact. The host supplies the existing
`benchmark.run` sample/output budgets; incomplete I/O or malformed/count-mismatched
artifacts fail. The collector moves each array to the raw metric protocol once,
retaining completed-byte/count/unrounded elapsed facts in its bounded sidecar.
Rust computes the recorded distributions. Sequential-write operation latency
excludes its final flush barrier, while aggregate elapsed includes it; synchronous
random-write operation latency includes that operation's flush. These are software
call windows, not physical-device latency. Explicitly selected storage paths must
succeed; unavailable default optional paths remain named unmeasured skips.
Human-only diagnostic invocation keeps its existing summaries and cannot supply
the strict collector's raw contract. Matching guest/harness source must be prepared
on both sides before paired measurements; no historical aggregate is promoted.

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

Disk/storage sample the exact already-running host service and observed
descendants only during their original guest exec (after readiness, before
artifact retrieval). The bounded sampler reuses the numeric budget at
`benchmark.criterion.resources`; `host_resources.budget_config` and its receipt
name that provenance, without claiming a Criterion child or fresh lifetime.
CPU is an observed live-counter lower bound, RSS/FD values are sampled tree
high-water observations, and optional PSS/USS remain absent when unavailable.
Unseen children, final-interval CPU, atomic/continuous peaks, and out-of-tree
Apple VZ helpers are not certified. Resource selection is independent of
`LOAD_MODES`, so disk/storage keep their existing guest CPU allocation.

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

Coverage is method-aware: a measured GET never covers a POST on the same path.
`[benchmark.routes]` names successful method/path/case fixtures and records each
remaining operation with its existing fixture owner and reason. The source-only
Citadel guard derives methods from service and gateway registrations, including
chained Axum handlers, and rejects missing, stale or duplicate classifications.
Streaming operations bind to their real stream collector and named raw metrics;
they never inherit JSON read-poll coverage.
A new route or method cannot silently inherit another method's coverage.

For every measured case, the `routes` collector records warm and cold latency
plus a 16-client kept-alive load window over its explicitly selected target scope.
Ordinary cases use service UDS and authenticated gateway; gateway-owned health,
OpenAPI and static surface cases use only the gateway, never a service HTTP404.
It retains raw concurrent latency, aggregate RPS and CPU samples. Managed claim
and close measure repeatable closed tombstones, not initial lifecycle operations.
Uncovered mutations require their real disposable lifecycle fixtures; successful
read timing and functional acceptance are not proof of mutation performance.
The collector currently declares 65 cases over 59 operations, including four
plugin-detail IDs, the existing stopped recorded session and one live image VM.
Untimed preflight validates selected public SDK schemas and private DTO/counter
contracts, checks nonempty recorded traffic/history and reconciles shared
counters. The original HTTP/JSON checks remain in every timed request. Security
ledger arrays can be empty in this recorded corpus; an idle Ask queue is explicitly
not pending approval performance. Initial managed claim/close and pending Ask
cases remain named `unmeasured_cases`; 54 other operations retain their owners.
The additional read corpus is config-owned under `[benchmark.routes.corpus]`:
64 workspace files plus two changed files, 1 MiB logs with 128-byte unique lines,
16-line filtered tails within 100 KiB reads, and one cached builtin tool metadata
entry. The cache grants no approval and calls no tool. The service endpoint stays
plain text; host log JSON uses explicit Accept negotiation. A retained service
stream generation leaves its active writer untouched. None of these cases uses
real credentials. Connection enrollment metadata remains uncovered because the
production connection profile is deliberately unconfigured. Network inspection
creates and deletes one actual isolated registry record through public APIs.
Its empty member list is real metadata, not a transport-log fixture: network
creation writes no network_events, so populated log timing stays uncovered.
Metric keys include method, template and case, so plugin cases never overwrite
one another. Sample counts, raw retention, fitness, CPU and statistics owners are
unchanged. Added coverage is source readiness until the exact-source native
collector runs complete; it is not a new performance result.

Route samples use the existing transport owner's strict `measure_request` path:
no retries, one total monotonic deadline and a config-owned response-body budget.
Raw headers, first client-observed body byte and complete-body durations are
retained for warm, cold and concurrent samples with exact requested counts.
First body byte means a completed client read, not a hardware wire timestamp.
Truncated framing, slow trickles, oversize or missing body observations fail the
collector. Ordinary functional HTTP requests retain their retry contract. The
strict receipt/config cohort differs from older completion samples which also
included JSON validation; do not present them as identical measurements.

Binary/archive route cases keep the same strict receipt and raw-store owners.
New file profiles declare 4KiB/1MiB original binary bodies and bounded per-case
sample/load/concurrency counts; older read profiles keep their defaults and an
explicit `--samples` still controls warm/cold samples. A real workload writes the
files. Each download goes through current file admission; final fixture stop
drains the producer before public security-rule and file-export counter checks.
The WARC profile checks every gzip member/footer, opening/final summary, all
record identities, counts, omissions and exact original bytes through `warcio`.
The WARC case serializes each session, respecting its one-active-export permit;
it does not claim multi-session export concurrency coverage.
Its independent bounded MessagePack oracle belongs to the logger session fixture
regenerator and standing parity test, not to the collector or exporter. Complete
event-body reads likewise compare original bytes and distinguish capture from
transport truncation. Response validation runs after receipt clocks; throughput
includes client validation work. CPU series still name service/gateway components,
not total VM policy CPU. Source readiness is not an executed native measurement.

Gateway-only static reads compare source version and exact owning service socket,
complete selected OpenAPI paths/schemas, and first-party launcher HTML/JS bytes.
Surface CSP, no-store, nosniff, frame denial, referrer and opener headers are
validated on every response after strict receipt clocks. The default launcher
uses a syntactically valid recorded VM ID and no bootstrap token or Xpra session;
these cases do not certify GUI bootstrap, rendering or native input latency.
Bounds remain the typed shared response budget; raw/stats/fitness owners and
ordinary sample counts are unchanged. Content validation is outside latency
clocks but remains part of measured load throughput.

The same inventory separately binds three operations to `stream`: real
stream-v3 echo/resize/terminal and exec transfers, rotated transcript tails, and
gateway JSON WebSocket events. The65 ordinary route cases cover59 operations;
these three streaming bindings bring explicit source coverage to62 operations,
with54 remaining operations plus7 internal controls and3 unmeasured states.
Bindings require each raw metric and its configured sample count; missing or
partial phases fail instead of silently inheriting a successful collector exit.

`[benchmark.stream]` owns sizes/counts and transcript/event bounds. The existing
16MiB terminal source transfer base64-expands past the real20MiB transcript
rotation threshold. A known foreground marker is emitted through the same
stream, then the disposable persistent owner stops/drains. Transcript timing
uses strict warm/cold HTTP receipts on service and gateway, exact base64/byte
count and nonempty bounded tail/marker validation; rotation metadata comes from
the real owner log generation, never fabricated PTY frames. All statistics and
whole-process-tree resource samples retain their existing owners.

`/events` is a separate JSON WebSocket, not stream-v3 or SSE. Matched Ping/Pong
samples are transport RTT and exclude upgrade time; they do not measure app
latency. A Running snapshot is primed before subscribe; no initial event is
assumed. After one actual stop, authoritative gateway status must report Stopped,
then the matching `status_poll` transition is required under byte/frame/deadline
budgets. Stop-to-event includes stop+poll; poll-to-event begins before that
status request. These are client-observed delivery clocks, not server timestamps.
One transition per collector run is deliberately not a p99/tail distribution;
no removed-VM event, general notification delivery, POST-stop endpoint coverage
or GUI/native rendering claim follows from it. Native matching-cohort execution
and fit repeats remain required before any performance conclusion.

## Benchmarks that are still pytest

Not yet migrated; they remain the gate for their paths.
The full `just test` graph runs these serial/guest/Kingslanding owners and an
early standing-fitness check; it does not implicitly run the canonical collector
dimension sweep. Explicit performance requirements need their collector runs in
addition to full functional qualification. A HUD or a declared dimension is not
evidence of a completed measurement.

| what | where |
|---|---|
| VM lifecycle: provision, exec-ready, exec, delete | `tests/capsem-serial/test_lifecycle_benchmark.py` |
| fork, image size, boot-from-image, data survival | same file, `test_fork_benchmark` |
| route latency gate | `tests/ironbank/test_route_health.py` |
| route latency artifact | `tests/capsem-serial/test_route_latency_benchmark.py` |

Each guards against the latest checked-in evidence by the same config-owned
relative factor. Do not author a second absolute ceiling beside one.

Read [references/guest-benchmarks.md](references/guest-benchmarks.md) before
changing guest categories, storage/rootfs attribution, or protocol deltas.

## When to run

- `just bench-quick` while working on the service or gateway hot paths
- explicit exact-source collector runs before claiming anything about performance
- the Criterion collector after touching MITM, SSE, JSON-RPC, DNS, the
  interpreters, or the logger's write path
- the lifecycle and fork suites after boot, teardown, image or VirtioFS changes

For finite image/catalogue and runtime preparation route cases, see
[image route evidence](references/image-routes.md).

For finite host-only policy/settings edits and their separate VM/ledger limits,
see [policy route evidence](references/policy-routes.md).

For finite exposure/surface controls and their gateway-only minting scope,
see [preview route evidence](references/preview-routes.md).

For original VM-only one-shot execution and owned stopped-session disposal,
see [run and purge route evidence](references/run-purge-routes.md).

For original ephemeral save claims and mandatory normal cold retention checks,
see [save route evidence](references/save-routes.md).

For original HTTP Ask resolution and separate pending/completion clocks,
see [Ask route evidence](references/ask-routes.md).

For live corporate reload and original flow block/restore checks,
see [reload route evidence](references/reload-routes.md).

For the finite real prepared CLI manifest check and its isolated provenance,
see [update check evidence](references/update-check-routes.md).
