---
name: preline-ui
description: Preline UI v4.1.3 free component library reference. 27 headless Tailwind CSS plugins, 70+ CSS component patterns, 55 custom variants, and a semantic design token system. Use when building interactive UI with Preline -- accordions, dropdowns, modals, tabs, selects, carousels, forms, navigation, or any component. Read this first for overview and quick reference, then load the relevant category file for details.
---

# Preline UI Reference (v4.1.3)

Preline is NOT like DaisyUI. It does not provide pre-built component classes. It provides:
1. **70+ CSS component patterns** composed from Tailwind utilities + semantic design tokens
2. **A semantic design token system** (200+ CSS variables for theming via `theme.css`)
3. 27 headless JS plugins and 55 custom variants (reference only -- **Capsem does NOT use Preline JS**)

**IMPORTANT: In Capsem, we use Preline CSS-only.** All interactivity is pure Svelte 5 runes + TypeScript. Copy the CSS class strings from Preline component docs, but drive active/open/selected state with Svelte `$state`/`$derived`, NOT with `data-hs-*` attributes or `hs-*-active:` variants. See `framework-integration.md` for the full setup.

## Installation

```css
/* global.css */
@import "tailwindcss";

/* Preline UI -- CSS tokens only */
@source "../../node_modules/preline";

/* Preline Themes */
@import "preline/css/themes/theme.css";
/* ... plus other themes as needed */
```

```bash
pnpm add preline
```

## Plugin Initialization Patterns

**CSS-class-based** (5 plugins): element has `.hs-{name}` class, options via CSS custom properties
- `.hs-accordion`, `.hs-collapse-toggle`, `.hs-dropdown`, `.hs-overlay`, `.hs-tooltip`

**Data-attribute JSON** (22 plugins): element has `data-hs-{name}='{json}'`
- All other plugins (carousel, combobox, datepicker, select, stepper, etc.)

**CSS custom property config** (dropdown + tooltip): `--trigger`, `--placement`, `--strategy`, `--auto-close`, `--offset`, `--scope`

## 27 JS Plugins Quick Reference

| Plugin | Init Selector | Key Methods | Primary Variant |
|--------|--------------|-------------|-----------------|
| HSAccordion | `.hs-accordion` | `show()`, `hide()`, `update()` | `hs-accordion-active:` |
| HSCarousel | `[data-hs-carousel]` | `goToPrev()`, `goToNext()`, `goTo(i)` | `hs-carousel-active:` |
| HSCollapse | `.hs-collapse-toggle` | `show()`, `hide()` | `hs-collapse-open:` |
| HSComboBox | `[data-hs-combo-box]` | `open()`, `close()`, `getCurrentData()` | `hs-combo-box-active:` |
| HSCopyMarkup | `[data-hs-copy-markup]` | `delete(target)` | -- |
| HSDataTable | `[data-hs-datatable]` | `destroy()` | `hs-datatable-ordering-asc:` |
| HSDatepicker | `[data-hs-datepicker]` | `formatDate()` | -- |
| HSDropdown | `.hs-dropdown` | `open()`, `close()`, `forceClearState()` | `hs-dropdown-open:` |
| HSFileUpload | `[data-hs-file-upload]` | `destroy()` | `hs-file-upload-complete:` |
| HSInputNumber | `[data-hs-input-number]` | `destroy()` | `hs-input-number-disabled:` |
| HSLayoutSplitter | `[data-hs-layout-splitter]` | `setSplitterItemSize()`, `updateFlexValues()` | `hs-layout-splitter-dragging:` |
| HSOverlay | `.hs-overlay` | `open()`, `close()`, `minify()` | `hs-overlay-open:` |
| HSPinInput | `[data-hs-pin-input]` | `destroy()` | `hs-pin-input-active:` |
| HSRangeSlider | `[data-hs-range-slider]` | `destroy()` | `hs-range-slider-disabled:` |
| HSRemoveElement | `[data-hs-remove-element]` | `destroy()` | `hs-removing:` |
| HSScrollNav | `[data-hs-scroll-nav]` | `goTo()`, `centerElement()` | `hs-scroll-nav-active:` |
| HSScrollspy | `[data-hs-scrollspy]` | `destroy()` | `hs-scrollspy-active:` |
| HSSelect | `[data-hs-select]` | `setValue()`, `open()`, `close()`, `addOption()` | `hs-selected:` |
| HSStepper | `[data-hs-stepper]` | `goToNext()`, `goToFinish()`, `setErrorNavItem()` | `hs-stepper-active:` |
| HSStrongPassword | `[data-hs-strong-password]` | `recalculateDirection()` | `hs-strong-password:` |
| HSTabs | `[role="tablist"]` | `destroy()` | `hs-tab-active:` |
| HSTextareaAutoHeight | `[data-hs-textarea-auto-height]` | `destroy()` | -- |
| HSThemeSwitch | `[data-hs-theme-switch]` | `setAppearance()` | `hs-dark-mode-active:` |
| HSToggleCount | `[data-hs-toggle-count]` | `countUp()`, `countDown()` | -- |
| HSTogglePassword | `[data-hs-toggle-password]` | `show()`, `hide()` | -- |
| HSTooltip | `.hs-tooltip` | `show()`, `hide()` | `hs-tooltip-shown:` |
| HSTreeView | `[data-hs-tree-view]` | `getSelectedItems()`, `changeItemProp()` | `hs-tree-view-selected:` |

## CSS Component Categories

| Category | Components |
|----------|-----------|
| Layout & Content | Container, Columns, Grid, Typography, Images, Links, Dividers, KBD, Custom Scrollbar |
| Base Components | Alerts, Avatar, Avatar Group, Badge, Blockquote, Buttons, Button Group, Card, Chat Bubbles, Devices, Lists, List Group, Legend Indicator, Progress, Ratings, Skeleton, Spinners, Styled Icons, Toasts, Timeline |
| Navigations | Navbar, Mega Menu, Navs, Sidebar, Breadcrumb, Pagination |
| Basic Forms | Input, Input Group, Textarea, File Input, Checkbox, Radio, Switch, Select, Range Slider, Color Picker, Time Picker |
| Overlays | Context Menu, Modal, Offcanvas/Drawer, Popover |
| Tables | Tables |
| Third-Party | Charts (ApexCharts), Clipboard, Datamaps, Datatables, Drag and Drop, File Upload (Dropzone), Maps, Toast Notifications, WYSIWYG Editor |

## Reference Files

Read the relevant file when you need details:

| File | Contents |
|------|----------|
| `preline-docs/javascript-api.md` | Import patterns, auto-init, getInstance, events, TypeScript, base plugin API |
| `preline-docs/framework-integration.md` | Astro + Svelte setup, SPA re-init, TypeScript declarations |
| `preline-docs/plugins-layout.md` | Accordion, Tabs, Collapse, Stepper, Scrollspy, ScrollNav |
| `preline-docs/plugins-overlays.md` | Dropdown, Overlay/Modal, Tooltip, ComboBox, Select |
| `preline-docs/plugins-forms.md` | InputNumber, PinInput, TogglePassword, StrongPassword, TextareaAutoHeight, ToggleCount, Datepicker, RangeSlider, FileUpload |
| `preline-docs/plugins-content.md` | Carousel, CopyMarkup, RemoveElement, DataTable, TreeView, LayoutSplitter, ThemeSwitch |
| `preline-docs/components-base.md` | Alerts, Avatar, Badge, Buttons, Card, Chat Bubbles, Lists, Progress, Skeleton, Spinners, Toasts, Timeline, etc. |
| `preline-docs/components-navigation.md` | Navbar, Mega Menu, Navs, Sidebar, Breadcrumb, Pagination |
| `preline-docs/components-forms.md` | Input, Textarea, Checkbox, Radio, Switch, Select (native), File Input |
| `preline-docs/components-overlays.md` | Context Menu, Modal, Offcanvas/Drawer, Popover |
| `preline-docs/components-layout.md` | Container, Columns, Grid, Typography, Images, Dividers, KBD, Scrollbar |
| `preline-docs/variants.md` | All 55 @custom-variant declarations with usage examples |
| `preline-docs/tokens.md` | Design token system, theming, dark mode, customization, premade themes |

## Semantic Token Pattern

Preline components use semantic tokens, not raw Tailwind colors:

```html
<!-- Buttons use token classes -->
<button class="bg-primary text-primary-foreground hover:bg-primary-hover">Solid</button>
<button class="bg-layer border border-layer-line text-layer-foreground hover:bg-layer-hover">White</button>

<!-- Cards use token classes -->
<div class="bg-card border border-card-line rounded-xl">
  <div class="bg-surface border-b border-card-divider rounded-t-xl py-3 px-4">Header</div>
  <div class="p-4 text-foreground">Content</div>
</div>

<!-- Navigation uses tiered tokens -->
<nav class="bg-navbar border-b border-navbar-border">
  <a class="text-navbar-nav-foreground hover:bg-navbar-nav-hover">Link</a>
</nav>
```

Dark mode is automatic: add `.dark` to `<html>` and all tokens flip.
