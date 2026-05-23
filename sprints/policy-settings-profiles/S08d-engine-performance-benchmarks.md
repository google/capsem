# S08d - Security Engine Performance Benchmarks

## Status

In progress. Inserted on 2026-05-21 as the S08 exit benchmark sprint.

## Goal

Prove the runtime speed of Capsem's normalized Security Engine before public
CLI/UI/docs/marketing surfaces make speed claims.

S08d starts only after S08b stabilizes the canonical policy context ABI and
S08c supplies real shared event/rule corpora. Benchmarks must measure the
contract we intend to ship, not transitional `event.*` rules or synthetic-only
paths.

This sprint measures real VM-originated events flowing through Network/File/
Process/MCP/model paths into the Security Engine, then records how quickly
Capsem can allow, block, ask, and detect using CEL enforcement and
Sigma-compatible detection.

## Product Contract

- Performance claims must be backed by measured data, not intuition about CEL
  or Sigma.
- Speed claims must use canonical policy roots such as
  `http.request.host.contains("google")`, not internal envelope paths.
- Capsem must adapt the Howard John-style CEL benchmark methodology to measure
  our implementation. The suite should use comparable categories from
  <https://blog.howardjohn.info/posts/cel-fast/>: context/materialization cost,
  fast field access, slow/body/regex access, header lookup, and native Rust
  comparator implementations.
- The adaptation should use the concrete Agentgateway benchmark shape in
  `crates/agentgateway/src/cel/benches.rs` at commit
  `2f9ffa89c25a45f3eca34ba39bb6241a1e6d8a4b`, covering the same high-level
  benchmark families where they map to Capsem: compile, execute over borrowed
  policy/request context, execute over materialized/snapshot context, and
  low-level lookup comparisons.
- Benchmarks must include VM-originated events that cross the real transport/
  service/process boundary. Microbenchmarks are useful, but they are not enough
  for marketing or release claims.
- The benchmark harness must distinguish:
  - normalization latency;
  - preprocessor time;
  - CEL enforcement evaluation time;
  - ask/confirm handoff time when applicable;
  - detection evaluation time;
  - postprocessor time;
  - emitter/journal/write time;
  - end-to-end action latency visible to the VM.
- Results must include correctness and performance together: every measured
  event asserts the expected allow/block/detect result and the persisted
  resolved-event evidence.
- No marketing number ships unless S08d records the exact command, host/arch,
  profile/rule pack, event shape, sample size, and percentile summary.

## Benchmark Scope

### VM-Originated End-To-End Benchmarks

Extend the existing benchmark infrastructure with a security-engine benchmark
mode, likely under `capsem-bench security-engine` plus a host-side serial pytest
wrapper for stable artifact capture.

Measured scenarios:

- HTTP allow, block, and detection-only finding from inside the VM.
- DNS allow, block, and detection-only finding from inside the VM.
- MCP tool allow, block, and detection-only finding from inside the VM.
- Model request allow, block/rewrite where available, and detection-only
  finding from inside the VM.
- File write/create/delete detection and enforcement paths once File Engine
  cutover lands.
- Process exec detection and enforcement paths once Process Engine cutover
  lands.
- Mixed workload that exercises multiple event families concurrently.

For each scenario, capture:

- p50/p95/p99 end-to-end decision latency;
- events/sec throughput at low, medium, and burst concurrency;
- rule count scale: no-match, first-match, last-match, 10 rules, 100 rules,
  1,000 rules where reasonable;
- detection pack scale: single Sigma rule, common small pack, larger pack;
- cold compiled-plan load versus warm steady-state evaluation;
- resolved-event journal write overhead;
- false-negative/false-positive correctness assertions.

### Evaluator Microbenchmarks

Add Criterion or equivalent Rust microbenchmarks for the pieces that should be
extremely fast:

- Adapted CEL benchmark rig inspired by the Howard John post:
  - map the Agentgateway benchmark cases to Capsem equivalents:
    `simple_access`, `header`, `bbr`/body JSON extraction, `jwt`, `cidr`, and
    `regex`;
  - map the Agentgateway benchmark phases to Capsem equivalents: expression
    compile, borrowed/reference execution, materialized/snapshot execution, and
    lookup;
  - build/materialize CEL context repeatedly;
  - evaluate a fast field expression repeatedly;
  - evaluate a slower expression repeatedly;
  - compare header lookup through CEL versus optimized native Rust lookup;
  - compare regex/matches with compile-time/precompiled regex versus runtime
    work;
  - report allocations where the harness can measure them.
- Low-level lookup comparators should adapt the same categories as the upstream
  rig: direct native access, nested `match`, nested map lookup, and CEL lookup.
  Capsem may add additional borrowed-view/native-resolver variants after the
  canonical correctness path lands.
- Capsem canonical-root variants of that rig:
  - `http.request.host.contains("google")`;
  - `http.request.url.contains("google")`;
  - `http.request.path.startsWith("/admin")`;
  - `http.request.header("authorization").exists()`;
  - `http.request.body.text.contains("secret")`;
  - equivalent native Rust lookups over `PolicyContext` or borrowed views.
- CEL parse/compile once per rule pack.
- CEL warm evaluation over normalized events.
- CEL warm evaluation over the canonical `PolicyContext` map/materialized path
  versus any native/borrowed resolver path we add after correctness lands.
- Sigma-compatible detection lowering into the runtime predicate/CEL plan.
- Detection warm evaluation over normalized events.
- Evidence-signature dedup for default 100-row backtest responses.
- Registry atomic plan swap cost.

Microbenchmarks must not replace VM-originated benchmarks; they explain where
time goes when an end-to-end number regresses.

### Backtest And Hunt Benchmarks

S08c proves correctness. S08d adds time-series performance:

- enforcement backtest over shared corpus;
- detection backtest over shared corpus;
- detection hunt over a real session/timeline journal;
- default 100 matched-row evidence dedup path;
- larger historical corpus scans with documented event/rule counts.

## Output Artifacts

Commit benchmark outputs in the same style as existing benchmark artifacts:

```text
benchmarks/security-engine/data_<version>_<arch>.json
benchmarks/security-engine/README.md
```

The JSON should include:

- Capsem version/commit;
- host OS, architecture, CPU model where available;
- VM profile id/revision and asset identity;
- rule/detection pack ids, revisions, hashes, and rule counts;
- event family and workload name;
- sample count, warmup count, concurrency, and duration;
- p50/p95/p99/min/max latency;
- throughput;
- correctness counters;
- links or ids for captured resolved-event evidence.

Docs consume these artifacts through the benchmark docs page and S19a marketing
copy. Marketing may use qualitative claims before numbers only if the claim is
clearly not numerical and matches the sprint tracker.

## Tasks

- [~] Extend `capsem-bench` with `security-engine` mode or add an equivalent
  VM-originated benchmark harness that is invoked by `just bench`.
- [~] Add host-side serial pytest artifact capture for security-engine benchmark
  JSON under `benchmarks/security-engine/`.
- [~] Add Rust evaluator microbenchmarks for CEL, detection lowering/evaluation,
  evidence dedup, and registry plan swaps.
- [~] Adapt the Howard John-style CEL benchmark methodology into a Capsem local
  baseline artifact, using the Agentgateway `benches.rs` families/cases as the
  source model where they map, before drawing optimization conclusions.
- [~] Add correctness assertions for every benchmark scenario: expected final
  action, expected detection finding, and persisted resolved-event evidence.
- [ ] Add rule-pack and event fixtures for low/medium/high rule-count cases.
- [ ] Add concurrency/load cases that prove engine work remains bounded under burst
  VM activity.
- [~] Update `docs/src/content/docs/development/benchmarking.md` and
  `docs/src/content/docs/benchmarks/results.md` with the new benchmark mode and
  latest recorded results.
- [ ] Feed measured results into S19a marketing copy only after benchmark artifacts
  exist.
- [~] Add regression gates for gross latency regressions once the first stable
  baseline is recorded.

## Implementation Notes

- Slice 1 added `crates/capsem-security-engine/benches/security_engine_cel.rs`
  as the first Criterion microbench harness. It measures canonical CEL compile
  time, warm enforcement evaluation for `http.request.host`,
  `http.request.url`, `http.request.path`, `http.request.header(...)`, and
  `http.request.body.text`, a combined canonical HTTP policy, a 100-rule
  last-match path, policy-context projection/materialization, and a native
  Rust comparator over the same HTTP evidence. Verification: red `cargo bench
  -p capsem-security-engine --bench security_engine_cel --no-run` first failed
  on the missing bench file; after adding the harness, `--no-run` passed and
  the full `cargo bench -p capsem-security-engine --bench security_engine_cel`
  executed successfully. This is not yet a release benchmark artifact.
- Slice 2 committed the first host-side S08d microbenchmark artifact under
  `benchmarks/security-engine/`. The artifact records Criterion slope
  estimates for the canonical HTTP CEL cases, policy-context projection,
  100-rule last-match evaluation, and native lookup comparator from the local
  `cargo bench -p capsem-security-engine --bench security_engine_cel` run. The
  benchmark results docs surface those numbers with an explicit caveat that
  they are not VM-originated end-to-end latency claims.
- Slice 3 added `tests/capsem-serial/test_security_engine_benchmark.py` as the
  first VM-originated Security Engine benchmark. The test starts a real service
  and VM, installs a runtime CEL enforcement rule, sends repeated blocked
  shell exec requests through the service/process IPC path, asserts the block
  response, drains runtime match counters, verifies canonical `security_events`
  and `security_event_steps` rows in `session.db`, checks `logs` exposure, and
  archives
  `benchmarks/security-engine/data_1.1.1778860037_arm64_process_enforcement.json`.
  The latest local run measured eight blocked exec decisions at 9.438ms mean
  and 9.801ms max against a conservative 750ms gross-regression gate.
- Slice 4 extended `crates/capsem-security-engine/benches/security_engine_cel.rs`
  beyond enforcement CEL into detection evaluation, backtest evidence dedupe,
  and runtime registry operations. The refreshed
  `benchmarks/security-engine/data_1.1.1778860037_arm64_cel_microbench.json`
  now records single-rule detection at 23.247us, 100-rule last-match detection
  at 1.292ms, 100-row evidence dedupe at 19.417us, 1,000-row/100-unique dedupe
  at 167.09us, single-rule registry install/update at 145ns, and 100-rule
  enabled-rule projection at 7.453us. Remaining microbench debt: explicit
  Detection IR/Sigma lowering cost and atomic compiled-plan swap cost once the
  service-side swap path is factored into a stable benchmarkable boundary.
- Slice 5 wired the equivalent host-side Security Engine benchmark harness into
  `just bench`. The recipe now runs the Criterion Security Engine microbench
  and the VM-originated process-enforcement serial benchmark after the existing
  in-VM and lifecycle/fork benchmark stages. We are keeping a separate
  `capsem-bench security-engine` guest mode open until HTTP/DNS/MCP/model
  VM-originated scenarios can run from inside the VM with useful workload
  controls.
- Slice 6 extended the VM-originated benchmark harness with an HTTP request
  enforcement workload. The test installs a runtime CEL rule for a unique
  `https://example.com/...` path, warms the path once, runs a guest curl loop
  that is blocked before upstream dispatch, asserts each 403 block response,
  verifies runtime match counters, checks `security_events`/
  `security_event_steps` rows for `http.request`, confirms `logs` exposes the
  canonical decision, and archives
  `benchmarks/security-engine/data_1.1.1778860037_arm64_http_request_enforcement.json`.
  The benchmark now also opens one persistent TLS keep-alive connection and
  sends eight sequential blocked requests over it, asserting 17/17 HTTP
  resolved security events when combined with warmup and curl runs. That
  caught same-millisecond Security Event ID collapse in bursty logging; HTTP
  now carries a per-request event seed, DNS/MCP/file IDs use nanosecond
  timestamps, and synthetic block/error telemetry is enqueued at the decision
  point instead of response-body finalization. Latest local results are eight
  measured blocked HTTP curl requests at 9.091ms mean wall-clock and 3.997ms
  mean `time_starttransfer`, with a 0.683ms mean post-pretransfer first-byte
  slice and 2.145ms mean TLS appconnect. The keep-alive lane is 0.549ms mean
  first-byte / 0.556ms mean total response after the connection is established.
  The process benchmark refreshed at 9.356ms mean and 9.992ms max.
- Slice 7 added the missing non-VM microbench boundaries for runtime rule-plan
  rebuilds and Detection IR security-pack lowering. The
  `capsem-security-engine` Criterion harness now measures projection plus
  CEL compilation for 100 enforcement rules, 100 detection rules, rebuilding a
  `SecurityEngine` from 100 enforcement plus 100 detection rules, and updating
  one existing rule before rebuilding a 100-rule plan. A new
  `capsem-core` `security_packs` Criterion harness measures Detection IR V1
  JSON parse/validate, single-rule lowering, 100-rule lowering, and 100-rule
  lower-plus-compile cost against the committed Google-secret fixture. Latest
  local results: 307.684us to project/compile 100 enforcement rules,
  312.907us to project/compile 100 detection rules, 628.514us to rebuild a
  100+100 rule engine, 355.298us to update and rebuild a 100-rule plan,
  122.645us to parse/validate the Detection IR fixture, 1.075us to lower the
  single-rule fixture, 96.620us to lower 100 Detection IR rules, and 2.762ms
  to lower plus compile 100 Detection IR rules. `just bench` now runs both
  Criterion harnesses.
- Slice 8 wired DNS requests into the runtime Security Engine before upstream
  resolution and added the first VM-originated DNS enforcement benchmark. The
  benchmark installs a runtime CEL rule for a unique qname, triggers guest
  resolver lookups, asserts the lookup fails through a synthetic DNS denial,
  verifies runtime match counters, checks both canonical `security_events` and
  legacy `dns_events` policy rows, and confirms `capsem logs` exposes the DNS
  qname and rule attribution. Latest local result: eight guest resolver calls
  produced sixteen blocked DNS security events/`dns_events` rows at 1.109ms
  mean, 0.830ms median, 3.508ms p95/max against a 1,000ms gross-regression
  gate. This slice also expanded security-log projection with family-specific
  subject fields so DNS qname, HTTP host/path, MCP server/tool, model provider/
  name, file path, and process operation/class are visible in logs.

## Coverage Ledger

- Unit/contract: benchmark JSON schema, benchmark fixture parsing, rule-pack
  scale fixture generation, evaluator microbench setup, same-millisecond
  event-ID regression coverage for HTTP, DNS, MCP, and file security events,
  Detection IR parse/lowering fixture coverage.
- Functional: VM-originated process, HTTP request, and DNS request enforcement
  benchmarks assert correct block actions through the real service/process,
  guest-network/MITM, and guest DNS proxy paths. MCP/model/file detection and
  allow scenarios remain open.
- Adversarial: invalid rules, unsupported Sigma constructs, missing event
  fields, high-cardinality evidence, oversized packs, slow emitter/journal path.
- E2E/VM: real VM process exec, HTTP request, and DNS request events now verify
  action latency visible to the caller plus resolved-event evidence in session
  storage. MCP/model/file VM-originated benchmarks remain open.
- Telemetry: the first VM-originated benchmark confirms runtime enforcement
  match counters and security-log projection. The HTTP benchmark now proves
  fast synthetic block responses produce both `net_events` and
  `security_events` for every request in bursty keep-alive traffic. The DNS
  benchmark proves `dns_events`, `security_events`, runtime counters, and
  security logs all carry DNS qname/rule attribution. VM status/OTel counters
  for enforcement evaluations, detection evaluations, findings, errors, and
  forward-plugin metrics remain open.
- Performance: adapted CEL rig numbers, Capsem canonical-root microbench
  numbers, p50/p95/p99, throughput, rule-count scaling, cold/warm compiled plan
  behavior, context/materialization cost, allocations where measurable,
  detection evaluation, backtest evidence dedupe, runtime registry projection,
  runtime compiled-plan rebuild cost, Detection IR parse/lowering/compile cost,
  first process, HTTP request, and DNS request block latency artifacts,
  concurrency scaling, backtest/hunt scan rates.
- Missing/deferred: exact threshold gates are chosen after the first stable
  S08d baseline; until then, marketing uses artifact-backed qualitative claims
  or explicit measured numbers with context.
