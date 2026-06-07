# Preline Plugins: Overlays & Popups

## HSDropdown

**Init**: `.hs-dropdown:not(.--prevent-on-load-init)`

**Structure**:
```html
<div class="hs-dropdown relative inline-flex">
  <button class="hs-dropdown-toggle py-3 px-4 inline-flex items-center gap-x-2 text-sm font-medium rounded-lg bg-layer border border-layer-line text-layer-foreground shadow-2xs hover:bg-layer-hover">
    Actions
    <svg class="hs-dropdown-open:rotate-180 size-4"><!-- chevron --></svg>
  </button>
  <div class="hs-dropdown-menu transition-[opacity,margin] duration hs-dropdown-open:opacity-100 opacity-0 hidden min-w-60 bg-dropdown shadow-md rounded-lg mt-2" role="menu">
    <a class="flex items-center gap-x-3.5 py-2 px-3 text-sm text-dropdown-item-foreground rounded-lg hover:bg-dropdown-item-hover" href="#">Item 1</a>
    <a class="flex items-center gap-x-3.5 py-2 px-3 text-sm text-dropdown-item-foreground rounded-lg hover:bg-dropdown-item-hover" href="#">Item 2</a>
  </div>
</div>
```

**CSS custom property config** (on `.hs-dropdown` element via inline style or class):

| Property | Values | Default |
|----------|--------|---------|
| `--trigger` | `'click'`, `'hover'`, `'contextmenu'` | `'click'` |
| `--auto-close` | `'true'`, `'false'`, `'inside'`, `'outside'` | `'true'` |
| `--placement` | Any Floating UI placement | `'bottom-start'` |
| `--flip` | `'true'`, `'false'` | `'true'` |
| `--strategy` | `'fixed'`, `'absolute'` | -- |
| `--offset` | number (px) | `'10'` |
| `--gpu-acceleration` | `'true'`, `'false'` | `'true'` |
| `--adaptive` | `'adaptive'`, string | `'adaptive'` |
| `--scope` | `'window'` | -- (parent-scoped by default) |
| `--has-autofocus` | `'true'` | -- |
| `--autofocus-on-keyboard-only` | `'true'` | -- |

**Internal selectors**: `.hs-dropdown-toggle`, `.hs-dropdown-menu`, `.hs-dropdown-close`, `.hs-dropdown-toggle-wrapper`

**Menu roles**: `[role="menuitem"]`, `[role="menuitemcheckbox"]`, `[role="menuitemradio"]`

**CSS classes toggled**: `open` (on `.hs-dropdown` and menu when `--scope: window`)

**Methods**: `open(target?, openedViaKeyboard?)`, `close(isAnimated?)`, `forceClearState()`, `calculatePopperPosition()`, `destroy()`

**Events**: `open.hs.dropdown`, `close.hs.dropdown`

**Variants**: `hs-dropdown-open:` (open state), `hs-dropdown-item-disabled:` (disabled items), `hs-dropdown-item-checked:` (checked menu items `aria-checked="true"`)

---

## HSOverlay (Modal / Offcanvas / Drawer)

**Init**: `.hs-overlay:not(.--prevent-on-load-init)`

**Toggle buttons**: Any element with `data-hs-overlay="#overlay-id"` opens/closes the overlay.

**Structure (Modal)**:
```html
<button data-hs-overlay="#my-modal">Open Modal</button>

<div id="my-modal" class="hs-overlay hidden size-full fixed top-0 start-0 z-80 overflow-x-hidden overflow-y-auto" role="dialog" tabindex="-1" aria-labelledby="my-modal-label">
  <div class="hs-overlay-open:mt-7 hs-overlay-open:opacity-100 hs-overlay-open:duration-500 mt-0 opacity-0 ease-out transition-all sm:max-w-lg sm:w-full m-3 sm:mx-auto">
    <div class="bg-overlay border border-overlay-border shadow-2xs rounded-xl">
      <div class="flex justify-between items-center py-3 px-4 border-b border-overlay-divider">
        <h3 id="my-modal-label" class="font-bold text-foreground">Modal title</h3>
        <button data-hs-overlay="#my-modal" class="size-8 inline-flex justify-center items-center rounded-full bg-muted text-muted-foreground-1 hover:bg-muted-hover">
          <svg class="size-4"><!-- close icon --></svg>
        </button>
      </div>
      <div class="p-4 overflow-y-auto">Content</div>
      <div class="flex justify-end items-center gap-x-2 py-3 px-4 border-t border-overlay-divider">
        <button data-hs-overlay="#my-modal" class="py-2 px-3 text-sm font-medium rounded-lg border border-layer-line bg-layer text-layer-foreground">Cancel</button>
        <button class="py-2 px-3 text-sm font-medium rounded-lg bg-primary text-primary-foreground">Save</button>
      </div>
    </div>
  </div>
</div>
```

**Options** (via `data-hs-overlay-options` JSON on overlay element):

| Option | Type | Default |
|--------|------|---------|
| `hiddenClass` | string | `'hidden'` |
| `emulateScrollbarSpace` | boolean | `false` |
| `isClosePrev` | boolean | `true` |
| `backdropClasses` | string | `'hs-overlay-backdrop transition duration fixed inset-0 bg-gray-900/50 dark:bg-neutral-900/80'` |
| `backdropParent` | string/element | `document.body` |
| `backdropExtraClasses` | string | `''` |
| `moveOverlayToBody` | number/null | `null` (breakpoint to move) |

**CSS custom property config** (on `.hs-overlay`):

| Property | Values | Default |
|----------|--------|---------|
| `--body-scroll` | `'true'`, `'false'` | `'false'` |
| `--overlay-backdrop` | `'true'`, `'static'`, `'false'` | `'true'` |
| `--auto-close` | breakpoint number | -- |
| `--opened` | breakpoint number | -- |
| `--auto-hide` | ms number | `'0'` |
| `--has-dynamic-z-index` | `'true'`, `'false'` | `'false'` |
| `--close-when-click-inside` | `'true'`, `'false'` | `'false'` |
| `--tab-accessibility-limited` | `'true'`, `'false'` | `'true'` |
| `--is-layout-affect` | `'true'`, `'false'` | `'false'` |
| `--has-autofocus` | `'true'`, `'false'` | `'true'` |

**Additional data attrs**: `data-hs-overlay-minifier="#id"` (minify toggle), `data-hs-overlay-keyboard="false"` (disable ESC close)

**Methods**: `open(cb?)`, `close(forceClose?, cb?)`, `minify(isMinified, cb?)`, `updateToggles()`, `destroy()`

**Events**: `open.hs.overlay`, `close.hs.overlay`, `toggleClicked.hs.overlay`, `toggleMinifierClicked.hs.overlay`

**Variants**: `hs-overlay-open:` (open state), `hs-overlay-layout-open:` (body has open overlay), `hs-overlay-minified:` (minified state), `hs-overlay-backdrop-open:` (backdrop state)

**Offcanvas/Drawer**: Same HSOverlay plugin, just styled differently (positioned left/right/top/bottom with translate transforms).

---

## HSTooltip

**Init**: `.hs-tooltip:not(.--prevent-on-load-init)`

**Structure**:
```html
<div class="hs-tooltip inline-block">
  <button class="hs-tooltip-toggle py-3 px-4 inline-flex items-center gap-x-2 text-sm font-medium rounded-lg bg-layer border border-layer-line text-layer-foreground shadow-2xs hover:bg-layer-hover">
    Hover me
  </button>
  <span class="hs-tooltip-content hs-tooltip-shown:opacity-100 hs-tooltip-shown:visible opacity-0 invisible transition-opacity absolute z-10 py-1 px-2 bg-tooltip text-xs font-medium text-tooltip-foreground rounded shadow-sm" role="tooltip">
    Tooltip text
  </span>
</div>
```

**CSS custom property config** (on `.hs-tooltip`):

| Property | Values | Default |
|----------|--------|---------|
| `--trigger` | `'hover'`, `'click'` | `'hover'` |
| `--placement` | `'auto'`, any Floating UI placement | `'top'` |
| `--prevent-popper` | `'true'`, `'false'` | `'false'` |
| `--strategy` | `'fixed'`, `'absolute'` | -- |
| `--scope` | `'parent'`, `'window'` | `'parent'` |

**Internal selectors**: `.hs-tooltip-toggle`, `.hs-tooltip-content`

**Methods**: `show()`, `hide()`, `destroy()`

**Events**: `show.hs.tooltip`, `hide.hs.tooltip`

**Variant**: `hs-tooltip-shown:` -- style content when visible

---

## HSComboBox

**Init**: `[data-hs-combo-box]:not(.--prevent-on-load-init)`

**Structure**:
```html
<div data-hs-combo-box='{
  "groupingType": "default",
  "isOpenOnFocus": true,
  "apiUrl": "/api/search",
  "apiSearchQuery": "q",
  "apiDataPart": "results",
  "outputItemTemplate": "<div data-hs-combo-box-output-item><span data-hs-combo-box-search-text data-hs-combo-box-value></span></div>",
  "outputEmptyTemplate": "<div>No results</div>"
}'>
  <input data-hs-combo-box-input type="text" placeholder="Search...">
  <div data-hs-combo-box-output class="hidden absolute z-50 w-full bg-dropdown rounded-lg shadow-md">
    <div data-hs-combo-box-output-items-wrapper></div>
  </div>
</div>
```

**Key options**:

| Option | Type | Default |
|--------|------|---------|
| `gap` | number | `5` |
| `viewport` | string/element | `null` |
| `minSearchLength` | number | `0` |
| `apiUrl` | string | `null` |
| `apiDataPart` | string | `null` |
| `apiQuery` | string | `null` |
| `apiSearchQuery` | string | `null` |
| `apiHeaders` | object | `{}` |
| `apiGroupField` | string | `null` |
| `outputItemTemplate` | string | default HTML |
| `outputEmptyTemplate` | string | `"Nothing found..."` |
| `outputLoaderTemplate` | string | spinner HTML |
| `groupingType` | `'default'`/`'tabs'`/`null` | `null` |
| `preventSelection` | boolean | `false` |
| `isOpenOnFocus` | boolean | `false` |
| `keepOriginalOrder` | boolean | `false` |

**Internal data attrs**: `data-hs-combo-box-input`, `data-hs-combo-box-output`, `data-hs-combo-box-output-items-wrapper`, `data-hs-combo-box-output-item`, `data-hs-combo-box-toggle`, `data-hs-combo-box-close`, `data-hs-combo-box-search-text`, `data-hs-combo-box-value`

**Methods**: `getCurrentData()`, `open(val?)`, `close(val?, data?)`, `recalculateDirection()`, `destroy()`

**Event**: `select.hs.combobox` with currentData

**Variants**: `hs-combo-box-active:`, `hs-combo-box-has-value:`, `hs-combo-box-selected:`, `hs-combo-box-tab-active:`

---

## HSSelect

**Init**: `[data-hs-select]:not(.--prevent-on-load-init)`

**Structure**:
```html
<select data-hs-select='{
  "placeholder": "Select option...",
  "toggleClasses": "py-3 px-4 pe-9 flex gap-x-2 text-nowrap w-full cursor-pointer bg-layer border-layer-line rounded-lg text-sm focus:border-primary-focus focus:ring-primary-focus",
  "dropdownClasses": "mt-2 z-50 w-full max-h-72 p-1 space-y-0.5 bg-dropdown border border-dropdown-border rounded-lg overflow-hidden overflow-y-auto",
  "optionClasses": "py-2 px-4 w-full text-sm text-dropdown-item-foreground cursor-pointer hover:bg-dropdown-item-hover rounded-lg hs-selected:bg-dropdown-item-active",
  "hasSearch": true
}' class="hidden">
  <option value="">Choose</option>
  <option value="1">Option 1</option>
  <option value="2" selected>Option 2</option>
</select>
```

**Key options**:

| Option | Type | Default |
|--------|------|---------|
| `placeholder` | string | `'Select...'` |
| `hasSearch` | boolean | `false` |
| `minSearchLength` | number | `0` |
| `mode` | `'default'`/`'tags'` | `'default'` |
| `isOpened` | boolean | `false` |
| `scrollToSelected` | boolean | `false` |
| `toggleClasses` | string | -- |
| `dropdownClasses` | string | -- |
| `optionClasses` | string | -- |
| `searchPlaceholder` | string | -- |
| `searchMatchMode` | `'substring'`/`'chars-sequence'`/`'token-all'`/`'hybrid'` | `'substring'` |
| `dropdownScope` | `'parent'`/`'window'` | `'parent'` |
| `dropdownPlacement` | string | `null` |
| `isSelectedOptionOnTop` | boolean | -- |
| `apiUrl` | string | `null` |
| `apiFieldsMap` | object | `null` |
| `apiLoadMore` | boolean/object | -- |

**Option attributes**: `<option>` elements can have `data-hs-select-option='{"icon": "<svg>...", "description": "..."}'`

**Methods**: `setValue(val)`, `open()`, `close()`, `addOption(items)`, `removeOption(values)`, `recalculateDirection()`, `destroy()`

**Variants**: `hs-selected:` (selected option styling), `hs-select-disabled:`, `hs-select-active:`, `hs-select-opened:`
