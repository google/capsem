# Preline Design Token System

Preline's theme system uses semantic CSS variables mapped to Tailwind utilities via `@theme inline {}`. Import `theme.css` to get the full token system with light and dark mode.

```css
@import "preline/css/themes/theme.css";
```

## How Tokens Work

1. `theme.css` defines CSS variables in `:root` (light) and `.dark` (dark mode)
2. An `@theme inline {}` block maps each variable to a `--color-*` Tailwind token
3. Tailwind generates utilities: `bg-background`, `text-foreground`, `border-line-2`, etc.
4. Dark mode: add `.dark` class to `<html>` and all tokens flip automatically

## Token Families

### Global

| Token | Utility | Light Default | Dark Default |
|-------|---------|--------------|-------------|
| `--background` | `bg-background` | white | neutral-800 |
| `--background-1` | `bg-background-1` | gray-50 | neutral-900 |
| `--background-2` | `bg-background-2` | gray-100 | neutral-900 |
| `--background-plain` | `bg-plain` | white | neutral-800 |
| `--foreground` | `text-foreground` | gray-800 | neutral-200 |
| `--foreground-inverse` | `text-foreground-inverse` | white | white |
| `--inverse` | `bg-inverse` | gray-800 | neutral-950 |

### Borders

| Token | Utility | Light | Dark |
|-------|---------|-------|------|
| `--border` | `border-border` | gray-200 | neutral-700 |
| `--border-line-inverse` | `border-line-inverse` | white | -- |
| `--border-line-1` | `border-line-1` | gray-100 | neutral-800 |
| `--border-line-2` | `border-line-2` | gray-200 | neutral-700 |
| `--border-line-3` | `border-line-3` | gray-300 | neutral-600 |
| `--border-line-4` to `--border-line-8` | `border-line-4` to `border-line-8` | gray-400..800 | neutral-500..100 |

### Primary (brand color)

| Token | Utility | Light | Dark |
|-------|---------|-------|------|
| `--primary-50` to `--primary-950` | `bg-primary-50` to `bg-primary-950` | blue scale | blue scale |
| `--primary` | `bg-primary`, `text-primary` | blue-600 | blue-500 |
| `--primary-foreground` | `text-primary-foreground` | white | white |
| `--primary-hover` | `hover:bg-primary-hover` | blue-700 | blue-600 |
| `--primary-focus` | `focus:bg-primary-focus` | blue-700 | blue-600 |
| `--primary-active` | `bg-primary-active` | blue-700 | blue-600 |
| `--primary-checked` | `bg-primary-checked` | blue-600 | blue-500 |
| `--primary-line` | `border-primary-line` | transparent | transparent |

### Secondary

| Token | Utility | Light | Dark |
|-------|---------|-------|------|
| `--secondary` | `bg-secondary` | gray-900 | white |
| `--secondary-foreground` | `text-secondary-foreground` | white | -- |
| `--secondary-hover` | `hover:bg-secondary-hover` | gray-800 | neutral-100 |

### Layer (elevated surfaces)

| Token | Utility | Light | Dark |
|-------|---------|-------|------|
| `--layer` | `bg-layer` | white | neutral-800 |
| `--layer-line` | `border-layer-line` | gray-200 | neutral-700 |
| `--layer-foreground` | `text-layer-foreground` | gray-800 | white |
| `--layer-hover` | `hover:bg-layer-hover` | gray-50 | neutral-700 |

### Surface (1-5 scale, increasing intensity)

| Token | Utility | Light | Dark |
|-------|---------|-------|------|
| `--surface` | `bg-surface` | gray-100 | neutral-700 |
| `--surface-1` to `--surface-5` | `bg-surface-1` to `bg-surface-5` | gray-200..600 | neutral-600..400 |
| `--surface-foreground` | `text-surface-foreground` | gray-800 | neutral-200 |
| `--surface-hover` | `hover:bg-surface-hover` | gray-200 | neutral-600 |

### Muted

| Token | Utility | Light | Dark |
|-------|---------|-------|------|
| `--muted` | `bg-muted` | gray-50 | neutral-800 |
| `--muted-foreground` | `text-muted-foreground` | gray-400 | neutral-500 |
| `--muted-foreground-1` | `text-muted-foreground-1` | gray-500 | neutral-400 |
| `--muted-foreground-2` | `text-muted-foreground-2` | gray-600 | neutral-300 |
| `--muted-hover` | `hover:bg-muted-hover` | gray-100 | neutral-700 |

### Destructive

| Token | Utility | Light | Dark |
|-------|---------|-------|------|
| `--destructive` | `bg-destructive` | red-500 | red-500 |
| `--destructive-foreground` | `text-destructive-foreground` | white | -- |
| `--destructive-hover` | `hover:bg-destructive-hover` | red-600 | red-600 |

### Component Tokens

**Navbar** (3 tiers: default, -1, -2):

| Token Pattern | Utility Pattern |
|--------------|----------------|
| `--navbar` / `--navbar-1` / `--navbar-2` | `bg-navbar` / `bg-navbar-1` / `bg-navbar-2` |
| `--navbar-border` | `border-navbar-border` |
| `--navbar-divider` | `divide-navbar-divider` |
| `--navbar-nav-foreground` | `text-navbar-nav-foreground` |
| `--navbar-nav-hover` | `hover:bg-navbar-nav-hover` |
| `--navbar-nav-active` | `bg-navbar-nav-active` |
| `--navbar-inverse` | `bg-navbar-inverse` |

**Sidebar** (3 tiers, same pattern as navbar):
`bg-sidebar`, `border-sidebar-border`, `text-sidebar-nav-foreground`, `hover:bg-sidebar-nav-hover`, `bg-sidebar-nav-active`

**Card**: `bg-card`, `border-card-line`, `border-card-divider`, `bg-card-header`, `bg-card-footer`, `bg-card-inverse`

**Dropdown**: `bg-dropdown`, `bg-dropdown-1`, `border-dropdown-border`, `divide-dropdown-divider`, `text-dropdown-item-foreground`, `hover:bg-dropdown-item-hover`, `bg-dropdown-item-active`

**Select**: `bg-select`, `bg-select-1`, `text-select-item-foreground`, `hover:bg-select-item-hover`, `bg-select-item-active`

**Overlay**: `bg-overlay`, `border-overlay-border`, `divide-overlay-divider`

**Popover**: `bg-popover`, `border-popover-border`

**Tooltip**: `bg-tooltip`, `text-tooltip-foreground`, `border-tooltip-border`

**Table**: `border-table-line`, `divide-table-line`

**Footer**: `bg-footer`, `border-footer-border`, `bg-footer-inverse`

**Switch**: `bg-switch`

**Scrollbar**: `bg-scrollbar-track`, `bg-scrollbar-thumb`, `bg-scrollbar-track-inverse`, `bg-scrollbar-thumb-inverse`

**Charts**: `text-chart-primary`, `bg-chart-1` to `bg-chart-10`

## Premade Themes

Shipped in `preline/css/themes/`:

| Theme | File | Character |
|-------|------|-----------|
| Default | `theme.css` | Blue primary, neutral surfaces |
| Harvest | `harvest.css` | Warm amber/golden, eye-friendly |
| Retro | `retro.css` | High-contrast magenta, bold |
| Ocean | `ocean.css` | Cool teal, calm |
| Autumn | `autumn.css` | Rich amber, cozy |
| Moon | `moon.css` | Deep navy, night-friendly |
| Bubblegum | `bubblegum.css` | Bright pink, energetic |
| Cashmere | `cashmere.css` | Dusty rose, refined |
| Olive | `olive.css` | Muted olive-green, natural |

Activate: `<html data-theme="theme-harvest">`

Import all or specific ones:
```css
@import "preline/css/themes/theme.css";
@import "preline/css/themes/harvest.css";
```

## Customization

Copy `theme.css` to your project and modify. Three sections to edit:

**1. `@theme inline {}` block** -- add custom color palettes or new token mappings:
```css
@theme inline {
  --color-my-brand: var(--my-brand);
}
```

**2. `:root` block** -- light mode values:
```css
:root {
  --primary: var(--color-blue-600);
  --primary-hover: var(--color-blue-700);
  --background: oklch(100% 0 0);
}
```

**3. `.dark` block** -- dark mode overrides:
```css
.dark {
  --primary: var(--color-blue-500);
  --background: var(--color-neutral-800);
}
```

Values can use Tailwind color variables (`var(--color-blue-600)`), hex (`#2563eb`), or OKLCH (`oklch(55% 0.2 260)`).

**Custom fonts**:
```css
:root {
  --font-sans: "Inter", ui-sans-serif, system-ui, sans-serif;
}
```
