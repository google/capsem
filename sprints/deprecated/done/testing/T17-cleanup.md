# Sprint T17: Cleanup
## Goal
Verify VM delete actually cleans up (process killed, socket removed, session dir removed)

## Files
- tests/capsem-cleanup/conftest.py
- tests/capsem-cleanup/test_process_killed.py
- tests/capsem-cleanup/test_socket_removed.py
- tests/capsem-cleanup/test_session_dir_removed.py
- tests/capsem-cleanup/test_no_zombie.py

## Tasks
- [x] test_process_killed: create VM, get PID from info, delete, os.kill(pid,0) raises ProcessLookupError
- [x] test_socket_removed: create VM, verify instance socket exists, delete, verify gone
- [x] test_session_dir_removed: create VM, verify session dir exists, delete, verify gone
- [x] test_no_zombie: create+delete 5 VMs, pgrep capsem-process count is 0
- [x] Marked cleanup

## Verification
pytest tests/capsem-cleanup/ -m cleanup passes

## Depends On
T14
