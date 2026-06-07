# Sprint T20: Session Lifecycle
## Goal
Verify session.db exists after boot, has correct schema, events are written correctly

## Files
- tests/capsem-session-lifecycle/conftest.py
- tests/capsem-session-lifecycle/test_db_exists.py
- tests/capsem-session-lifecycle/test_db_schema.py
- tests/capsem-session-lifecycle/test_exec_events.py
- tests/capsem-session-lifecycle/test_file_events.py
- tests/capsem-session-lifecycle/test_db_survives_shutdown.py

## Tasks
- [x] test_db_exists_after_boot: provision VM, wait exec-ready, session.db exists, all 7 tables present
- [x] test_db_has_correct_schema: column names/types match capsem-logger CREATE_SCHEMA for every table
- [x] test_exec_creates_events: exec curl in VM, wait 2s flush, query net_events for matching domain
- [x] test_file_write_creates_fs_event: write file via API, query fs_events for matching path
- [x] test_db_survives_clean_shutdown: boot, exec, copy session.db, delete, verify copy is valid SQLite with data
- [x] Marked session_lifecycle

## Verification
pytest tests/capsem-session-lifecycle/ -m session_lifecycle

## Depends On
T14
