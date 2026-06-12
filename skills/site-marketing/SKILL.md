---
name: site-marketing
description: Capsem marketing website (capsem.org). Use when editing marketing copy, adding sections, working with components, or changing the site theme. Covers site structure, data-driven content, component library, Tailwind theme, and dev workflow.
---

# Marketing Website

The marketing site (capsem.org) is a single-page landing built with Astro 6 + Svelte 5 + Tailwind v4. Source lives in `site/`.

## Dev workflow

```bash
cd site && pnpm run dev     # localhost:4321
cd site && pnpm run build   # Production build
cd site && pnpm run preview # Preview production build
```

## Architecture

Single page (`site/src/pages/index.astro`) composed of Svelte components. All marketing copy is centralized in `site/src/lib/data.ts` -- edit that file to change text, not the components.

```
site/
  astro.config.mjs           Astro config (site: capsem.org, Svelte + Tailwind)
  package.json               capsem-marketing package
  src/
    pages/index.astro        Single landing page, composes all sections
    layouts/Base.astro       HTML shell (meta, fonts, skip-to-content)
    lib/data.ts              All copy: site metadata, nav, features, FAQ, footer
    lib/icons.ts             Icon SVG paths
    styles/global.css        Tailwind theme tokens, base styles, button utilities
    components/
      Nav.svelte             Top navigation (client:load)
      Hero.svelte            Hero section with install command
      Features.svelte        Feature cards grid
      ProductOverview.svelte Architecture diagram (host/guest/vsock)
      HowItWorks.svelte      Step-by-step explanation
      FAQ.svelte             Accordion FAQ (client:visible)
      CTA.svelte             Call-to-action (client:visible)
      Footer.svelte          Footer with link columns
      Section.svelte         Reusable section wrapper
      SectionHeader.svelte   Reusable heading + subtitle
      Card.svelte            Reusable card component
      Badge.svelte           Reusable badge component
      Icon.svelte            SVG icon component
      InstallCommand.svelte  Copy-to-clipboard install snippet
```

## Content editing

All text lives in `site/src/lib/data.ts` as typed const exports:

| Export | Content |
|--------|---------|
| `SITE` | Name, tagline, description, URLs (GitHub, docs, releases) |
| `NAV_LINKS` | Top nav items |
| `AGENTS` | Supported AI agents list |
| `SECURITY_BLOCKS` | Three security pillars (isolation, inspection, control) |
| `HOST_COMPONENTS` | Host-side architecture diagram items |
| `GUEST_COMPONENTS` | Guest-side architecture diagram items |
| `VSOCK_CHANNELS` | Vsock port labels for architecture diagram |
| `FAQS` | FAQ question/answer pairs |
| `FOOTER_COLUMNS` | Footer link groups |
| `MCP_TOOLS` | MCP tool examples |
| `PACKAGES` | Pre-installed packages list |
| `ROADMAP` | Roadmap items |

## Theme

Defined in `site/src/styles/global.css` using Tailwind v4 `@theme` tokens:

- **Accent**: `--color-accent` (blue), `--color-accent-secondary` (purple), gradient between them
- **Surfaces**: light (`--color-surface`) and dark (`--color-surface-dark`) variants
- **Text**: separate light-bg and dark-bg tokens, all WCAG AA compliant
- **Buttons**: 4 pill variants as `@utility` classes: `btn-primary` (gradient), `btn-dark`, `btn-outline`, `btn-outline-dark`
- **Font**: Inter (loaded from Google Fonts in Base.astro)

## Component patterns

- Sections alternate light/dark backgrounds using `section-dark` utility class
- `Section.svelte` and `SectionHeader.svelte` provide consistent spacing and headings
- Interactive components use Svelte hydration directives: `client:load` (Nav) or `client:visible` (FAQ, CTA)
- `gradient-text` utility for accent-colored headings

## Graphics and icons

Icons use inline SVG paths from `site/src/lib/icons.ts`, rendered via `Icon.svelte`. Favicons in `site/public/` are generated from `graphics/icon/1024w/capsem-logo-color.png`.
