# Sprint: Rust Plugin Engine Spike

## Tasks

- [x] Create sprint plan.
- [x] Add Rust workspace.
- [x] Add `capsem-plugin-engine` crate.
- [x] Add separated engine integration tests.
- [x] Add Axum server crate depending on engine.
- [x] Add extension package contribution metadata for rules, tools, skills, and UI.
- [x] Add Rust compiler/executor traits for the plugin engine boundary.
- [x] Add Rust-only `wasmtime-wat` compile/load/call execution lane.
- [x] Load runtime artifacts at install time and cache loaded runtime handles.
- [x] Add `/plugins/install-run` for one-shot lifecycle benchmarking.
- [x] Add minimal WASM object/context/output host ABI.
- [x] Add ABI size limit and oversized-output rejection test.
- [x] Add Rust release benchmark example for the current Wasmtime ABI path.
- [x] Add separate fuel-budgeted Wasmtime lane.
- [x] Benchmark fuel overhead against the unbudgeted baseline.
- [x] Add memory/table/store limits to the budgeted Wasmtime lane.
- [x] Consolidate runtime and compiler performance into
  `docs/performance-recap.md`.
- [x] Run verification.

## Notes

- No Node on this path.
- `*.cwasm` is a placeholder runtime artifact extension for the Rust spike.
- The engine crate owns manifests, BLAKE3 identity, artifact writing, registry,
  and run tracing.
- The compiler and executor are Rust traits, so TypeScript compilation,
  Wasmtime component precompile, and alternative execution engines can plug in
  without changing the registry API.
- The `wasmtime-wat` lane is intentionally not the authoring story. It proves
  that the Rust crate can compile source to WASM bytes, write a runtime artifact,
  load the artifact through Wasmtime, call the exported callback, and return a
  traced result without Node.
- Runtime loading now happens at install time. The registry stores a loaded
  runtime handle per plugin, so repeated runs avoid reloading the Wasmtime
  module from disk.
- The first WASM ABI shape is now concrete: Rust writes serialized object and
  context JSON into guest memory, invokes
  `callback(object_ptr, object_len, context_ptr, context_len) -> i32`, and
  accepts returned object bytes through imported `capsem.emit_output`.
- The byte ABI currently caps object, context, and emitted output JSON at one
  megabyte each. Oversized output fails even if guest code ignores the
  `emit_output` return code.
- Benchmark harness: `cargo run -p capsem-plugin-engine --release --example
  benchmark`, with `CAPSEM_BENCH_REPEATS` and `CAPSEM_BENCH_BATCH_RUNS` knobs.
- Fuel mode: `CAPSEM_BENCH_ENGINE=wasmtime-wat-fuel`. Successful calls annotate
  `wasm_fuel_budget`, `wasm_fuel_remaining`, and `wasm_fuel_consumed`; depleted
  stores return `wasm fuel exhausted`.
- Budgeted mode also applies Wasmtime store limits: default memory cap is 2 MiB,
  table cap is 1024 elements, and each store is limited to one instance and one
  memory. Memory-limit traps are normalized as `wasm budget trap`.
- Extension packages now record `contributes.rules`, `contributes.tools`,
  `contributes.skills`, and `contributes.ui`; routing for each contribution type
  is deferred, but the package shape is in the Rust contract.
- Verification on 2026-05-21: `cargo test` passed before contribution metadata.
- Verification after contribution metadata: `cargo test` passed with 3 engine
  integration tests; `npm run typecheck` passed after restoring the legacy Node
  prototype contracts to a consistent state.
- HTTP smoke on 2026-05-21: `cargo run -p capsem-plugin-server` served
  `GET /`, `POST /plugins/install`, and `POST /plugins/run` on `127.0.0.1:8788`.
- Verification after Rust-only WASM lane: `cargo test` passed with 5 engine
  integration tests, including compiler/executor trait dispatch and a
  `wasmtime-wat` callback execution test.
- Verification after install-time load cache and install-run endpoint:
  `cargo test` passed with 6 engine integration tests.
- Verification after WASM object/context/output ABI: `cargo test` passed with 7
  engine integration tests.
- Verification after ABI size limit: `cargo test` passed with 8 engine
  integration tests.
- Benchmark baseline after ABI size limit, release mode, no HTTP:
  - `CAPSEM_BENCH_REPEATS=200`, `CAPSEM_BENCH_BATCH_RUNS=25`: 5,000 callbacks,
    compile `3.444875ms`, install load `2.611166ms`, warm wall `82.375ms`,
    `60,698` callbacks/sec, p50 `0.01425ms`, p95 `0.021833ms`.
  - `CAPSEM_BENCH_REPEATS=1000`, `CAPSEM_BENCH_BATCH_RUNS=25`: 25,000
    callbacks, compile `3.709834ms`, install load `2.901208ms`, warm wall
    `423.64075ms`, `59,012` callbacks/sec, p50 `0.014834ms`, p95
    `0.021958ms`.
- Benchmark after fuel lane, release mode, no HTTP, clean sequential run:
  - Unbudgeted `wasmtime-wat`: 25,000 callbacks, compile `3.766792ms`,
    install load `2.975375ms`, warm wall `412.549208ms`, `60,599`
    callbacks/sec, p50 `0.014041ms`, p95 `0.022958ms`.
  - Budgeted `wasmtime-wat-fuel`: 25,000 callbacks, compile `3.652708ms`,
    install load `3.003666ms`, warm wall `424.932625ms`, `58,833`
    callbacks/sec, p50 `0.014583ms`, p95 `0.023583ms`.
  - Observed fuel overhead on this tiny ABI fixture: about `3%` throughput,
    about `0.5us` p50.
- Benchmark after memory/table/store limits were added to budgeted mode:
  - Budgeted `wasmtime-wat-fuel`: 25,000 callbacks, compile `3.857583ms`,
    install load `2.817ms`, warm wall `426.85275ms`, `58,568` callbacks/sec,
    p50 `0.014666ms`, p95 `0.023291ms`.
  - Memory limiter did not materially move the tiny-fixture hot path beyond the
    fuel-budgeted baseline.
- Consolidated performance recap: `docs/performance-recap.md` now carries the
  runtime, HTTP, legacy Node, componentize-js, and AssemblyScript rows in one
  matrix.
- HTTP smoke after Rust-only WASM lane: `CAPSEM_PLUGIN_ENGINE=wasmtime-wat`
  served health, installed a WAT-backed `onModelOutput` plugin, and ran it twice
  through `/plugins/run`; Wasmtime returned `wasm_status: 7` and trace hashes.
- HTTP smoke after install-run endpoint: `POST /plugins/install-run` installed a
  WAT-backed `onModelOutput` plugin and ran it 5 times. Observed timings:
  compile `8.032541ms`, install load `6.580584ms`, warm load `0.000042ms`,
  total run `0.399208ms`, first callback `0.265125ms`, later callbacks around
  `0.03ms`; Wasmtime returned `wasm_status: 13`.

## Coverage Ledger

- Unit/contract: `crates/capsem-plugin-engine/tests/install_run.rs` covers
  install/run shape, runtime artifact naming, contribution metadata, unsafe
  contribution path rejection, compiler/executor trait dispatch, and Wasmtime
  callback execution, install-run lifecycle, object/context/output ABI, and
  oversized output rejection.
- Functional: `cargo test` covers engine install then run from registry through
  both placeholder and Rust-only `wasmtime-wat` lanes.
- Adversarial: `cargo test` covers unknown plugin rejection, unsafe contribution
  paths, oversized output, infinite-loop fuel exhaustion, and memory growth
  rejection.
- E2E/HTTP: manual smoke covered Axum health, install, and run endpoints in
  placeholder and `wasmtime-wat` modes.
- Telemetry: deferred.
- Performance: first install-run smoke records compile, install-load, warm-load,
  total run, and per-callback timing fields for a tiny WAT module. Release
  benchmark records no-HTTP Wasmtime ABI throughput and latency baseline plus
  fuel-budgeted comparison.
- Missing/deferred: TypeScript compiler, Wasmtime component ABI/precompile,
  canonical serialization, out-of-process host.
