---
name: frontend-design
description: Capsem frontend design system. Use when building UI components, styling views, choosing colors, or working with Svelte 5 runes and the component library.
---

# Frontend Design

## Stack

- **Astro 7** -- static site generator, renders `index.astro` as a thin shell
- **Svelte 5** -- reactive UI framework, loaded via `client:only="svelte"`
- **Tailwind v4** -- utility-first CSS (via Vite plugin, `@source` directives in `global.css`)
- **Capsem-owned semantic CSS** -- `src/styles/capsem-theme.css` owns the complete token contract (`bg-primary`, `text-foreground`, and peers). Production code must never install, import, scan, or execute Preline or another component library. Historical component references may inspire class composition only; copy the result into Capsem-owned code.

## Loading into capsem-app (Tauri)

`tauri::generate_context!()` bakes `web/app/dist/**` into the `capsem-app` binary at cargo compile time (via the `custom-protocol` feature). This means:

- `pnpm run build` alone has **no effect** on a running `./target/**/capsem-app` -- the bundle is embedded in the binary.
- After any `web/app/` change you intend to test in the desktop app, run `just build` (chains frontend build + `cargo build -p capsem-app`).
- `just dev ui` (`cargo tauri dev`) bypasses this by loading `http://localhost:5173` -- good for iteration, but the production code path goes through the embedded bundle.
- The Toolbar shows `build YYYY-MM-DD HH:MM:SS` as a quick visual sanity check -- if it's stale after you rebuilt, you forgot `cargo build -p capsem-app`.

Also: iframe `src` for bundled pages **must end in `index.html`** (e.g. `/vm/terminal/index.html`). Tauri's custom protocol on macOS does not auto-append `index.html` for trailing-slash paths the way Vite/Astro dev server does. A `/vm/terminal/` src loads fine in Chrome dev mode and silently 404s in the Tauri app.

## Design principles

**Simplicity and correctness above all else.** Every line of frontend code must earn its place.

- Capsem-owned semantic tokens for theming + Tailwind utilities for layout
- All interactivity via Svelte 5 runes + TypeScript -- no JS plugins, no jQuery, no framework plugins
- Custom `@theme` tokens in `global.css` for domain-specific colors (status, providers, charts)
- **Visual verification required** -- every UI change must be verified via Chrome DevTools MCP (see `/dev-testing-frontend`)
- **No component-library dependency** -- do not add Preline, DaisyUI, or an equivalent package to production or build dependencies.

## Framework references

- `references/preline.md` and `references/preline-docs/` are historical pattern catalogues only. Their upstream install/import examples are forbidden in Capsem; the checked-in theme and Svelte components are authoritative.
- Read `references/tailwind.md` for Tailwind v4 utility patterns, responsive design, and CSS-first config.
- Read `references/svelte5.md` for Svelte 5 patterns and `@sveltejs/mcp` CLI doc lookups.
- Read `references/astro.md` for Astro framework patterns (components, content collections, SSR).

## Surface hierarchy (global.css overrides)

The UI uses a two-tone surface system. Semantic token names map to specific roles:

| Token | Light | Dark | Role |
|-------|-------|------|------|
| `--background` | `#ffffff` (white) | `#282828` (rgb 40,40,40) | Main canvas (content area) |
| `--background-1` | `#f4f3f2` (rgb 244,243,242) | `#282828` | Recessed (address bar, inset panels) |
| `--background-2` | `#f4f3f2` | `#282828` | Most recessed (inactive tabs) |
| `--layer` | `#ffffff` (white) | `#3c3c3c` (rgb 60,60,60) | Elevated/selected (active tab, toolbar, cards) |

The pattern: **selected = white/lighter, inactive = slightly gray/darker**. In dark mode, the base is very dark (#282828) and elevated surfaces pop with #3c3c3c. In light mode, the canvas is white and recessed areas use a warm off-white.

These are set in `:root` and `.dark` blocks in `global.css`. All accent themes share the same surfaces -- only `--primary-*` changes per accent.

## Color scheme (firm -- do not deviate)

- **Blue** = main/positive color (allowed, running, ok states). Use Capsem `primary` tokens (`bg-primary`, `text-primary-foreground`, etc.)
- **Purple** = negative color (denied, stopped, error states). Keep the owned `destructive` tokens purple, not red.
- **No green or red anywhere in the UI** -- use blue for positive, purple for negative
- Chart colors: blue `oklch(0.7 0.15 250)` for allowed, purple `oklch(0.65 0.15 300)` for denied
- Terminal emulation colors (xterm #4ade80 green) are fine -- that's xterm, not UI chrome
- **Do NOT hardcode colors in components.** Change the owned semantic contract in `capsem-theme.css` or the deliberate surface/accent overrides in `global.css`; theme selection uses `data-theme` on `<html>`.

## Terminal theme contrast

All 24 terminal themes (12 families x dark/light) must pass WCAG AA 4.5:1 contrast ratio for foreground text and all 6 ANSI colors (red, green, yellow, blue, magenta, cyan) against their background. This is enforced by `theme-contrast.test.ts`.

Contrast utilities (`parseHex`, `relativeLuminance`, `contrastRatio`) are exported from `themes.ts` and used in tests. When adding or modifying terminal themes, run `pnpm test` to catch any violations.

## Component patterns

Use Capsem's semantic token classes for all UI components. Historical pattern references may guide layout, but no upstream CSS or JavaScript may enter the build.

- **Buttons**: `bg-primary text-primary-foreground hover:bg-primary-hover` (solid), `bg-layer border border-layer-line text-layer-foreground` (white), etc.
- **Cards**: `bg-card border border-card-line rounded-xl`, headers `bg-surface border-b border-card-divider`
- **Forms**: `border-line-2 rounded-lg bg-layer text-foreground focus:border-primary focus:ring-primary`
- **Navigation**: `bg-navbar border-navbar-border text-navbar-nav-foreground hover:bg-navbar-nav-hover`
- **Overlays**: `bg-overlay border-overlay-border`, `bg-dropdown text-dropdown-item-foreground`
- **Text hierarchy**: `text-foreground` (primary), `text-muted-foreground-1` (secondary), `text-muted-foreground` (tertiary)

Do NOT use raw Tailwind colors (`bg-gray-200`, `text-blue-600`) for UI chrome. Always use semantic tokens so themes work.

### Settings section layout (SettingsSection.svelte)

The Appearance section in `SettingsPage.svelte` is the reference pattern. All dynamic settings sections must match it:

- **Section title**: `<h2 class="text-xl font-medium text-foreground">` (not `font-bold`)
- **Subsection headings**: `<h3 class="text-xs font-semibold text-foreground uppercase tracking-wider">` (use `text-foreground`, not `text-muted-foreground-1`)
- **Cards wrap leaf items only**: A non-toggle group wraps children in `bg-card border border-card-line rounded-xl` ONLY when it has direct leaf/action children. Groups containing only subgroups render flat (heading + children, no card). This prevents nested grey card boxes.
- **Leaf padding**: All leaf items inside cards use `px-4` for horizontal padding, matching the Appearance rows.
- **Toggle-gated groups**: Standalone cards with `bg-card border border-card-line rounded-xl mb-3`. Never nest inside another card wrapper.
- **Warning/error colors**: Use `text-warning` / `text-destructive` and `bg-warning/5` / `bg-destructive/10`. Never raw Tailwind colors (`text-amber-700`, `text-red-700`, `bg-amber-50`).

## Custom design tokens (`global.css`)

Domain-specific tokens defined in `@theme { }` block:

| Category | Tokens | Purpose |
|----------|--------|---------|
| Status | `--color-allowed`, `--color-denied`, `--color-caution` | Decision states |
| Providers | `--color-provider-anthropic`, `-google`, `-openai`, `-mistral` | Brand identity |
| Token types | `--color-token-input`, `-output`, `-cache` | Usage tracking |
| Snapshots | `--color-snap-manual`, `-auto` | Snapshot types |
| File actions | `--color-file-created`, `-modified`, `-deleted` | FS events |
| Syntax | `--color-json-*`, `--color-sh-*` | Code highlighting |
| Spans | `--color-span-thinking`, `-tool`, `-answer` | Trace viewer |
| Charts | `--color-chart-grid`, `-label` | Chart infrastructure |

## Svelte 5 rune patterns (mandatory -- no legacy `$:`)

All components and stores use Svelte 5 runes exclusively. No legacy reactive statements.

- `$state<T>(initial)` -- reactive state declaration
- `$derived(expression)` -- derived value (recomputes when deps change)
- `$derived.by(() => { ... })` -- derived with complex logic
- `$effect(() => { ... })` -- side effect that re-runs on dependency changes
- `$props()` -- type-safe component props with destructuring
- Class-based stores with `$state` fields (singleton pattern, `.svelte.ts` extension)
- `onMount` for async data loading, `onDestroy` for cleanup (intervals, charts)

### Store pattern

```typescript
// stores/example.svelte.ts
class ExampleStore {
  items = $state<Item[]>([]);
  activeId = $state<string | null>(null);
  active = $derived(this.items.find(i => i.id === this.activeId));

  async load() { this.items = await api.getItems(); }
  setActive(id: string) { this.activeId = id; }
}
export const exampleStore = new ExampleStore();
```

### Icon pattern

```svelte
<script lang="ts">
  let { class: cls = 'size-5' }: { class?: string } = $props();
</script>
<svg class={cls}>...</svg>
```

## View routing

Chrome browser shell. Tabs = sessions, toolbar = controls. Views switched by `tabStore.active.view`:

- `'new-tab'` -- session/profile dashboard (NewTabPage), sortable table of real sessions
- `'terminal'` -- sandboxed iframe with xterm.js (VMFrame), one iframe per VM
- `'settings'` -- appearance, general, security, network, storage, advanced, about
- Future: focused typed views for logs/debug surfaces as needed.

Tab store (`stores/tabs.svelte.ts`): `openVM()` creates a terminal tab or activates existing.

## Data fetching

The frontend talks to the backend through **capsem-gateway** -- a TCP-to-UDS reverse proxy (default port 19222) that forwards HTTP requests to capsem-service over UDS. Bearer token auth is required (token generated at gateway startup, written to `~/.capsem/run/gateway.token`).

Key gateway endpoints:

| Endpoint | Purpose |
|----------|---------|
| `GET /` | Health check (no auth) |
| `GET /status` | Aggregated VM status (1s cache TTL) |
| `GET /terminal/{id}` | WebSocket terminal stream |
| Explicit allowlist | Profile, session, stats, enforcement, detection, plugin, MCP, credential, snapshot, and debug routes used by the UI/TUI |

The gateway forwards only routes that are deliberately registered in its route table.
Unknown, retired, or misspelled routes must return 404 instead of falling through to
capsem-service.

Typed data contract:

- **Per-session telemetry**: use typed routes such as `/vms/{id}/stats/detail`,
  `/vms/{id}/timeline`, `/vms/{id}/security/latest`,
  `/vms/{id}/detection/latest`, and protocol-specific routes.
- **Cross-session state**: use dedicated service routes such as `/status`,
  `/stats`, `/vms/list`, and profile/plugin/MCP routes.
- **Never add frontend raw SQL helpers**. The old Inspector and `/inspect`
  surfaces are burned; UI code reflects typed API contracts only.

Mocks must exercise the same typed routes the product uses.

## Code reuse

Before creating new components, stores, or helpers, check what exists:
- **Stores** (`web/app/src/lib/stores/`): extend existing rune stores
- **Components** (`web/app/src/lib/components/`): extend existing patterns
- **Views** (`web/app/src/lib/views/`): main view containers with sub-views
- **Models** (`web/app/src/lib/models/`): pure TS business logic (no Svelte deps)
- **Helpers** (`api.ts`, `types.ts`): use existing typed route clients, formatters, and types
