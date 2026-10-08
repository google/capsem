---
name: site-marketing
description: The marketing website, capsem.org. Use when editing copy, adding sections, or changing the site theme.
---

# Marketing Website

The marketing site (capsem.org) is a single-page landing built with Astro 7 + Svelte 5 + Tailwind v4. Source lives in `web/marketing/`.

## Purpose and content budget

`capsem.org` answers: “Why Capsem, what can I do, and how quickly can I get value?”
Lead with the benefit, concrete use cases, and a short, accurate path to the
first useful result.

`docs.capsem.org` answers: “Exactly how does this work, how do I
operate/integrate/administer it, and what is the contract?” Link to the
approved manual for detailed procedures, configuration, interfaces, and
guarantees; `/site-infra` owns those instructions.

Use the minimum copy and pages needed to answer the marketing questions.
Keep technical detail only when it helps a visitor assess value or take the
next step, and verify claims against current product behavior. Avoid repeating
the manual on the landing page.

New pages require explicit user approval before creation. Prefer updating an
existing section within the requested scope. The user will tell us what to
write; do not proactively propose pages, outlines, or topics. A general
website-improvement request does not approve new pages.

## Dev workflow

```bash
cd web/marketing && pnpm run dev     # localhost:4321
cd web/marketing && pnpm run build   # Production build
cd web/marketing && pnpm run preview # Preview production build
```

## CI and deploy rail

`ci.yaml` runs the merge-blocking `site-build` job under `pr-gate`. `site.yaml`
deploys only on every push to `main` and smokes `https://capsem.org/`. This
deploy rail is independent from binary releases, manual VM asset releases, and
the `release.capsem.org` asset-channel workflow.

## Architecture

Single page (`web/marketing/src/pages/index.astro`) composed of Svelte components. All marketing copy is centralized in `web/marketing/src/lib/data.ts` -- edit that file to change text, not the components.

```
web/marketing/
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

All text lives in `web/marketing/src/lib/data.ts` as typed const exports:

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

Defined in `web/marketing/src/styles/global.css` using Tailwind v4 `@theme` tokens:

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

Icons use inline SVG paths from `web/marketing/src/lib/icons.ts`, rendered via `Icon.svelte`. Favicons in `web/marketing/public/` are generated from `web/graphics/icon/1024w/capsem-logo-color.png`.
