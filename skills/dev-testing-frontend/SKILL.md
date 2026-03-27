---
name: dev-testing-frontend
description: Testing the Capsem frontend (Astro 5 + Svelte 5 + Tailwind v4 + DaisyUI v5). Use when writing frontend tests, running type checks, debugging UI issues, or doing visual verification with Chrome DevTools MCP. Covers vitest, svelte-check, astro check, mock mode, and systematic visual verification workflow.
---

# Frontend Testing

## Stack

Astro 5 + Svelte 5 + Tailwind v4 + DaisyUI v5 + LayerChart v2.

## Running tests

```bash
cd frontend
pnpm run check              # astro check + svelte-check (type errors)
npx vitest run --coverage   # Unit tests with coverage
pnpm run build              # Production build (catches bundling issues dev misses)
```

All three run as part of `just test`. The production build is important -- Tailwind v4's Vite plugin can miss `client:only` components in the SSR module graph, so `@source` directives in `global.css` must explicitly include `.svelte` and `.ts` files.

## Test files

Tests live in `frontend/src/lib/__tests__/`. Use vitest with standard patterns:

```ts
import { describe, it, expect } from 'vitest';
```

## Mock mode

When `window.__TAURI_INTERNALS__` is absent (browser via `just ui`), `api.ts` auto-switches all IPC calls to return fake data from `mock.ts`. Mock includes: VM state, network events, settings tree, state timeline, terminal banner. All views are fully functional with mock data.

This means you can test the full UI without a VM by running `just ui`.

## Visual verification with Chrome DevTools MCP

For systematic visual testing when `just ui` is running:

### Quick health check
1. `navigate_page` to `http://localhost:5173`
2. `list_console_messages` types=["error","warn"] -- expect zero
3. `take_screenshot` fullPage=true -- verify page renders

### Full walkthrough
1. Navigate to each view via sidebar (Terminal, Sessions, Network, Settings)
2. Screenshot each view in both light and dark themes
3. Check console for new errors after each navigation

### Settings view specifics
Click through every section (AI Providers, Package Registries, Search, Guest Environment, Network, VM, Appearance). Verify:
- Provider toggle enables/disables child settings visually
- API key reveal button works (password <-> text)
- Advanced settings expand/collapse
- Theme toggle switches live
- Lint warnings display inline

### Color rules (firm)
- Blue (`info`) = positive (allowed, running, ok). No green in UI chrome.
- Purple (`secondary`) = negative (denied, stopped, error). No red in UI chrome.
- Terminal emulation colors (xterm green) are fine -- that's xterm, not UI.

## Svelte 5 reference

Read `references/svelte5.md` for Svelte 5 patterns and the `@sveltejs/mcp` CLI for doc lookups.

## Gotchas

- `vm-state-changed` payload is `{ state, trigger }` (object), not a plain string
- Dynamic Svelte components: use `<svelte:component this={item.icon} />`, not `<item.icon />`
- Tailwind v4 + `client:only`: needs `@source` directives to scan Svelte files
