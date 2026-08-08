# T1: pipeline-and-hook-traits

**Status:** Not Started
**Depends on:** T0
**Blocks:** T2, T3, T4

## Goal

Introduce the single `Hook` trait + `Event<'_>` enum + `EventMask` + `HookCtx::emit()`. Rewire existing policy / decompression / AI parsing / telemetry as `Hook` impls. Lock the logging contract: every hook execution is in an `#[instrument(target = "mitm.<area>")]` span recording `decision`. Every counter/histogram from the plan's § Metrics fires from real call sites.

## Deliverables

- `crates/capsem-core/src/net/mitm/hooks.rs` — `Hook` trait, `EventMask`, `HookCtx`, `HookOutcome`, `StopAction`.
- `crates/capsem-core/src/net/mitm/events.rs` — `Event<'a>` enum with L1/L2/L3 ladder; cycle prevention by event-kind layering.
- `crates/capsem-core/src/net/mitm/pipeline.rs` — ordered dispatch, `ctx.emit()` recursion, priority ordering, per-connection slot map for hook carry-over state.
- `PolicyHook`, `DecompressionHook`, `TelemetryHook`, `<Provider>InterpreterHook`, `SseParserHook` impls — the existing logic, expressed through the new trait.
- `RawRequestChunk` / `RawResponseChunk` carry `&mut Bytes` per the streaming-body-mutation contract; the `test_chunk_carry_over` test locks the surface for the future security engine.
- `metrics::counter!` / `histogram!` calls wired into every seam.
- `mitm_proxy.rs` shrinks to a thin facade re-exporting from `mitm/` (or is deleted if all consumers migrate).

## Acceptance

- All T0 tests still pass.
- New tests: hook dispatch order, cycle prevention (L3 cannot re-emit L2/L1), `EventMask` filtering correctness, `ctx.emit()` recursion depth bounds, `test_chunk_carry_over`.
- An in-process `metrics_runtime` recorder captures every documented counter/histogram firing during a smoke run.
- `RUST_LOG=capsem::net::mitm=debug` on a smoke session shows every hook invocation with `decision`.
- `cargo bench -p capsem-core --bench mitm_pipeline` shows empty-hook overhead < 100µs/req (perf budget from plan).
- Behavior parity vs T0 baseline: no regression in `mitm-load` p99 at any concurrency level.

## Commit shape

Three expected commits:
1. `feat(mitm): single Hook trait + Event ladder + pipeline dispatch` — trait, dispatch, cycle prevention, unit tests.
2. `feat(mitm): rewire policy/decompression/AI parsing/telemetry as hooks` — the existing logic flowing through the new pipeline.
3. `feat(mitm): wire metrics counters + tracing decision contract` — every seam emits.

## Notes

- Hook ordering policy: registration order with explicit `priority: i32` overrides. Lean stable.
- Per-domain hook filtering punts to T2 (where it actually matters for plain-HTTP host-header policy).
- `HookCtx::state::<T>()` slot mechanism is the load-bearing surface for the future security engine — the test must exercise insert / read / mutate / drop within one connection.
