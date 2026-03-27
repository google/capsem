# Preline CSS Components: Base

These are Tailwind utility patterns using Preline's semantic design tokens. No JS plugins needed unless noted.

## Buttons

Six styles: solid, outline, ghost, soft, white, link.

```html
<!-- Solid -->
<button class="py-3 px-4 inline-flex items-center gap-x-2 text-sm font-medium rounded-lg bg-primary border border-primary-line text-primary-foreground hover:bg-primary-hover focus:outline-hidden focus:bg-primary-focus disabled:opacity-50 disabled:pointer-events-none">
  Solid
</button>

<!-- Outline -->
<button class="py-3 px-4 inline-flex items-center gap-x-2 text-sm font-medium rounded-lg border border-layer-line text-muted-foreground-1 hover:border-primary-hover hover:text-primary-hover focus:outline-hidden focus:border-primary-focus focus:text-primary-focus disabled:opacity-50 disabled:pointer-events-none">
  Outline
</button>

<!-- Ghost -->
<button class="py-3 px-4 inline-flex items-center gap-x-2 text-sm font-medium rounded-lg border border-transparent text-primary hover:bg-primary-100 hover:text-primary-800 focus:outline-hidden focus:bg-primary-100 focus:text-primary-800 disabled:opacity-50 disabled:pointer-events-none dark:hover:bg-primary-500/20 dark:hover:text-primary-400">
  Ghost
</button>

<!-- Soft -->
<button class="py-3 px-4 inline-flex items-center gap-x-2 text-sm font-medium rounded-lg border border-transparent bg-primary-100 text-primary-800 hover:bg-primary-200 focus:outline-hidden focus:bg-primary-200 disabled:opacity-50 disabled:pointer-events-none dark:bg-primary-500/20 dark:text-primary-400">
  Soft
</button>

<!-- White -->
<button class="py-3 px-4 inline-flex items-center gap-x-2 text-sm font-medium rounded-lg border border-layer-line bg-layer text-layer-foreground shadow-2xs hover:bg-layer-hover focus:outline-hidden focus:bg-layer-focus disabled:opacity-50 disabled:pointer-events-none">
  White
</button>

<!-- Link -->
<button class="py-3 px-4 inline-flex items-center gap-x-2 text-sm font-medium rounded-lg border border-transparent text-primary hover:text-primary-hover focus:outline-hidden focus:text-primary-hover disabled:opacity-50 disabled:pointer-events-none">
  Link
</button>
```

**Sizes**: Small `py-2 px-3`, Default `py-3 px-4`, Large `p-4 sm:p-5`
**Shapes**: Pilled `rounded-full`, Block `w-full justify-center`
**Icon-only**: Fixed size `size-11 flex justify-center items-center`
**Loading**: Add `animate-spin` spinner SVG, `disabled` attribute

## Alerts

```html
<!-- Soft alert -->
<div class="bg-primary-100 border border-primary-200 text-sm text-primary-800 rounded-lg p-4 dark:bg-primary-500/20 dark:border-primary-900 dark:text-primary-400" role="alert">
  <span class="font-bold">Info</span> alert message
</div>

<!-- Bordered alert with icon -->
<div class="bg-teal-50 border-t-2 border-teal-500 rounded-lg p-4 dark:bg-teal-800/30" role="alert">
  <div class="flex">
    <div class="shrink-0"><span class="inline-flex justify-center items-center size-8 rounded-full border-4 border-teal-100 bg-teal-200 text-teal-800"><!-- icon --></span></div>
    <div class="ms-3">
      <h3 class="text-foreground font-semibold">Title</h3>
      <p class="text-sm text-foreground">Description</p>
    </div>
  </div>
</div>

<!-- Dismissible (uses HSRemoveElement plugin) -->
<div id="alert-1" class="hs-removing:translate-x-5 hs-removing:opacity-0 transition duration-300 bg-teal-50 border border-teal-200 rounded-lg p-4" role="alert">
  <p>Alert text</p>
  <button data-hs-remove-element="#alert-1">Dismiss</button>
</div>
```

## Card

```html
<!-- Basic card -->
<div class="flex flex-col bg-card border border-card-line shadow-2xs rounded-xl">
  <div class="p-4 md:p-5">
    <h3 class="text-lg font-bold text-foreground">Title</h3>
    <p class="mt-2 text-muted-foreground-1">Description</p>
  </div>
</div>

<!-- Card with header/footer -->
<div class="bg-card border border-card-line shadow-2xs rounded-xl">
  <div class="bg-surface border-b border-card-divider rounded-t-xl py-3 px-4 md:px-5">
    <p class="text-sm text-muted-foreground-1">Header</p>
  </div>
  <div class="p-4 md:p-5">Content</div>
  <div class="bg-surface border-t border-card-divider rounded-b-xl py-3 px-4 md:px-5">Footer</div>
</div>

<!-- Card with image -->
<div class="flex flex-col bg-card border border-card-line shadow-2xs rounded-xl overflow-hidden group">
  <img class="w-full h-auto group-hover:scale-105 transition-transform duration-500" src="..." />
  <div class="p-4 md:p-5">
    <h3 class="text-lg font-bold text-foreground">Title</h3>
  </div>
</div>
```

**Sizes**: `p-3` (small), `p-4 md:p-5` (default), `p-4 sm:p-7` (large)
**Bordered top**: `border-t-4 border-t-primary`
**Horizontal**: `sm:flex` on card, `shrink-0 relative w-full sm:max-w-60` on image container

## Avatar

```html
<!-- Sizes -->
<span class="inline-flex items-center justify-center size-8 rounded-full bg-surface"><span class="text-xs font-medium text-surface-foreground">AB</span></span>
<img class="inline-block size-10 rounded-full" src="..." />

<!-- With status -->
<div class="relative inline-block">
  <img class="inline-block size-10 rounded-full" src="..." />
  <span class="absolute bottom-0 end-0 block size-2.5 rounded-full ring-2 ring-white bg-teal-400"></span>
</div>
```

**Avatar group**: Stack with `-me-2` margin and `ring-2 ring-white`

## Badge

```html
<!-- Solid -->
<span class="inline-flex items-center gap-x-1.5 py-1.5 px-3 rounded-full text-xs font-medium bg-primary text-primary-foreground">Badge</span>

<!-- Soft -->
<span class="inline-flex items-center gap-x-1.5 py-1.5 px-3 rounded-full text-xs font-medium bg-primary-100 text-primary-800 dark:bg-primary-500/20 dark:text-primary-400">Badge</span>

<!-- Outline -->
<span class="inline-flex items-center gap-x-1.5 py-1.5 px-3 rounded-full text-xs font-medium border border-primary text-primary">Badge</span>
```

## Progress

```html
<div class="flex w-full h-2 bg-muted rounded-full overflow-hidden" role="progressbar" aria-valuenow="25" aria-valuemin="0" aria-valuemax="100">
  <div class="flex flex-col justify-center rounded-full overflow-hidden bg-primary text-xs text-white text-center" style="width: 25%"></div>
</div>
```

## Spinners

```html
<!-- Border -->
<div class="animate-spin inline-block size-6 border-3 border-current border-t-transparent text-primary rounded-full" role="status"><span class="sr-only">Loading...</span></div>

<!-- Grow -->
<div class="animate-spin inline-block size-6 bg-current rounded-full opacity-75 text-primary" role="status"><span class="sr-only">Loading...</span></div>
```

## Skeleton

```html
<div class="animate-pulse">
  <div class="h-4 bg-muted rounded-full w-48 mb-4"></div>
  <div class="h-2 bg-muted rounded-full max-w-[360px] mb-2.5"></div>
  <div class="h-2 bg-muted rounded-full mb-2.5"></div>
  <div class="h-2 bg-muted rounded-full max-w-[330px]"></div>
</div>
```

## Toasts

```html
<div class="max-w-xs bg-layer border border-layer-line rounded-xl shadow-lg" role="alert">
  <div class="flex p-4">
    <div class="shrink-0"><svg class="size-4 text-teal-500 mt-0.5"><!-- icon --></svg></div>
    <div class="ms-3"><p class="text-sm text-foreground">Toast message</p></div>
  </div>
</div>
```

## Timeline

```html
<div>
  <div class="flex gap-x-3">
    <div class="relative after:absolute after:top-7 after:bottom-0 after:start-3.5 after:w-px after:bg-line-2">
      <div class="relative z-10 size-7 flex justify-center items-center"><div class="size-2 rounded-full bg-surface-3"></div></div>
    </div>
    <div class="grow pt-0.5 pb-8">
      <h3 class="flex gap-x-1.5 font-semibold text-foreground">Event title</h3>
      <p class="mt-1 text-sm text-muted-foreground-1">Description</p>
      <time class="mt-1 text-xs text-muted-foreground">Feb 3, 2024</time>
    </div>
  </div>
</div>
```

## Lists & List Group

```html
<!-- List group -->
<ul class="flex flex-col divide-y divide-line-1">
  <li class="inline-flex items-center gap-x-2 py-3 px-4 text-sm font-medium bg-layer text-foreground -mt-px first:rounded-t-lg first:mt-0 last:rounded-b-lg border border-layer-line">
    List item
  </li>
</ul>
```

## Other Components

- **Blockquote**: `border-s-4 border-line-3 ps-4 italic text-foreground`
- **Chat Bubbles**: Flexbox layout with `bg-primary text-primary-foreground rounded-2xl` (sent) or `bg-muted rounded-2xl` (received)
- **Devices**: Wrapper divs with borders and rounded corners simulating device frames
- **Legend Indicator**: `<span class="size-2.5 inline-block rounded-full bg-primary"></span>`
- **Ratings**: Star SVGs with `text-yellow-400` (filled) and `text-muted` (empty)
- **Styled Icons**: `<span class="inline-flex justify-center items-center size-12 rounded-full bg-primary-100 text-primary-800">`
