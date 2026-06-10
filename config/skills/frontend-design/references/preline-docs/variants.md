# Preline Custom Tailwind Variants

Preline provides 55 `@custom-variant` declarations imported via `@import "preline/variants.css"`. Use them as Tailwind class prefixes to style elements based on plugin state.

## Usage Pattern

```html
<!-- Show/hide based on dropdown state -->
<svg class="hs-dropdown-open:rotate-180 size-4 transition-transform"><!-- chevron --></svg>

<!-- Style active tab -->
<button class="hs-tab-active:bg-primary hs-tab-active:text-primary-foreground py-3 px-4 rounded-lg" data-hs-tab="#panel-1">
  Tab 1
</button>

<!-- Animate element removal -->
<div class="hs-removing:translate-x-5 hs-removing:opacity-0 transition duration-300">
  Dismissible content
</div>
```

Variants match both the element itself AND its descendants when a parent has the state class, unless noted.

## Accordion

| Variant | Matches When |
|---------|-------------|
| `hs-accordion-active:` | `.hs-accordion.active` (open), its direct children, toggle children |
| `hs-accordion-selected:` | `.selected` inside `.hs-accordion` (selectable items) |
| `hs-accordion-outside-active:` | Element itself has `.active` class |

## Carousel

| Variant | Matches When |
|---------|-------------|
| `hs-carousel-active:` | Element or parent has `.active` (current slide/dot) |
| `hs-carousel-disabled:` | Element or parent has `.disabled` (prev/next at boundary) |
| `hs-carousel-dragging:` | Element or parent has `.dragging` (during drag) |

## Collapse

| Variant | Matches When |
|---------|-------------|
| `hs-collapse-open:` | `.hs-collapse.open` or `.hs-collapse-toggle.open`, and their children |

## ComboBox

| Variant | Matches When |
|---------|-------------|
| `hs-combo-box-active:` | Element or parent has `.active` (dropdown open) |
| `hs-combo-box-has-value:` | Element or parent has `.has-value` |
| `hs-combo-box-selected:` | Element or parent has `.selected` |
| `hs-combo-box-tab-active:` | Element itself has `.active` (grouping tab) |

## DataTable

| Variant | Matches When |
|---------|-------------|
| `hs-datatable-ordering-asc:` | Element or parent has `.dt-ordering-asc` |
| `hs-datatable-ordering-desc:` | Element or parent has `.dt-ordering-desc` |

## Datepicker

| Variant | Matches When |
|---------|-------------|
| `hs-vc-date-today:` | `[data-vc-date-today]` attribute |
| `hs-vc-date-hover:` | `[data-vc-date-hover]` attribute |
| `hs-vc-date-hover-first:` | `[data-vc-date-hover="first"]` and children |
| `hs-vc-date-hover-last:` | `[data-vc-date-hover="last"]` and children |
| `hs-vc-date-selected:` | `[data-vc-date-selected]` attribute |
| `hs-vc-calendar-selected-middle:` | `[data-vc-date-selected="middle"]` and children |
| `hs-vc-calendar-selected-first:` | `[data-vc-date-selected="first"]` and children |
| `hs-vc-calendar-selected-last:` | `[data-vc-date-selected="last"]` and children |
| `hs-vc-date-weekend:` | `[data-vc-date-weekend]` attribute |
| `hs-vc-week-day-off:` | `[data-vc-week-day-off]` attribute |
| `hs-vc-date-month-prev:` | `[data-vc-date-month="prev"]` |
| `hs-vc-date-month-next:` | `[data-vc-date-month="next"]` |
| `hs-vc-calendar-hidden:` | `[data-vc-calendar-hidden]` and children |
| `hs-vc-months-month-selected:` | `[data-vc-months-month-selected]` |
| `hs-vc-years-year-selected:` | `[data-vc-years-year-selected]` |

## Dropdown

| Variant | Matches When |
|---------|-------------|
| `hs-dropdown-open:` | `.hs-dropdown.open` direct children, toggle children, menu children |
| `hs-dropdown-item-disabled:` | `.disabled` item inside open dropdown menu |
| `hs-dropdown-item-checked:` | `[aria-checked="true"]` item inside open dropdown menu |

## File Upload

| Variant | Matches When |
|---------|-------------|
| `hs-file-upload-complete:` | Element or parent has `.complete` |

## Input Number

| Variant | Matches When |
|---------|-------------|
| `hs-input-number-disabled:` | Element or parent has `.disabled` |

## Layout Splitter

| Variant | Matches When |
|---------|-------------|
| `hs-layout-splitter-dragging:` | Element or parent has `.dragging` |
| `hs-layout-splitter-prev-limit-reached:` | Element or parent has `.prev-limit-reached` |
| `hs-layout-splitter-next-limit-reached:` | Element or parent has `.next-limit-reached` |
| `hs-layout-splitter-prev-pre-limit-reached:` | Element or parent has `.prev-pre-limit-reached` |
| `hs-layout-splitter-next-pre-limit-reached:` | Element or parent has `.next-pre-limit-reached` |

## Overlay

| Variant | Matches When |
|---------|-------------|
| `hs-overlay-open:` | Element or parent has `.open` |
| `hs-overlay-layout-open:` | `body.hs-overlay-body-open` and children |
| `hs-overlay-minified:` | `.minified` or `body.hs-overlay-minified` and children |
| `hs-overlay-backdrop-open:` | `.hs-overlay-backdrop` and children |

## PIN Input

| Variant | Matches When |
|---------|-------------|
| `hs-pin-input-active:` | Element or parent has `.active` (all fields filled) |

## Range Slider

| Variant | Matches When |
|---------|-------------|
| `hs-range-slider-disabled:` | Element or parent has `.disabled` |

## Remove Element

| Variant | Matches When |
|---------|-------------|
| `hs-removing:` | Element has `.hs-removing` class (during removal animation) |

## Scroll Nav

| Variant | Matches When |
|---------|-------------|
| `hs-scroll-nav-active:` | Element itself has `.active` |
| `hs-scroll-nav-disabled:` | Element or parent has `.disabled` |

## Scrollspy

| Variant | Matches When |
|---------|-------------|
| `hs-scrollspy-active:` | Element itself has `.active` (current section link) |

## Select

| Variant | Matches When |
|---------|-------------|
| `hs-selected:` | Element or parent has `.selected` (selected option) |
| `hs-select-disabled:` | Element or parent has `.disabled` |
| `hs-select-active:` | Element or parent has `.active` |
| `hs-select-opened:` | Element has `.opened` (dropdown visible) |

## Stepper

| Variant | Matches When |
|---------|-------------|
| `hs-stepper-active:` | Element or parent has `.active` (current step) |
| `hs-stepper-success:` | Element or parent has `.success` (completed step) |
| `hs-stepper-completed:` | Element or parent has `.completed` (all steps done) |
| `hs-stepper-error:` | Element or parent has `.error` |
| `hs-stepper-processed:` | Element or parent has `.processed` |
| `hs-stepper-disabled:` | Element or parent has `.disabled` |
| `hs-stepper-skipped:` | Element or parent has `.skipped` |

## Strong Password

| Variant | Matches When |
|---------|-------------|
| `hs-password-active:` | Element or parent has `.active` (toggle active) |
| `hs-strong-password:` | Element or parent has `.passed` (strength strip passed) |
| `hs-strong-password-accepted:` | Element or parent has `.accepted` (all checks pass) |
| `hs-strong-password-active:` | Element itself has `.active` (individual rule passed) |

## Tabs

| Variant | Matches When |
|---------|-------------|
| `hs-tab-active:` | `[data-hs-tab].active` and its children |

## Theme Switch

| Variant | Matches When |
|---------|-------------|
| `hs-default-mode-active:` | `html.default` descendant |
| `hs-light-mode-active:` | `html.light:not(.auto)` descendant |
| `hs-dark-mode-active:` | `html.dark:not(.auto)` descendant |
| `hs-auto-mode-active:` | `html.auto` descendant |
| `hs-auto-dark-mode-active:` | `html.auto.dark` descendant |
| `hs-auto-light-mode-active:` | `html.auto.light` descendant |

## Tooltip

| Variant | Matches When |
|---------|-------------|
| `hs-tooltip-shown:` | `.hs-tooltip-content.show` or child of `.hs-tooltip.show` |

## Tree View

| Variant | Matches When |
|---------|-------------|
| `hs-tree-view-selected:` | `[data-hs-tree-view-item].selected` and direct children |
| `hs-tree-view-disabled:` | `[data-hs-tree-view-item].disabled` and direct children |

## Global Variants

| Variant | Matches When |
|---------|-------------|
| `hs-success:` | `.success` element or descendant of `.success` |
| `hs-error:` | `.error` element or descendant of `.error` |
| `hs-apexcharts-tooltip-dark:` | `.dark` element (ApexCharts tooltip in dark mode) |
| `hs-dragged:` | `.dragged` element (Sortable.js) |
| `hs-toastify-on:` | `.toastify.on` element or descendant (Toastify active toast) |
