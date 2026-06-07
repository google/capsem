# Preline CSS Components: Navigation

## Navbar

Uses `bg-navbar` token family. Three style tiers: default, `-1`, `-2`. Mobile collapse uses HSCollapse plugin.

```html
<header class="bg-navbar border-b border-navbar-border">
  <nav class="max-w-7xl mx-auto flex items-center justify-between py-3 px-4">
    <a class="text-xl font-semibold text-foreground" href="#">Brand</a>

    <!-- Mobile toggle (uses HSCollapse) -->
    <button class="hs-collapse-toggle md:hidden size-9 flex justify-center items-center rounded-lg bg-muted text-muted-foreground-1" data-hs-collapse="#navbar-collapse">
      <svg class="hs-collapse-open:hidden size-4"><!-- hamburger --></svg>
      <svg class="hidden hs-collapse-open:block size-4"><!-- close --></svg>
    </button>

    <!-- Nav links -->
    <div id="navbar-collapse" class="hs-collapse hidden md:block">
      <div class="flex flex-col md:flex-row md:items-center gap-5">
        <a class="text-sm text-navbar-nav-foreground hover:bg-navbar-nav-hover rounded-lg py-2 px-3" href="#">Home</a>
        <a class="text-sm text-navbar-nav-foreground hover:bg-navbar-nav-hover rounded-lg py-2 px-3" href="#">About</a>
      </div>
    </div>
  </nav>
</header>
```

**Token tiers**:
- Default: `bg-navbar`, `border-navbar-border`, `text-navbar-nav-foreground`, `hover:bg-navbar-nav-hover`
- Tier 1: `bg-navbar-1`, `border-navbar-1-border`, `text-navbar-1-nav-foreground`, `hover:bg-navbar-1-nav-hover`
- Tier 2: `bg-navbar-2`, `border-navbar-2-border`, `text-navbar-2-nav-foreground`, `hover:bg-navbar-2-nav-hover`

## Mega Menu

Uses HSCollapse plugin for toggling. Content is a grid layout inside the collapse target.

```html
<div class="hs-collapse hidden" id="mega-menu-content">
  <div class="max-w-7xl mx-auto grid md:grid-cols-3 gap-4 p-4">
    <div>
      <h4 class="text-sm font-semibold text-foreground mb-2">Category</h4>
      <a class="block py-2 text-sm text-muted-foreground-1 hover:text-foreground" href="#">Link</a>
    </div>
  </div>
</div>
```

## Navs

Horizontal or vertical link groups, often used for sub-navigation.

```html
<!-- Pills -->
<nav class="flex gap-x-1">
  <a class="py-2 px-3 text-sm font-medium rounded-lg bg-primary text-primary-foreground" href="#" aria-current="page">Active</a>
  <a class="py-2 px-3 text-sm font-medium rounded-lg text-muted-foreground-1 hover:text-foreground" href="#">Link</a>
</nav>

<!-- Underline (with HSTabs) -->
<nav class="flex gap-x-1 border-b border-line-1" aria-label="Tabs" role="tablist">
  <button class="hs-tab-active:border-primary hs-tab-active:text-primary py-4 px-1 text-sm font-medium border-b-2 border-transparent text-muted-foreground-1 active" data-hs-tab="#panel-1" role="tab">Tab 1</button>
  <button class="hs-tab-active:border-primary hs-tab-active:text-primary py-4 px-1 text-sm font-medium border-b-2 border-transparent text-muted-foreground-1" data-hs-tab="#panel-2" role="tab">Tab 2</button>
</nav>
```

## Sidebar

Uses `bg-sidebar` token family. Three style tiers like navbar.

```html
<aside class="fixed inset-y-0 start-0 z-50 w-64 bg-sidebar border-e border-sidebar-border">
  <div class="p-4">
    <a class="text-xl font-semibold text-foreground" href="#">Brand</a>
  </div>

  <nav class="p-4 space-y-1">
    <a class="flex items-center gap-x-3 py-2 px-3 text-sm text-sidebar-nav-foreground rounded-lg hover:bg-sidebar-nav-hover" href="#">
      <svg class="size-4"><!-- icon --></svg>
      Dashboard
    </a>
    <a class="flex items-center gap-x-3 py-2 px-3 text-sm text-sidebar-nav-foreground rounded-lg bg-sidebar-nav-active" href="#" aria-current="page">
      <svg class="size-4"><!-- icon --></svg>
      Active Item
    </a>

    <!-- Collapsible section (uses HSAccordion) -->
    <div class="hs-accordion" id="sidebar-section">
      <button class="hs-accordion-toggle flex items-center gap-x-3 py-2 px-3 w-full text-sm text-sidebar-nav-foreground rounded-lg hover:bg-sidebar-nav-hover">
        <svg class="size-4"><!-- icon --></svg>
        Section
        <svg class="hs-accordion-active:rotate-180 ms-auto size-4"><!-- chevron --></svg>
      </button>
      <div class="hs-accordion-content hidden w-full overflow-hidden transition-[height] duration-300">
        <ul class="ps-7 space-y-1 mt-1">
          <li><a class="py-2 px-3 text-sm text-sidebar-nav-foreground rounded-lg hover:bg-sidebar-nav-hover block" href="#">Sub Item</a></li>
        </ul>
      </div>
    </div>
  </nav>
</aside>
```

**Token tiers**:
- Default: `bg-sidebar`, `border-sidebar-border`, `text-sidebar-nav-foreground`, `hover:bg-sidebar-nav-hover`, `bg-sidebar-nav-active`
- Tier 1: `bg-sidebar-1`, `border-sidebar-1-border`, etc.
- Tier 2: `bg-sidebar-2`, etc.

## Breadcrumb

```html
<ol class="flex items-center whitespace-nowrap">
  <li class="inline-flex items-center">
    <a class="flex items-center text-sm text-muted-foreground-1 hover:text-primary" href="#">Home</a>
    <svg class="shrink-0 mx-2 size-4 text-muted-foreground"><!-- chevron --></svg>
  </li>
  <li class="inline-flex items-center">
    <a class="flex items-center text-sm text-muted-foreground-1 hover:text-primary" href="#">Category</a>
    <svg class="shrink-0 mx-2 size-4 text-muted-foreground"><!-- chevron --></svg>
  </li>
  <li class="inline-flex items-center text-sm font-semibold text-foreground truncate" aria-current="page">
    Current Page
  </li>
</ol>
```

## Pagination

```html
<nav class="flex items-center gap-x-1">
  <button class="min-h-9.5 min-w-9.5 py-2 px-2.5 inline-flex justify-center items-center gap-x-2 text-sm rounded-lg text-muted-foreground-1 hover:bg-muted-hover disabled:opacity-50" disabled>
    <svg class="size-3.5"><!-- prev --></svg>
  </button>
  <div class="flex items-center gap-x-1">
    <button class="min-h-9.5 min-w-9.5 flex justify-center items-center bg-primary text-primary-foreground py-2 px-3 text-sm rounded-lg">1</button>
    <button class="min-h-9.5 min-w-9.5 flex justify-center items-center text-muted-foreground-1 hover:bg-muted-hover py-2 px-3 text-sm rounded-lg">2</button>
    <button class="min-h-9.5 min-w-9.5 flex justify-center items-center text-muted-foreground-1 hover:bg-muted-hover py-2 px-3 text-sm rounded-lg">3</button>
  </div>
  <button class="min-h-9.5 min-w-9.5 py-2 px-2.5 inline-flex justify-center items-center gap-x-2 text-sm rounded-lg text-muted-foreground-1 hover:bg-muted-hover">
    <svg class="size-3.5"><!-- next --></svg>
  </button>
</nav>
```
