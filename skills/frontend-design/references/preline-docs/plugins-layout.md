# Preline Plugins: Layout & Navigation

## HSAccordion

**Init**: `.hs-accordion:not(.--prevent-on-load-init)`

**Structure**:
```html
<div class="hs-accordion-group">
  <div class="hs-accordion active" id="acc-1">
    <button class="hs-accordion-toggle" aria-expanded="true" aria-controls="acc-1-content">
      <span>Accordion title</span>
      <svg class="hs-accordion-active:hidden size-4"><!-- plus icon --></svg>
      <svg class="hs-accordion-active:block hidden size-4"><!-- minus icon --></svg>
    </button>
    <div id="acc-1-content" class="hs-accordion-content w-full overflow-hidden transition-[height] duration-300" role="region" aria-labelledby="acc-1">
      <p class="p-4">Content here</p>
    </div>
  </div>
</div>
```

**Internal selectors**: `.hs-accordion-toggle`, `.hs-accordion-content`, `.hs-accordion-group`, `.hs-accordion-selectable`

**Group options** (CSS classes on `.hs-accordion-group`):
- `data-hs-accordion-always-open` -- multiple items open simultaneously

**CSS property config** (on `.hs-accordion`):
- `--stop-propagation`: `'false'` (default) -- prevents parent accordion from toggling
- `--keep-one-open`: `'false'` (default) -- on group, only one open at a time

**TreeView mode**: Add `data-hs-accordion-options='{"isTreeView": true}'` on `.hs-accordion-treeview-root`

**Methods**: `show()`, `hide()`, `update()`, `destroy()`

**Events**:
- `beforeOpen.hs.accordion` / `open.hs.accordion`
- `beforeClose.hs.accordion` / `close.hs.accordion`

**Variants**: `hs-accordion-active:` (toggle/content styling when open), `hs-accordion-selected:` (selectable items), `hs-accordion-outside-active:` (external active state)

**Static**: `HSAccordion.getInstance(el)`, `HSAccordion.show(el)`, `HSAccordion.hide(el)`, `HSAccordion.treeView(el)`

---

## HSTabs

**Init**: `[role="tablist"]:not(select):not(.--prevent-on-load-init)`

**Structure**:
```html
<nav class="flex gap-x-1" aria-label="Tabs" role="tablist" aria-orientation="horizontal">
  <button type="button" class="hs-tab-active:bg-primary hs-tab-active:text-primary-foreground py-3 px-4 text-sm font-medium rounded-lg active" id="tab-1" aria-selected="true" data-hs-tab="#content-1" aria-controls="content-1" role="tab">
    Tab 1
  </button>
  <button type="button" class="hs-tab-active:bg-primary hs-tab-active:text-primary-foreground py-3 px-4 text-sm font-medium rounded-lg" id="tab-2" aria-selected="false" data-hs-tab="#content-2" aria-controls="content-2" role="tab">
    Tab 2
  </button>
</nav>

<div class="mt-3">
  <div id="content-1" role="tabpanel" aria-labelledby="tab-1">First content</div>
  <div id="content-2" class="hidden" role="tabpanel" aria-labelledby="tab-2">Second content</div>
</div>
```

**Data attributes**:
- `data-hs-tab="#content-id"` -- on each tab toggle, points to content panel
- `data-hs-tabs='{"eventType": "hover"}'` -- on `[role="tablist"]`, options JSON
- `data-hs-tab-select="#select-id"` -- companion `<select>` for responsive tab switching
- `data-hs-tabs-vertical` -- vertical tab orientation

**Options**: `eventType`: `'click'` (default) | `'hover'`, `preventNavigationResolution`: breakpoint

**CSS classes toggled**: `active` (on toggle), `hidden` (on content panels)

**Event**: `change.hs.tab` with payload `{ el, tabsId, prev, current }`

**Variant**: `hs-tab-active:` -- style active tab toggle and its children

---

## HSCollapse

**Init**: `.hs-collapse-toggle:not(.--prevent-on-load-init)`

**Structure**:
```html
<button type="button" class="hs-collapse-toggle" data-hs-collapse="#collapse-content" aria-expanded="false" aria-controls="collapse-content">
  <span class="hs-collapse-open:hidden">Show</span>
  <span class="hs-collapse-open:block hidden">Hide</span>
</button>

<div id="collapse-content" class="hs-collapse hidden w-full overflow-hidden transition-[height] duration-300">
  <p class="p-4">Collapsible content</p>
</div>
```

**Data attribute**: `data-hs-collapse="#target-selector"` -- on toggle button, CSS selector for content

**CSS classes toggled**: `open` (on trigger and content), `hidden`/`block` (on content)

**Methods**: `show()`, `hide()`, `destroy()`

**Events**: `beforeOpen.hs.collapse`, `open.hs.collapse`, `hide.hs.collapse`

**Variant**: `hs-collapse-open:` -- style toggle/content when expanded

**Mega menu support**: Works with `.hs-mega-menu-content` for mega menu dropdowns

---

## HSStepper

**Init**: `[data-hs-stepper]:not(.--prevent-on-load-init)`

**Structure**:
```html
<div data-hs-stepper='{ "currentIndex": 1, "mode": "linear" }'>
  <!-- Navigation -->
  <ul class="flex gap-x-2">
    <li class="flex items-center gap-x-2" data-hs-stepper-nav-item='{ "index": 1 }'>
      <span class="hs-stepper-active:bg-primary hs-stepper-success:bg-primary size-8 flex justify-center items-center rounded-full">
        <span class="hs-stepper-success:hidden">1</span>
        <svg class="hidden hs-stepper-success:block size-3"><!-- check icon --></svg>
      </span>
      <span>Step 1</span>
    </li>
  </ul>

  <!-- Content -->
  <div data-hs-stepper-content-item='{ "index": 1 }'>Step 1 content</div>
  <div data-hs-stepper-content-item='{ "index": 2 }' style="display: none;">Step 2 content</div>

  <!-- Buttons -->
  <button data-hs-stepper-back-btn disabled>Back</button>
  <button data-hs-stepper-next-btn>Next</button>
  <button data-hs-stepper-finish-btn style="display: none;">Finish</button>
  <button data-hs-stepper-reset-btn>Reset</button>
</div>
```

**Options**: `currentIndex`: 1 (default), `mode`: `'linear'` (default), `isCompleted`: false

**Nav item attrs** (`data-hs-stepper-nav-item`): `index`, `isFinal`, `isCompleted`, `isSkip`, `isOptional`, `isDisabled`, `isProcessed`, `hasError`

**Content item attrs** (`data-hs-stepper-content-item`): `index`, `isFinal`, `isCompleted`, `isSkip`

**Button data attrs**: `data-hs-stepper-back-btn`, `data-hs-stepper-next-btn`, `data-hs-stepper-skip-btn`, `data-hs-stepper-complete-step-btn='{"completedText": "Done"}'`, `data-hs-stepper-finish-btn`, `data-hs-stepper-reset-btn`

**Methods**: `goToNext()`, `goToFinish()`, `setProcessedNavItem(n?)`, `unsetProcessedNavItem(n?)`, `disableButtons()`, `enableButtons()`, `setErrorNavItem(n?)`, `destroy()`

**Events**: `active.hs.stepper`, `back.hs.stepper`, `beforeNext.hs.stepper`, `next.hs.stepper`, `skip.hs.stepper`, `complete.hs.stepper`, `beforeFinish.hs.stepper`, `finish.hs.stepper`, `reset.hs.stepper`

**Variants**: `hs-stepper-active:`, `hs-stepper-success:`, `hs-stepper-completed:`, `hs-stepper-error:`, `hs-stepper-processed:`, `hs-stepper-disabled:`, `hs-stepper-skipped:`

---

## HSScrollspy

**Init**: `[data-hs-scrollspy]:not(.--prevent-on-load-init)`

**Structure**:
```html
<div data-hs-scrollspy="#scrollspy-content" data-hs-scrollspy-options='{ "ignoreScrollUp": false }'>
  <a href="#section-1" class="hs-scrollspy-active:text-primary">Section 1</a>
  <a href="#section-2" class="hs-scrollspy-active:text-primary">Section 2</a>
</div>

<div id="scrollspy-content">
  <div id="section-1">...</div>
  <div id="section-2">...</div>
</div>
```

**Data attributes**:
- `data-hs-scrollspy="#container"` -- CSS selector for scrollable content
- `data-hs-scrollspy-options='{ "ignoreScrollUp": false }'` -- JSON options
- `data-hs-scrollspy-scrollable-parent="#parent"` -- custom scroll container
- `data-hs-scrollspy-group` -- group multiple scrollspy instances

**CSS property**: `--scrollspy-offset`: `'0'` (default) -- offset from top in px

**Options**: `ignoreScrollUp`: false (default)

**Events**: `beforeScroll.hs.scrollspy`, `afterScroll.hs.scrollspy`

**Variant**: `hs-scrollspy-active:` -- style active nav link

---

## HSScrollNav

**Init**: `[data-hs-scroll-nav]:not(.--prevent-on-load-init)`

**Structure**:
```html
<div data-hs-scroll-nav='{ "paging": true, "autoCentering": false }'>
  <button class="hs-scroll-nav-prev disabled">Prev</button>
  <div class="hs-scroll-nav-body overflow-x-auto flex gap-x-2">
    <a class="active" href="#">Item 1</a>
    <a href="#">Item 2</a>
    <a href="#">Item 3</a>
  </div>
  <button class="hs-scroll-nav-next">Next</button>
</div>
```

**Options**: `paging`: true (default), `autoCentering`: false (default)

**Internal selectors**: `.hs-scroll-nav-body`, `.hs-scroll-nav-prev`, `.hs-scroll-nav-next`

**CSS classes toggled**: `disabled` (on prev/next when at boundary)

**Methods**: `getCurrentState()` returns `{ first, last, center }`, `goTo(el, cb?)`, `centerElement(el, behavior?)`, `destroy()`

**Variants**: `hs-scroll-nav-active:`, `hs-scroll-nav-disabled:`
