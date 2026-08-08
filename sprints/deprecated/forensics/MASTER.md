# Meta-Sprint: Forensic Capabilities

Complete forensic layer for capsem: structured logging to SQLite, FTS5 full-text search, namespaced REST API, CLI commands with generic formatting and JSON output.

## Status

| Phase | Name | Status | Commits | Dependencies |
|-------|------|--------|---------|--------------|
| 0 | API restructure | Not Started | 0a | None |
| 1 | Process logs to SQLite | Not Started | 1a, 1b | Phase 0 |
| 2 | FTS5 search | Not Started | 2a, 2b, 2c | Phase 1 |
| 3 | Forensic CLI | Not Started | 3a-3f | Phase 0, 2 |
| 4 | MCP + integration + tests | Not Started | 4a-4f | Phase 3 |
| 5 | Documentation | Not Started | 5a-5e | Phase 4 |

## Architecture

### Symmetry Principle

VM and service scopes are isomorphic -- same capabilities, different data sources:

| Capability | VM scope | Service scope |
|------------|----------|---------------|
| info | VM metadata + session.db summary | Service metadata + main.db summary |
| query | SQL against session.db | SQL against main.db |
| search | FTS5 across session.db tables | FTS5 across main.db tables |
| logs | Event timeline from session.db (via query) | (VM-only: no service event timeline) |
| syslog / service logs | Raw serial.log + process.log | Raw service.log |

### API Namespaces

- `/vm/{id}/...` -- VM-scoped operations (info, query, search, logs, exec, etc.)
- `/vm/search` -- cross-VM FTS5 search (no id)
- `/service/...` -- service-scoped operations (info, query, search, logs, provision, list, etc.)
- `/image/...` -- image operations (list, inspect, delete)

### Data Flow

```
events -> DbWriter channel -> session.db (WAL) -> DbReader -> /vm/{id}/query -> CLI/MCP
                                                            -> /vm/{id}/search (FTS5)
                                                            
tracing -> DbTracingLayer -> process_events table (in session.db)
                          -> stderr -> process.log (text fallback)

main.db (session rollups) -> /service/query -> CLI/MCP
                          -> /service/search (FTS5)
```

### CLI Commands

```
capsem info [id]                  # VM info (defaults to latest)
capsem query <sql> [id]           # SQL against VM session.db
capsem search <term> [--id <id>]  # FTS5 search across ALL VMs
capsem logs [id]                  # Event timeline from session.db
capsem syslog [id]                # Raw serial/process text logs

capsem service info               # Service metadata
capsem service query <sql>        # SQL against main.db
capsem service search <term>      # FTS5 search across main.db
capsem service logs               # Raw service.log
```

All commands support `--json` for machine-readable columnar JSON.

### Query/Search Protocol

Request/response format is uniform across all query/search endpoints:

```
POST {endpoint}  { "sql": "SELECT ..." }           -- query
POST {endpoint}  { "term": "...", "limit": 20 }    -- search
```

Response (always columnar):
```json
{ "columns": ["col1", "col2"], "rows": [["val1", "val2"], ...], "has_more": true }
```

**Pagination**: All endpoints support `limit` + `offset` + cursor-based `after_id` for efficient deep paging.

**Search ranking**: FTS5 `bm25()` normalized across tables, sortable by relevance or timestamp. Results include `_source` (table) and `_score` (relevance) columns.

**Timeline projection**: All event tables map to a standard shape: `timestamp`, `event_type`, `summary`, `status`, `duration_ms`, `metadata` (JSON).

**Security**: 4 layers -- string validation, SQLITE_OPEN_READ_ONLY, authorizer callback (DENY non-read), PRAGMA query_only. Search terms always bind-parameterized.

Generic table formatter in CLI renders any columnar response as a comfy-table with smart formatting for known column patterns (bytes, tokens, cost, duration).

## Key just recipes

```bash
just test           # All tests including forensic E2E + adversarial
just shell          # Boot VM for manual testing
```

## Plan

Full implementation details: [plan.md](plan.md)
