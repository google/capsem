# S05-048: Documentation holding page

## Objective

Publish `docs.capsem.org` as a single, clear Capsem 0.6 pre-release holding
page while retaining every detailed documentation source file in Git. Remove
repository-root links that invite readers into unavailable installation,
latest-release, or deep-documentation paths.

## Boundaries

- Start from exact `origin/main` in a dedicated worktree and branch.
- Keep every existing file under `docs/src/content/docs/` tracked and intact.
- Change only documentation deployment/build surfaces, root README references,
  documentation smoke/contracts, this sprint record, and the changelog.
- Do not change `site/`, `release-site/`, release manifests, installer code, or
  the shared worktree.
- Do not push, merge, or deploy externally.

## Decisions

- Treat this as deployment presentation, not deletion or rewriting of the
  detailed manual. Source stays available for future 0.6 publication.
- The production build must emit only `/` plus a top-level `404.html` platform
  sentinel; old documentation routes must not survive as HTML, redirects, or
  sitemap entries.
- The holding page must not offer installation or release-download actions.
- Tests own both positive content and adversarial route absence before the
  implementation changes.

## Expected source scope

- `docs/astro.config.mjs` and the minimal docs page/layout/style surface needed
  to build the holding page.
- `.github/workflows/docs.yaml` smoke route.
- Root `README.md` links.
- Documentation source contracts and `skills/site-infra/SKILL.md`.
- `CHANGELOG.md` and this sprint's plan/tracker.

## Order

1. Inventory current docs routes, build configuration, README links, smoke, and
   source contracts.
2. Add failing contracts for the holding-page build and absence of old routes.
3. Implement the smallest deployment-only holding surface.
4. Build docs and inspect `dist/` for both expected content and forbidden routes.
5. Run scoped tests/lint/diff checks, update tracker/changelog, and commit.

## Done

- Branch remains based on exact `origin/main` and only this worktree changes.
- Every detailed docs source file remains byte-for-byte present in Git.
- The built artifact contains a clear Capsem 0.6 pre-release page at `/`, a
  generic top-level 404, and no former deep documentation route.
- README no longer links installation, latest-release, or deep docs.
- Deploy smoke checks only the holding page.
- Focused contracts, docs build, scoped lint, and diff checks pass.

## Proof matrix

- Unit/contract: focused Python source/deploy contracts, captured RED then GREEN.
- Functional: `pnpm run build` in `docs/` renders the real production artifact.
- Adversarial: enumerate `docs/dist` and reject old route HTML, redirects, and
  sitemap references.
- E2E: inspect the built root HTML; external deployment is intentionally out of
  scope because no push/deploy is authorized.
- Telemetry: not applicable; static documentation emits no product telemetry.
- Performance: not applicable; no performance claim changes.
