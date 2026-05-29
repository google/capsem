# Sprint: JS to WASM Component Spike

## Tasks

- [x] Define no-debt spike scope.
- [x] Scaffold local HTTP service.
- [x] Add fixed WIT world.
- [x] Wire `jco componentize` / `componentize-js` compile step.
- [x] Wire component execution step.
- [x] Return result object and timings from `POST /compile-run`.
- [x] Add request/output size limits.
- [x] Add compile/run timeouts.
- [x] Add temp directory isolation and cleanup.
- [x] Add structured per-request log record.
- [x] Add success-path tests.
- [x] Add compile failure test.
- [x] Add run failure test.
- [x] Add invalid JSON result test.
- [x] Add timeout test.
- [x] Add benchmark command/test.
- [x] Extend fixed WIT world to two callbacks: `invoke` and `inspect`.
- [x] Add callback selection and bounded repeated runs.
- [x] Add per-callback trace records with iteration, timing, result, and result size.
- [x] Update `/` HTML interface for repeated runs and dual-callback execution.
- [x] Run verification.

## Notes

- Single lane only: JS module + WIT + componentize-js/jco + WASM component execution.
- Do not add Javy or any alternate runtime path in this sprint.
- The HTTP API is intentionally simple: source, object, context in; result and timings out.
- The spike measures toolchain viability. Object algebra, manifest, LSP, and Capsem integration come later.
- Discovery: thrown JavaScript errors cross the component/transpiled boundary as `RuntimeError: unreachable`; the original JS error message is not preserved by this path.
- Discovery: `npx` from an isolated temp build directory resolved a dependency-confusion placeholder for `jco`; the runner now uses the repo-local `node_modules/.bin/jco` explicitly.
- Benchmark on 2026-05-21 (`N=3 npm run bench`): compile 1507-1775 ms, transpile 3121-3158 ms, instantiate 36-45 ms, run 12-14 ms, generated component about 11.99 MB.
- Delta: the fixed WIT now requires both `invoke` and `inspect`, and the runner imports the transpiled component once before executing a bounded callback matrix.
- Verification on 2026-05-21: `npm run typecheck` passed; `npm test` passed with 9 tests including two-callback repeated run tracing.
- Dual-callback benchmark on 2026-05-21 (`N=2 npm run bench`, 3 runs x 2 callbacks): compile 1178-1199 ms, transpile 2864-2875 ms, instantiate 30 ms, total run 11.7-11.8 ms, generated component about 11.44 MB. First callback pays about 11 ms; subsequent calls are about 0.06-0.37 ms.

## Coverage Ledger

- Unit/contract: `npm test` covers request validation, response schema, phase errors, timing fields, function selection, repeated-run trace shape.
- Functional: `npm test` covers two callbacks packed into one WASM component, repeated invocation after one import, compile-run success with object/context, and HTTP `POST /`.
- Adversarial: `npm test` covers syntax error, callback throw, invalid JSON, timeout, and oversized source. Oversized output is implemented but not directly asserted yet.
- E2E/VM: deferred by design.
- Telemetry: structured per-request log is emitted; log assertion deferred.
- Performance: `N=3 npm run bench` records repeated compile/transpile/import/callback timing and artifact size for `invoke` + `inspect` with three runs each.
- Missing/deferred: real Capsem integration, real plugin host process, full object model, manifest, host ABI, signing, marketplace.
