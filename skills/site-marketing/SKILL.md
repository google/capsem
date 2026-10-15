---
name: site-marketing
description: The marketing website, capsem.org. Use when editing copy, adding sections, or changing the site theme.
---

# Marketing Website

The single-page marketing site in `web/marketing/` uses Astro 7, Svelte 5,
and Tailwind v4. It explains benefits and use cases; precise procedures and
contracts belong on the documentation site (see `/site-infra`).

## Development and publication

Run these commands through the repository's bounded-command wrapper:

```bash
pnpm --dir web/marketing run dev
pnpm --dir web/marketing run check
pnpm --dir web/marketing run build
pnpm --dir web/marketing run preview
```

`ci.yaml` owns the merge-blocking `site-build` job. Pushes to `main` trigger
`site.yaml`, which builds, deploys to Cloudflare Pages, and smokes `capsem.org`.
Changes to `web/diagrams/` trigger both public-site builds and deployments.
This rail is independent from binary and runtime releases. Use `/release-process`
for channel publication at `release.capsem.org`.

## Architecture and content

`src/pages/index.astro` composes these sections:

| Component | Purpose |
| --- | --- |
| `Nav.svelte` | Responsive navigation; hydrated on load. |
| `Hero.svelte` | Downloads, copyable install command, and animated demo; hydrated on load. |
| `Agents.astro` | Supported agents and bundled tools. |
| `Performance.astro` | Performance claims, methodology, caveats, and sources. |
| `Security.astro` | Shared JointJS security diagram and architecture link. |
| `Control.astro` | Activity, CEL rule, and detection examples. |
| `Footer.svelte` | Brand, license, and source link; rendered statically. |

Keep all copy, UI labels, reviewed documentation destinations, benchmark
disclosures, and fictional hero output in `src/lib/data.ts`. Its exports are
`SITE`, `DOCS`, `NAV_LINKS`, `DOWNLOAD`, `UI`, `AGENTS`, `SPEED`, `SECURITY`,
`SECURITY_DIAGRAM`, `CONTROL`, `HERO_DEMO`, and `HERO_TERMINAL_SCRIPT`.

`Download.svelte` uses `lib/download.ts` for platform detection and the shared
`Modal.svelte` dialog. Keep focus, Escape, backdrop, and close-button behavior
in that dialog. `HeroDemo.svelte` uses `lib/hero-terminal.ts` for playback;
verify animated, paused, replay, and reduced-motion states when changing it.

## Diagrams

Use `@capsem/diagrams/Diagram.astro` from the shared `web/diagrams/` package.
Keep marketing definitions and labels in `data.ts`; do not fork the viewer,
add another renderer, or introduce JointJS+ without an explicit decision.
Use the shared blue/violet accent tones for marketing; the security manual's
permission-level palette serves a separate explanation.

Preserve hardware and process boundaries and connection direction when editing.
Verify readable labels, zoom/pan/fit controls, the text connection table, and
phone layouts in a real browser. Both sites must build after a viewer change.

## Theme and assets

`src/styles/global.css` owns Tailwind `@theme` tokens and shared layout utilities:
`content-shell`, `section-space`, `panel`, `button`, `text-link`, and
`brand-gradient`. `.section-dark` remaps semantic surfaces and text. The hero's
scene styling lives in `styles/hero-demo.css`. Inter is loaded by `Base.astro`.

The visible logo is `public/logo-color.png`. Agent logos in `public/agents/`
retain attribution in `SOURCES.md` and licensing in `LICENSE`. Canonical brand
graphics live in `web/graphics/`; favicons derive from
`web/graphics/icon/1024w/capsem-logo-color.png`.
