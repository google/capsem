# Sprint: TypeScript Compiler Spike

## Goal

Evaluate TypeScript-family compiler candidates behind the public
`RuntimeCompiler` trait without changing the proven Rust/Wasmtime runtime wall.

The current runtime baseline is strong:

- WAT to Wasmtime core module.
- Object/context JSON copied into guest memory.
- Output object emitted through host ABI.
- Fuel and memory limits in the budgeted lane.
- Release throughput around `58k-60k` callbacks/sec for the tiny ABI fixture.

This sprint asks whether TypeScript authoring can produce a runtime artifact
with acceptable install-time cost, artifact size, and future execution shape.

## Rules

- No Capsem integration.
- No Node in the runtime path.
- External toolchains are allowed only as compiler candidates and must be
  labeled as such.
- Every candidate must report compile timing and artifact size.
- Do not claim execution unless the artifact is actually executed through the
  Rust engine.

## Candidate 1: TypeScript -> tsc -> jco/componentize-js

This candidate is implemented as an example that implements `RuntimeCompiler`
outside the engine crate:

```bash
cargo run -p capsem-plugin-engine --release --example compiler_spike
```

It uses the already-installed local spike dependencies:

- `node_modules/.bin/tsc`
- `node_modules/.bin/jco componentize`

Output artifact kind:

```text
componentize-js-component
```

This artifact is a WebAssembly component. The current Rust executor handles
core modules, so execution is not claimed by this candidate yet.

## Candidate 2: AssemblyScript -> asc

This candidate is implemented in the same comparison example:

```bash
cargo run -p capsem-plugin-engine --release --example compiler_spike
```

It uses:

- `node_modules/.bin/asc`
- the existing Rust core-module executor
- the existing object/context/output ABI

Output artifact kind:

```text
wasmtime-core-module
```

This candidate is a constrained TypeScript-like language, not normal
JavaScript. Its advantage is that it emits a tiny core module that runs through
the current Rust security wall.

## Done

- A compiler candidate implements `RuntimeCompiler`.
- Compiler candidates compile TypeScript-family source to WASM-family
  artifacts.
- Candidates report phase timings and artifact size.
- At least one candidate executes through the Rust engine.
- Results are recorded in `tracker.md`.

## Missing / Next

- Decide whether constrained AssemblyScript authoring is acceptable for
  security plugins.
- Execute component artifacts through Wasmtime component APIs only if we still
  need the full JavaScript compatibility lane.
- Spike Javy codegen only if we need more JavaScript semantics than
  AssemblyScript provides and can tolerate a larger runtime surface.
