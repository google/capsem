# Sprint: Chat Shell UI Surface

## Tasks

- [x] Plan sprint and proof matrix.
- [x] Convert primary preview into chat shell.
- [x] Render selected A2UI surface inside assistant message.
- [x] Preserve inspector panes.
- [x] Post a visible authored card draft.
- [x] Run proof gates.
- [x] Browser verification.
- [x] Changelog.
- [x] Commit.

## Notes

- The chat shell is a product-placement proof, not a real chat backend.
- The rendered component still comes from the Rust UI tool result.
- Posted an authored `ui.card` draft with title `Chat shell is live`; the
  browser rendered it inside the assistant message.

## Coverage Ledger

- Unit/contract:
  - `cargo test -p capsem-plugin-engine -- --nocapture`
  - `cargo test -p capsem-plugin-server -- --nocapture`
  - `npm test -- --run`
- Functional:
  - `npm run ui:build`
  - `POST /ui/tools/run` with `ui.card` returned `ok == true`,
    `recipe.component == "card"`, and `componentCount == 7`.
- E2E/browser:
  - Chrome loaded `http://127.0.0.1:8787/?v=chat-shell-card` and verified
    `Capsem Chat`, `Chat shell is live`, and the authored description inside
    the assistant message.
- Telemetry: deferred.
- Performance: deferred.
- Missing/deferred: actual chat transport, prompt parser, streaming events,
  and multi-message component history.
