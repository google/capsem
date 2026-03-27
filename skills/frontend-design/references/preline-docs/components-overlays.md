# Preline CSS Components: Overlays

All overlay components use the HSOverlay plugin for behavior. This file covers the CSS markup patterns for different overlay types.

## Modal

Uses HSOverlay. Centered dialog with backdrop.

```html
<button data-hs-overlay="#modal-1" class="py-3 px-4 inline-flex items-center gap-x-2 text-sm font-medium rounded-lg bg-primary text-primary-foreground hover:bg-primary-hover">
  Open modal
</button>

<div id="modal-1" class="hs-overlay hidden size-full fixed top-0 start-0 z-80 overflow-x-hidden overflow-y-auto" role="dialog" tabindex="-1">
  <div class="hs-overlay-open:mt-7 hs-overlay-open:opacity-100 hs-overlay-open:duration-500 mt-0 opacity-0 ease-out transition-all sm:max-w-lg sm:w-full m-3 sm:mx-auto">
    <div class="bg-overlay border border-overlay-border shadow-2xs rounded-xl">
      <div class="flex justify-between items-center py-3 px-4 border-b border-overlay-divider">
        <h3 class="font-bold text-foreground">Modal title</h3>
        <button data-hs-overlay="#modal-1" class="size-8 inline-flex justify-center items-center rounded-full bg-muted text-muted-foreground-1 hover:bg-muted-hover">
          <svg class="size-4"><!-- X icon --></svg>
        </button>
      </div>
      <div class="p-4 overflow-y-auto"><p class="text-muted-foreground-1">Content</p></div>
      <div class="flex justify-end items-center gap-x-2 py-3 px-4 border-t border-overlay-divider">
        <button data-hs-overlay="#modal-1" class="py-2 px-3 text-sm font-medium rounded-lg border border-layer-line bg-layer text-layer-foreground hover:bg-layer-hover">Close</button>
        <button class="py-2 px-3 text-sm font-medium rounded-lg bg-primary text-primary-foreground hover:bg-primary-hover">Save</button>
      </div>
    </div>
  </div>
</div>
```

**Sizes** (on inner wrapper):
- Small: `sm:max-w-sm`
- Default: `sm:max-w-lg`
- Large: `sm:max-w-2xl`
- Full screen: `max-w-full m-0 h-full` (remove rounded corners)

**Vertically centered**: Replace `m-3 sm:mx-auto` with `min-h-[calc(100%-3.5rem)] flex items-center m-3 sm:mx-auto`

**Scrollable body**: Add `max-h-[calc(100vh-200px)] overflow-y-auto` to content div

**Static backdrop** (can't close by clicking outside): `style="--overlay-backdrop: static"`

## Offcanvas / Drawer

Uses HSOverlay. Slide-in panel from any edge.

```html
<button data-hs-overlay="#drawer-right" class="py-3 px-4 inline-flex items-center gap-x-2 text-sm font-medium rounded-lg bg-layer border border-layer-line text-layer-foreground hover:bg-layer-hover">
  Open drawer
</button>

<!-- Right drawer -->
<div id="drawer-right" class="hs-overlay hs-overlay-open:translate-x-0 hidden translate-x-full fixed top-0 end-0 transition-all duration-300 transform h-full max-w-xs w-full z-80 bg-overlay border-s border-overlay-border" role="dialog" tabindex="-1">
  <div class="flex justify-between items-center py-3 px-4 border-b border-overlay-divider">
    <h3 class="font-bold text-foreground">Drawer title</h3>
    <button data-hs-overlay="#drawer-right" class="size-8 inline-flex justify-center items-center rounded-full bg-muted text-muted-foreground-1 hover:bg-muted-hover">
      <svg class="size-4"><!-- X icon --></svg>
    </button>
  </div>
  <div class="p-4"><p class="text-muted-foreground-1">Content</p></div>
</div>
```

**Directions**:
- Left: `hs-overlay-open:translate-x-0 -translate-x-full fixed top-0 start-0 border-e`
- Right: `hs-overlay-open:translate-x-0 translate-x-full fixed top-0 end-0 border-s`
- Top: `hs-overlay-open:translate-y-0 -translate-y-full fixed top-0 inset-x-0 border-b max-h-72`
- Bottom: `hs-overlay-open:translate-y-0 translate-y-full fixed bottom-0 inset-x-0 border-t max-h-72`

**Body scroll enabled**: `style="--body-scroll: true"`

## Context Menu

Uses HSDropdown with `--trigger: contextmenu`.

```html
<div class="hs-dropdown" style="--trigger: contextmenu">
  <div class="hs-dropdown-toggle p-6 bg-muted rounded-lg cursor-context-menu">
    Right-click here
  </div>
  <div class="hs-dropdown-menu hs-dropdown-open:opacity-100 opacity-0 hidden min-w-40 bg-dropdown shadow-md rounded-lg" role="menu">
    <a class="flex items-center gap-x-3 py-2 px-3 text-sm text-dropdown-item-foreground rounded-lg hover:bg-dropdown-item-hover" href="#">Cut</a>
    <a class="flex items-center gap-x-3 py-2 px-3 text-sm text-dropdown-item-foreground rounded-lg hover:bg-dropdown-item-hover" href="#">Copy</a>
    <a class="flex items-center gap-x-3 py-2 px-3 text-sm text-dropdown-item-foreground rounded-lg hover:bg-dropdown-item-hover" href="#">Paste</a>
  </div>
</div>
```

## Popover

Similar to tooltip but with richer content. Uses HSTooltip pattern with `--trigger: click`.

```html
<div class="hs-tooltip inline-block" style="--trigger: click; --placement: bottom">
  <button class="hs-tooltip-toggle py-3 px-4 inline-flex items-center gap-x-2 text-sm font-medium rounded-lg bg-layer border border-layer-line text-layer-foreground">
    Click me
  </button>
  <div class="hs-tooltip-content hs-tooltip-shown:opacity-100 hs-tooltip-shown:visible opacity-0 invisible transition-opacity absolute z-10 max-w-xs w-full bg-popover border border-popover-border rounded-xl shadow-lg" role="tooltip">
    <div class="p-4">
      <h4 class="text-sm font-semibold text-foreground">Popover Title</h4>
      <p class="mt-1 text-sm text-muted-foreground-1">Popover description with more detail.</p>
    </div>
  </div>
</div>
```
