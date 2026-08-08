# Sprint T21: Config Runtime
## Goal
Verify config values are applied at runtime inside VMs

## Files
- tests/capsem-config-runtime/conftest.py
- tests/capsem-config-runtime/test_default_resources.py
- tests/capsem-config-runtime/test_custom_resources.py
- tests/capsem-config-runtime/test_blocked_domain.py

## Tasks
- [x] test_default_cpu_ram: provision with no args, exec nproc -> 4, exec free -> ~4096MB
- [x] test_custom_cpu_ram: provision with cpus=2 ram_mb=2048, exec nproc -> 2, exec free -> ~2048MB
- [x] test_blocked_domain: exec curl to blocked domain, verify fails or denied in net_events
- [x] Marked config_runtime

## Verification
pytest tests/capsem-config-runtime/ -m config_runtime

## Depends On
T14
