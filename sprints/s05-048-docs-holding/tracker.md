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
- [x] Diagnose stale Cloudflare cached-route deployment failure
- [x] Capture RED tombstone/header/inventory/smoke contracts
- [x] Materialize source-derived tombstones and no-store Pages headers
- [x] Build and prove the exact holding/tombstone artifact graph
- [x] Run focused follow-up verification and commit without pushing

## Notes

- Worktree: `/home/elieb_google_com/capsem-s05-048`
- Branch: `sprinty/s05-048-docs-holding`
- Base: `229f60d5d8ad6999b5e89edab812836737341616`
- Forward-fix parent: `f8f8fdd530cf3caa4d9001d36d1b4355c8438b15`
- The marketing and release sites are explicit no-touch boundaries.
- The detailed manual contains 48 tracked Markdown/MDX files. Starlight's
  integration currently materializes them; removing only that integration can
  retain the source without producing the detailed Starlight pages.
- RED: six focused failures proved the missing artifact verifier, custom root
  page, holding-page smoke, README boundary, and updated site-infra contract.
- A second RED/GREEN slice proved the build cannot copy the retained public
  installer or image assets into `dist/`.
- A third RED/GREEN slice added a top-level `404.html`, so Cloudflare Pages can
  return a real missing-route response instead of falling back to the holding
  page with a successful status.
- Cache forward-fix RED: eight failures proved the verifier did not derive an
  inventory from the manual, the tombstone route and `_headers` did not exist,
  and the workflow still treated HTTP status as proof.
- The first real forward-fix build caught Astro's hoisted `getStaticPaths`
  boundary; the final route owns its raw source glob inside the function and
  does not compile or publish the retained Markdown/MDX bodies.

## Coverage ledger

- Unit/contract: fourteen focused docs/site/marketing/source-syntax contracts
  pass after the recorded RED slices.
- Functional: `bash scripts/check-web-surface.sh docs` passes the canonical
  production build entrypoint.
- Adversarial: the build verifier independently derives all 47 tombstones from
  the 48 retained sources, permits exactly those plus `index.html`, `404.html`,
  and `_headers`, and rejects missing/extra files or old Starlight/install
  content. The real artifact contains exactly those 50 files.
- E2E: the built root, 404, and `/getting-started/` tombstone contain the
  required qualification/noindex copy, while `_headers` declares no-store for
  browser and CDN caches. Live forward-fix deployment is deferred because this
  task forbids pushing or external effects.
- Telemetry: not applicable to a static docs holding page.
- Performance: not applicable; no performance claim.
- Missing/deferred: live `docs.capsem.org` smoke can run only after an authorized
  merge/push triggers `.github/workflows/docs.yaml`.
- Inherited baseline: the broad release-doctor contract run has 181 passes
  when its one known Linux parity assertion is deselected. That targeted test
  still fails because the `origin/main` `linux-rust` plan no longer renders
  the Docker commands its existing contract expects. The changed site-infra
  contract passes; this sprint does not change that gate or its contract.

## Cached-route incident

- The root holding page deployed successfully, but the warmed
  `/getting-started/` edge object remained the old Starlight guide (HTTP 200,
  cache age 53210, `s-maxage=604800`). Removing a file from a Pages deployment
  did not invalidate that already-cached URL.
- The forward fix replaces every former source-derived route with a real
  noindex holding tombstone and publishes no-store response headers. The live
  smoke will validate the tombstone body rather than treating a status code as
  proof.
