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

- [ ] Extend `capsem-bench` with `security-engine` mode or add an equivalent
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

## Coverage Ledger

- Unit/contract: benchmark JSON schema, benchmark fixture parsing, rule-pack
  scale fixture generation, evaluator microbench setup.
- Functional: the first VM-originated process enforcement benchmark asserts the
  correct block action through the real service/process path. HTTP/DNS/MCP/
  model/file detection and allow scenarios remain open.
- Adversarial: invalid rules, unsupported Sigma constructs, missing event
  fields, high-cardinality evidence, oversized packs, slow emitter/journal path.
- E2E/VM: real VM process exec events now verify action latency visible to the
  caller plus resolved-event evidence in session storage. HTTP/DNS/MCP/model/
  file VM-originated benchmarks remain open.
- Telemetry: the first VM-originated benchmark confirms runtime enforcement
  match counters and security-log projection. VM status/OTel counters for
  enforcement evaluations, detection evaluations, findings, errors, and
  forward-plugin metrics remain open.
- Performance: adapted CEL rig numbers, Capsem canonical-root microbench
  numbers, p50/p95/p99, throughput, rule-count scaling, cold/warm compiled plan
  behavior, context/materialization cost, allocations where measurable,
  first process block latency artifact, concurrency scaling, backtest/hunt scan
  rates.
- Missing/deferred: exact threshold gates are chosen after the first stable
  S08d baseline; until then, marketing uses artifact-backed qualitative claims
  or explicit measured numbers with context.
