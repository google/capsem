# Sprint: dev-install-clean-latest

## Tasks

- [x] Create sprint plan and tracker
- [x] Add failing policy tests for hard uninstall, native install path, and network gate -- `uv run python -m pytest tests/test_release_workflow_policy.py -q` failed on the three new tests before the recipe change.
- [x] Harden `just install` -- recipe now preserves settings, runs forced uninstall, asserts clean state, uses native install commands, verifies installed layout/version, service, gateway, and guest DNS/HTTPS.
- [x] Fix uninstall side effect found by live gate -- live `just install` caught `capsem uninstall --yes` recreating `~/.capsem/update-check.json`; `cargo test -p capsem uninstall_does_not_refresh_update_cache -- --nocapture` now covers the guard.
- [x] Fix stale app bundle rebuild blocker -- live `just install` caught a permission-denied stale `target/release/bundle/macos/Capsem.app`; the recipe now removes that bundle before Tauri rebuild with a sudo fallback.
- [x] Update changelog
- [x] Verification gate -- `cargo fmt --check`, `uv run python -m pytest tests/test_release_workflow_policy.py -q`, `cargo test -p capsem uninstall_does_not_refresh_update_cache -- --nocapture`, and `just --list | rg "install|test-install"` passed.
- [ ] Commit

## Notes

- Discovery: current `just install` builds release binaries and package assets, but the post-install gate only checks the service socket and does not prove guest DNS/HTTPS.
- Discovery: current `just install` opens the macOS installer instead of using the `install.sh` command path (`sudo installer -pkg ... -target /`).
- Direction change: `just install` must start by force-uninstalling the existing local install, preserve settings outside the clean tree, verify the clean state, then install and verify.
- Changed approach: local settings are backed up to a temporary directory before uninstall and restored after package install. `setup-state.json` and runtime/update state are intentionally not preserved, so a clean install cannot be masked by stale setup state.
- Live run: first `just install` failed correctly because `capsem uninstall --yes` recreated `~/.capsem/update-check.json`; fixed by skipping background update refresh for uninstall.
- Live run: second `just install` reached Tauri bundling and failed on a stale, non-removable `target/release/bundle/macos/Capsem.app`; fixed by clearing the stale bundle before rebuilding.
- Live run: third `just install` passed the hard clean gate and reached the stale-bundle sudo fallback, then stopped because this Codex session cannot provide the macOS password.
- Local aftermath: `~/.capsem` and `~/Library/LaunchAgents/com.capsem.service.plist` are currently absent because the live run stopped after the clean phase and before package installation.

## Coverage Ledger

- Unit/contract: `tests/test_release_workflow_policy.py` covers clean-before-install, native installer command parity with `install.sh`, stale Tauri bundle cleanup, installed binary/version checks, and guest network gate strings. `crates/capsem/src/main.rs` unit coverage verifies uninstall does not refresh the update cache.
- Functional: `just --list | rg "install|test-install"` proves the justfile parses after the recipe rewrite.
- Adversarial: tests require service-only health to be insufficient and require DNS resolution plus HTTPS from inside the installed VM.
- E2E/VM or integration: live `just install` passed the hard-clean phase, then blocked at the macOS password prompt before native package installation and VM DNS/HTTPS verification.
- Telemetry/observability: recipe emits named phases for clean uninstall, installed layout, service, gateway, and guest DNS/HTTPS checks.
- Performance: not applicable
- Missing/deferred: live native package installation plus VM DNS/HTTPS gate still need a terminal run where the user can enter the macOS sudo password.
