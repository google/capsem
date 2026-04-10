# Sprint 07: CI + Ship

Frontend is production-ready and gated in CI.

Worktree: `worktrees/capsem-ui` (branch: `frontend-ui`)
Depends on: Sprint 06

## Acceptance Criteria

### CI Pipeline
- [ ] Frontend steps added to `.github/workflows/ci.yaml`
- [ ] `pnpm install --frozen-lockfile`
- [ ] `pnpm run check` (astro check + svelte-check)
- [ ] `npx vitest run --coverage`
- [ ] `pnpm run build` (production build)
- [ ] Coverage uploaded to codecov with threshold

### Lint Checks
- [ ] Zero DaisyUI classes in `frontend/src/` (CI grep check)
- [ ] Zero `@tauri-apps/api` imports in `frontend/src/` (CI grep check)
- [ ] No `$:` reactive statements (Svelte 5 runes only)

### Just Recipes
- [ ] `just ui` — dev mode (astro dev on port 5173)
- [ ] `just ui-build` — production build
- [ ] `just ui-check` — type check + vitest + lint checks

### Production Build
- [ ] Static output in `frontend/dist/`
- [ ] Correct asset paths (relative, no absolute)
- [ ] No dev dependencies in output
- [ ] Build size reasonable (track and report)

### End-to-End Verification
- [ ] Gateway serves built frontend static files
- [ ] Gateway + service + frontend all running: full workflow (create VM, open terminal, run command, view stats, check logs)
- [ ] Mock mode works without gateway (standalone dev)

### Ship Readiness
- [ ] CHANGELOG.md updated with frontend rebuild entry
- [ ] No console errors or warnings in production build
- [ ] All 7 sprint trackers marked complete
- [ ] Parent tracker (`sprints/frontend-rebuild/tracker.md`) updated

## Testing Gate

- [ ] CI passes on a test PR
- [ ] `pnpm run build` produces working static output
- [ ] `pnpm run check` clean (zero errors)
- [ ] `npx vitest run --coverage` meets threshold
- [ ] Full Chrome DevTools MCP visual pass (all views, both themes)
- [ ] E2E: gateway serves frontend, terminal works, VM lifecycle works
