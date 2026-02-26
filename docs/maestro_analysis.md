# Maestro Analysis -- Features & Ideas for Capsem

Analysis of [Maestro](https://github.com/RunMaestro/Maestro), an Electron-based desktop app for orchestrating AI coding agents. Focus on UI patterns, features, and ideas that could improve capsem.

## What Maestro Is

Cross-platform Electron app (React + Tailwind) that wraps multiple AI coding agents (Claude Code, Codex, OpenCode, Factory Droid) in a unified multi-session interface. It is a terminal orchestrator -- it spawns agent processes and relays their PTY output, similar to what capsem does but without the VM sandboxing.

Key difference from capsem: Maestro is a *pass-through* to existing agents (no sandboxing, no network isolation). Capsem's value-add is the security sandbox. But Maestro's UI/UX polish and feature set have ideas worth borrowing.

---

## 1. Session Management (HIGH relevance)

**What Maestro does:**
- Multiple concurrent agent sessions in tabs (TabBar + session list sidebar)
- Session list sidebar with star/rename/delete
- Session discovery -- auto-imports existing sessions from supported agents
- Draft auto-save per session (never lose unsent input)
- Session merging (combine sessions)
- Rename sessions with modal
- Session activity graph (sparkline per session)
- Dual-mode tabs: each session has both an "AI Terminal" and a "Command Terminal", toggled with Cmd+J

**Capsem takeaway:**
- We already plan multi-session (v2). Maestro's tab bar + session list is a good reference model.
- Star/pin favorite sessions is low-effort, high-value for the Sessions view
- Auto-save of unsent input ("drafts") prevents data loss during accidental navigation
- Session activity graph (mini sparkline showing activity over time) would be great in SessionsView
- Dual-mode tabs (AI vs shell) is interesting but less relevant since capsem's terminal IS the shell

---

## 2. Auto Run / Playbooks (MEDIUM relevance)

**What Maestro does:**
- File-system-based task runner: reads markdown checklists, sends each task to an agent session
- "Playbooks" = reusable markdown task lists with batch processing
- Progress tracking per item, loop mode (re-run playbooks)
- Each task gets its own clean session context
- Marketplace for sharing playbooks

**Capsem takeaway:**
- The concept of "playbooks" for sandbox setup (install packages, configure env, run tests) could be useful as capsem matures -- think "sandbox recipes"
- The AutoRun UI (checklist with per-item status) is a good pattern for any multi-step workflow
- A marketplace/sharing model for sandbox configs could be a future differentiator

---

## 3. Git Integration (HIGH relevance)

**What Maestro does:**
- GitStatusWidget: shows current branch + dirty state in the header bar
- GitDiffViewer: inline diff view with syntax highlighting
- GitLogViewer: commit log browser
- Git worktree support: spawn sub-agents on isolated branches, create PRs with one click
- Automatic repo detection per session
- File completion with `@` mentions (reference files in prompts)

**Capsem takeaway:**
- Git status in the status bar is table stakes for a dev tool -- we should show branch + dirty state
- Diff viewer is nice-to-have but lower priority than network telemetry for capsem
- The `@` file mention pattern in input is very useful for agent-style interactions (future)
- Worktree isolation is less relevant since capsem already isolates at the VM level

---

## 4. Onboarding Wizard System (HIGH relevance)

Maestro has two wizard variants -- a full-screen onboarding wizard and an inline `/wizard` command.

### Full-Screen Onboarding (5 steps)

1. **Agent Selection** -- choose AI provider, name the project, optional SSH config
2. **Directory Selection** -- pick project folder, detect existing files
3. **Project Discovery** -- AI-driven conversation with a confidence meter (0-100%). AI responses are parsed as `{confidence, ready, message}`. At 80%+ confidence, the wizard advances.
4. **Preparing Plan** -- document generation phase with loading state
5. **Phase Review** -- edit/preview generated docs (Cmd+E toggles view/edit)

Key UX details:
- **Resume functionality**: wizard state persisted after step 1, restored on app launch
- **Confidence meter**: color-coded (red 0-39, orange 40-69, yellow 70-79, green 80+) with glow effect at ready threshold
- **Filler phrases**: rotating placeholder messages while AI is thinking ("Analyzing your codebase...", "Researching best practices...") with typewriter effect
- **Screen reader announcements** for step changes, focus trapping, Escape key handling

### Inline Wizard (`/wizard` slash command)

Runs inside an existing tab rather than full-screen. Same confidence gauge but compact. Different tabs can run wizards simultaneously. Includes fun "Austin Facts" rotating display during generation, and confetti on completion.

### Tour System (12 steps)

Post-onboarding guided walkthrough:
1. Auto Run panel
2. Document selector
3. File Explorer
4. History
5. Main menu
6. Remote Control
7. Agents & Groups
8. AI Terminal & Tabs
9. Agent Sessions
10. Input Area
11. Terminal Mode
12. Keyboard Shortcuts

Features spotlight cutouts, tooltip positioning (top/bottom/left/right/center), transition animations, and shortcut placeholder substitution.

**Capsem takeaway:**
- A first-run wizard would significantly improve capsem onboarding: select AI agent to install, configure network policy, set resource limits
- The confidence meter pattern is interesting for any conversational setup flow
- **Tour system is very high value** -- capsem has multiple views (Console, Sessions, Network, Settings) that benefit from a guided walkthrough
- Spotlight + tooltip tour is a well-established UX pattern we should adopt
- Resume-on-relaunch for incomplete wizards prevents frustration

---

## 5. Themes & Design System (MEDIUM relevance)

### Theme Library

16 curated themes across 3 categories:
- **Dark (6)**: Dracula, Monokai, Nord, Tokyo Night, Catppuccin Mocha, Gruvbox Dark
- **Light (6)**: GitHub, Solarized Light, One Light, Gruvbox Light, Catppuccin Latte, Ayu Light
- **Vibe (4)**: Pedurple, Maestro's Choice, Dre Synth, InQuest
- Plus 1 fully custom user-configurable theme

### 13 Color Tokens Per Theme

| Token | Purpose |
|-------|---------|
| bgMain | Main content background |
| bgSidebar | Sidebar background |
| bgActivity | Interactive element background |
| border | Border/divider lines |
| textMain | Primary text |
| textDim | Secondary/dimmed text |
| accent | Primary accent/highlight |
| accentDim | Dimmed accent (with alpha) |
| accentText | Text on accent backgrounds |
| accentForeground | Foreground on accent |
| success | Positive states |
| warning | Caution states |
| error | Negative states |

### Accessibility

- **Colorblind-safe palette** (Wong's palette + IBM Design): 10 distinct colors verified for protanopia, deuteranopia, tritanopia
- **Pattern fills** for additional distinction (solid, diagonal, dots, crosshatch)
- **prefers-reduced-motion** disables all animations
- Custom scrollbars (thin 6px, transparent track)

### Animation Library

Rich set of purposeful animations: fade-in, slide-up/down, scale-in, highlight pulse, skeleton shimmer, dashboard card enter, wand sparkle, conducting motion. All respect reduced-motion preference.

### Responsive Design

Container queries for progressive element hiding at breakpoints (700px down to 300px): session name -> cost widget -> UUID -> git status -> git branch -> context display.

**Capsem takeaway:**
- Our DaisyUI theme system is already solid with light/dark. The 13-token model is a good reference for ensuring complete coverage.
- **Colorblind palette support** is worth adding -- capsem's network view uses blue/purple which may not be distinguishable for some users
- **Container queries** for responsive header is a great pattern -- our status bar could hide elements gracefully
- The custom theme builder (CustomThemeBuilder.tsx) is a nice power-user feature for later
- Reduced-motion support is important and we should ensure we have it

---

## 6. Keyboard Shortcuts & Mastery (HIGH relevance)

### 27 Configurable Shortcuts

Organized into categories:
- **Navigation**: toggle sidebar, toggle right panel, cycle prev/next, nav back/forward
- **Instance management**: new instance, new group, kill, move to group
- **UI modes**: toggle AI/shell mode, quick actions (Cmd+K), settings, help
- **Focus**: focus input, focus sidebar, jump to bottom
- **Tabs**: prev/next tab, toggle star, image carousel
- **Features**: files, history, auto run, git diff, git log, sessions, logs, process monitor, usage dashboard, markdown mode, prompt composer, wizard, file search, bookmark, symphony, auto-scroll, director notes

### Keyboard Mastery Gamification

5-tier proficiency tracking:
1. **Beginner** (0%) -- "Just starting out"
2. **Student** (25%) -- "Learning the basics"
3. **Performer** (50%) -- "Getting comfortable"
4. **Virtuoso** (75%) -- "Almost there"
5. **Keyboard Maestro** (100%) -- "Complete mastery"

Celebration modal with confetti (intensity scales with level), music-themed colors, progress bars. Respects reduced-motion.

### Cmd+K Quick Actions Modal

Command palette for discovering and executing any action. Similar to VS Code's command palette or Raycast.

**Capsem takeaway:**
- **Cmd+K command palette** is table stakes for keyboard-first apps -- capsem should have one
- We currently have minimal keyboard shortcuts; we need at minimum: toggle sidebar, switch views, focus terminal, open settings
- The mastery gamification is cute but probably not a priority for capsem
- **Shortcuts help modal** (Cmd+/) showing all available shortcuts is essential
- Configurable shortcuts (stored in settings) is important for power users

---

## 7. Conductor Badge / Achievement System (LOW relevance)

11-tier system based on cumulative Auto Run time:
- Apprentice Conductor (15m) -> Titan of the Baton (10y)
- Each badge has flavor text, historical conductor examples, Wikipedia links

The `AchievementCard.tsx` component displays these with conductor-themed styling.

**Capsem takeaway:**
- Fun but not relevant for capsem's security-focused use case
- Could potentially track "sandbox hours" or "agents sandboxed" as lightweight gamification later

---

## 8. Icons & Branding (MEDIUM relevance)

### Agent Icons (emoji-based)

Simple emoji mapping per agent type: Claude (robot), Codex (diamond), Terminal (laptop), etc. Quick and recognizable without custom icon design.

### Conductor Silhouette Branding

- `MaestroSilhouette.tsx` -- static and animated conductor silhouette
- Light/dark PNG variants
- Animated version with 2s conducting rotation (+-3 degrees)
- Used as visual identity throughout the app

### Modal Priority System (Z-Index Layering)

40+ modal layers with well-defined priorities:
- 1100+ : Celebration overlays
- 1000-1099 : Critical modals (quit confirm)
- 750-999 : High priority (wizard, create instance)
- 400-750 : Standard modals (settings, batch runner)
- 100-399 : Overlays (file preview, git diff)
- 1-99 : Autocomplete and filters

**Capsem takeaway:**
- We should define a proper z-index layering system for our modals/overlays rather than ad-hoc values
- A consistent branding element (capsem "shield" or "capsule" silhouette?) could strengthen identity
- Emoji icons for VM states or agent types is a lightweight approach worth considering

---

## 9. Process Monitor & Notifications (MEDIUM relevance)

**ProcessMonitor.tsx**: Shows running processes with resource usage (CPU, memory). Useful for debugging.

**NotificationsPanel.tsx**: Notification center for alerts and events.

**Speakable Notifications**: Text-to-speech announcements when agents complete tasks. Audio alerts configurable.

**Capsem takeaway:**
- Process monitor showing guest VM resource usage (CPU, memory, disk) would be very valuable
- Notification for "agent task complete" or "network request blocked" events would improve UX
- TTS notifications are overkill for capsem but audio pings for blocked requests could be useful

---

## 10. Group Chat (LOW relevance for now)

Multiple AI agents in a single conversation, orchestrated by a moderator AI that routes questions to the right agent.

**Capsem takeaway:**
- Not directly relevant, but if capsem ever manages multiple VMs, cross-VM orchestration could use a similar pattern

---

## 11. Mobile Remote Control (LOW-MEDIUM relevance)

Built-in web server with QR code access. Monitor and control agents from phone. Supports local network + Cloudflare tunneling.

**Capsem takeaway:**
- A web dashboard for monitoring sandboxed agents remotely could be interesting for enterprise use cases
- QR code for quick mobile access is a clever UX touch

---

## Priority Summary for Capsem

### High Priority (should implement soon)

| Feature | Effort | Impact |
|---------|--------|--------|
| Cmd+K command palette | Medium | High -- essential for keyboard-first UX |
| Keyboard shortcuts help (Cmd+/) | Low | High -- discoverability |
| First-run onboarding wizard | Medium | High -- reduces setup friction |
| Tour system (spotlight walkthrough) | Medium | High -- helps users discover features |
| Git status in status bar | Low | Medium -- table stakes for dev tools |
| Session starring/pinning | Low | Medium -- quick access to favorites |

### Medium Priority (v2-v3 timeframe)

| Feature | Effort | Impact |
|---------|--------|--------|
| Colorblind-safe palette option | Low | Medium -- accessibility |
| Container queries for responsive layout | Low | Medium -- graceful degradation |
| Draft auto-save for terminal input | Low | Medium -- prevents data loss |
| VM resource monitor (CPU/mem/disk) | Medium | Medium -- debugging aid |
| Z-index layering system | Low | Low-Medium -- code quality |
| Notification center | Medium | Medium -- event awareness |
| Configurable keyboard shortcuts | Medium | Medium -- power users |

### Low Priority (nice-to-have)

| Feature | Effort | Impact |
|---------|--------|--------|
| Custom theme builder | High | Low -- DaisyUI themes sufficient |
| Session activity sparklines | Medium | Low -- visual polish |
| Achievement/badge system | Medium | Low -- gamification |
| Mobile remote control | High | Low -- enterprise feature |
| Sandbox "playbook" recipes | High | Medium -- future differentiator |
| Group chat / multi-VM orchestration | High | Low -- future |

---

## 12. Usage Dashboard & Analytics (MEDIUM relevance)

### Architecture

- SQLite database (better-sqlite3, WAL mode) with tables: `query_events`, `auto_run_sessions`, `auto_run_tasks`
- Recharts-based visualizations with error boundaries and skeleton loaders
- Real-time updates via IPC event broadcasting with 300ms debounce

### Dashboard Tabs

| Tab | Metrics |
|-----|---------|
| Overview | Sessions, total queries, total time, avg duration, top agent, interactive % |
| Agents | Per-agent breakdown, git repos vs folders, remote vs local |
| Activity | Usage heatmap, weekday comparison, duration trends |
| Auto Run | Success rate, avg tasks/session, longest runs |

### Time Range Filtering

Persistent user preference across: Today, This Week (default), This Month, This Quarter, This Year, All Time.

**Capsem takeaway:**
- A usage dashboard for capsem would track: sandbox hours, network requests (allowed/denied), domains accessed, data transferred
- The heatmap visualization (activity over time) maps perfectly to our network telemetry
- Time range filtering is essential for any analytics view
- Error boundaries per chart section prevents one broken chart from killing the whole dashboard

---

## 13. UI Layout & Component Architecture (HIGH relevance)

### Three-Panel Layout

Maestro uses a resizable three-panel system:
1. **Left sidebar** (SessionList) -- agent/session navigation, collapsible
2. **Center** (MainPanel) -- active content with TabBar on top, InputArea on bottom
3. **Right sidebar** (RightPanel) -- Files, History, Auto Run tabs

All panel widths are persisted to settings. The right panel has 3 sub-tabs.

### MainPanel Content Hierarchy

```
TabBar (top)
Content Area (one of: TerminalOutput, LogViewer, InlineWizard, etc.)
InputArea (bottom)
```

### InputArea (Rich Input)

Maestro's input area is sophisticated:
- Markdown editor with syntax highlighting
- Image upload/staging area
- Slash command completion
- `@` mention completion (files/folders)
- Command history navigation (Cmd+L)
- Toggle between Enter-to-send and Cmd+Enter-to-send
- Per-tab thinking mode toggle (off/on/sticky)
- Context usage warning sash (yellow at 60%, red at 80%)
- Draft auto-save

### Tab System

- Tabs can be AI sessions OR file previews (unified tab bar)
- Drag-to-reorder across both types
- Rich right-click context menu: star, rename, merge, export HTML, publish Gist, close variations
- Tab naming: custom name -> session ID octet -> "New"
- Keyboard shortcuts: Cmd+1-9 for direct tab access

### Empty State

Clean welcome screen when no sessions exist, with centered content and menu overlay offering: New Agent, Wizard, Settings, Shortcuts, About, Start Tour.

### Lazy Loading

Heavy modals (Settings, LogViewer, Marketplace) are code-split with dynamic imports for performance.

**Capsem takeaway:**
- Our sidebar + main panel layout is already similar. Consider adding a right panel for network activity or file explorer
- The **rich input area** pattern (slash commands, file mentions, context warnings) would be very useful for capsem's terminal input
- **Unified tab bar** mixing different content types is a good model for when capsem has multiple views per session
- **Empty state with guided actions** is better than an empty terminal -- we should show a welcome screen with quick-start options
- Lazy loading modals is a good Svelte pattern too (dynamic `import()`)

---

## 14. Git Diff/Log Viewers (MEDIUM relevance)

### GitDiffViewer

- Tab system for multiple files in diff
- Syntax highlighting via `react-diff-view`
- Keyboard navigation: Cmd+[ and Cmd+] for tab switching
- Image diff support (ImageDiffViewer component)
- Auto-scrolls active tab into view
- Registered as modal in LayerStack

### GitLogViewer

- Loads 200 most recent commits with total count
- List navigation: arrow keys, vim keys, page up/down
- Shows commit author, date, refs (tags/branches)
- Fetches diff on commit selection, displays inline
- Image diff support

### GitStatusWidget

- Compact mode (narrow): file count + icon
- Full mode (wider): +additions, -deletions, ~modified breakdown
- GitHub-style diff bars
- Memoized to prevent re-renders

**Capsem takeaway:**
- Git integration is lower priority for capsem than network telemetry, but git status in the status bar is table stakes
- The compact/full mode pattern (adapting display to available width) is a good general pattern

---

## 15. Process Monitor (MEDIUM relevance)

Hierarchical tree view of active processes grouped by agent sessions, groups, and individual processes.

**Tracked per process:**
- PID, start time, runtime (formatted: "2m 30s", "1h 5m")
- Current working directory
- Tool type (agent implementation)
- Alive/dead status

**Actions:** Kill process, navigate to parent session, expand/collapse trees.

**Capsem takeaway:**
- A process monitor for the guest VM would be very valuable: show running processes inside the sandbox
- PID, runtime, resource usage (CPU%, MEM%) in a tree view
- Kill action maps to sending signals through vsock control channel

---

## 16. Director's Notes (LOW relevance)

An "encore feature" (disabled by default, opt-in) with three tabs:
1. **Unified History** -- chronological list of all agent activity with search, filter (AUTO/USER), activity graph, infinite scroll
2. **AI Overview** -- AI-generated synopsis of recent activity with configurable lookback (1-90 days)
3. **Help Tab** -- built-in reference guide

**Capsem takeaway:**
- The "AI overview of recent activity" concept could apply to capsem: "summarize what this agent did in the sandbox" based on network telemetry + terminal history
- Feature gating pattern (completely invisible when disabled) is clean

---

## 17. Thinking Status Indicator (HIGH relevance)

`ThinkingStatusPill.tsx` shows when AI is processing:
- Session name, bytes received, elapsed time
- Dropdown list of all busy tabs (multi-session)
- Stop/interrupt buttons
- Live elapsed time counter (1s updates)
- Auto-Run variant with different styling

**Capsem takeaway:**
- A status pill showing VM state (booting/running/idle) with elapsed time would improve feedback
- When a network request is in-flight through the MITM proxy, showing request status would be useful

---

## Priority Summary for Capsem

### High Priority (should implement soon)

| Feature | Effort | Impact |
|---------|--------|--------|
| Cmd+K command palette | Medium | High -- essential for keyboard-first UX |
| Keyboard shortcuts help (Cmd+/) | Low | High -- discoverability |
| First-run onboarding wizard | Medium | High -- reduces setup friction |
| Tour system (spotlight walkthrough) | Medium | High -- helps users discover features |
| Empty state with guided actions | Low | High -- better first impression |
| Git status in status bar | Low | Medium -- table stakes for dev tools |
| Session starring/pinning | Low | Medium -- quick access to favorites |
| Status pill (VM state + elapsed time) | Low | Medium -- feedback |

### Medium Priority (v2-v3 timeframe)

| Feature | Effort | Impact |
|---------|--------|--------|
| Colorblind-safe palette option | Low | Medium -- accessibility |
| Container queries for responsive layout | Low | Medium -- graceful degradation |
| Draft auto-save for terminal input | Low | Medium -- prevents data loss |
| VM resource/process monitor | Medium | Medium -- debugging aid |
| Z-index layering system (LayerStack) | Low | Low-Medium -- code quality |
| Notification center | Medium | Medium -- event awareness |
| Configurable keyboard shortcuts | Medium | Medium -- power users |
| Network analytics dashboard | Medium | High -- unique to capsem |
| Right panel (network activity feed) | Medium | Medium -- real-time monitoring |

### Low Priority (nice-to-have)

| Feature | Effort | Impact |
|---------|--------|--------|
| Custom theme builder | High | Low -- DaisyUI themes sufficient |
| Session activity sparklines | Medium | Low -- visual polish |
| Achievement/badge system | Medium | Low -- gamification |
| Mobile remote control | High | Low -- enterprise feature |
| Sandbox "playbook" recipes | High | Medium -- future differentiator |
| Group chat / multi-VM orchestration | High | Low -- future |
| AI summary of sandbox activity | High | Medium -- differentiation |
| Rich input (slash commands, @ mentions) | High | Medium -- power users |

---

## Key Architectural Patterns Worth Noting

1. **LayerStack for modals**: Instead of ad-hoc z-index, Maestro uses a centralized layer stack that manages Escape key handling, focus trapping, and stacking order. This prevents the "escape closes the wrong modal" bug. Each modal registers with `{ type, priority, blocksLowerLayers, capturesFocus, focusTrap, onEscape }`.

2. **IPC namespace pattern**: All IPC calls are organized under namespaces (`window.maestro.settings.*`, `window.maestro.git.*`, `window.maestro.stats.*`, `window.maestro.notification.*`). Capsem's Tauri invoke calls could benefit from similar organization.

3. **Template variables in prompts**: Wizard prompts use `{{agentName}}`, `{{path}}` etc. for dynamic content. Useful pattern for any generated content.

4. **Per-agent output parsers**: Each AI agent has its own output parser and error pattern definitions. If capsem ever supports multiple AI agents, this pattern scales well.

5. **Settings hook pattern**: `useSettings.ts` wraps all settings access with automatic persistence. Good pattern for our Svelte stores.

6. **Responsive container queries**: Progressive hiding of header elements based on available width. Much better than breakpoint-based media queries for component-level responsiveness.

7. **Focused context subscriptions**: Instead of one giant store, Maestro provides targeted accessors like `getFileCount(sessionId)` that prevent re-renders of unrelated components. Prevents O(n) cascade when session list updates.

8. **Stable callback pattern**: Tab/list callbacks receive an ID as first arg instead of closures, preventing new function allocations on each render. Important for list performance.

9. **Memoization strategy**: SessionItem, GitStatusWidget, ThinkingStatusPill sub-components are all memoized. Only the active session gets full detail rendering.

10. **Feature gating ("Encore Features")**: Completely invisible when disabled -- no shortcuts registered, no menu entries, no command palette entries. Clean opt-in for advanced features without cluttering the default UI.
