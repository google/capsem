# Sprint: Hyper A2UI Contract Lane

## Tasks

- [x] Plan sprint and proof matrix.
- [x] Add Hyper-only Rust crate.
- [x] Add `POST /a2ui/render` contract endpoint.
- [x] Return A2UI messages plus recipe metadata and conformance status.
- [x] Prove alert tone survives.
- [x] Prove card link label/href survive.
- [x] Add Hyper E2E tests over TCP.
- [x] Add TypeScript client/decoder library.
- [x] Add TypeScript tests.
- [x] Run proof gates.
- [x] Changelog.
- [x] Commit.

## Notes

- This lane intentionally does not use Axum or Svelte. It is the protocol
  correctness lane.
- `capsem-a2ui-hyper` exposes `POST /a2ui/render` and validates returned A2UI
  with the Rust conformance helper.
- `ui.alert` now preserves `tone` in recipe metadata.
- `ui.card` now preserves `linkLabel` and `linkHref` in recipe metadata.
- The Svelte renderer consumes those contract fields so the chat page no
  longer hardcodes neutral alerts or `Card link`.
- The isolated UI theme now defines a `warning` semantic token so
  `bg-warning/10` is real in this prototype.

## Coverage Ledger

- Unit/contract:
  - `cargo test -p capsem-plugin-engine -- --nocapture`
  - `npm test -- --run`
  - `npm run typecheck`
- Functional:
  - `cargo test -p capsem-a2ui-hyper -- --nocapture`
  - `npm run ui:build`
  - Browser smoke on `/chat` confirmed warning token class renders with
    non-transparent warning background.
  - Browser smoke confirmed card link text `visit homepage` and href
    `https://gemini.google.com/`.
- Adversarial:
  - Hyper E2E rejects bad paths and bad JSON.
  - TS parser rejects malformed response shapes.
- E2E/browser: not required for this sprint.
- Telemetry: deferred.
- Performance: deferred.
- Missing/deferred: generated Rust/TS from schema, full catalogue coverage,
  and production Capsem gateway integration.
