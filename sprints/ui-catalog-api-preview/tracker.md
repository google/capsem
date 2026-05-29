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
  logged `ask.yes`, and rendered card/chat data bindings.
- Correction: `Weather Card` was a bad demo label because `WeatherCard` is not
  an A2UI Basic component. The live preview now shows `Card`, backed by
  `component: "Card"`, and renders it with the Preline card token pattern.
- Crate check: crates.io has `a2ui-types`, `a2ui-core`, and `a2ui-validation`
  from `applegrew/a2ui-rs`. We added `a2ui-types` and cross-check outgoing
  Rust messages against its v0.9 `ServerToClientMessage`. The crate leaves
  catalog component bodies as JSON values, so Capsem still needs its strict
  Basic component enum port for component-level type safety.
- Preline correction: rendering now appears above the serialized A2UI JSON,
  and the shell follows Preline docs/card patterns: max-width page container,
  stone canvas, blue primary tabs, semantic card tokens, card headers, and
  Svelte-owned state.
- Preline CSS correction: the preview now imports the real `preline` package
  through Tailwind v4/Vite (`@tailwindcss/vite`, `@tailwindcss/forms`,
  `@source "../../node_modules/preline"`, and the packaged theme CSS). Browser
  verification shows no Tailwind CDN and live Preline runtime tokens such as
  `--card-line`, `--primary`, and `--primary-hover`.
- Preline renderer correction: the Svelte renderer now maps A2UI fields into
  Preline recipe slots instead of recursively dumping every node as generic
  boxes. Examples implemented against the docs recipes: soft alert, simple
  card, top-border status callout, card-style tabs, pill badges, Preline
  button classes, and Svelte-owned modal shell/body/actions.
- FIXME: `docs/ui-fixme.md` now tracks the remaining UI errors: semantic-token
  leakage, generic fallback rendering, missing Preline variants/slots in the
  Rust API, and the unresolved question of whether A2UI Basic can carry enough
  recipe intent or needs a Capsem UI catalogue layer.
- Recipe metadata: preview examples now carry `{ component, variant, docsUrl }`
  from Rust. The Svelte renderer uses that metadata for supported Preline
  adapters instead of inferring the recipe from component ids.

## Coverage Ledger

- Unit/contract: `cargo test -p capsem-plugin-engine` covers upstream A2UI
  weather, modal, and chat fixture round trips plus typed helper validation and
  `a2ui-types` v0.9 server message parsing.
- Functional: `cargo test -p capsem-plugin-server`; `curl /ui/spec/demo`
  returned five examples; Rust server served `/`; `npm test` passed the
  prototype JS test lane without sweeping private upstream checkouts.
- Adversarial: Rust tests reject invented component names, non-Basic catalogs,
  and updates before create.
- E2E/VM: in-app browser preview loaded from the Rust server, confirmed the
  rendered section appears before the A2UI JSON, no Tailwind CDN is loaded, real
  Preline tokens drive rendered card/button styles, all five tabs render, and
  the ask modal opens with mapped body/actions. VM path deferred for this
  isolated prototype.
- Telemetry: deferred.
- Performance: `npm run ui:build` produced the preview bundle in ~150ms on this
  machine after the real Preline/Tailwind v4 pipeline landed; no runtime
  throughput claim in this sprint.
- Missing/deferred: live Capsem gateway/MCP integration, plugin ABI rename from
  `ui.emit` to typed `context.ui.*`, full A2UI Basic renderer coverage,
  full production typed variant/slot APIs, and renderer fixtures that assert
  exact Preline recipe strings.
