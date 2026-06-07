# Sprint 01: Terminal + Iframe Isolation + Themes

Xterm.js running inside a sandboxed iframe with full theme sync via postMessage.

Worktree: `worktrees/capsem-ui` (branch: `frontend-ui`)

## Architecture Decisions

- **Static route**: `/vm/terminal/` instead of `/vm/[id]/`. VM ID sent via postMessage, not URL. Astro static output doesn't support dynamic routes without `getStaticPaths`. Security isolation comes from `sandbox` attribute, not path-based origin separation.
- **Dev sandbox relaxation**: `sandbox="allow-scripts allow-same-origin"` in dev, `sandbox="allow-scripts"` in prod. Vite dev server can't serve module scripts to `null` origin iframes (CORS). Production static build serves same-origin so this isn't an issue.
- **CSP is production-only**: `script-src 'self'` blocks Vite's inline HMR scripts. CSP meta tag conditionally applied via `import.meta.env.PROD`.
- **No `$effect()` in module singletons**: Svelte 5 throws `effect_orphan`. Theme store uses explicit setter sync instead.
- **VMOverview removed**: Clicking a VM opens a terminal directly. There is no overview intermediary.

## Done

- [x] `VMFrame.svelte` -- iframe host with `sandbox="allow-scripts"` (prod) / relaxed (dev)
- [x] `pages/vm/terminal.astro` -- iframe entry point with production CSP
- [x] Unique iframe per VM tab (one iframe per VM, not shared)
- [x] Iframe visibility tied to active tab
- [x] `postmessage.ts` -- typed message protocol with validators
- [x] Messages: `theme-change`, `vm-id`, `focus`, `ws-ticket`, `clipboard-paste/copy/request`, `terminal-resize`, `title-update`, `ready`, `error`
- [x] Origin validation via `event.source` check (not origin string -- opaque origins are all `"null"`)
- [x] Message type/payload validation (unknown types silently dropped)
- [x] Xterm.js v6 inside iframe with FitAddon
- [x] WebGL addon with canvas fallback on context loss
- [x] ResizeObserver-driven refit (debounced via rAF)
- [x] Mock local echo + boot sequence banner
- [x] 14px font, 10000 line scrollback
- [x] OSC 8 hyperlinks disabled (linkHandler.activate is no-op)
- [x] Title change sanitization (control chars stripped, 128 char max)
- [x] Theme store (`theme.svelte.ts`): UI mode (auto/light/dark) + terminal theme family + accent + font settings, all localStorage-persisted
- [x] 12 terminal theme families (default, one, dracula, catppuccin, monokai, gruvbox, solarized, nord, rose-pine, tokyo-night, kanagawa, everforest) -- each with dark + light variant, canonical colors from iTerm2-Color-Schemes
- [x] Parent broadcasts theme + font settings to iframe via postMessage on `ready` and on change
- [x] TerminalFrame listens for postMessage and applies theme + font + focus + clipboard-paste
- [x] Terminal theme families auto-switch dark/light variant based on UI mode
- [x] 9 UI accent colors (blue, cyan, slate, amber, fuchsia, orange, pink, purple, lime) -- primary-only overrides, consistent dark/light base
- [x] Bundled fonts: Google Sans Flex (UI), Google Sans Code + 10 mono fonts (terminal) -- zero external deps
- [x] Settings page: Interface section (mode/accent/UI size) + Terminal section (live preview/color scheme/font/font size)
- [x] Flash-prevention inline script in Layout.astro (dark class + data-theme + UI font size)
- [x] Simplified Preline CSS: removed 8 theme imports (~2000 lines), replaced with ~90 lines of primary-only overrides
- [x] Two VM tabs open simultaneously without interference
- [x] Closing one VM tab doesn't affect others
- [x] Tab switching preserves terminal state (iframe not destroyed)
- [x] Click VM row -> opens terminal tab directly (no overview intermediary)
- [x] `pnpm run check` passes (0 errors)
- [x] Vite CORS configured for dev sandboxed iframes
- [x] Rate limiter class ready (`rate-limiter.ts`)
- [x] Terminal hardening config (`terminal-config.ts`)

## Remaining

- [x] Vitest tests for postMessage validators
- [x] Vitest tests for theme store (localStorage round-trip)
- [x] Vitest tests for rate limiter
- [x] Terminal theme picker in Settings page
- [x] Chrome DevTools MCP screenshot: all accents with terminal
- [x] WCAG 4.5:1 contrast for all terminal theme ANSI colors (unit tested)
- [x] Fix VMFrame import paths (.svelte -> .svelte.ts)
- [x] Remove legacy mode/theme controls from toolbar dropdown
- [x] Dark/light UI surface overrides (#282828/#3c3c3c dark, #f4f3f2/#ffffff light)

## Testing Gate

- [x] `pnpm run check` passes
- [x] Terminal renders in iframe, WebGL confirmed
- [x] Theme switches propagate from parent to terminal iframe
- [x] Two VM tabs open simultaneously -- verified isolation
- [x] Vitest tests for postMessage protocol and theme store
- [x] Chrome DevTools MCP: terminal in dark + light, dracula theme verified
- [x] 137 vitest tests pass (postmessage, rate-limiter, theme-store, theme-contrast)
