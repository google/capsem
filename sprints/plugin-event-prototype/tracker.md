# Sprint: Plugin Event Prototype

## Tasks

- [x] Create isolated prototype workspace and git branch.
- [x] Capture plugin event/context/manifest/ABI contract.
- [ ] Choose concrete prototype plugins.
- [ ] Scaffold standalone TypeScript package.
- [ ] Implement event card helpers.
- [ ] Implement security event card helpers.
- [ ] Implement UI event card helpers.
- [ ] Implement immutable security/UI context copies.
- [ ] Implement plugin manifest validation.
- [ ] Implement simulated plugin-host/WASM boundary validation.
- [ ] Model out-of-process invocation serialization.
- [ ] Implement capability-gated HTTPS ABI stub.
- [ ] Implement capability-gated model ABI stub for dynamic enforcement.
- [ ] Implement invocation log records.
- [ ] Add tests for rewrite and throttle decisions.
- [ ] Add tests for UI callbacks and declarative UI mutations.
- [ ] Add tests for context immutability and non-return.
- [ ] Add tests for HTTPS capability gating.
- [ ] Add tests for model ABI capability gating and bounded schema result.
- [ ] Add tests for manifest callback declarations.
- [ ] Run verification.

## Notes

- Course correction: context is copied into WASM but never returned. Only event crosses back.
- WASM boundary must validate returned event shape before host acceptance.
- WASM should not run in the main Capsem process. Default model is one plugin-host subprocess per VM/session.
- Inside the plugin-host process, each plugin gets its own Wasmtime instance/store. Stricter one-process-per-plugin isolation is deferred but should remain compatible.
- Since plugin execution is out-of-process, event/context/result/log envelopes must be serialized. Prefer a canonical envelope so hashes and replay are stable.
- Decision actions include `throttle`, not only allow/ask/block/rewrite.
- Rewrite is represented as declarative mutations on the event.
- HTTPS access is not ambient. It is an explicit ABI capability declared in the manifest and enabled by host configuration.
- Model access for dynamic enforcement is also an explicit ABI capability. Host owns credentials/provider selection; plugins only submit bounded requests through `context.abi.model.ask`.
- Lifecycle callbacks such as `on_vm_start()` are in scope.
- Context should expose MCP servers/tools, skills, profile/plugin configuration, and relevant trace/history snapshots.
- The plugin engine should support at least two callback worlds: `SecurityCallback()` for `SecurityEvent`, and `UICallback()` for `UIEvent`.
- UI callbacks mutate UI event cards with declarative UI mutations. They do not share the `SecurityEvent` object.
- We want a Chrome-manifest-like `capsem_plugin.json` with entrypoints, callbacks, contributions, and capabilities.

## Coverage Ledger

- Unit/contract: pending TypeScript tests for security event validation, UI event validation, context immutability, manifest validation, HTTPS/model capability gating, serialization envelope, logging.
- Functional: pending runtime harness tests for simulated host -> plugin-host -> plugin callback -> validated event return.
- Adversarial: pending malformed returned security/UI event, illegal mutation path, undeclared HTTPS/model capability, oversized model request/result, context mutation attempt.
- E2E/VM: deferred; prototype is intentionally independent of Capsem.
- Telemetry: pending structured invocation log assertions; Capsem DB wiring deferred.
- Performance: pending duration field assertion only; real fuel/memory/timeout metrics deferred.
- Missing/deferred: real WASM runtime, signing, marketplace install flow, Capsem hook integration.
