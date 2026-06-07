# Sprint T9: Configuration Tests

## Goal

Verify that configuration is obeyed at every level: VM count limits, resource bounds validation, per-VM overrides, hot-reload of policies, and correct defaults.toml fallback behavior.

## Files

```
tests/capsem-config/
    conftest.py
    test_vm_limits.py
    test_resource_limits.py
    test_per_vm_config.py
    test_hot_reload.py
    test_defaults.py
```

Marker: `config`

## Tasks

### VM Limits (`test_vm_limits.py`)
- [ ] Create VMs up to the configured limit, verify all succeed
- [ ] Attempt to create limit+1 VM, verify error returned
- [ ] Delete one VM, then create a new one, verify it succeeds
- [ ] Change the VM limit in config and verify the new limit is enforced

### Resource Bounds (`test_resource_limits.py`)
- [ ] Reject CPU count < 1
- [ ] Reject CPU count > 8
- [ ] Reject RAM < 1 GB
- [ ] Reject RAM > 16 GB
- [ ] Accept valid CPU and RAM values within bounds

### Per-VM Config (`test_per_vm_config.py`)
- [ ] Create two VMs with different RAM allocations, verify each has the correct amount
- [ ] Create two VMs with different CPU counts, verify each has the correct count
- [ ] Create VMs with different snapshot intervals, verify each fires at its own cadence

### Hot Reload (`test_hot_reload.py`)
- [ ] Allow a domain, verify access works, change config to block it, trigger reload, verify blocked immediately
- [ ] Change MCP tool policy, trigger reload, verify new policy takes effect without VM restart

### Defaults (`test_defaults.py`)
- [ ] Boot with no user config, verify every setting in defaults.toml is applied
- [ ] Set a user override for one setting, verify it wins over default
- [ ] Set a corp-level override, verify it wins over both user and default

### Infrastructure (`conftest.py`)
- [ ] Create shared fixture: service running with configurable settings
- [ ] Register `config` pytest marker
- [ ] Helper to modify config and trigger reload

## Verification

```bash
pytest tests/capsem-config/ -m config -v
```

All tests green. Config changes take effect at the correct precedence level without requiring restarts (where hot-reload is supported).

## Depends On

None (tests configuration subsystem directly).
