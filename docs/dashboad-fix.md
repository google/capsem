# UI Fix Plan



## 2. Charts by Tab


### Dashboard (cross-session, from main.db)

- 4 stat cards: Sessions, Total Cost, Total Tokens, Tool Calls
- Provider Usage: bar chart per provider (tokens, calls, cost)
- Top 5 Tools: horizontal bar chart (calls, avg duration tooltip)
- Session History: scrollable table (ID, mode, status, start, duration, tokens, cost, requests)
- Session Detail: expanded 7 stat cards (duration, tokens, cost, tools, HTTPS ok/denied, mode, disk+RAM)

### AI (per-session model usage)

- Row 1: Usage Per Provider (vertical bar), Cost Per Provider (doughnut with center total), Total Tokens card
- Row 2: Tokens Over Time (stacked bar, 5 traces/bucket), Cost Over Time (stacked bar by provider)
- Row 3: Model Usage tokens (horizontal bar), Model Cost (horizontal bar)
- Below: Trace viewer with expandable spans (existing)

### MCP

- 4 stat cards: Total Calls, Allowed, Warned, Denied
- Calls Over Time: stacked bar (5 calls/bucket, color by server)
- Top Servers: vertical bar
- Top Tools: vertical bar (color by server)

### Network (Traffic)

- 4 stat cards: Total Requests, Accepted, Denied, Avg Latency (ms)
- Requests Over Time: stacked bar (allowed/denied)
- HTTP Methods: doughnut
- Top Domains: stacked bar (allowed/denied)
- Top Processes: vertical bar

### Files

- 4 stat cards: Total, Created, Modified, Deleted
- Action Breakdown: doughnut
- Events Over Time: stacked bar (10 events/bucket)
- Search box (debounced) + event log table

