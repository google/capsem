# Sprint T8: Multi-VM Isolation Tests

## Goal

Prove that multiple VMs are fully independent: filesystem writes, session databases, network policies, and MCP gateway state never leak between VMs. Verify that resume preserves per-VM state after selective teardown.

## Files

```
tests/capsem-isolation/
    conftest.py
    test_filesystem.py
    test_session_db.py
    test_network.py
    test_mcp_gateway.py
    test_resume.py
```

Marker: `isolation`

Fixture: 2+ VMs booted and exec-ready before each test module.

## Tasks

### Filesystem Isolation (`test_filesystem.py`)
- [ ] Write a file in VM-A, verify it is absent in VM-B
- [ ] Write different content to the same path in A and B, verify each reads its own
- [ ] Delete a file in VM-B, verify it still exists in VM-A
- [ ] Boot a new VM-C, verify it has no leftover files from VM-B

### Session DB Isolation (`test_session_db.py`)
- [ ] Exec a command in VM-A, verify event appears only in A's session.db
- [ ] Query VM-B's session.db and confirm zero events from VM-A
- [ ] Verify each VM has a separate session_dir on the host

### Network Policy Isolation (`test_network.py`)
- [ ] Configure VM-A to allow domain X, VM-B to block domain X
- [ ] Verify VM-A can reach domain X
- [ ] Verify VM-B cannot reach domain X
- [ ] Verify each VM's policy is independently enforced

### MCP Gateway Isolation (`test_mcp_gateway.py`)
- [ ] Configure different MCP tool lists for VM-A and VM-B
- [ ] Verify each VM sees only its own tools
- [ ] Call a tool in VM-A, verify no cross-talk in VM-B's mcp_calls table

### Resume (`test_resume.py`)
- [ ] Start VM-A and VM-B, write a marker file in VM-A
- [ ] Delete VM-B while VM-A continues running
- [ ] Verify VM-A's marker file is still present
- [ ] Verify exec still works in VM-A after VM-B deletion

### Infrastructure (`conftest.py`)
- [ ] Create shared fixture: 2+ VMs booted and exec-ready
- [ ] Register `isolation` pytest marker
- [ ] Teardown: delete all VMs after test module completes

## Verification

```bash
pytest tests/capsem-isolation/ -m isolation -v
```

All tests green. Each VM operates in complete isolation with no state leakage.

## Depends On

None (tests VM isolation primitives directly).
