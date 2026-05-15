# Clean Latest Local Install

## Goal

Make `just install` the command developers can safely run all the time to put the current checkout onto the machine. It must hard-clean the existing local install first, preserve user settings, install through the same native package path as `install.sh`, and fail immediately when the installed VM cannot resolve DNS or reach the network.

## Why

The release-level DNS regression was not caught by the local install loop because local validation stopped at service health. A developer could have a VM booting, service and gateway answering, and still ship an initrd/runtime combination where guest DNS resolution fails.

## Decisions

- Keep the workflow centered on the existing `just install` and `just smoke`; do not add another top-level test system.
- `just install` first runs a forced Capsem uninstall, verifies the old install is gone, then runs the package install path.
- User settings are backed up outside `~/.capsem` before uninstall and restored after the fresh install; runtime state, assets, sockets, stale binaries, and units must not survive the clean step.
- macOS local install uses the same native command as `install.sh`: `sudo installer -pkg ... -target /`.
- Linux local install uses the same native command as `install.sh`: `sudo apt install -y ...`.
- The post-install gate must exercise the installed CLI and the guest network path, including DNS resolution and HTTPS.
- Cleanup is scoped to Capsem service/gateway/tray/process state and runtime files.

## Files

- `justfile`
- `tests/test_release_workflow_policy.py`
- `CHANGELOG.md`
- `sprints/dev-install-clean-latest/tracker.md`
- `sprints/dev-install-clean-latest/MASTER.md`

## Done

- Red tests demonstrate the missing local install invariants.
- `just install` builds from the current checkout, force-uninstalls the existing local install, proves the old install is gone except backed-up settings, installs the platform package, restores settings, and runs service/gateway/guest DNS/HTTPS checks.
- Targeted tests pass.
- Remaining live-install verification debt is explicit if the local machine cannot complete the installer run inside this turn.

## Testing Proof Matrix

- Unit/contract: static policy tests for `just install` recipe shape and required checks.
- Functional: `just --list` and targeted pytest.
- Adversarial: tests require stale-bin replacement and DNS checks so service-only success is insufficient.
- E2E/VM or integration: intended `just install` run; record exact outcome in tracker.
- Telemetry/observability: install logs include explicit service, gateway, and guest network gate names.
- Performance: no dedicated benchmark; recipe is intentionally a full local install path.
