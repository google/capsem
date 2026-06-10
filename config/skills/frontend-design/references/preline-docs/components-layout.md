# Preline CSS Components: Layout & Content

## Container

```html
<div class="max-w-[85rem] mx-auto px-4 sm:px-6 lg:px-8">
  <!-- Content -->
</div>
```

Preline uses `max-w-[85rem]` (1360px) as the standard container width.

## Grid

Standard Tailwind grid patterns:

```html
<!-- 2 columns -->
<div class="grid sm:grid-cols-2 gap-4">
  <div>Column 1</div>
  <div>Column 2</div>
</div>

<!-- 3 columns -->
<div class="grid sm:grid-cols-2 lg:grid-cols-3 gap-4">
  <div>Column 1</div>
  <div>Column 2</div>
  <div>Column 3</div>
</div>

<!-- Sidebar layout -->
<div class="grid lg:grid-cols-[256px_1fr] gap-4">
  <aside>Sidebar</aside>
  <main>Content</main>
</div>
```

## Columns

CSS multi-column layout:

```html
<div class="columns-1 sm:columns-2 lg:columns-3 gap-4 space-y-4">
  <div class="break-inside-avoid">Item 1</div>
  <div class="break-inside-avoid">Item 2</div>
  <div class="break-inside-avoid">Item 3</div>
</div>
```

## Typography

```html
<!-- Headings -->
<h1 class="text-3xl font-bold text-foreground sm:text-4xl">Heading 1</h1>
<h2 class="text-2xl font-bold text-foreground sm:text-3xl">Heading 2</h2>
<h3 class="text-xl font-semibold text-foreground">Heading 3</h3>

<!-- Body -->
<p class="text-foreground">Default body text</p>
<p class="text-muted-foreground-1">Secondary text</p>
<p class="text-muted-foreground">Muted text</p>

<!-- Lead text -->
<p class="text-xl text-muted-foreground-1">Lead paragraph for introductions.</p>

<!-- Small text -->
<p class="text-xs text-muted-foreground">Fine print</p>
```

## Images

```html
<!-- Responsive -->
<img class="w-full h-auto rounded-xl" src="..." alt="...">

<!-- With hover zoom -->
<div class="overflow-hidden rounded-xl">
  <img class="w-full h-auto hover:scale-105 transition-transform duration-500" src="..." alt="...">
</div>

<!-- Aspect ratio -->
<div class="relative pt-[56.25%] rounded-xl overflow-hidden">
  <img class="absolute top-0 start-0 object-cover size-full" src="..." alt="...">
</div>
```

## Links

```html
<a class="text-primary hover:text-primary-hover font-medium" href="#">Default link</a>
<a class="text-primary decoration-2 hover:underline font-medium" href="#">Underline on hover</a>
<a class="text-muted-foreground-1 underline underline-offset-4 hover:text-foreground hover:decoration-2" href="#">Subtle link</a>
```

## Dividers

```html
<!-- Basic -->
<hr class="border-line-1">

<!-- With text -->
<div class="flex items-center text-xs text-muted-foreground uppercase before:flex-1 before:border-t before:border-line-1 before:me-6 after:flex-1 after:border-t after:border-line-1 after:ms-6">
  Or
</div>
```

## KBD

```html
<kbd class="inline-flex justify-center items-center py-1 px-1.5 bg-layer border border-layer-line font-mono text-xs text-muted-foreground-1 rounded-md shadow-[0px_2px_0px_0px_rgba(0,0,0,0.08)]">
  Ctrl
</kbd>
```

## Custom Scrollbar

Uses `scrollbar-track` and `scrollbar-thumb` tokens:

```html
<div class="h-48 overflow-y-auto
  [&::-webkit-scrollbar]:w-2
  [&::-webkit-scrollbar-track]:rounded-full [&::-webkit-scrollbar-track]:bg-scrollbar-track
  [&::-webkit-scrollbar-thumb]:rounded-full [&::-webkit-scrollbar-thumb]:bg-scrollbar-thumb">
  <!-- Scrollable content -->
</div>
```

## Tables

```html
<div class="flex flex-col">
  <div class="-m-1.5 overflow-x-auto">
    <div class="p-1.5 min-w-full inline-block align-middle">
      <div class="border border-table-line rounded-lg overflow-hidden">
        <table class="min-w-full divide-y divide-table-line">
          <thead class="bg-muted">
            <tr>
              <th class="px-6 py-3 text-start text-xs font-medium text-muted-foreground-1 uppercase">Name</th>
              <th class="px-6 py-3 text-start text-xs font-medium text-muted-foreground-1 uppercase">Email</th>
            </tr>
          </thead>
          <tbody class="divide-y divide-table-line">
            <tr>
              <td class="px-6 py-4 whitespace-nowrap text-sm font-medium text-foreground">John</td>
              <td class="px-6 py-4 whitespace-nowrap text-sm text-muted-foreground-1">john@example.com</td>
            </tr>
          </tbody>
        </table>
      </div>
    </div>
  </div>
</div>
```

Token: `border-table-line` / `divide-table-line` for table borders.
