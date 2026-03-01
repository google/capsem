# Frontend Development Skill

## Stack

- **Astro 5** -- static site generator, renders `index.astro` as a thin shell
- **Svelte 5** -- reactive UI framework, loaded via `client:only="svelte"`
- **Tailwind v4** -- utility-first CSS (via Vite plugin, `@source` directives in `global.css`)
- **DaisyUI v5** -- Tailwind component library (themes, badges, tables)
- **Chart.js** -- charting (bar, doughnut, line)

## Svelte 5 Rune Patterns

- `$state<T>(initial)` -- reactive state declaration
- `$derived(expression)` -- derived value (recomputes when deps change)
- `$derived.by(() => { ... })` -- derived with complex logic
- `$effect(() => { ... })` -- side effect that re-runs on dependency changes (chart rendering, polling)
- Class-based stores with `$state` fields (see `network.svelte.ts`, `sidebar.svelte.ts`)
- `onMount` for async data loading, `onDestroy` for cleanup (intervals, charts)

## Data Fetching Strategy

Two databases, two query strategies:

- **Per-session data** (info.db): Use `queryAll<T>(sql)` / `queryOne<T>(sql)` from `api.ts` with SQL constants from `sql.ts`. The `queryDb` command runs read-only SQL against the active session's info.db.
- **Cross-session data** (main.db): Use dedicated Tauri commands (`getGlobalStats`, `getTopProviders`, `getTopTools`, `getSessionHistory`).

Helper signatures in `api.ts`:
```ts
queryOne<T>(sql: string): Promise<T | null>  // first row as typed object
queryAll<T>(sql: string): Promise<T[]>        // all rows as typed objects
```

Both work identically in mock mode (sql.js runs against `fixtures/test.db`).

## Color Scheme (FIRM -- do not deviate)

- **Blue** = main/positive color (allowed, running, ok states). DaisyUI: `text-info`, `badge-info`
- **Purple** = negative color (denied, stopped, error states). DaisyUI: `text-secondary`, `badge-secondary`
- **No green or red anywhere in the UI** -- use `info` for positive, `secondary` for negative
- Chart colors: blue `rgba(59, 130, 246, 0.85)` for allowed/positive, purple `rgba(139, 92, 246, 0.85)` for denied/negative
- Terminal emulation colors (xterm #4ade80 green) are fine -- that's xterm, not UI chrome

## Chart.js Conventions

- Register only needed controllers/elements per component
- Reuse chart instances via `chart.update('none')` (skip animation on data updates)
- Destroy charts in `onDestroy`
- Common config: `gridColor = 'rgba(128,128,128,0.1)'`, `tickColor = 'rgba(128,128,128,0.6)'`, `tickFont = { size: 10 }`, `monoFont = { size: 10, family: 'monospace' }`

## CSS Patterns

- Card: `rounded-lg border border-base-300 bg-base-200/50 p-3`
- Stat label: `text-[10px] font-semibold text-base-content/50 uppercase tracking-wider`
- Stat value: `text-xl font-semibold tabular-nums`
- Section header: `text-xs font-semibold text-base-content/60 mb-2`
- Empty state: `flex items-center justify-center h-48 text-[10px] text-base-content/40`

## Formatting Helpers

Common patterns used across analytics views:

```ts
function formatTokens(n: number): string {
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`;
  if (n >= 1_000) return `${(n / 1_000).toFixed(1)}K`;
  return `${n}`;
}

function formatCost(usd: number): string {
  if (usd === 0) return '$0.00';
  if (usd < 0.01) return `$${usd.toFixed(4)}`;
  return `$${usd.toFixed(2)}`;
}
```

## Dev Workflow

1. `just ui` -- start Astro dev server on `http://localhost:5173` (mock mode, no VM needed)
2. Edit Svelte components in `frontend/src/lib/`, dev server hot-reloads
3. `cd frontend && pnpm run check` -- astro check + svelte-check
4. `cd frontend && pnpm run build` -- production build (catches bundling issues)
5. `just dev` -- full Tauri app with hot-reloading (needs VM assets)

## Visual Verification

Use Chrome DevTools MCP to inspect the running UI:
- `take_screenshot` with `fullPage: true` for each view
- `take_snapshot` for a11y tree / element UIDs
- Walk all views (Terminal, Analytics sub-views, Settings) in both light and dark themes
