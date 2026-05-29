# Sprint: TypeScript Compiler Spike

## Tasks

- [x] Add `RuntimeCompiler` candidate example for TypeScript/componentize-js.
- [x] Add `RuntimeCompiler` candidate example for AssemblyScript.
- [x] Run cold/warm candidate benchmarks.
- [x] Record blocker: component artifact is not yet executable by current core
  module executor.
- [x] Execute smaller core-module candidate through the current Rust engine.
- [x] Consolidate compiler and runtime performance into
  `docs/performance-recap.md`.
- [ ] Decide whether AssemblyScript constraints are acceptable for security
  plugins.

## Results

Candidate: `typescript -> tsc -> jco componentize`

First run:

- source: `687` bytes
- artifact: `11,996,566` bytes
- `tsc`: `160.553041ms`
- `componentize`: `1386.198709ms`
- total: `1548.21525ms`

Warm run:

- source: `687` bytes
- artifact: `11,996,563` bytes
- `tsc`: `152.639875ms`
- `componentize`: `1309.262625ms`
- total: `1463.380083ms`

With `jco componentize --disable stdio random clocks http fetch-event`:

- source: `687` bytes
- artifact: `11,953,757` bytes
- `tsc`: `154.735459ms`
- `componentize`: `1282.526459ms`
- total: `1438.775375ms`

Final verification run after formatting:

- source: `687` bytes
- artifact: `11,953,714` bytes
- `tsc`: `159.13525ms`
- `componentize`: `1325.180125ms`
- total: `1485.788375ms`

## Notes

- This candidate is install-time plausible but heavy: roughly `1.4s` and
  `~12MB` for a tiny callback.
- Disabling optional WASI features barely changes the result.
- The output is a WebAssembly component, not the core-module ABI currently
  executed by the Rust engine.
- Treat this as a correctness/reference compiler candidate, not the preferred
  production choice yet.
- AssemblyScript is much smaller and runs through the current Rust engine:
  roughly `1.3KB`, `~390-396ms` compile, and `~0.27ms` for the single
  install/run smoke call.
- AssemblyScript is not normal JavaScript. It is a constrained TypeScript-like
  language, which is probably acceptable for security enforcement plugins if we
  provide a strong SDK and authoring diagnostics.
- Consolidated recap: `docs/performance-recap.md` now compares the compiler
  candidates against the Rust runtime and legacy Node baselines in one table.

## Side-by-Side

Final warm comparison:

| Candidate | Artifact | Compile | Size | Rust execution |
| --- | ---: | ---: | ---: | --- |
| `typescript -> tsc -> jco componentize` | component | `1568.0475ms` | `11,953,674` bytes | no |
| `assemblyscript -> asc` | core module | `395.907083ms` | `1,293` bytes | yes |

Interpretation:

- `componentize-js` is the compatibility/reference lane.
- `AssemblyScript` is the current production-shaped security lane candidate.
- The next decision is product/authoring: constrained TypeScript-like language
  versus full JavaScript semantics.

## Coverage Ledger

- Unit/contract: candidate implements the public `RuntimeCompiler` trait in
  `crates/capsem-plugin-engine/examples/compiler_spike.rs`.
- Functional: release example compiles TypeScript to a WebAssembly component and
  AssemblyScript to a core module that runs through the Rust engine.
- Adversarial: deferred.
- E2E/runtime: deferred because component execution is not wired.
- Telemetry: example prints structured JSON timings.
- Performance: cold/warm compile timings and artifact sizes recorded above.
- Missing/deferred: Wasmtime component execution, smaller Rust-owned compiler
  candidate beyond AssemblyScript, timeout enforcement around external compiler
  command.

## Verification

- `cargo test` passed with 11 Rust integration tests.
- `cargo run -p capsem-plugin-engine --release --example compiler_spike`
  produced the final verification result above.
