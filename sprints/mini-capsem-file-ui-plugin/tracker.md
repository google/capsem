# Sprint: Mini-Capsem File UI Plugin

## Tasks

- [x] Create sprint plan.
- [x] Read real Capsem file-event, fs, gateway, and UI boundaries.
- [x] Add fs and UI block ABI imports to the Rust Wasmtime executor.
- [x] Add fetch ABI import to the Rust Wasmtime executor.
- [x] Add fluent AssemblyScript plugin source.
- [x] Add mini-Capsem acceptance test.
- [x] Update overview docs.
- [x] Run verification.

## Notes

- Capsem file events are currently written as `fs_events` rows from host-side
  VirtioFS monitoring.
- `.git` is excluded from the monitor, so git context must come from explicit
  fs read capability, not passive file events.
- The gateway already has a broadcast event path and the UI already has remote
  boundaries such as iframe `postMessage`; the prototype should emit UI blocks
  as data, not touch UI directly.
- The plugin callback must return the file operation object. UI mutation is a
  side channel on `context.ui`, captured by the host and routed later.
- Network access uses Chrome-extension vocabulary: manifest capability
  `fetch`, plugin surface `context.fetch(url)`, host-enforced URL policy, and
  traceable host execution.
- Authoring must stay as close to JavaScript and Chrome extension mental models
  as possible. Little Codex, future agents, and human plugin authors need
  familiar analogies: manifest permissions, callback hooks, `fetch`, and narrow
  host-provided APIs.
- Focused acceptance test passes: the AssemblyScript plugin compiles, handles
  `on_file_create`, reads `workspace/.git/HEAD` and `workspace/.git/config`
  through `context.fs.read`, calls `context.fetch(...)` for GitHub repo
  statistics, returns the `FileCreate` object, updates the
  `workspace.context` side panel through `context.ui.sidePanel(...).replace`,
  and fails when `fs.read` or `fetch` is not declared.
- Deterministic fetch proof uses `context.fetch.responses` fixture data so the
  acceptance test does not depend on GitHub availability.
- Live proof exists as an ignored test that hits `https://api.github.com` and
  verifies stats are emitted.
- Full verification: `cargo test` passed with 14 integration tests total; the
  mini-Capsem suite passed with 3 default tests and 1 ignored live GitHub test.
  `cargo test --examples` passed. The ignored live test also passed when run
  explicitly.

## Coverage Ledger

- Unit/contract: host ABI enforces declared `fs.read` and `fetch` capabilities.
- Functional: `mini_capsem_file_ui.rs` compiles and runs the real
  `git_context_badge.ts` plugin.
- Adversarial: missing `fs.read` and `fetch` capabilities fail the callback.
- E2E/VM or integration: mini-Capsem harness exercises registry install/run and
  Wasmtime.
- Telemetry/observability: emitted UI blocks are captured on the returned object
  as `ui_mutations` for prototype inspection.
- Network: deterministic `fetch.responses` fixture covers normal path; ignored
  live GitHub test covers host `fetch` wiring.
- Performance: deferred.
- Missing/deferred: actual Capsem gateway route and frontend renderer.
