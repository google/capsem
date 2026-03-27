# Preline JavaScript API

## Import Patterns

**Auto-initialization** (recommended): imports all plugins and auto-inits on DOMContentLoaded.
```typescript
import "preline";
// or in HTML: <script src="./node_modules/preline/dist/preline.js"></script>
```

**Non-auto** (manual control): imports classes but does NOT auto-init. You must instantiate manually.
```typescript
import { HSDropdown, HSOverlay, HSSelect } from "preline/non-auto";
new HSSelect(document.querySelector('[data-hs-select]'));
```

**Individual plugins** (tree-shaking):
```typescript
import HSDropdown from "preline/plugins/dropdown";
import HSOverlay from "preline/plugins/overlay";
```

## Auto-Init

After importing `"preline"`, all components with matching selectors auto-initialize on page load.

**Re-initialize all** (after dynamic DOM changes or SPA navigation):
```typescript
window.HSStaticMethods.autoInit();
```

**Re-initialize specific plugins**:
```typescript
window.HSStaticMethods.autoInit('dropdown');
window.HSStaticMethods.autoInit(['dropdown', 'tooltip', 'select']);
```

**Clean collections** (remove tracked instances before re-init):
```typescript
window.HSStaticMethods.cleanCollection('select');
window.HSStaticMethods.cleanCollection('all');
```

## Preventing Auto-Init

Add `--prevent-on-load-init` class to skip automatic initialization, then init manually:

```html
<select data-hs-select='{ "placeholder": "Select..." }' class="hidden --prevent-on-load-init">
  <option value="">Choose</option>
</select>
```

```typescript
document.addEventListener('DOMContentLoaded', () => {
  document.querySelectorAll('[data-hs-select].--prevent-on-load-init')
    .forEach((el) => new HSSelect(el));
});
```

## getInstance

Retrieve an existing plugin instance by element or selector:

```typescript
// Returns { id, element } where element is the plugin instance
const result = HSOverlay.getInstance('#my-modal', true);
if (result) {
  result.element.close();
}

// Without `true`, returns just the element
const dropdown = HSDropdown.getInstance('.my-dropdown');
```

Every plugin class has a static `getInstance(target, isInstance?)` method.

## Event Listening

**Plugin events** via `on()` method on instances:
```typescript
const result = HSOverlay.getInstance('#my-modal', true);
result.element.on('open.hs.overlay', () => {
  console.log('Modal opened');
});
```

**DOM custom events** via addEventListener:
```typescript
window.addEventListener('open.hs.overlay', (evt) => {
  console.log('Any overlay opened');
});
```

**Common event naming**: `{action}.hs.{plugin}` -- e.g., `open.hs.dropdown`, `close.hs.overlay`, `change.hs.tab`, `select.hs.combobox`, `completed.hs.pinInput`

## Common Methods

All plugins share:
- `destroy()` -- removes event listeners, cleans up instance from global collection

Most interactive plugins have a subset of:
- `open()` / `close()` -- overlays, dropdowns, comboboxes, selects
- `show()` / `hide()` -- accordions, collapses, tooltips, toggle-password
- `update()` -- accordions (recalculates tree view state)

## TypeScript

Declare the global interface to avoid TS warnings:

```typescript
import type { IStaticMethods } from "preline/preline";

declare global {
  interface Window {
    HSStaticMethods: IStaticMethods;
  }
}
export {};
```

This is the only declaration needed. Do NOT add jQuery, lodash, Dropzone, or other third-party types unless you specifically use HSDataTable, HSFileUpload, HSRangeSlider, or HSDatepicker (see "External Dependencies" below).

## Base Plugin Pattern

All 27 plugins extend `HSBasePlugin<Options, HTMLElement>`:

```typescript
class HSBasePlugin<O, E = HTMLElement> {
  el: E;                    // the DOM element
  options: O;               // merged options
  events: Record<string, Function>;

  createCollection(collection, element);  // registers instance in global collection
  fireEvent(evt: string, payload?);       // triggers registered event handler
  on(evt: string, cb: Function);          // registers event handler
}
```

Global collections stored on `window` as `$hs{PluginName}Collection` arrays. Each entry: `{ id, element }`.

Static methods available on every plugin class:
- `ClassName.autoInit()` -- find and init all matching elements
- `ClassName.getInstance(target, isInstance?)` -- retrieve existing instance
- `ClassName.on(el, evt, cb)` -- register event on instance by element (some plugins)

## External Dependencies

23 of 27 plugins work with zero external deps. Only 4 plugins require third-party libraries loaded globally BEFORE preline:

| Plugin | Requires | Bundle Size | Global Check |
|--------|----------|-------------|-------------|
| HSDataTable | datatables.net-dt + jQuery | ~90KB (jQuery alone) | `window.DataTable`, `window.jQuery` |
| HSFileUpload | dropzone + lodash | ~70KB (lodash alone) | `window.Dropzone`, `window._` |
| HSRangeSlider | nouislider | ~30KB | `window.noUiSlider` |
| HSDatepicker | vanilla-calendar-pro | ~50KB | `window.VanillaCalendarPro` |

If the global is missing, the plugin silently skips initialization -- no errors.

**Bundled (no action needed)**: @floating-ui/dom (used by HSDropdown and HSTooltip for positioning).

**Recommendation**: Avoid the 4 heavy plugins unless their specific functionality is required. Use native HTML elements or lighter alternatives instead. Capsem does not use any of them.
