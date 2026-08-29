# Capsem-Owned Framework Integration

## Capsem setup (Astro 7 + Svelte 5)

Capsem uses Astro 7 as a static shell with Svelte 5 components loaded through
`client:only="svelte"`. Capsem owns the complete semantic token contract in
`web/app/src/styles/capsem-theme.css`.

Preline is not a package, build input, runtime, or authority. The surrounding
reference directory is retained only as a historical catalogue of component
shapes. Never install Preline, scan its package, import its CSS or JavaScript,
use `data-hs-*`, or call `HSStaticMethods`.

### CSS

```css
@import "tailwindcss";
@import "./capsem-theme.css";
```

`global.css` owns the deliberate surface and accent overrides. Components use
the resulting semantic utilities such as `bg-layer`, `text-foreground`, and
`border-card-line`.

### State and interaction

All interaction belongs to Svelte runes and conditional classes:

```svelte
<script lang="ts">
  let open = $state(false);
</script>

<button
  class={open ? 'bg-layer text-primary-active' : 'bg-muted text-muted-foreground-1'}
  onclick={() => open = !open}
>
  Toggle
</button>
```

Do not recreate component-library plugin initialization. Shared behavior
belongs in Capsem-owned Svelte components or stores.

### Layout

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

### Prohibited dependency patterns

- a `preline` entry in any package manifest or lockfile;
- `@source` against `node_modules/preline`;
- an import from `preline/variants.css`, `preline/css`, or Preline JavaScript;
- `data-hs-*`, `hs-*-active:`, or `HSStaticMethods`;
- adding a different component library to replace the same dependency.

The frontend contract tests enforce the package and CSS boundaries. Production
build plus visual verification proves the owned implementation.
