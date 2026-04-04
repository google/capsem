# Sprint T25: Session DB Tables
## Goal
Exhaustive per-table session.db tests that trigger events and verify row data

## Files
- tests/capsem-session-exhaustive/conftest.py
- tests/capsem-session-exhaustive/test_net_events_data.py
- tests/capsem-session-exhaustive/test_model_calls_data.py
- tests/capsem-session-exhaustive/test_tool_calls_data.py
- tests/capsem-session-exhaustive/test_mcp_calls_data.py
- tests/capsem-session-exhaustive/test_fs_events_data.py
- tests/capsem-session-exhaustive/test_snapshot_events_data.py
- tests/capsem-session-exhaustive/test_cross_table_data.py

## Tasks
- [x] net_events: curl allowed domain -> row with domain/decision=allowed/status_code/bytes/duration; curl blocked -> decision=denied; verify timestamp ISO 8601, port=443
- [x] model_calls: (needs AI key, skip if unavailable) API call -> provider/model/tokens/cost/trace_id/duration
- [x] tool_calls: native tool -> origin=native/tool_name/arguments; MCP tool -> origin=mcp/mcp_call_id FK
- [x] tool_responses: after tool_call -> matching call_id/content_preview; error -> is_error=1
- [x] mcp_calls: initialize/tools_list/tools_call methods; blocked call -> decision=denied/error_message
- [x] fs_events: write file -> action=created/path/size; rm file -> action=deleted; timestamp ISO 8601
- [x] snapshot_events: auto -> origin=auto/slot/files_count; manual -> origin=manual/name/hash; fs_event_id range valid
- [x] cross_table: tool_calls.model_call_id FK valid, tool_responses.call_id FK valid, mcp origin FK valid, snapshot fs range valid
- [x] Marked session_exhaustive

## Verification
pytest tests/capsem-session-exhaustive/ -m session_exhaustive

## Depends On
T14
