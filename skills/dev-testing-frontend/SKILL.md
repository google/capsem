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

When `window.__TAURI_INTERNALS__` is absent (browser via `just ui`), `api.ts` auto-switches all IPC calls to return fake data from `mock.ts`. Settings data comes from `mock-settings.generated.ts` (auto-generated from `config/defaults.json` by the builder). Other mock data (MCP servers, VM state, logs) lives in `mock.ts`.

This means you can test the full UI without a VM by running `just ui`.

**Generated mock data**: `mock-settings.generated.ts` is produced by `scripts/generate_schema.py` from the TOML configs in `guest/config/`. It runs as part of `just run` and `just test` via the `_generate-settings` recipe. Never hand-edit this file.

## Visual verification with Chrome DevTools MCP

**Every UI change requires visual verification via Chrome DevTools MCP. No exceptions.** Type checks and unit tests pass on broken UIs all the time. The only way to know the UI actually works is to look at it.

### Workflow for every UI change

1. Start `just ui` (if not already running)
2. `navigate_page` to `http://localhost:5173`
3. `list_console_messages` types=["error","warn"] -- expect zero
4. Navigate to the view(s) affected by your change
5. `take_screenshot` each affected view -- visually confirm it renders correctly
6. If the change affects multiple views or layout, screenshot all views (Terminal, Sessions, Network, Settings)
7. Check console again after navigation for new errors

### Settings view

Click through every section (AI Providers, Repositories, Security, VM, Appearance). Verify:
- All settings from `defaults.json` are present (currently 68 leaf settings)
- Provider toggle enables/disables child settings visually
- API key reveal button works (password <-> text)
- Snapshots section shows auto_max, manual_max, auto_interval
- VM Resources section shows all resource settings including min_content_sessions
- Theme toggle switches live
- Lint warnings display inline

### After changing TOML configs or generated mock data

When modifying `guest/config/*.toml` or regenerating `mock-settings.generated.ts`:
1. Run `just _generate-settings` (or let `just run`/`just test` do it)
2. Start `just ui`
3. Navigate to Settings view
4. Screenshot and verify new/changed settings appear correctly
5. Check that setting counts match (grep `mockSettings.find` in generated file)

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
