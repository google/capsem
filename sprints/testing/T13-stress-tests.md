# Sprint T13: Stress and Load Tests

## Goal

Validate system behavior under load: concurrent VM operations, rapid lifecycle churn, large file transfers, resource cleanup, crash recovery, client disconnection, and rapid command execution.

## Files

```
tests/capsem-stress/
    conftest.py
    test_concurrent_vms.py
    test_rapid_lifecycle.py
    test_large_files.py
    test_resource_cleanup.py
    test_process_crash.py
    test_client_disconnect.py
```

Marker: `stress`

## Tasks

### Concurrent VMs (`test_concurrent_vms.py`)
- [ ] Boot 10 VMs simultaneously, verify all reach exec-ready state
- [ ] Attempt to boot VM #11 at the configured limit, verify error returned
- [ ] Run 5 concurrent exec commands across different VMs, verify all return correct results

### Rapid Lifecycle (`test_rapid_lifecycle.py`)
- [ ] Run 10 create/delete VM cycles in sequence, verify no file descriptor leaks
- [ ] Run 10 create/delete VM cycles in sequence, verify no RSS memory growth

### Large Files (`test_large_files.py`)
- [ ] Write a 1MB file to a VM workspace, read it back, verify content matches

### Resource Cleanup (`test_resource_cleanup.py`)
- [ ] Boot and delete 10 VMs, verify all temporary directories are cleaned up
- [ ] Verify no orphan processes remain after VM deletion
- [ ] Verify UDS socket files are removed after service shutdown

### Process Crash Recovery (`test_process_crash.py`)
- [ ] Kill a capsem-process with `kill -9`, verify the service detects it
- [ ] After process crash, verify the service can create a new VM successfully
- [ ] Verify the service marks the crashed VM as failed (not running)

### Client Disconnect (`test_client_disconnect.py`)
- [ ] Disconnect a CLI client mid-exec, verify the service does not hang
- [ ] Restart the service, verify it discovers and cleans up stale VM instances
- [ ] Run 100 rapid exec commands against a single VM, verify all complete successfully

### Infrastructure (`conftest.py`)
- [ ] Create shared fixture: service running with stress-appropriate config
- [ ] Register `stress` pytest marker
- [ ] Helper to measure FD count and RSS for a given PID
- [ ] Longer timeout defaults for stress tests

## Verification

```bash
pytest tests/capsem-stress/ -m stress -v --timeout=300
```

All tests green. No resource leaks, no hangs, no orphan processes. The system recovers gracefully from crashes and client disconnections.

## Depends On

None (tests system behavior under load directly).
