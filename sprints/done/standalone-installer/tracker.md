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
- [x] `scripts/build-pkg.sh` -- assemble .pkg from Tauri .app + companions (no VM assets -- downloaded on first use)
- [x] `scripts/pkg-scripts/postinstall` -- copy bins, codesign, register service, run setup
- [x] `scripts/pkg-distribution.xml` -- productbuild distribution descriptor with build timestamp
- [x] `release.yaml` -- build companion binaries, build .pkg, notarize, upload
- [x] `install.sh` -- updated to download .pkg and open macOS Installer GUI (was .dmg)
- [x] `postinstall` -- SUDO_USER detection for `sudo installer` CLI path
- [x] Verify: .pkg installs on macOS, service registered, `capsem status` works

## SS4: Linux .deb with Companion Binaries
- [x] `scripts/repack-deb.sh` -- inject companion binaries + postinst into Tauri .deb
- [x] `scripts/deb-postinst.sh` -- register systemd unit, run setup, pass XDG_RUNTIME_DIR
- [x] `release.yaml` -- build companions, repack .deb, validate
- [x] `repack-deb.sh` -- build timestamp in deb version for upgrade detection
- [x] `platform.rs` -- added LinuxDeb layout detection for /usr/bin/
- [x] Verify: .deb installs in Docker, postinst runs, binaries accessible

## SS5: Test Hardening
- [x] `RUSTFLAGS="-D warnings" cargo check --workspace` -- zero warnings (excluding capsem-ui)
- [x] `cargo test -p capsem` -- 135 tests pass (+ LinuxDeb layout test)
- [x] `cargo test --workspace` -- 1425 pass, 1 pre-existing env-dependent failure (validate_github_token_real)
- [x] Install flow on macOS -- 6 binaries installed, codesigned, service registered+running, health check passes
- [x] `just test-install` -- Docker e2e: builds real .deb, dpkg -i, 25 passed, 33 skipped (live_system)
- [x] Test suite split: `@pytest.mark.live_system` for tests needing VM assets, auto-skipped in packaging tests
- [ ] Manual macOS: tray icon verification (deferred: requires UI)

## SS6: Install Pipeline Hardening (added during review)
- [x] `build-pkg.sh` + `repack-deb.sh` -- missing binaries are fatal errors (was WARNING)
- [x] `setup.rs` -- atomic save_state (temp file + rename, prevents corruption)
- [x] `just install` -- unified: builds .pkg (macOS) or .deb (Linux), installs native package
- [x] `just test-install` -- named Docker volumes for cargo cache, CI=true for pnpm
- [x] `Dockerfile.host-builder` -- added libxdo-dev for capsem-tray linking
- [x] `Dockerfile.install-test` -- handle UID 1000 conflict from base image
- [x] `hypervisor/mod.rs` -- allow dead code in KVM module (WIP, Linux-only)
- [x] `.gitignore` -- `packages/` for built .pkg/.deb artifacts

## Bugfixes (discovered during sprint)
- [x] `client.rs` -- check HTTP status code before deserializing response (fixes delete-nonexistent returning success)
- [x] `main.rs` -- guard auto-setup behind `auto_launch` flag (skip when --uds-path provided)
- [x] `postinstall` -- user detection for `sudo installer` (SUDO_USER -> console owner fallback)
- [x] `deb-postinst.sh` -- XDG_RUNTIME_DIR propagation for systemctl --user via su
- [x] `conftest.py` -- clean_state handles directories in run dir (instances/)

## SS7: Acceptance Gate
- [x] All implementation complete (SS0-SS6)
- [x] Tests pass (unit + integration + Docker e2e)
- [x] Install flow verified on macOS (.pkg)
- [x] Install flow verified on Linux (.deb in Docker)
- [x] CHANGELOG.md updated
- [x] Sprint tracker complete

## Housekeeping
- [x] Moved `sprints/native-installer` to `sprints/done/`
- [x] Moved `sprints/install-lifecycle` to `sprints/done/`
- [x] Created `sprints/standalone-installer/` with plan + tracker
- [x] Moved `sprints/standalone-installer` to `sprints/done/`
