# Sprint: Marketing FAQ Page

## Tasks

- [x] Plan route and shared-content approach.
- [x] Add hypervisor-vs-container FAQ as the first entry.
- [x] Create standalone `/faq` page.
- [x] Update FAQ navigation links.
- [x] Update changelog.
- [x] Run marketing build and inspect generated FAQ output.

## Notes

- Existing worktree has unrelated changes in `.gitignore`, `frontend/src/lib/mock-settings.generated.ts`, and zip files. Leaving those untouched.
- Local Astro dev server is running at `http://127.0.0.1:4321/`; `/faq` returned `200 OK` via `curl -I`.

## Coverage Ledger

- Unit/contract: `pnpm run build` in `site/` passed and generated `/faq/index.html`.
- Functional: `rg` against `site/dist/faq/index.html` confirmed the hypervisor-vs-container entry renders as `faq-1`.
- Adversarial: `rg` against generated pages confirmed nav/download/footer links use root-qualified routes (`/faq`, `/#features`, `/#download`) instead of page-local anchors.
- E2E/VM: not applicable; marketing-only static route.
- Telemetry: not applicable.
- Performance: not applicable.
- Missing/deferred: none expected for this static content change.
