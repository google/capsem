# UI-Now Sprint: Chrome Browser Shell

Build a performant multi-tab browser shell from scratch with Preline. Pure frontend, mock data, no backend wiring. Focus on interaction quality, animations, and correctness.

Stack: Astro 5 + Svelte 5 + Tailwind v4 + Preline
Worktree: worktrees/capsem-ui (branch: frontend-ui)
Directory: frontend/

## Architecture

This is a standalone browser app. No Tauri dependency in this sprint. The existing frontend/ contents are replaced entirely -- none of the old code (DaisyUI, Tauri IPC, poll-based terminal) is reused.

The app will later connect to capsem-gateway (TCP + auth) which proxies to capsem-service (UDS) which talks to VMs via vsock. But this sprint is **mock data only** -- no fetch, no backend, no network calls.

```
Browser (localhost:5173)
  |
  v
Astro 5 + Svelte 5 + Preline (static SPA)
  |
  v
mock.ts (static data, no network)
```

### Design System

- **Colors**: Blue for positive/running states, Purple for negative/error states. NO green or red.
- **Semantic tokens**: surface, foreground, muted, accent, destructive (not raw Tailwind colors)
- **Theme**: Light/dark via Preline CSS custom properties
- **Reference**: skills/frontend-design/SKILL.md

## Sub-sprints

### SS1: Scaffold + Preline Setup

Status: In progress

- [x] Establish worktree: `worktrees/capsem-ui` on branch `frontend-ui`
- [x] Add `worktrees/` to `.gitignore`
- [ ] Fresh Astro 5 app (replace existing frontend/ contents)
- [ ] Add Svelte 5, Tailwind v4, Preline dependencies
- [ ] Configure global.css: Tailwind + Preline plugin + design tokens
- [ ] Design token system: semantic colors (surface, foreground, muted, accent, destructive)
- [ ] Light/dark theme via Preline CSS custom properties
- [ ] Verify: `pnpm dev` renders a styled hello world page

### SS2: Tab System

Status: Not started

The core interaction model. Chrome-style tabs with full keyboard support.

- [ ] TabBar.svelte: horizontal tab strip, overflow scroll, add button
- [ ] Tab.svelte: individual tab (icon, title, close button, active state)
- [ ] tabs.svelte.ts store: tab list, active tab, add/remove/reorder/switch
- [ ] Drag-to-reorder tabs (HTML drag API or pointer events)
- [ ] Keyboard shortcuts: Cmd+T (new), Cmd+W (close), Cmd+1-9 (switch), Cmd+Shift+[ / ] (prev/next)
- [ ] Tab overflow: scroll buttons or horizontal scroll when >8 tabs
- [ ] Tab animations: slide-in on create (150ms), fade-out on close (100ms)
- [ ] Verify: open 10 tabs, switch between them, drag reorder, close any, keyboard nav works

### SS3: App Shell Layout

Status: Not started

Full-window layout that contains everything.

- [ ] BrowserShell.svelte: top-level layout (tab bar top, sidebar left, content right)
- [ ] Sidebar.svelte: view navigation for active tab (icons + labels)
- [ ] Sidebar collapse/expand: smooth width animation, icon-only collapsed state
- [ ] Content area: renders the active view for the active tab
- [ ] Responsive: sidebar auto-collapses below 768px
- [ ] Verify: full layout renders, sidebar toggles, content area fills remaining space

### SS4: Component Library (Preline)

Status: Not started

Reusable components used across all views.

- [ ] Button.svelte: primary, secondary, ghost, danger variants; sizes sm/md/lg; loading state
- [ ] Card.svelte: header, body, footer slots; stat card variant
- [ ] Badge.svelte: status badges (running=blue, stopped=muted, error=purple); pulsing variant
- [ ] Modal.svelte: overlay + panel, close on Esc/click-outside, enter/exit transitions
- [ ] Input.svelte: text, number, textarea; label, error state, helper text
- [ ] Select.svelte: single select with Preline dropdown
- [ ] Toggle.svelte: on/off switch with label
- [ ] DataTable.svelte: column sorting, row click, empty state
- [ ] Toast.svelte: success/error/info; auto-dismiss; stack from bottom-right
- [ ] Skeleton.svelte: loading placeholder (text lines, card, table rows)
- [ ] Accordion.svelte: collapsible sections with smooth height animation
- [ ] Verify: component showcase page renders all components in both themes

### SS5: Mock Data Layer

Status: Not started

Static data matching capsem-service API shapes. Reference: `crates/capsem-service/src/api.rs`

- [ ] types.ts: TypeScript types matching service API (SandboxInfo, ExecResponse, etc.)
- [ ] mock.ts: static data instances
- [ ] Mock VMs: 5 VMs in varied states (2 running, 1 stopped, 1 error, 1 booting)
- [ ] Mock VM details: ram, cpus, persistent flag, version, uptime
- [ ] Mock exec results: 3 canned command outputs (ls, uname, cat)
- [ ] Mock file tree: nested directory structure with file contents
- [ ] Mock logs: 50+ log entries across serial and process logs
- [ ] Mock SQL results: session DB query with columns and rows
- [ ] Mock terminal: local echo mode + simulated boot sequence text

### SS6: View -- New Tab Page

Status: Not started

Default view when opening a new tab. The "home page."

- [ ] NewTabPage.svelte: default view when opening a new tab
- [ ] VM list: cards or table showing all VMs from mock data
- [ ] Status badges, quick actions (stop, shell, delete) per VM
- [ ] Create VM button -> modal with form (name, ram, cpus, persistent, image select)
- [ ] Empty state: illustration + "No VMs running" + create button
- [ ] Click VM -> opens/focuses tab for that VM

### SS7: View -- Overview

Status: Not started

Dashboard for a single VM within a tab.

- [ ] OverviewView.svelte: dashboard for a single VM
- [ ] Status hero: large badge, VM name, uptime, ID
- [ ] Stat cards: RAM, CPUs, persistent/ephemeral, version
- [ ] Action buttons: Stop, Restart, Delete, Persist, Fork
- [ ] Confirm dialog for destructive actions (delete)
- [ ] Recent activity: last 5 mock log entries

### SS8: View -- Terminal

Status: Not started

xterm.js terminal, mock-only in this sprint.

- [ ] TerminalView.svelte: xterm.js in a resizable container
- [ ] Local echo mock: keystrokes appear, Enter triggers mock response
- [ ] Mock boot sequence: animated text output on tab open
- [ ] Fit addon: terminal resizes with container
- [ ] WebGL addon: hardware-accelerated rendering
- [ ] Theme sync: terminal colors match light/dark theme

### SS9: View -- Exec

Status: Not started

Run commands and see output.

- [ ] ExecView.svelte: command input + output display
- [ ] Command input: text field + Run button (Cmd+Enter shortcut)
- [ ] Output: stdout in default color, stderr in error color
- [ ] Exit code badge: 0 = success, non-zero = error
- [ ] Command history: last 10 commands, click to re-run
- [ ] Loading state: skeleton while "executing"

### SS10: View -- Files

Status: Not started

File browser and editor.

- [ ] FileView.svelte: split pane (tree left, editor right)
- [ ] File tree: collapsible directories, file icons by extension
- [ ] File content display: monospace, line numbers
- [ ] Edit mode: textarea for writing files (mock only)
- [ ] Breadcrumb path display

### SS11: View -- Logs

Status: Not started

Serial and process log viewer.

- [ ] LogView.svelte: log entry list with filters
- [ ] Filter bar: source (serial/process), level (info/warn/error), text search
- [ ] Log entry: timestamp, source badge, message, expandable details
- [ ] Auto-scroll to bottom with "jump to latest" button
- [ ] Time-relative display ("2m ago") with absolute tooltip

### SS12: View -- Inspector

Status: Not started

SQL query editor against session telemetry DB.

- [ ] InspectorView.svelte: SQL editor + results table
- [ ] SQL input: monospace textarea with basic syntax highlighting
- [ ] Run query button (Cmd+Enter)
- [ ] Results: DataTable with column headers and rows from mock
- [ ] Error display for invalid queries
- [ ] Saved queries: 3 preset queries (events, net_events, fs_events)

### SS13: View -- Settings

Status: Not started

VM configuration.

- [ ] SettingsView.svelte: VM configuration form
- [ ] Accordion sections: General, Resources, Network, Environment, MCP
- [ ] General: name, persistent toggle
- [ ] Resources: RAM slider, CPU selector
- [ ] Environment: key-value editor (add/remove rows)
- [ ] All changes are mock-only (no save endpoint)

### SS14: Animations + Polish

Status: Not started

Performance and visual quality pass.

- [ ] Tab switch: crossfade content area (100ms)
- [ ] View switch: slide transition within tab (150ms)
- [ ] Modal: fade overlay + scale panel (150ms)
- [ ] Sidebar: width transition (200ms ease)
- [ ] Status change: badge color pulse animation
- [ ] Toast: slide-in from right (200ms)
- [ ] Skeleton shimmer animation
- [ ] 60fps verified: no jank on tab switch with 10 tabs open
- [ ] Accessibility: focus rings, keyboard navigation, aria labels

## Directory Structure

```
frontend/
  src/
    lib/
      mock.ts                  # Static mock data for all views
      types.ts                 # UI types + service API type mirrors
      components/
        shell/
          BrowserShell.svelte  # Top-level layout
          TabBar.svelte        # Chrome-style tab strip
          Tab.svelte           # Individual tab (draggable)
          Sidebar.svelte       # View navigation
          NewTabPage.svelte    # VM list + create prompt
        views/
          OverviewView.svelte
          TerminalView.svelte
          ExecView.svelte
          FileView.svelte
          LogView.svelte
          InspectorView.svelte
          SettingsView.svelte
        ui/                    # Preline component library
          Button.svelte
          Card.svelte
          Badge.svelte
          Modal.svelte
          DataTable.svelte
          Input.svelte
          Select.svelte
          Toggle.svelte
          Toast.svelte
          Skeleton.svelte
          Accordion.svelte
      stores/
        tabs.svelte.ts         # Tab state (list, active, order)
        theme.svelte.ts        # Light/dark mode
    pages/
      index.astro              # SPA entry
    styles/
      global.css               # Tailwind + Preline + design tokens
  astro.config.mjs
  package.json
```

## Acceptance Criteria (Sprint Gate)

- [ ] 10 tabs open simultaneously, smooth switching, no memory leak
- [ ] All 8 views render with mock data in both light and dark themes
- [ ] Tab drag reorder works
- [ ] All keyboard shortcuts work (Cmd+T, Cmd+W, Cmd+1-9, Cmd+Shift+[/])
- [ ] Sidebar collapse/expand animates smoothly
- [ ] Component library is consistent across all views
- [ ] No backend calls anywhere (verified: no fetch in codebase)
- [ ] `pnpm build` produces a static site with no errors

## Depends On

Nothing. Pure frontend work.

## Blocks

- UI wiring sprint (replace mock.ts with real fetch to gateway)
- Tray sprint (reuses component patterns established here)

## Reference

- Service API types: `crates/capsem-service/src/api.rs`
- Design system: `skills/frontend-design/SKILL.md`
- Capsem overview: `skills/dev-capsem/SKILL.md`
