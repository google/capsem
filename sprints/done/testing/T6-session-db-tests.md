# Sprint T6: Session DB Tests

## Goal

Verify that `session.db` telemetry is correct after running various workloads. These tests boot a VM, run a known workload, then open the session database with `sqlite3` and assert on row counts, field values, and cross-table foreign key integrity.

## Files

**Create:**
- `tests/capsem-session/conftest.py` -- fixtures: boot VM, run workload, open session.db
- `tests/capsem-session/test_net_events.py`
- `tests/capsem-session/test_model_calls.py`
- `tests/capsem-session/test_tool_calls.py`
- `tests/capsem-session/test_file_events.py`
- `tests/capsem-session/test_mcp_calls.py`
- `tests/capsem-session/test_snapshot_events.py`

## Tasks

### Fixtures (`conftest.py`)

- [ ] `booted_vm` fixture: provision and boot a VM with default settings
- [ ] `workload_runner` fixture: helper to exec commands and trigger known telemetry patterns
- [ ] `session_db` fixture: after workload, copy session.db from VM, open with `sqlite3`, yield connection
- [ ] Cleanup: delete VM and temp files on teardown

### net_events (`test_net_events.py`)

- [ ] Allowed request: row exists with correct `domain`, `port`, `method`, `status_code`, `direction`
- [ ] Denied request: row exists with `decision = 'deny'`, `deny_reason` populated
- [ ] All required fields non-null: `timestamp`, `domain`, `port`, `protocol`
- [ ] Duplicate requests create separate rows (no dedup)
- [ ] Timestamp ordering matches request order

### model_calls (`test_model_calls.py`)

- [ ] Row exists with correct `provider` (e.g., "anthropic", "openai")
- [ ] Row has `model` field matching the model used
- [ ] `input_tokens` and `output_tokens` are positive integers
- [ ] `cost` is non-negative float
- [ ] `trace_id` is non-null and unique per call
- [ ] `timestamp` is within expected time range

### tool_calls (`test_tool_calls.py`)

- [ ] Native tool call: `origin = 'native'`, `tool_name` matches
- [ ] MCP tool call: `origin = 'mcp'`, `mcp_call_id` is non-null
- [ ] `tool_name` is non-empty for all rows
- [ ] `timestamp` is within expected time range

### tool_responses (`test_tool_calls.py` continued or separate)

- [ ] Response content is non-null for completed calls
- [ ] `is_error = 0` for successful tool calls
- [ ] `is_error = 1` for failed tool calls, with error content populated
- [ ] Each response has a matching tool_call row (FK integrity)

### file_events (`test_file_events.py`)

- [ ] File create event: `event_type = 'create'`, `path` matches, `size` is correct
- [ ] File modify event: `event_type = 'modify'`, `path` matches, `size` updated
- [ ] File delete event: `event_type = 'delete'`, `path` matches
- [ ] `path` is absolute and within the workspace directory
- [ ] `size` is non-negative for create/modify events

### mcp_calls (`test_mcp_calls.py`)

- [ ] Row has `method` field (e.g., "tools/call")
- [ ] Row has `tool_name` matching the MCP tool invoked
- [ ] `decision` field is either 'allow' or 'deny'
- [ ] Denied MCP call has `deny_reason` populated
- [ ] `timestamp` is within expected time range

### snapshot_events (`test_snapshot_events.py`)

- [ ] Auto snapshot: `trigger = 'auto'`, `fs_event_id_start` and `fs_event_id_end` define valid range
- [ ] Manual snapshot: `trigger = 'manual'`
- [ ] `fs_event_id_start <= fs_event_id_end` for all rows
- [ ] Referenced `fs_event_id` values exist in the file_events table

### Cross-table FK integrity

- [ ] Every `tool_calls.model_call_id` references a valid `model_calls.id`
- [ ] Every `tool_responses.tool_call_id` references a valid `tool_calls.id`
- [ ] Every `snapshot_events.fs_event_id_start` and `fs_event_id_end` reference valid `file_events.id` ranges
- [ ] No orphan rows in child tables

## Verification

- `pytest tests/capsem-session/ -m session -v` passes all tests
- Tests produce clear output showing which telemetry fields were checked
- Total suite completes in under 10 minutes (includes VM boot time)

## Depends On

- **T0-infrastructure** (test directories, markers, session DB schema knowledge)
- **T5-service-integration** (service must handle provisioning and exec reliably)
