---
name: frontend-design
description: Capsem frontend design system. Use when building UI components, styling views, working with the design system, choosing colors, or understanding the component library. Covers the stack (Astro 5 + Svelte 5 + Tailwind v4 + Preline), color scheme, Svelte 5 rune patterns, data fetching, and code reuse policy.
---

# Frontend Design

## Stack

- **Astro 5** -- static site generator, renders `index.astro` as a thin shell
- **Svelte 5** -- reactive UI framework, loaded via `client:only="svelte"`
- **Tailwind v4** -- utility-first CSS (via Vite plugin, `@source` directives in `global.css`)
- **Preline** -- Tailwind UI component library (27 headless plugins, 70+ component patterns, semantic token system)

## Framework references

- Read `references/preline.md` for Preline UI overview and quick reference. Detailed docs in `references/preline-docs/` covering JS plugins, CSS components, variants, tokens, and framework integration.
- Read `references/tailwind.md` for Tailwind v4 utility patterns, responsive design, and CSS-first config.
- Read `references/svelte5.md` for Svelte 5 patterns and `@sveltejs/mcp` CLI doc lookups. Official sveltejs.
- Read `references/astro.md` for Astro framework patterns (components, content collections, SSR). Official Astro team.

## Color scheme (firm -- do not deviate)

- **Blue** = main/positive color (allowed, running, ok states). Use Preline `primary` tokens (`bg-primary`, `text-primary-foreground`, etc.)
- **Purple** = negative color (denied, stopped, error states). Override Preline `destructive` tokens with purple, not red.
- **No green or red anywhere in the UI** -- use blue for positive, purple for negative
- Chart colors: blue `oklch(0.7 0.15 250)` for allowed, purple `oklch(0.65 0.15 300)` for denied
- Terminal emulation colors (xterm #4ade80 green) are fine -- that's xterm, not UI chrome

## Component patterns

Use Preline's semantic token classes for all UI components. Read `references/preline.md` for the overview and load the relevant `preline-docs/` reference for details.

- **Buttons**: `bg-primary text-primary-foreground hover:bg-primary-hover` (solid), `bg-layer border border-layer-line text-layer-foreground` (white), etc.
- **Cards**: `bg-card border border-card-line rounded-xl`, headers `bg-surface border-b border-card-divider`
- **Forms**: `border-line-2 rounded-lg bg-layer text-foreground focus:border-primary focus:ring-primary`
- **Navigation**: `bg-navbar border-navbar-border text-navbar-nav-foreground hover:bg-navbar-nav-hover`
- **Overlays**: `bg-overlay border-overlay-border`, `bg-dropdown text-dropdown-item-foreground`
- **Text hierarchy**: `text-foreground` (primary), `text-muted-foreground-1` (secondary), `text-muted-foreground` (tertiary)

Do NOT use raw Tailwind colors (`bg-gray-200`, `text-blue-600`) for UI chrome. Always use semantic tokens so themes work.

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
