# Capsem Design System

## Semantic-First Color Architecture

Component code uses **only domain-semantic token names**. No DaisyUI color classes (`badge-info`, `badge-secondary`, `btn-primary`, `toggle-primary`, `text-info`, `bg-info`, `text-secondary`, `bg-secondary`, `text-error`, `text-warning`, `text-success`) ever appear in `.svelte` or `.ts` files.

The UI uses two layers:

1. **DaisyUI base surfaces**: `base-100`, `base-200`, `base-300`, `base-content` for backgrounds, borders, and text hierarchy. These are theme-aware and switch automatically between light and dark mode.
2. **Domain-semantic tokens**: Custom `@theme` tokens in `global.css` for all application-specific color. The indirection means we can change any color without touching component code.

DaisyUI provides **structure** (`badge`, `menu`, `collapse`, `tabs`, `card`, `table`, `btn`, `toggle`, `input`, `select`, `form-control`, `loading`, `divider`). DaisyUI **color classes** are banned from component code.

## @theme Token Names

All tokens are defined in `frontend/src/styles/global.css` under `@theme`. Tailwind v4 auto-generates `text-*`, `bg-*`, `bg-*/15`, `border-*` utilities from these.

### Interactive (purple H=277)
- `--color-interactive` -- active nav, buttons, toggles, focused inputs, hover highlights

### Status
- `--color-allowed` -- blue (H=233), positive decisions, running state
- `--color-denied` -- purple (H=300), negative decisions, errors
- `--color-caution` -- orange (H=84), booting state, warnings

### Providers (brand identity)
- `--color-provider-anthropic` -- H=60 orange
- `--color-provider-google` -- H=250 blue
- `--color-provider-openai` -- H=145 green
- `--color-provider-mistral` -- H=25 red
- `--color-provider-fallback` -- low-chroma blue

### Token types
- `--color-token-input` -- H=250 blue
- `--color-token-output` -- H=145 green
- `--color-token-cache` -- H=210 sky

### File actions
- `--color-file-created` -- H=233 blue (positive)
- `--color-file-modified` -- H=210 sky
- `--color-file-deleted` -- H=300 purple (negative)

### JSON syntax
- `--color-json-key` -- H=250 blue
- `--color-json-string` -- H=145 green
- `--color-json-bool` -- H=300 purple
- `--color-json-number` -- H=60 orange

### Chart infrastructure
- `--color-chart-grid` -- low-chroma blue with alpha
- `--color-chart-label` -- low-chroma blue

## DaisyUI Component Policy

### Structure only (use freely)
`badge`, `menu`, `collapse`, `tabs`, `card`, `table`, `btn`, `toggle`, `input`, `select`, `form-control`, `loading`, `divider`, `stat`

### Banned in component code
- `badge-info`, `badge-secondary`, `badge-primary` -- use `bg-allowed/15 text-allowed` or `bg-denied/15 text-denied`
- `text-info`, `bg-info`, `text-secondary`, `bg-secondary` -- use `text-allowed`, `bg-allowed`, etc.
- `btn-primary` -- use `btn bg-interactive text-white`
- `toggle-primary` -- use bare `toggle` (checked color handled by global CSS override)
- `text-error` -- use `text-denied`
- `text-warning` -- use `text-caution`
- `text-success` -- use `text-allowed`
- `bg-warning`, `bg-error` -- use `bg-caution`, `bg-denied`

### Acceptable DaisyUI tokens
- `base-100/200/300`, `base-content` -- surface hierarchy (always OK)
- `btn-ghost`, `badge-outline`, `tab-active`, `menu-active` -- structural modifiers (always OK)

## Component Patterns

### Badge
```html
<!-- Decision badges -->
<span class="badge bg-allowed/15 text-allowed">allowed</span>
<span class="badge bg-denied/15 text-denied">denied</span>

<!-- File action badges -->
<span class="badge bg-file-created/15 text-file-created">created</span>
<span class="badge badge-outline text-file-modified">modified</span>
<span class="badge bg-file-deleted/15 text-file-deleted">deleted</span>

<!-- Neutral badges (no domain color) -->
<span class="badge badge-xs badge-outline">mcp</span>
```

### Button
```html
<button class="btn bg-interactive text-white btn-sm">Primary action</button>
<button class="btn btn-ghost btn-sm">Secondary action</button>
```

### Toggle
```html
<!-- Bare toggle; checked color comes from global.css override -->
<input type="checkbox" class="toggle toggle-sm" />
```

### Pagination
```html
<button class="btn btn-xs {active ? 'bg-interactive text-white' : 'btn-ghost'}">1</button>
```

## Rules

1. **No raw color values** in `.svelte` or `.ts` files. All oklch values live in `global.css` only.
2. **No DaisyUI color classes** (`primary`, `secondary`, `accent`, `info`, `warning`, `error`, `success`) in component code.
3. **No `style=`** inline color overrides.
4. **Icons use `currentColor`** -- inherit from parent text color.
5. **Text hierarchy** via `base-content` opacity: `/30` muted, `/40` secondary, `/50` hint, `/60` body, full for primary.
6. **Chart colors** read from CSS custom properties via `css-var.ts` helper, not hardcoded.
