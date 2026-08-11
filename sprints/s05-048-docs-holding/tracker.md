# Sprint: S05-048 documentation holding page

## Tasks

- [x] Create isolated branch/worktree from exact `origin/main`
- [x] Inventory docs build, routes, README, workflow smoke, and contracts
- [x] Capture focused RED contracts
- [x] Implement holding-page-only build without deleting detailed sources
- [x] Update README, docs smoke, site-infra skill, and changelog
- [x] Build docs and prove old routes absent from `dist`
- [x] Run scoped tests and lint/diff checks
- [x] Commit the functional milestone

## Notes

- Worktree: `/home/elieb_google_com/capsem-s05-048`
- Branch: `sprinty/s05-048-docs-holding`
- Base: `229f60d5d8ad6999b5e89edab812836737341616`
- The marketing and release sites are explicit no-touch boundaries.
- The detailed manual contains 48 tracked Markdown/MDX files. Starlight's
  integration currently materializes them; removing only that integration can
  retain and schema-check the source without producing public routes.
- RED: six focused failures proved the missing artifact verifier, custom root
  page, holding-page smoke, README boundary, and updated site-infra contract.
- A second RED/GREEN slice proved the build cannot copy the retained public
  installer or image assets into `dist/`.
- A third RED/GREEN slice added a top-level `404.html`, so Cloudflare Pages can
  return a real missing-route response instead of falling back to the holding
  page with a successful status.

## Coverage ledger

- Unit/contract: nine focused docs/site/marketing contracts pass after the
  recorded RED slices, and the repository-wide source syntax acceptance check
  passes.
- Functional: `bash scripts/check-web-surface.sh docs` passes the canonical
  production build entrypoint.
- Adversarial: the build verifier accepts exactly `index.html` and `404.html`;
  it rejects a former route or copied `install.sh`, and the real artifact
  inventory contains exactly those two HTML files.
- E2E: the built root and 404 HTML contain their required release-status copy;
  live deployment is deferred because this task forbids pushing or external
  effects.
- Telemetry: not applicable to a static docs holding page.
- Performance: not applicable; no performance claim.
- Missing/deferred: live `docs.capsem.org` smoke can run only after an authorized
  merge/push triggers `.github/workflows/docs.yaml`.
- Inherited baseline: the complete `test_release_doctor_contract.py` run has
  187 passes and one unrelated Linux parity assertion failure because the
  `origin/main` `linux-rust` plan no longer renders the Docker commands that
  its existing contract expects. The changed site-infra contract passes in
  isolation; this sprint does not change that gate or its contract.
