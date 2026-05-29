# Sprint: Plugin Install Registry

## Goal

Change the prototype lifecycle from request-time compilation to Capsem-shaped
plugin installation:

```text
source + public manifest
  -> validate
  -> compile at install/load time
  -> save artifacts/<artifact_blake3>.wasm
  -> keep internal manifest in memory
  -> run callbacks by plugin id
```

The runtime path must not accept raw source. Source is inspected and compiled
once during installation.

## Decisions

- Artifact identity uses canonical `blake3:<64 lowercase hex>`.
- The prototype uses native `@napi-rs/blake-hash`; no WASM/JS BLAKE3 fallback.
- Rust landing must preserve Capsem's existing rule:
  `[profile.dev.package.blake3] opt-level = 3`.
- The internal manifest is in-memory for this spike. Disk persistence beyond
  `artifacts/<hash>.wasm` is deferred.
- The Node prototype still transpiles once after install only because Node
  needs a JS wrapper. Production Rust should load the `.wasm` component
  directly.

## Files

- `src/types.ts`
- `src/component-runner.ts`
- `src/server.ts`
- `test/compile-run.test.ts`
- `scripts/bench.ts`
- `package.json`
- `package-lock.json`

## Done

- `POST /plugins/install` compiles source and writes `artifacts/<hash>.wasm`.
- `POST /plugins/run` runs callbacks from the in-memory plugin registry.
- `/` UI has separate install and run actions.
- Tests cover install, run, artifact naming, and legacy compile-run behavior.
- Typecheck and test suite pass.

## Coverage Matrix

- Unit/contract: request validation, canonical BLAKE3 shape, manifest shape.
- Functional: install then run callbacks from registry.
- Adversarial: missing plugin id, unsupported callback, compile/run failures.
- Performance: repeated run after install excludes compile cost.
- E2E/VM: deferred; this remains isolated from Capsem.
