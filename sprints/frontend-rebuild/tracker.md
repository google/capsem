# Sprint: Frontend Rebuild

Ground-up rewrite as Chrome browser shell with Preline + Svelte 5 runes + Phosphor icons.

Worktree: `worktrees/capsem-ui` (branch: `frontend-ui`)
Plan: `sprints/frontend-rebuild/plan.md`

## Before You Write Any Code

1. **Load `/frontend-design`** -- Preline CSS-only rules, color scheme (blue=positive, purple=negative, NO green/red), semantic token classes, rune patterns. Do NOT use DaisyUI, raw Tailwind colors, or Preline JS plugins.
2. **Load `/dev-testing-frontend`** -- Chrome DevTools MCP visual verification is mandatory for every UI change. Screenshot every view, both themes, check console for errors.
3. **Read `skills/frontend-design/references/preline.md`** -- Preline token reference (buttons, cards, forms, nav, overlays, text hierarchy). Use these class patterns, not your own.
4. **Use Phosphor icons** (`phosphor-svelte`) for all icons -- not custom SVGs, not Heroicons, not Lucide.
5. **Svelte 5 runes only** -- `$state`, `$derived`, `$derived.by()`, `$effect`, `$props()`. No legacy `$:` reactive statements. Class-based singleton stores with `.svelte.ts` extension.
6. **No Tauri** -- this is a standalone browser app. Data comes from capsem-gateway HTTP API (Phase 2) or mock.ts (Phase 1).
7. **Code simplicity and correctness above all else.** Every line must earn its place. No wrapper components for basic UI elements. No abstractions until you need them three times.
8. **Write vitest tests for every store and non-trivial component.** Stores get unit tests (state transitions, edge cases). Components with logic get `@testing-library/svelte` tests. Run `npx vitest run --coverage` before considering any phase done.
9. **Security is load-bearing.** The frontend talks to a multi-VM backend -- every input is an attack surface:
   - **Cross-VM isolation**: each VM tab runs in its own `<iframe>` with a unique origin (e.g. `http://127.0.0.1:19222/vm/{id}/`) and strict sandboxing (`sandbox="allow-scripts"`). VM A's iframe cannot access VM B's DOM, stores, or data. The shell (tab bar, toolbar) lives in the parent frame and communicates with VM iframes via `postMessage` only. This mirrors Chrome's one-process-per-tab model -- a compromised VM tab cannot reach other tabs.
   - **No VM escape via UI**: user-controlled strings (VM names, file contents, command output, log entries) must be treated as untrusted. No `{@html}` on user data. No `eval()`. No `innerHTML`.
   - **Gateway auth**: Bearer token must never appear in logs, error messages, DOM, or browser history. Store in memory only, never in localStorage/sessionStorage/URL params.
   - **SQL injection via inspector**: the inspector view sends raw SQL to `/inspect/{id}`. The backend enforces read-only (SELECT only), but the frontend must also refuse non-SELECT queries and sanitize display of results.
   - **WebSocket terminal**: terminal input goes to a real shell. Never inject control sequences or framing outside the xterm.js data path. Validate VM ID format before connecting (alphanumeric + hyphens only).
   - **Path traversal**: file view paths come from the guest. Validate and display only -- never construct host-side paths from guest data.

## Done

- [x] Astro 5 scaffold with Layout.astro + index.astro
- [x] Preline setup: Tailwind v4 + Preline themes (9 themes) + variants + dark mode
- [x] Tab store (`tabs.svelte.ts`): rune class with add/close/activate/reorder/openVM
- [x] App shell (`App.svelte`): TabBar + Toolbar + content routing by active tab view
- [x] Tab bar (`TabBar.svelte`): Chrome-style tabs, drag reorder, close, new tab (+), Preline tokens
- [x] Toolbar (`Toolbar.svelte`): VM actions (restart/stop/destroy/fork), search bar, menu dropdown (dark mode toggle, theme picker, settings, about), Phosphor icons
- [x] New tab page (`NewTabPage.svelte`): sortable VM table, status badges, per-VM actions
- [x] VM overview (`VMOverview.svelte`): hero + stat cards + action buttons
- [x] Settings page (`SettingsPage.svelte`): sidebar nav, appearance (mode + theme), general, security, network, storage, advanced, about
- [x] Mock data (`mock.ts`): 5 VMs in varied states

## Sprints

Broken into 7 focused sprints. Sprints 2-4 can run in parallel after Sprint 1. Sprint 5 needs all views done. Sprints 6-7 are sequential.

| Sprint | Focus | Depends on | Status |
|--------|-------|------------|--------|
| [01](sprint-01/tracker.md) | Terminal + iframe isolation + themes | -- | Done |
| [02](sprint-02/tracker.md) | Stats, Logs, Service Logs | 01 | Done |
| [03](sprint-03/tracker.md) | Complex views: Files, Inspector | 01 | Done |
| [04](sprint-04/tracker.md) | Settings system | 01 | Not started |
| [05](sprint-05/tracker.md) | Gateway wiring | 01-04 | Not started |
| [06](sprint-06/tracker.md) | Polish / shortcuts / a11y | 05 | Not started |
| [07](sprint-07/tracker.md) | CI + ship | 06 | Not started |

## Testing Gate (all sprints)

- [ ] `pnpm run check` (astro check + svelte-check)
- [ ] `npx vitest run --coverage`
- [ ] `pnpm run build` (production build)
- [ ] Chrome DevTools MCP: screenshot every view, both themes, no console errors
- [ ] Zero DaisyUI classes in codebase
- [ ] Zero `@tauri-apps/api` imports
- [ ] Mock mode works standalone
- [ ] Live mode works with gateway + service

## Notes

- Phosphor icons via `phosphor-svelte` (not custom SVGs)
- Preline themes via `data-theme` attribute on `<html>`, dark mode via `.dark` class
- Tab views: new-tab, overview, terminal, exec, files, logs, inspector, settings
