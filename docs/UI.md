# Capsem UI Reference

Snapshot of the UI as of 2026-03-02 before the analytics rebuild.
Astro 5 + Svelte 5 + Tailwind v4 + DaisyUI v5.

## Views

### Terminal (`TerminalView.svelte`)

Primary view. Always mounted (visibility-toggled to avoid xterm refit flash).

**Elements:**
- Full-screen xterm.js terminal (web component `capsem-terminal`)
- Footer bar (terminal-bg colored):
  - Model stats: tokens count, tool count, cost (from `networkStore.modelStats`)
  - "Session stats" button linking to Analytics > Models

**State:**
- `themeStore.theme` -- terminal background/foreground color derivation
- `networkStore.modelStats` -- `{ model_call_count, total_input_tokens, total_output_tokens, total_model_duration_ms, total_estimated_cost_usd }`
- `networkStore.toolCount` -- total tool call count

**Formatters:** `formatTokens(n)` (K/M), `formatCost(usd)` ($X.XX / $0.00XX)

---

### Analytics (`AnalyticsView.svelte`)

Container with SubMenu left rail + 5 sub-sections.

**Sub-sections:**
1. Dashboard
2. AI (Models)
3. MCP
4. Network (Traffic)
5. Files

**State:** `sidebarStore.analyticsSection` ('dashboard' | 'models' | 'mcp' | 'traffic' | 'files')

#### Dashboard (`analytics/DashboardSection.svelte`)

Cross-session overview from main.db.

**Elements:**
- 4 stat cards: Sessions, Total Cost, Total Tokens, Tool Calls
- Provider Usage: bar chart per provider (tokens, calls, cost)
- Top 5 Tools: horizontal bar chart (calls, avg duration tooltip)
- Session History: scrollable table (ID, mode, status badge, start time, duration, tokens, cost, requests)
- Session Detail: expanded view with 7 stat cards (duration, tokens, cost, tools, HTTPS ok/denied, mode, disk+RAM)

**SQL (main.db):** `GLOBAL_STATS_SQL`, `TOP_PROVIDERS_SQL`, `TOP_TOOLS_SQL`, `SESSION_HISTORY_SQL`

**Types:** `GlobalStats`, `ProviderSummary`, `ToolSummary`, `SessionRecord`

#### Models (`analytics/ModelsSection.svelte`)

Per-session AI model usage. Chart.js charts.

**Elements:**
- Row 1: Usage Per Provider (vertical bar), Cost Per Provider (doughnut with center total), Total Tokens card
- Row 2 (if traces): Tokens Over Time (stacked bar, 5 traces/bucket), Cost Over Time (stacked bar by provider)
- Row 3 (if models): Model Usage tokens (horizontal bar), Model Cost (horizontal bar)

**SQL (session db):** `PROVIDER_USAGE_SQL`, `TRACES_SQL` (last 50), `MODEL_STATS_SQL`

**Types:** `ProviderTokenUsage`, `TraceSummary`

**Colors:** Provider-keyed (Google=blue, Anthropic=orange, OpenAI=green, Mistral=red)

#### MCP (`analytics/McpSection.svelte`)

MCP tool call analytics.

**Elements:**
- 4 stat cards: Total Calls, Allowed, Warned, Denied
- Calls Over Time: stacked bar (5 calls/bucket, color by server)
- Bottom row: Top Servers (vertical bar), Top Tools (vertical bar, color by server)

**SQL (session db):** `MCP_CALLS_SQL`, `MCP_STATS_SQL`, `MCP_BY_SERVER_SQL`

**Types:** `McpCall`, `McpServerCallCount`

#### Traffic (`analytics/TrafficSection.svelte`)

Network request analytics with charts.

**Elements:**
- 4 stat cards: Total Requests, Accepted, Denied, Avg Latency (ms)
- Row 2: Requests Over Time (stacked bar, allowed=blue/denied=purple), HTTP Methods (doughnut)
- Row 3: Top Domains (stacked bar, allowed/denied), Top Processes (vertical bar)

**Data:** All from `networkStore` (polled every 2s): `timeBuckets`, `methodDist`, `topDomains`, `processDist`, `netStats`, `avgLatency`

**Types:** `TimeBucket`, `DomainCount`

#### Files (`analytics/FilesSection.svelte`)

Filesystem event analytics.

**Elements:**
- 4 stat cards: Total, Created, Modified, Deleted
- Charts: Action Breakdown (doughnut), Events Over Time (stacked bar, 10 events/bucket)
- Search box (debounced 300ms by path)
- Event log table: Time (relative), Action (badge), Path (truncated), Size (KB/MB)

**SQL (session db):** `FILE_STATS_SQL`, `FILE_EVENTS_SQL`, `FILE_EVENTS_SEARCH_SQL`

**Types:** `FileEvent`, `FileAction`

**Colors:** Created=blue, Modified=sky, Deleted=purple (no green/red)

---

### Network (`NetworkView.svelte`)

Network policy inspector + event log. NOT analytics -- this is the policy/event view.

**Elements:**
- 3 summary cards: Total calls, Allowed, Denied (from `networkStore`)
- Active Policy (collapsible):
  - Default action badge (deny/allow)
  - Corp managed badge
  - Conflict count badge
  - Expandable: Conflicts list, Allowed domains list, Blocked domains list
- Search box (debounced 300ms, SQL-driven by domain/method/path/rule)
- Event log table: Time, Domain, Request (method+path), Status code, Decision badge

**State:**
- `networkStore.totalCalls`, `.allowedCount`, `.deniedCount`, `.events`
- `policy: NetworkPolicyResponse` (loaded on mount via `getNetworkPolicy()`)
- `search`, `searchResults`, `policyExpanded`, `allowExpanded`, `blockExpanded`

**SQL (session db):** `NET_EVENTS_SQL`, `NET_EVENTS_SEARCH_SQL`

**Types:** `NetEvent`, `NetworkPolicyResponse`

---

### Settings (`SettingsView.svelte`)

Hierarchical settings tree with SubMenu navigation.

**Elements:**
- SubMenu left rail: one group per top-level settings tree node
- SettingsSection (recursive):
  - Group headers (collapsible, with enable/disable toggle)
  - Leaf controls: bool (toggle), apikey/password (reveal/hide), number (min/max), text+choices (select), text (input), file (JSON editor with syntax highlighting)
  - Badges: "corp" (locked), "modified" (user override)
  - Lint issue messages below fields

**State:**
- `settingsStore.tree`, `.sections`, `.issues`, `.loading`, `.needsSetup`
- `sidebarStore.settingsSection` (dynamic, derived from tree)
- Local: `expandedGroups`, `showAdvanced`, `revealedKeys`, `editingFiles`, `fileDrafts`, `pathDrafts`, `copiedId`

**API:** `getSettingsTree()`, `lintConfig()`, `updateSetting(id, value)`

**Types:** `SettingsNode` (= `SettingsGroup` | `SettingsLeaf`), `ConfigIssue`, `ResolvedSetting`

---

### Wizard (`WizardView.svelte`)

First-run setup prompt. Shown when `settingsStore.needsSetup` is true (no enabled API keys with values).

**Elements:**
- Centered welcome card
- Button to jump to Settings
- Button to skip to Terminal

---

## Shared Components

### App.svelte
Root layout. Sidebar + content area. Terminal always mounted (visibility-toggled). On mount: init theme, VM, network polling, settings load. First-run check navigates to wizard.

### Sidebar.svelte
Vertical icon rail (w-12). Items: Console (terminal), Analytics, Settings. Active state: bg-primary/15 text-primary.

### StatusBar.svelte
Footer bar. Left: VM state indicator + renderer type. Center: HTTPS ok/denied counts. Right: Analytics toggle button.

### SubMenu.svelte
Multi-group navigation menu used by Analytics and Settings views. Props: groups, active, onSelect.

### Terminal.svelte
Web component wrapper for xterm.js. Input batching (5ms/4KB), poll-based output, theme sync.

### TraceDetail.svelte
Expandable trace detail view for model call inspection.

### VmStateIndicator.svelte
Colored dot + state text. Uses vmStore.dotColor, vmStore.statusColor.

### ThemeToggle.svelte
Light/dark mode toggle button.

---

## Stores

### vm.svelte.ts
- `vmState: string` -- current VM state
- `statusColor`, `dotColor` -- derived CSS classes
- `isRunning: boolean` -- derived
- `terminalRenderer: string` -- 'webgl' | 'canvas' | ''

### network.svelte.ts
Polls 9 SQL queries every 2s.
- `events: NetEvent[]` -- last 200 net events
- `netStats` -- { net_total, net_allowed, net_denied, net_error, net_bytes_sent, net_bytes_received }
- `topDomains: DomainCount[]`
- `timeBuckets: TimeBucket[]`
- `avgLatency: number` (ms)
- `methodDist: MethodDistRow[]`
- `processDist: ProcessDistRow[]`
- `modelStats` -- { model_call_count, total_input_tokens, total_output_tokens, total_model_duration_ms, total_estimated_cost_usd }
- `toolCount: number`
- Derived: `totalCalls`, `allowedCount`, `deniedCount`

### settings.svelte.ts
- `tree: SettingsNode[]` -- hierarchical
- `issues: ConfigIssue[]` -- lint results
- `loading: boolean`, `error: string | null`
- Derived: `sections` (top-level names), `flatLeaves`, `needsSetup`

### sidebar.svelte.ts
- `activeView: ViewName` ('terminal' | 'analytics' | 'settings' | 'wizard')
- `analyticsSection: AnalyticsSection` ('dashboard' | 'models' | 'mcp' | 'traffic' | 'files')
- `settingsSection: SettingsSection` (string)

### theme.svelte.ts
- `theme: 'dark' | 'light'` -- persisted to localStorage

---

## Data Layer

### api.ts
Typed Tauri invoke wrappers with auto-mock fallback. Non-SQL commands only.
Key commands: vmStatus, serialInput, terminalResize, terminalPoll, getNetworkPolicy, getSettings, getSettingsTree, lintConfig, updateSetting, getVmState, getSessionInfo.

### db.ts
Unified SQL gateway. `queryDb(sql, params, db)` routes to Tauri `query_db` or mock sql.js fixture. Helpers: `queryOne<T>`, `queryAll<T>`.

### sql.ts
All SQL queries. Session db: NET_STATS, TOP_DOMAINS, NET_TIME_BUCKETS, AVG_LATENCY, METHOD_DIST, PROCESS_DIST, MODEL_STATS, TOOL_COUNT, PROVIDER_USAGE, TOOL_USAGE, TRACES, MCP_STATS, MCP_BY_SERVER, MCP_CALLS, FILE_STATS, FILE_EVENTS, NET_EVENTS, MODEL_CALLS + search variants. Main db: SESSION_HISTORY, GLOBAL_STATS, TOP_PROVIDERS, TOP_TOOLS, TOP_MCP_TOOLS.

### mock.ts
Browser dev mode. sql.js loads test.db fixture for session queries. Static mock data for main.db (MOCK_SESSION_HISTORY, provider/tool/mcp usage). Mock settings (30+ ResolvedSetting), mock tree builder, mock API for non-SQL commands.

---

## Icons

Analytics-related: AiIcon, AnalyticsIcon, DashboardIcon, FilesIcon, McpIcon, ModelIcon, SessionsIcon, TrafficIcon
Settings-related: EnvironmentIcon, McpSettingsIcon, NetworkPolicyIcon, PaletteIcon, ProvidersIcon, ResourcesIcon, SettingsIcon
Core: TerminalIcon
