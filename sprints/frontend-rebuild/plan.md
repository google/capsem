# Sprint: Frontend Rebuild

## What

Ground-up rewrite of the Capsem frontend as a Chrome browser shell. Pure Preline semantic tokens, Svelte 5 runes, Phosphor icons, mock data first, then gateway wiring. Replaces the entire existing DaisyUI/Tauri frontend.

## Why

- DaisyUI frontend is dead code -- Preline is the design system
- Chrome browser shell metaphor: tabs = VMs, toolbar = controls, not a sidebar app
- Gateway HTTP API replaces Tauri IPC -- frontend becomes a standalone browser app
- Mock-first development: build the full UI before wiring to backend

## Architecture

```
Browser (localhost:5173)
  |
  v
Parent frame: shell (tab bar + toolbar + theme)
  |
  +-- <iframe sandbox="allow-scripts" src="/vm/{id}/"> VM tab A (own origin, isolated)
  +-- <iframe sandbox="allow-scripts" src="/vm/{id}/"> VM tab B (own origin, isolated)
  +-- <iframe sandbox="allow-scripts" src="/vm/{id}/"> VM tab C (own origin, isolated)
  |
  |-- Parent <-> iframe: postMessage only
  |-- iframe -> gateway: HTTP fetch + WebSocket (Bearer auth)
  |
  v
capsem-gateway (127.0.0.1:19222, Bearer auth)
  |
  v
capsem-service -> capsem-process -> guest VM
```

### Process isolation via iframes

Each VM tab runs in a sandboxed `<iframe>` with its own origin. This is the Chrome multi-process model applied to a web app:

- **Origin isolation**: each VM iframe gets a unique origin (e.g. `/vm/{id}/`). Cross-origin policy prevents one VM tab from accessing another's DOM, stores, cookies, or storage.
- **Sandbox**: `sandbox="allow-scripts"` -- no `allow-same-origin` (prevents escaping to parent), no `allow-top-navigation` (prevents hijacking the shell).
- **Communication**: parent shell and VM iframes talk via `postMessage` only. Messages are typed and validated on both sides.
- **Shell stays simple**: the parent frame owns only the tab bar, toolbar, and theme. All VM-specific state (terminal, stats, settings, logs) lives inside the iframe.
- **Crash isolation**: if a VM iframe crashes or hangs, other tabs and the shell are unaffected.

## Current State

Worktree: `worktrees/capsem-ui` (branch: `frontend-ui`)
Code: `worktrees/capsem-ui/frontend/`

### What's built

| Component | File | Status |
|-----------|------|--------|
| Layout | `layouts/Layout.astro` | Done -- full-viewport shell, Preline theme imports |
| Entry | `pages/index.astro` | Done -- `client:only="svelte"` |
| Global CSS | `styles/global.css` | Done -- Preline themes (9 themes loaded), variants, hover layer |
| Tab store | `stores/tabs.svelte.ts` | Done -- rune class, add/close/activate/reorder/openVM |
| App shell | `shell/App.svelte` | Done -- TabBar + Toolbar + content routing |
| Tab bar | `shell/TabBar.svelte` | Done -- Chrome-style tabs, drag reorder, close, new tab button, Preline tokens |
| Toolbar | `shell/Toolbar.svelte` | Done -- VM actions, search bar, menu dropdown (theme picker, settings, about), Phosphor icons |
| New tab page | `shell/NewTabPage.svelte` | Done -- sortable VM table, status badges, action buttons, Preline table tokens |
| VM overview | `shell/VMOverview.svelte` | Done -- hero, stat cards, action buttons, Preline card tokens |
| Settings page | `shell/SettingsPage.svelte` | Done -- sidebar nav, appearance (mode + theme), general, security, network, storage, advanced, about |
| Mock data | `mock.ts` | Done -- 5 VMs in varied states (running, stopped, booting, error) |

### Design patterns established

- **Preline CSS-only**: semantic token classes (`bg-primary`, `text-foreground`, `bg-card`, `border-card-line`, `bg-dropdown`, `text-dropdown-item-foreground`, etc.). Zero DaisyUI. Zero raw Tailwind colors.
- **Phosphor icons**: `phosphor-svelte` for all icons (ArrowClockwise, Stop, Trash, GitFork, MagnifyingGlass, Sun, Moon, etc.)
- **Svelte 5 runes only**: `$state`, `$derived`, `$derived.by()`, `$props()`, class-based stores
- **No Tauri dependency**: pure browser app, mock data
- **9 Preline themes**: default, ocean, moon, harvest, retro, autumn, bubblegum, cashmere, olive
- **Dark mode**: `.dark` class on `<html>`, Preline tokens auto-adapt

## What's Next

### Phase 1: Remaining Views (mock data)

Build out the remaining views with static mock data:

- Terminal view (xterm.js, mock local echo + boot sequence)
- Exec view (command input + output, mock responses)
- Files view (file tree + content viewer, mock directory structure)
- Logs view (log entries, filters, auto-scroll)
- Inspector view (SQL editor + results table, mock query results)
- Stats view (model stats, network events, tool calls -- mock data)

### Phase 2: Gateway Wiring

Replace mock data with real gateway HTTP:

- `api.ts`: `fetch()` to `http://127.0.0.1:19222` with Bearer token
- `db.ts`: `POST /inspect/{id}` for SQL queries
- Terminal: WebSocket to `ws://127.0.0.1:19222/terminal/{id}`
- Status polling: `GET /status` (1s cache)
- Mock detection: fallback when gateway unreachable

### Phase 3: Polish

- Keyboard shortcuts (Cmd+T/W/1-9/Shift+[/])
- Tab overflow scrolling
- View switch transitions
- Responsive layout
- Accessibility (focus rings, aria, keyboard nav)

## Key Decisions

1. **Rewrite, not swap.** The old frontend/ is replaced entirely. Nothing from the DaisyUI codebase carries over.
2. **Preline CSS-only.** No JS plugins. Interactivity is pure Svelte 5 runes.
3. **Phosphor icons.** Consistent icon library, not custom SVGs.
4. **Mock-first.** Every view works with static data before any backend wiring.
5. **Gateway is the only backend.** No Tauri, no UDS, no invoke().
6. **Chrome DevTools MCP for verification.** Screenshot every view, both themes.

## Depends On

- capsem-gateway (done)
- capsem-service (done)

## Reference

- Worktree: `worktrees/capsem-ui` (branch: `frontend-ui`)
- Gateway API: `sprints/gateway/tracker.md`
- Design system: `skills/frontend-design/SKILL.md`
- Preline reference: `skills/frontend-design/references/preline.md`
- Testing: `skills/dev-testing-frontend/SKILL.md`
- Original ui-now plan: `sprints/ui-now/tracker.md`
