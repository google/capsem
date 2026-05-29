# Sprint: Rust Plugin Engine Spike

## Goal

Move the prototype center of gravity to Rust:

```text
capsem-plugin-engine crate
  -> validates source + public manifest
  -> records extension contributions for plugins, rules, tools, skills, and UI
  -> writes a BLAKE3-addressed runtime artifact
  -> routes compilation through a Rust `RuntimeCompiler` trait
  -> routes execution through a Rust `PluginExecutor` trait
  -> loads runtime artifacts at install time and stores loaded runtime handles
  -> passes serialized object/context bytes into WASM callbacks
  -> accepts returned object bytes through a minimal host ABI
  -> stores an internal manifest in a registry
  -> runs selected callbacks through the registry contract

capsem-plugin-server crate
  -> Axum HTTP wrapper over the engine crate
  -> one-shot `/plugins/install-run` route for lifecycle benchmarking
```

This is a scaffold for the production shape. It intentionally does not call
Node or `jco transpile`. The default runtime artifact remains a deterministic
placeholder, but the crate now has a Rust-only `wasmtime-wat` lane that compiles
WAT to WASM with Rust crates, writes the artifact, loads it with Wasmtime, and
calls the exported callback.

The installable unit is an extension package, not merely one plugin. A package
can contribute:

- `Plugin()` compiled to a runtime artifact.
- `Rule()` routed to IR/CEL later.
- `Tool()` registered as a callable capability later.
- `Skill()` indexed for agent workflows later.
- `UI()` registered as a user-facing extension later.

## Files

- `Cargo.toml`
- `crates/capsem-plugin-engine/src/lib.rs`
- `crates/capsem-plugin-engine/tests/install_run.rs`
- `crates/capsem-plugin-server/src/main.rs`

## Done

- Rust workspace builds.
- Engine crate has separated integration tests.
- Server crate uses Axum and depends on the engine crate.
- BLAKE3 is optimized in dev profile.
- Tests prove install writes `<blake3>.cwasm` and run loads it.
- Tests prove extension contribution metadata is carried into the internal
  manifest and unsafe contribution paths are rejected.
- Tests prove the registry uses swappable compiler/executor traits.
- Tests prove the Rust-only `wasmtime-wat` lane executes an exported WASM
  callback without Node.
- Tests prove the one-shot install-run lifecycle.
- Tests prove the minimal WASM ABI copies object/context into guest memory and
  receives output object bytes through `capsem.emit_output`.
- A release benchmark example records the current Rust-only Wasmtime ABI
  baseline before adding fuel, timeout, or compiler changes.
- A separate fuel-budgeted Wasmtime lane measures deterministic execution
  budgeting without erasing the unbudgeted baseline.

## Coverage Matrix

- Unit/contract: integration test for install/run contract and extension
- Unit/contract: integration test for install/run contract, extension
  contribution path validation, compiler/executor trait dispatch, and Wasmtime
  callback execution, install-run lifecycle, and object/context/output ABI.
- Functional: engine install/run path through placeholder and `wasmtime-wat`.
- Adversarial: unknown plugin rejection.
- E2E/HTTP: server exposes placeholder and `wasmtime-wat` modes.
- Performance: install/run responses report compile, install-load, warm-load,
  total run, and per-callback timings. `cargo run -p capsem-plugin-engine
  --release --example benchmark` records the no-HTTP Wasmtime ABI baseline.
  `CAPSEM_BENCH_ENGINE=wasmtime-wat-fuel` records the fuel-budgeted comparison.
- Missing/deferred: real TypeScript compiler, Wasmtime component ABI,
  canonical serialization, Rule/Tool/Skill/UI routing.
