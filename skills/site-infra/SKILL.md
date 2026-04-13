---
name: site-infra
description: Capsem documentation site infrastructure and conventions. Use when writing, editing, or maintaining docs in the docs/ directory (docs.capsem.org), adding new doc pages, updating the sidebar, or working with Astro Starlight. Covers site structure, frontmatter, writing style, sidebar config, release pages, and dev workflow.
---

# Documentation Site

The documentation site (docs.capsem.org) uses [Astro Starlight](https://starlight.astro.build/) (Astro 6 + Tailwind v4). Docs live in `docs/src/content/docs/` as markdown/MDX files.

## Dev workflow

```bash
cd docs && pnpm run dev     # localhost:4321
cd docs && pnpm run build   # Production build
```

## Writing style

Tight and to the point, like a manual. One topic per page. No filler, no marketing language. Tables over prose when listing configs or test cases. Code examples only when they clarify usage. Diagrams in mermaid.

## Frontmatter

Every doc page must include `title` and `description`. Starlight handles `lastUpdated` from git history automatically. No `layout:` field -- Starlight provides its own.

```markdown
---
title: Page Title
description: One-line summary for SEO and sidebar tooltips.
sidebar:
  order: 10
---
```

## Site structure

```
docs/src/content/docs/
  getting-started.md
  architecture/
    hypervisor.md         Hypervisor abstraction, Apple VZ + KVM backends (5 mermaid diagrams)
    settings.md           Settings grammar, value resolution, presets, IPC, boot injection
    build-system.md       capsem-builder architecture, TOML configs, Jinja, multi-arch
    custom-images.md      Corporate image customization guide
    settings-schema.md    Two-node schema, JSON Schema, Pydantic, cross-language conformance
  security/
    overview.md           Security model overview
    network-isolation.md  Air-gapped networking, domain policy
    virtualization.md     VM isolation guarantees
    build-verification.md Build reproducibility, checksums
    kernel-hardening.md   Custom kernel, allnoconfig, minimal attack surface
  benchmarks/
    results.md            Current performance results (boot, disk, CLI, HTTP, snapshots)
  debugging/
    capsem-doctor.md      In-VM diagnostic suite
    troubleshooting.md    Common issues and solutions
  development/
    benchmarking.md       How to run and extend capsem-bench
    getting-started.md    Dev environment setup (stub)
    skills.md             AI agent skills system
  releases/
    0-8.md through 0-14.md   One page per minor version
```

## Sidebar

Configured in `docs/astro.config.mjs` under `starlight({ sidebar: [...] })`. Uses `autogenerate: { directory: '<category>' }` for each section. Page ordering within a section uses `sidebar: { order: N }` in frontmatter.

## Adding a new doc page

1. Create `docs/src/content/docs/<category>/<topic>.md` with frontmatter
2. It auto-appears in the sidebar via `autogenerate`
3. Set `sidebar: { order: N }` to control position (lower = higher in list)

## Adding a new category

1. Create the directory under `docs/src/content/docs/`
2. Add a sidebar entry in `docs/astro.config.mjs`:
   ```js
   { label: 'Category Name', autogenerate: { directory: 'category-slug' } }
   ```

## Release pages

- Path: `docs/src/content/docs/releases/<major>-<minor>.md` (hyphens, not dots)
- Each page consolidates all patch releases for that minor version
- Higher `sidebar.order` = newer = listed first (reverse-chrono)
- When bumping to a new minor, create a new page

## Mermaid diagrams

The site uses `astro-mermaid` for rendering. Use fenced code blocks:

````markdown
```mermaid
graph LR
  A --> B --> C
```
````

## Astro reference

Read `references/astro.md` for Astro framework patterns (components, content collections, SSR, CLI). From the official Astro team.

## Theme

Custom CSS in `docs/src/styles/custom.css`. Accent colors and fonts. Logo at `docs/src/assets/logo.svg`.

## Graphics and icons

Source of truth for all icons: `graphics/` at the project root.

```
graphics/
  icon/                        Brand icon in multiple sizes and variants
    icon-mainfile.ai           Illustrator source file
    22w/                       22px (menu bar)
    1x/                        726px (standard)
    2x/                        1450px (retina)
    3x/                        2176px
    4x/                        2900px
    1024w/                     1024px (app store, high-res)
    Variants: capsem-logo-{black,color,grey,white}.png
  tauri/                       Pre-built Tauri app icon set
    32x32.png, 128x128.png, 128x128@2x.png
    icon.icns, icon.ico, icon.svg
```

Site favicons in `docs/public/` are generated from `graphics/icon/1024w/capsem-logo-color.png`. To regenerate:

```bash
sips -z 16 16 graphics/icon/1024w/capsem-logo-color.png --out docs/public/favicon-16x16.png
sips -z 32 32 graphics/icon/1024w/capsem-logo-color.png --out docs/public/favicon-32x32.png
sips -z 180 180 graphics/icon/1024w/capsem-logo-color.png --out docs/public/apple-touch-icon.png
sips -z 192 192 graphics/icon/1024w/capsem-logo-color.png --out docs/public/android-chrome-192x192.png
sips -z 512 512 graphics/icon/1024w/capsem-logo-color.png --out docs/public/android-chrome-512x512.png
```

## Drafts

`tmp/build_sprint/custom-images.md` -- 443-line draft for the custom images doc. Covers quick start, config reference, CLI reference, manifest, corporate deployment, troubleshooting.

## Page scope boundaries

- **`development/getting-started.md`** is strictly about environment setup: prerequisites, clone, bootstrap, build-assets, codesign, first run. Troubleshooting in this page must be limited to setup failures (doctor, codesign, build-assets OOM/clock, missing assets). Runtime issues (disk full, boot hangs, cross-compile errors, network problems) belong in `debugging/troubleshooting.md` -- link there instead of duplicating.
- **`debugging/troubleshooting.md`** is the catch-all for runtime issues. New troubleshooting entries go here unless they are specifically about first-time env setup.

## Keep docs in sync

When features change (settings, CLI flags, MCP tools, security invariants, benchmarks), update the corresponding doc page. When cutting a new minor release, create a new release page. Most pages are still stubs -- fill them in as features stabilize.
