# Preline Framework Integration

## Capsem Setup (Astro 6 + Svelte 5)

Capsem uses Astro 6 as a static shell with Svelte 5 components loaded via `client:only="svelte"`. **Preline is CSS-only** -- we use its design tokens and CSS component patterns but NOT its JS plugins. All interactivity is pure Svelte 5 runes + TypeScript.

### Install
```bash
pnpm add preline
```

### CSS (`src/styles/global.css`)
```css
@import "tailwindcss";

/* Preline UI -- CSS tokens and component patterns only */
@source "../../node_modules/preline";

/* Preline Themes -- all loaded, activated via data-theme on <html> */
@import "preline/css/themes/theme.css";
@import "preline/css/themes/harvest.css";
@import "preline/css/themes/retro.css";
@import "preline/css/themes/ocean.css";
@import "preline/css/themes/bubblegum.css";
@import "preline/css/themes/autumn.css";
@import "preline/css/themes/moon.css";
@import "preline/css/themes/cashmere.css";
@import "preline/css/themes/olive.css";
```

### What we do NOT use

- **No `preline/variants.css`** -- `hs-*-active:` variants require Preline JS plugins and `data-hs-*` attributes. We drive active/open/selected state with Svelte runes and conditional classes instead.
- **No `import "preline"` JS** -- no `HSStaticMethods`, no `autoInit()`, no `global.d.ts` type declarations.
- **No `data-hs-*` attributes** -- no `data-hs-tab`, `data-hs-dropdown`, etc.

### How to replicate Preline component behavior in Svelte

Preline docs show components like:
```html
<button class="hs-tab-active:bg-layer hs-tab-active:text-primary-active bg-muted ..." data-hs-tab="#panel">
```

In Capsem, extract the CSS class strings and drive state with Svelte:
```svelte
<button class="{active ? 'bg-layer text-primary-active' : 'bg-muted text-muted-foreground-1'} ...">
```

Use `$state`, `$derived`, and class-based stores for all interactive state.

### Layout (`src/layouts/Layout.astro`)
```astro
---
import "../styles/global.css";
---
<!doctype html>
<html lang="en">
  <head>
    <meta charset="utf-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1" />
    <title>Capsem</title>
  </head>
  <body class="bg-background text-foreground antialiased">
    <slot />
  </body>
</html>
```

### Base styles in global.css

```css
@layer base {
  button:not(:disabled),
  [role="button"]:not(:disabled) {
    cursor: pointer;
  }
}

@custom-variant hover (&:hover);

html, body {
  height: 100%;
  overflow: hidden;
  margin: 0;
  padding: 0;
}
```

---

## Heavy Plugins (optional, not used in Capsem)

Four plugins wrap third-party libraries. They are NOT needed for the core Preline experience. Only add them if you specifically need their functionality:

| Plugin | Requires | Why |
|--------|----------|-----|
| HSDataTable | `datatables.net-dt` + `jQuery` | jQuery is a 90KB legacy dep. Use a native table solution instead. |
| HSFileUpload | `dropzone` + `lodash` | lodash is 70KB. Consider a native file input or lighter uploader. |
| HSRangeSlider | `nouislider` | Adds 30KB. Native `<input type="range">` covers most cases. |
| HSDatepicker | `vanilla-calendar-pro` | Adds 50KB. Native `<input type="date">` may suffice. |

These deps must be loaded globally on `window` BEFORE importing preline. If the global is missing, the plugin silently skips init -- no errors.

---

## Generic Astro Setup (reference)

Same as Capsem setup above. The key difference for vanilla Astro (without Svelte) is that `astro:page-load` handles re-init for View Transitions automatically.

## Generic SvelteKit Setup (reference)

For pure SvelteKit (without Astro), the setup differs slightly:

### CSS (`src/app.css`)
```css
@import "tailwindcss";
@import "preline/variants.css";
@source "../node_modules/preline/dist/*.js";
@import "./themes/theme.css";
```

### Client init (`src/lib/client/init.ts`)
```typescript
import("preline/dist");
```

### Hook (`src/hooks.client.ts`)
```typescript
import "./lib/client/init";
```

### Re-init on navigation (`src/routes/+layout.svelte`)
```svelte
<script lang="ts">
  import { afterNavigate } from "$app/navigation";

  afterNavigate(() => {
    window.HSStaticMethods.autoInit();
  });
</script>
```
