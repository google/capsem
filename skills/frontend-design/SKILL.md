---
name: frontend-design
description: Capsem frontend design system. Use when building UI components, styling views, working with the design system, choosing colors, or understanding the component library. Covers the stack (Astro 5 + Svelte 5 + Tailwind v4 + Preline), color scheme, Svelte 5 rune patterns, data fetching, and code reuse policy.
---

# Frontend Design

## Stack

- **Astro 5** -- static site generator, renders `index.astro` as a thin shell
- **Svelte 5** -- reactive UI framework, loaded via `client:only="svelte"`
- **Tailwind v4** -- utility-first CSS (via Vite plugin, `@source` directives in `global.css`)
- **Preline** -- Tailwind UI component library (migrating from DaisyUI v5)

## Framework references

- Read `references/preline.md` for the Preline theme generator. Step-by-step docs in `references/preline-docs/`. Official htmlstream team.
- Read `references/tailwind.md` for Tailwind v4 utility patterns, responsive design, and CSS-first config.
- Read `references/svelte5.md` for Svelte 5 patterns and `@sveltejs/mcp` CLI doc lookups. Official sveltejs.
- Read `references/astro.md` for Astro framework patterns (components, content collections, SSR). Official Astro team.

## Color scheme (firm -- do not deviate)

- **Blue** (`info`) = main/positive color (allowed, running, ok states)
- **Purple** (`secondary`) = negative color (denied, stopped, error states)
- **No green or red anywhere in the UI** -- use blue for positive, purple for negative
- Chart colors: blue `oklch(0.7 0.15 250)` for allowed, purple `oklch(0.65 0.15 300)` for denied
- Terminal emulation colors (xterm #4ade80 green) are fine -- that's xterm, not UI chrome

## Svelte 5 rune patterns

- `$state<T>(initial)` -- reactive state declaration
- `$derived(expression)` -- derived value (recomputes when deps change)
- `$derived.by(() => { ... })` -- derived with complex logic
- `$effect(() => { ... })` -- side effect that re-runs on dependency changes
- Class-based stores with `$state` fields (see `network.svelte.ts`, `sidebar.svelte.ts`)
- `onMount` for async data loading, `onDestroy` for cleanup (intervals, charts)

## Data fetching

Two databases, two strategies:

- **Per-session** (info.db): `queryAll<T>(sql)` / `queryOne<T>(sql)` from `api.ts`
- **Cross-session** (main.db): dedicated Tauri commands (`getGlobalStats`, `getTopProviders`, etc.)

Both work identically in mock mode (sql.js runs against `fixtures/test.db`).

## Code reuse

Before creating new components, stores, or helpers, check what exists:
- **Stores** (`frontend/src/lib/stores/`): extend existing rune stores
- **Components** (`frontend/src/lib/components/`): extend existing patterns
- **Helpers** (`api.ts`, `mock.ts`, `types.ts`): use existing formatters and types
