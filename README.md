# Capsem Plugin Prototype

This repository is the isolated development lane for the Capsem extension
engine. It is intentionally separate from Capsem proper so the plugin system can
be developed, reviewed, benchmarked, and audited as an independent crate before
Capsem consumes it.

## Audit Boundary

The Rust crates are the product surface:

- `crates/capsem-plugin-engine`: manifest validation, BLAKE3 identity,
  extension contribution metadata, runtime compiler/executor traits, runtime
  artifact bookkeeping, Wasmtime callback execution, and callback tracing.
- `crates/capsem-plugin-server`: Axum wrapper for HTTP smoke testing the engine
  crate.

The TypeScript/Node files are retained as spike history only. They are not the
target runtime architecture and should not be used to define production
security boundaries.

## Rust-Only WASM Lane

The default engine remains a deterministic placeholder while the TypeScript
compiler is designed. For execution proof, the crate also exposes:

```rust
PluginRegistry::wasmtime_wat(artifact_dir)
```

That mode compiles WAT source to WASM using Rust crates, writes the runtime
artifact by BLAKE3, loads it with Wasmtime, calls an exported callback such as
`onModelOutput`, and returns the traced result. WAT is not the authoring story;
it is the temporary source format that proves the Rust-owned WASM execution
path.

The current Wasmtime ABI shape is intentionally small:

```text
callback(object_ptr, object_len, context_ptr, context_len) -> i32
```

The host writes serialized object/context JSON into guest memory. The guest can
return an output object by calling:

```text
capsem.emit_output(output_ptr, output_len) -> i32
```

Only the emitted output object crosses back. Context is copied in and never
returned. The prototype currently caps object, context, and emitted output JSON
at one megabyte each.

## Design Discipline

- Capsem proper is a future consumer, not the implementation workspace.
- Source packages are audit/rebuild artifacts.
- Runtime artifacts are execution artifacts.
- Intermediate portable WASM is disposable unless retained for debugging or
  provenance.
- All identity uses BLAKE3.
- Plugin callbacks receive an object copy and a context copy. Only the object
  can cross back.

## Current Verification

The consolidated performance matrix is in
[`docs/performance-recap.md`](docs/performance-recap.md).
Compiler-path measurements are in
[`docs/compiler-benchmark.md`](docs/compiler-benchmark.md).

```bash
cargo test
```

The current Rust spike proves install/run registry behavior, BLAKE3-addressed
runtime artifact paths, extension contribution metadata, compiler/executor trait
dispatch, Rust-only WASM execution, and basic rejection of unsafe contribution
paths.

For one-shot lifecycle measurement, run the server in the Rust-only WASM mode:

```bash
CAPSEM_PLUGIN_ENGINE=wasmtime-wat cargo run -p capsem-plugin-server
```

Then post to `POST /plugins/install-run` with source, manifest, object, context,
callback list, and run count. The response includes install compile time,
install load time, warm run load time, per-call timings, BLAKE3 hashes, and the
final object.

For a repeatable local baseline without HTTP overhead:

```bash
cargo run -p capsem-plugin-engine --release --example benchmark
```

Optional knobs:

```bash
CAPSEM_BENCH_REPEATS=1000 CAPSEM_BENCH_BATCH_RUNS=25 \
  cargo run -p capsem-plugin-engine --release --example benchmark
```

Compare the fuel-budgeted lane:

```bash
CAPSEM_BENCH_ENGINE=wasmtime-wat-fuel \
CAPSEM_BENCH_REPEATS=1000 CAPSEM_BENCH_BATCH_RUNS=25 \
  cargo run -p capsem-plugin-engine --release --example benchmark
```

The budgeted lane enforces deterministic fuel plus memory/table/store limits.

## Compiler Spike

The TypeScript compiler spike lives behind the public `RuntimeCompiler` trait:

```bash
cargo run -p capsem-plugin-engine --release --example compiler_spike
```

Current candidate:

```text
TypeScript -> tsc -> jco componentize -> WebAssembly component
AssemblyScript -> asc -> WebAssembly core module
```

`componentize-js` is a compatibility/reference lane and currently produces a
component artifact that is not executed by the core-module executor.
AssemblyScript is constrained TypeScript-like authoring and produces a small
core module that runs through the current Rust ABI.
