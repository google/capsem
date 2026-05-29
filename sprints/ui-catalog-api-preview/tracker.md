# Sprint: UI Catalog API Preview

## Tasks

- [x] Plan sprint and proof matrix.
- [x] Implement Rust typed A2UI Basic/API models.
- [x] Add Rust validation and upstream fixture round-trip tests.
- [x] Add Rust server preview endpoint.
- [x] Add Svelte/Preline preview renderer.
- [x] Serve preview from Rust server.
- [x] Verify tests/build/browser.
- [x] Update docs with plugin-system feed-in.
- [x] Changelog.
- [ ] Commit.

## Notes

- The sprint deliberately keeps JSON as transport only. Authoring and parsing
  are Rust structs/enums/builders.
- The baseline catalog is A2UI v0.9 Basic. Capsem APIs are typed helpers that
  emit A2UI Basic messages.
- Chat UI is a baseline surface convention over the same A2UI message engine,
  not a second format.
- Svelte must not receive plugin-authored HTML/classes. It receives typed,
  validated component operations.
- Browser verification loaded `http://127.0.0.1:8787/`, opened the ask modal,
  logged `ask.yes`, and rendered weather/chat data bindings.
- Correction: `Weather Card` was a bad demo label because `WeatherCard` is not
  an A2UI Basic component. The live preview now shows `Card`, backed by
  `component: "Card"`, and renders it with the Preline card token pattern.
- Crate check: crates.io has `a2ui-types`, `a2ui-core`, and `a2ui-validation`
  from `applegrew/a2ui-rs`. We added `a2ui-types` and cross-check outgoing
  Rust messages against its v0.9 `ServerToClientMessage`. The crate leaves
  catalog component bodies as JSON values, so Capsem still needs its strict
  Basic component enum port for component-level type safety.

## Coverage Ledger

- Unit/contract: `cargo test -p capsem-plugin-engine` covers A2UI weather,
  modal, and chat fixture round trips plus typed helper validation and
  `a2ui-types` v0.9 server message parsing.
- Functional: `cargo test -p capsem-plugin-server`; `curl /ui/spec/demo`
  returned five examples; Rust server served `/`; `npm test` passed the
  prototype JS test lane without sweeping private upstream checkouts.
- Adversarial: Rust tests reject invented component names, non-Basic catalogs,
  and updates before create.
- E2E/VM: in-app browser preview loaded from the Rust server, opened ask modal,
  clicked allow, and rendered weather/chat bindings. VM path deferred for this
  isolated prototype.
- Telemetry: deferred.
- Performance: `npm run ui:build` produced the preview bundle in ~300ms on this
  machine; no runtime throughput claim in this sprint.
- Missing/deferred: live Capsem gateway/MCP integration, plugin ABI rename from
  `ui.emit` to typed `context.ui.*`, full A2UI Basic renderer coverage.
