# Preline Plugins: Content & Data

## HSCarousel

**Init**: `[data-hs-carousel]:not(.--prevent-on-load-init)`

```html
<div data-hs-carousel='{
  "currentIndex": 0,
  "isAutoPlay": false,
  "isDraggable": true,
  "isInfiniteLoop": false,
  "isCentered": false,
  "isSnap": false,
  "slidesQty": { "sm": 1, "md": 2, "lg": 3 },
  "speed": 4000
}'>
  <div class="hs-carousel relative overflow-hidden">
    <div class="hs-carousel-body flex transition-transform duration-700">
      <div class="hs-carousel-slide flex-none w-full">Slide 1</div>
      <div class="hs-carousel-slide flex-none w-full">Slide 2</div>
      <div class="hs-carousel-slide flex-none w-full">Slide 3</div>
    </div>
  </div>

  <button class="hs-carousel-prev disabled:opacity-50">Prev</button>
  <button class="hs-carousel-next disabled:opacity-50">Next</button>

  <div class="hs-carousel-pagination flex justify-center gap-x-2 mt-4">
    <span class="hs-carousel-active:bg-primary size-3 rounded-full bg-muted cursor-pointer"></span>
    <span class="hs-carousel-active:bg-primary size-3 rounded-full bg-muted cursor-pointer"></span>
    <span class="hs-carousel-active:bg-primary size-3 rounded-full bg-muted cursor-pointer"></span>
  </div>
</div>
```

**Options**:

| Option | Type | Default |
|--------|------|---------|
| `currentIndex` | number | `0` |
| `isAutoPlay` | boolean | `false` |
| `isDraggable` | boolean | `false` |
| `isInfiniteLoop` | boolean | `false` |
| `isCentered` | boolean | `false` |
| `isSnap` | boolean | `false` |
| `hasSnapSpacers` | boolean | `true` |
| `isAutoHeight` | boolean | `false` |
| `isRTL` | boolean | `false` |
| `slidesQty` | number/object | `1` (or `{ "sm": 1, "md": 2 }`) |
| `speed` | number | `4000` (ms, autoplay interval) |
| `updateDelay` | number | `0` |
| `loadingClasses` | string | -- (comma-sep: remove,add,afterAdd) |
| `dotsItemClasses` | string | -- |

**Internal selectors**: `.hs-carousel`, `.hs-carousel-body`, `.hs-carousel-slide`, `.hs-carousel-prev`, `.hs-carousel-next`, `.hs-carousel-pagination`, `.hs-carousel-info-current`, `.hs-carousel-info-total`

**Methods**: `recalculateWidth()`, `goToPrev()`, `goToNext()`, `goTo(i)`, `destroy()`

**Event**: `update` with currentIndex

**Variants**: `hs-carousel-active:` (active slide/dot), `hs-carousel-disabled:` (prev/next at boundary), `hs-carousel-dragging:` (during drag)

---

## HSCopyMarkup

**Init**: `[data-hs-copy-markup]:not(.--prevent-on-load-init)`

```html
<div data-hs-copy-markup='{
  "targetSelector": "#copy-target",
  "wrapperSelector": "#copy-wrapper",
  "limit": 5
}'>
  <button type="button">Add item</button>
</div>

<div id="copy-wrapper">
  <div id="copy-target">
    <span>Item content</span>
    <button data-hs-copy-markup-delete-item>Delete</button>
  </div>
</div>
```

**Options**: `targetSelector`: CSS selector for element to clone, `wrapperSelector`: CSS selector for container, `limit`: max copies (optional)

**Internal attr**: `data-hs-copy-markup-delete-item` on delete buttons inside cloned items

**Methods**: `delete(target)`, `destroy()`

**Events**: `copy.hs.copyMarkup`, `delete.hs.copyMarkup`

---

## HSRemoveElement

**Init**: `[data-hs-remove-element]:not(.--prevent-on-load-init)`

```html
<div id="alert-1" class="hs-removing:translate-x-5 hs-removing:opacity-0 transition duration-300 bg-teal-50 border border-teal-200 rounded-lg p-4">
  <p>Alert message</p>
  <button data-hs-remove-element="#alert-1" data-hs-remove-element-options='{ "removeTargetAnimationClass": "hs-removing" }'>
    Dismiss
  </button>
</div>
```

**Data attrs**:
- `data-hs-remove-element="#target"` -- CSS selector for element to remove
- `data-hs-remove-element-options` -- JSON with `removeTargetAnimationClass` (default: `'hs-removing'`)

**Behavior**: Adds animation class to target, waits for transition to end, removes element from DOM.

**Variant**: `hs-removing:` -- style the element during removal animation

---

## HSDataTable

**Init**: `[data-hs-datatable]:not(.--prevent-on-load-init)`

**Requires**: `datatables.net-dt` + `jQuery` loaded globally

```html
<div data-hs-datatable='{
  "searching": true,
  "lengthChange": false,
  "order": [],
  "rowSelectingOptions": {
    "selectAllSelector": "#select-all",
    "individualSelector": ".row-select"
  },
  "pagingOptions": { "pageBtnClasses": "..." }
}'>
  <input data-hs-datatable-search type="text" placeholder="Search..." />
  <select data-hs-datatable-page-entities>
    <option value="5">5</option>
    <option value="10" selected>10</option>
  </select>

  <table class="w-full">
    <thead>
      <tr>
        <th class="--exclude-from-ordering"><input id="select-all" type="checkbox" /></th>
        <th>Name</th>
        <th>Email</th>
      </tr>
    </thead>
    <tbody>
      <tr>
        <td><input class="row-select" type="checkbox" /></td>
        <td>John</td>
        <td>john@example.com</td>
      </tr>
    </tbody>
  </table>

  <div data-hs-datatable-paging>
    <button data-hs-datatable-paging-prev>Prev</button>
    <div data-hs-datatable-paging-pages></div>
    <button data-hs-datatable-paging-next>Next</button>
  </div>

  <div data-hs-datatable-info>
    Showing <span data-hs-datatable-info-from></span> to <span data-hs-datatable-info-to></span>
    of <span data-hs-datatable-info-length></span>
  </div>
</div>
```

**Options**: Extends datatables.net Config + `rowSelectingOptions`, `pagingOptions`

**Internal data attrs**: `data-hs-datatable-search`, `data-hs-datatable-page-entities`, `data-hs-datatable-paging`, `data-hs-datatable-paging-pages`, `data-hs-datatable-paging-prev`, `data-hs-datatable-paging-next`, `data-hs-datatable-info`, `data-hs-datatable-info-from`, `data-hs-datatable-info-to`, `data-hs-datatable-info-length`

**Variants**: `hs-datatable-ordering-asc:`, `hs-datatable-ordering-desc:`

---

## HSTreeView

**Init**: `[data-hs-tree-view]:not(.--prevent-on-load-init)`

```html
<div data-hs-tree-view='{
  "controlBy": "checkbox",
  "autoSelectChildren": true,
  "isIndeterminate": true
}'>
  <div data-hs-tree-view-item='{ "value": "src", "id": "1", "isDir": true }'>
    <input type="checkbox" value="1" class="hs-tree-view-selected:text-primary" />
    <span>src/</span>
    <div class="ps-4">
      <div data-hs-tree-view-item='{ "value": "index.ts", "id": "2", "isDir": false }'>
        <input type="checkbox" value="2" />
        <span>index.ts</span>
      </div>
    </div>
  </div>
</div>
```

**Options**: `controlBy`: `'button'` (default) | `'checkbox'`, `autoSelectChildren`: false, `isIndeterminate`: true

**Item attr** (`data-hs-tree-view-item`): `{ value, id, isDir, isSelected? }`

**CSS class toggled**: `selected` on items, `disabled` prevents selection. Checkboxes get `indeterminate` state.

**Methods**: `update()`, `getSelectedItems()` returns `ITreeViewItem[]`, `changeItemProp(id, prop, val)`, `destroy()`

**Event**: `click.hs.treeView` with `{ el, data }`

**Variants**: `hs-tree-view-selected:`, `hs-tree-view-disabled:`

---

## HSLayoutSplitter

**Init**: `[data-hs-layout-splitter]:not(.--prevent-on-load-init)`

```html
<div data-hs-layout-splitter='{
  "horizontalSplitterClasses": "bg-muted hover:bg-primary cursor-col-resize w-1",
  "horizontalSplitterTemplate": "<div></div>"
}'>
  <div data-hs-layout-splitter-horizontal-group>
    <div data-hs-layout-splitter-item='{ "dynamicSize": 50, "minSize": 20 }'>Left panel</div>
    <div data-hs-layout-splitter-item='{ "dynamicSize": 50, "minSize": 20 }'>Right panel</div>
  </div>
</div>
```

**Options**: `horizontalSplitterClasses`, `horizontalSplitterTemplate`, `verticalSplitterClasses`, `verticalSplitterTemplate`, `isSplittersAddedManually`

**Item config** (`data-hs-layout-splitter-item`): `dynamicSize` (% width), `minSize` (% minimum), `preLimitSize` (% threshold for pre-limit event)

**Group attrs**: `data-hs-layout-splitter-horizontal-group`, `data-hs-layout-splitter-vertical-group`

**Methods**: `getSplitterItemSingleParam(item, name)`, `getData(el)`, `setSplitterItemSize(el, size)`, `updateFlexValues(data)`, `destroy()`

**Events**: `drag.hs.layoutSplitter`, `onNextLimit.hs.layoutSplitter`, `onPrevLimit.hs.layoutSplitter`, `onNextPreLimit.hs.layoutSplitter`, `onPrevPreLimit.hs.layoutSplitter`

**Variants**: `hs-layout-splitter-dragging:`, `hs-layout-splitter-prev-limit-reached:`, `hs-layout-splitter-next-limit-reached:`, `hs-layout-splitter-prev-pre-limit-reached:`, `hs-layout-splitter-next-pre-limit-reached:`

---

## HSThemeSwitch

**Init**: `[data-hs-theme-switch]:not(.--prevent-on-load-init)` (change type) or `[data-hs-theme-click-value]:not(.--prevent-on-load-init)` (click type)

**Toggle switch** (change type):
```html
<input data-hs-theme-switch type="checkbox" class="relative w-11 h-6 rounded-full cursor-pointer" />
```

**Button group** (click type):
```html
<button data-hs-theme-click-value="light" class="hs-light-mode-active:bg-primary py-2 px-3 rounded-lg">Light</button>
<button data-hs-theme-click-value="dark" class="hs-dark-mode-active:bg-primary py-2 px-3 rounded-lg">Dark</button>
<button data-hs-theme-click-value="auto" class="hs-auto-mode-active:bg-primary py-2 px-3 rounded-lg">Auto</button>
```

**Options**: `theme`: from localStorage `hs_theme` or `'default'`, `type`: `'change'` | `'click'`

**CSS classes toggled on `<html>`**: `light`, `dark`, `default`, `auto`

**Storage**: `localStorage.setItem('hs_theme', theme)`

**Custom event**: `on-hs-appearance-change` dispatched on `window` with `detail: theme`

**Methods**: `setAppearance(theme?, isSaveToLocalStorage?, isDispatchEvent?)`, `destroy()`

**Variants**: `hs-default-mode-active:`, `hs-light-mode-active:`, `hs-dark-mode-active:`, `hs-auto-mode-active:`, `hs-auto-dark-mode-active:`, `hs-auto-light-mode-active:`
