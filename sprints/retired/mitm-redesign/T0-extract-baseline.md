# T0: extract-and-re-baseline

**Status:** Not Started
**Depends on:** observability sprint W2 (sequencing options in MASTER.md)
**Blocks:** T1, T2, T3, T4

## Goal

Reorganize the existing 2847-line `crates/capsem-core/src/net/mitm_proxy.rs` into a layered module tree with one parser per file. **Zero behavior change.** Every existing test passes against the new layout. Pre-rewrite performance baselines are committed for later regression gates.

## Deliverables

- `crates/capsem-core/src/net/mitm/` skeleton: `mod.rs`, `listener.rs`, `connection.rs`, `tls.rs`, `http.rs`, `events.rs`, `body.rs`, `upstream.rs`, `telemetry.rs`, `protocol.rs`, `cert_authority.rs` (moved from `net/`), `metrics.rs` (declarations only, no recorder).
- `crates/capsem-core/src/net/parsers/`: `sse_parser.rs` + `sse_parser/{tests.rs, fixtures/}`, `jsonrpc_parser.rs`, `dns_parser.rs` placeholders.
- `crates/capsem-core/src/net/interpreters/`: `anthropic_interpreter.rs`, `openai_interpreter.rs`, `google_interpreter.rs`, `mcp_interpreter.rs` each with sibling `tests.rs` + `fixtures/`.
- AI parser tests reorganized from inline `#[cfg(test)] mod tests { ... }` blocks (where they exist) into sibling `tests.rs` files per CLAUDE.md.
- `crates/capsem-core/benches/` with criterion stubs for `mitm_pipeline.rs`, `mitm_hook_dispatch.rs`, parser microbenches; `criterion = "0.5"` added as dev-dep.
- `benchmarks/mitm-load/` with a baseline run of the existing proxy under concurrency 1/10/50/200 captured and committed as `baseline.json`.
- `crates/capsem-core/benches/baselines/` with criterion outputs from current code.
- `init_telemetry` stub (see MASTER.md sequencing note option b).

## Acceptance

- `cargo test -p capsem-core net` — all existing tests pass.
- `cargo test --workspace` — green.
- `just test` — green.
- `cargo bench -p capsem-core` — runs end-to-end (baselines saved).
- `capsem-bench mitm-load --concurrency 1,10,50,200` — runs end-to-end and produces `benchmarks/mitm-load/baseline.json`.
- `wc -l` on every new file is below the level of the original monolith — no >800-line files except the inevitable ones.
- No public-API changes outside `capsem-core::net::*`.

## Commit shape

Three commits expected:
1. `chore(mitm): reorganize net/mitm module tree (no behavior change)` — module split + cert_authority move + AI parser file split + tests.rs reorg.
2. `bench(mitm): add criterion + capsem-bench mitm-load baselines` — bench harness + baseline JSON.
3. `chore(mitm): wire init_telemetry stub + metrics declarations` — observability seam ready for T1.

## Notes

- Existing `ai_traffic/sse.rs` body is moved into `parsers/sse_parser.rs` byte-for-byte; only the path changes.
- Existing `ai_traffic/{anthropic,openai,google}.rs` move into `interpreters/<provider>_interpreter.rs`.
- `cert_authority.rs` moves from `net/` to `net/mitm/`. Only consumer is the proxy itself.
- `metrics.rs` registers counter/histogram names from § Metrics in the plan but does not yet emit values — wiring happens in T1.
- The criterion baselines run against the un-redesigned code so T5's regression gate has a real reference. Without this, "no regression" is unverifiable.
