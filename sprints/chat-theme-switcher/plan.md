# Chat Theme Switcher Sprint

## Goal

Add a visible `/chat` theme switcher and dark-mode toggle so generated A2UI components can be checked against Preline semantic tokens.

## Decisions

- Theme switching is local to the preview shell and uses Preline's `data-theme` values on `<html>`.
- Dark mode uses the existing `.dark` class on `<html>`.
- Import the packaged Preline theme CSS files so the switcher actually changes tokens.
- Tighten the generated UI renderer away from raw palette classes where the dark-mode check would otherwise lie.

## Files

- `ui-preview/src/ChatPage.svelte`
- `ui-preview/src/A2Node.svelte`
- `ui-preview/src/a2ui.ts`
- `ui-preview/src/style.css`
- `test/ui-semantic-tokens.test.ts`
- `CHANGELOG.md`

## Done

- `/chat` exposes a theme select and dark-mode switch.
- Choices persist in `localStorage`.
- The generated A2UI renderer uses semantic tokens for text, buttons, and modal shell.
- A frontend test catches raw palette utilities in the chat renderer path.
- Browser verification proves theme/dark controls update `<html>` and the generated table remains visible.

## Proof Matrix

- Unit/contract: Vitest semantic-token scan.
- Functional: Svelte build and typecheck.
- E2E/UI: Browser interaction on `/chat`.
- Adversarial: semantic-token scan rejects raw Tailwind palette utilities.
- Telemetry: not relevant in isolated preview.
- Performance: not relevant.
