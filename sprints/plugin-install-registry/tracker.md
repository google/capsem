# Sprint: Plugin Install Registry

## Tasks

- [x] Write sprint plan.
- [x] Add native BLAKE3 artifact identity.
- [x] Add install-time compiler/registry.
- [x] Add run-by-plugin-id path.
- [x] Update UI for install then run.
- [x] Update tests and benchmark.
- [x] Run verification.

## Notes

- Capsem already forces optimized Rust BLAKE3 in dev builds:
  `[profile.dev.package.blake3] opt-level = 3`. Keep that when this moves to
  Rust.
- The prototype dependency is `@napi-rs/blake-hash`, which loads native NAPI
  bindings for the current platform. Do not replace with a WASM/JS fallback.
- `installPlugin()` writes `artifacts/<artifact blake3 hex>.wasm` and stores
  source/artifact/manifest BLAKE3 hashes in the internal manifest.
- `runPlugin()` accepts only `plugin_id`, object, context, selected callbacks,
  and run count. Raw source is no longer on the runtime path.
- Verification on 2026-05-21: `npm run typecheck` passed; `npm test` passed
  with 10 tests; `N=2 npm run bench` passed. Install wrote
  `artifacts/82d1d52852fd43f155ebf667d251c34bface958a326c9323fbba7ddbadac65a0.wasm`.
- Benchmark sample: install compile 1405 ms, install transpile 3051 ms, then
  registry runs instantiate in 37-39 ms and execute 3x2 callbacks in 14.7-15.1 ms.

## Coverage Ledger

- Unit/contract: `npm test` covers canonical BLAKE3 artifact naming, manifest
  shape, and callback declarations.
- Functional: `npm test` covers install then run from registry, plus legacy
  compile-run compatibility.
- Adversarial: `npm test` covers oversized source, syntax errors, callback
  throw, invalid JSON, and timeouts on the compatibility path. Missing plugin id
  rejection is implemented but not directly asserted yet.
- E2E/VM: deferred.
- Telemetry: structured logs stay local stdout for this spike.
- Performance: `npm run bench` now installs once, then runs from registry by
  plugin id.
- Missing/deferred: persisted internal manifest, Rust Wasmtime execution,
  out-of-process plugin host.
