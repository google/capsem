# Marketing FAQ Page

## Goal

Add a dedicated FAQ page to the marketing site and make the hypervisor-vs-container answer the first FAQ entry.

## Decisions

- Keep FAQ copy in `site/src/lib/data.ts` so the homepage FAQ section and the standalone page share one source of truth.
- Create a static Astro route at `/faq` rather than moving the existing homepage accordion.
- Point top navigation and footer FAQ links to `/faq` while preserving homepage section anchors for feature and workflow links.

## Files

- `site/src/lib/data.ts`
- `site/src/pages/faq.astro`
- `site/src/components/FAQ.svelte`
- `CHANGELOG.md`

## Done

- `/faq` renders all FAQ entries from shared data.
- The first FAQ answers why Capsem uses a hypervisor instead of containers.
- Site navigation links to the dedicated FAQ page.
- The marketing site build passes.

## Testing Matrix

- Unit/contract: Astro type/build check through `pnpm run build`.
- Functional: generated `/faq/index.html` includes the new first FAQ.
- Adversarial: navigation avoids page-local `#faq` links from the standalone page.
- E2E/VM: not applicable; marketing-only static route.
- Telemetry: not applicable.
- Performance: not applicable.
