# Sprint: Standalone Installer -- Tracker

## SS0: Binary Parity
- [x] `uninstall.rs` -- add capsem-gateway + capsem-tray to CAPSEM_BINARIES
- [x] `uninstall.rs` -- add pkill for capsem-gateway + capsem-tray
- [x] `conftest.py` -- update BINARIES to all 6
- [x] `conftest.py` -- update _kill_service() to kill gateway + tray
- [x] `conftest.py` -- fix docstring "all 4" -> "all 6"
- [x] Fix test_smoke.py, test_installed_layout.py, test_reinstall.py docstrings
- [x] Fix paths.rs comment to include gateway + tray
- [x] Verify: `cargo test -p capsem` passes (135 tests)

## SS1: Graceful Stop + Health Check + Auto-Setup in `just install`
- [x] `justfile` -- pre-install: stop existing service before binary overwrite
- [x] `justfile` -- post-install: health check (service responding on UDS)
- [x] `justfile` -- post-install: auto-setup if setup-state.json missing
- [x] Verify: install steps work end-to-end (6 binaries, codesign, service registered, health check passes)

## SS2: CLI Auto-Setup on First Use
- [x] `main.rs` -- auto-detect missing setup-state.json, run non-interactive setup
- [x] Skip auto-setup when --uds-path is explicit (tests, CI)
- [x] Verify: `cargo check -p capsem` passes
- [x] Verify: 82 CLI+service integration tests pass (0 failed)

## SS3: macOS .pkg Installer
- [x] `scripts/build-pkg.sh` -- assemble .pkg from Tauri .app + companions
- [x] `scripts/pkg-scripts/postinstall` -- copy bins, codesign, register service, run setup
- [x] `scripts/pkg-distribution.xml` -- productbuild distribution descriptor
- [x] `release.yaml` -- build companion binaries, build .pkg, notarize, upload
- [ ] Verify: install .pkg on clean macOS (requires cut-release)

## SS4: Linux .deb with Companion Binaries
- [x] `scripts/repack-deb.sh` -- inject companion binaries + postinst into Tauri .deb
- [x] `scripts/deb-postinst.sh` -- register systemd unit, run setup
- [x] `release.yaml` -- build companions, repack .deb, validate
- [ ] Verify: install repacked .deb in Docker (requires cut-release)

## SS5: Test Hardening
- [x] `RUSTFLAGS="-D warnings" cargo check --workspace` -- zero warnings (excluding capsem-ui)
- [x] `cargo test -p capsem` -- 135 tests pass
- [x] `cargo test --workspace` -- 1425 pass, 1 pre-existing env-dependent failure (validate_github_token_real)
- [x] Install flow on macOS -- 6 binaries installed, codesigned, service registered+running, health check passes, `capsem list` works
- [x] CLI+service integration tests -- 82 passed, 0 failed, 4 skipped
- [ ] `just test-install` -- Docker e2e (deferred: needs Docker/systemd environment)
- [ ] Manual macOS: tray icon verification (deferred: requires UI)

## Bugfixes (discovered during sprint)
- [x] `client.rs` -- check HTTP status code before deserializing response (fixes delete-nonexistent returning success)
- [x] `main.rs` -- guard auto-setup behind `auto_launch` flag (skip when --uds-path provided)

## SS6: Acceptance Gate
- [x] All implementation complete (SS0-SS4)
- [x] Tests pass (unit + integration)
- [x] Install flow verified on macOS
- [ ] CHANGELOG.md updated (in commit)

## Housekeeping
- [x] Moved `sprints/native-installer` to `sprints/done/`
- [x] Moved `sprints/install-lifecycle` to `sprints/done/`
- [x] Created `sprints/standalone-installer/` with plan + tracker
