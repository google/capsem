# Capsem Element Foundation Sprint

## Goal

Create a reusable Web Component foundation for generated UI islands. Charts,
rich blocks, and future plugin/model UI should share one host pattern instead
of each feature inventing its own Shadow DOM, lifecycle, and event bridge.

## Decisions

- The base custom element tag is `<capsem-elt>`.
- The TypeScript base class is `CapsemElement<TSpec>`.
- Shadow DOM is open by default so the host app can inspect and test output.
- Custom properties cross the Shadow DOM boundary, so Preline semantic tokens
  remain the theme source of truth.
- Component events leave the shadow boundary through composed `capsem:*`
  custom events.
- This is UI isolation, not a security boundary. Rust contracts and sandboxed
  plugin execution remain the security boundary.

## Files

- `ui-preview/src/elements/capsem-elt.ts`
- `ui-preview/src/elements/index.ts`
- `ui-preview/src/main.ts`
- `test/capsem-element.test.ts`
- `CHANGELOG.md`

## Done

- `<capsem-elt>` is registered when the preview app starts.
- The base class owns Shadow DOM setup, spec updates, render scheduling,
  cleanup callbacks, and event emission.
- Tests guard the foundation pattern so specialized elements reuse it.
- Browser verification creates a `<capsem-elt>`, sets a spec, and observes
  shadow output plus composed event emission.

## Proof Matrix

- Unit/contract: Vitest source contract for element foundation.
- Frontend: `npm run ui:build`, `npm run typecheck`, `npm test -- --run`.
- E2E/UI: browser DevTools verifies `<capsem-elt>` behavior.
- Adversarial: base element renders a typed error state for render failures.
- Telemetry: not applicable.
- Performance: not applicable.
