# Clean Latest Local Install Sprint

## Status

| Area | State | Notes |
| --- | --- | --- |
| Planning | Complete | Scope is hard clean-install for `just install`. |
| Tests | Complete | Red policy tests added and observed failing before recipe changes. |
| Implementation | Complete | `just install` now hard-cleans, clears stale app bundles, uses native install commands, and checks installed network path. |
| Verification | Partial | Static/unit gates pass; live sudo install reached password prompt after hard clean. |

## Release Holds

- Keep hold active until `just install` can no longer pass with service-only health while guest DNS is broken.
- Keep hold active until `just install` verifies a forced uninstall removed stale local install state before package installation.
- Keep hold active until local install uses the same native install commands as `install.sh`.

## Current Verification

- `uv run python -m pytest tests/test_release_workflow_policy.py -q` -- passed.
- `cargo test -p capsem uninstall_does_not_refresh_update_cache -- --nocapture` -- passed.
- `cargo fmt --check` -- passed.
- `just --list | rg "install|test-install"` -- passed.
- Live `just install` -- passed the hard-clean phase; blocked at macOS sudo password before native package installation and VM DNS/HTTPS verification.

## Verification Commands

- `uv run python -m pytest tests/test_release_workflow_policy.py -q`
- `just --list`
- `just install`
