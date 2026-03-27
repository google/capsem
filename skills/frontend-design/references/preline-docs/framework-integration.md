# Preline Framework Integration

## Capsem Setup (Astro 5 + Svelte 5)

Capsem uses Astro 5 as a static shell with Svelte 5 components loaded via `client:only="svelte"`. This is the primary setup -- the generic Astro/Svelte sections below are reference only.

### Install
```bash
npm i preline
```

No other dependencies needed. Preline's core 23 plugins (accordion, carousel, collapse, combobox, copy-markup, dropdown, input-number, layout-splitter, overlay, pin-input, remove-element, scroll-nav, scrollspy, select, stepper, strong-password, tabs, textarea-auto-height, theme-switch, toggle-count, toggle-password, tooltip, tree-view) work with zero external deps. Floating UI for positioning is bundled.

Do NOT install jQuery, lodash, Dropzone, nouislider, datatables.net, or vanilla-calendar-pro unless you specifically need HSDataTable, HSFileUpload, HSRangeSlider, or HSDatepicker.

### CSS (`src/styles/global.css`)
```css
@import "tailwindcss";

/* Preline UI */
@source "../../node_modules/preline";
@import "preline/variants.css";

/* Preline Theme (semantic design tokens) */
@import "preline/css/themes/theme.css";
```

### Type declarations (`global.d.ts`)
```typescript
import type { IStaticMethods } from "preline/preline";

declare global {
  interface Window {
    HSStaticMethods: IStaticMethods;
  }
}
export {};
```

Only declare `HSStaticMethods`. Do not add jQuery/lodash/Dropzone types -- they are not used.

### Loader script (`src/scripts/preline.ts`)
```typescript
async function initPreline() {
  try {
    await import('preline');
    window.HSStaticMethods?.autoInit();
  } catch (e) {
    console.warn('[preline] init error:', e);
  }
}

if (document.readyState === 'loading') document.addEventListener('DOMContentLoaded', initPreline);
else initPreline();

// Re-init after Astro page transitions (if using View Transitions)
document.addEventListener('astro:page-load', initPreline);
```

### Layout (`src/layouts/Layout.astro`)
```astro
---
import "../styles/global.css";
---
<!doctype html>
<html lang="en">
  <body>
    <slot />
    <script>
      import '../scripts/preline.ts';
    </script>
  </body>
</html>
```

### Svelte component re-init

Since Svelte components mount client-side after Astro's initial render, Preline must re-init when components mount. Add `onMount` to any Svelte component that uses Preline plugins:

```svelte
<script lang="ts">
  import { onMount } from "svelte";

  onMount(() => {
    window.HSStaticMethods?.autoInit();
  });
</script>
```

For selective re-init (better performance when you know which plugins the component uses):
```typescript
onMount(() => {
  window.HSStaticMethods?.autoInit(['dropdown', 'tooltip']);
});
```

### Optional base styles

Preline includes opinionated styles for pointer cursor on buttons and hover behavior:

```css
@layer base {
  button:not(:disabled),
  [role="button"]:not(:disabled) {
    cursor: pointer;
  }
}

@custom-variant hover (&:hover);
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
