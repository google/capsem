# Sprint: Native CLI Installer

## Tasks

### Phase 0: E2E Install Test Harness
- [x] `crates/capsem/build.rs` -- embed CAPSEM_BUILD_HASH (git short SHA + build timestamp) for binary identity
- [x] Update `Commands::Version` in main.rs to print `capsem {version} (build {hash})`
- [x] `scripts/simulate-install.sh` -- single source of truth for install layout (bins + assets -> ~/.capsem/)
- [x] `just install` recipe -- _build-host + simulate-install.sh + codesign on macOS (replaces old gate recipe)
- [x] `docker/Dockerfile.install-test` -- extends capsem-host-builder, systemd as PID 1, dbus, non-root capsem user, XDG_RUNTIME_DIR
- [x] `tests/capsem-install/conftest.py` -- fixtures: installed_layout (calls simulate-install.sh), clean_state, run_capsem(), get_build_hash(), systemd_available
- [x] `test_systemd_works` smoke test -- verify `systemctl --user status` works in container before anything else
- [x] `just test-install` recipe -- Docker with --privileged --cgroupns=host, systemd PID 1, run pytest
- [x] Add `test-install` job to `ci.yaml` (runs on every PR)
- [x] Add `test-install` as gate in `release.yaml`
- [x] Add `# TODO(WB7): update for native installer` to existing `tests/test_install_sh.py`
- [x] Verify harness runs green with smoke test only
- [ ] Commit: `feat: Docker-based e2e install test harness with just install`

### WB1: CLI Auto-Launch + Asset Path Fix
- [ ] `crates/capsem/src/paths.rs` -- discover_paths() with installed-first, dev fallback
- [ ] `crates/capsem/src/main.rs` -- try_ensure_service() (systemd/launchctl if unit exists, else direct spawn)
- [ ] `crates/capsem/src/main.rs` -- consolidate post()/get()/delete() into request() with retry-on-connect-fail
- [ ] `crates/capsem/src/main.rs` -- route Version before UdsClient creation
- [ ] `crates/capsem-mcp/src/main.rs:96-100` -- installed-first asset path fallback
- [ ] `tests/capsem-install/test_auto_launch.py` -- 5 tests (3 happy + 2 error: bad binary, missing assets)
- [ ] Commit: `feat: CLI auto-launches service on first command`

### WB3: Service Installation Commands
- [ ] `crates/capsem/src/service_install.rs` -- generate_plist(), generate_systemd_unit() (pure)
- [ ] Rust unit tests for pure generators -- 6 tests (plist XML valid, absolute paths, systemd restart policy, etc.)
- [ ] `crates/capsem/src/service_install.rs` -- is_service_installed(), install_service(), uninstall_service(), service_status() (side-effecting)
- [ ] `crates/capsem/src/main.rs` -- Service(ServiceCommands) with Install/Uninstall/Status, dispatch before UdsClient
- [ ] Update try_ensure_service() to prefer systemd/launchctl when installed
- [ ] `tests/capsem-install/test_service_install.py` -- 6 tests (4 happy + idempotent + uninstall-when-not-installed)
- [ ] Commit: `feat: capsem service install/uninstall/status`

### WB5: Remote Manifest + Background Download
- [ ] `crates/capsem-core/src/asset_manager.rs` -- fetch_remote_manifest()
- [ ] `crates/capsem-core/src/asset_manager.rs` -- fetch_latest_manifest()
- [ ] `crates/capsem-core/src/asset_manager.rs` -- start_background_download() with BackgroundProgress channel
- [ ] Commit: `feat: remote manifest fetch and background asset download`

### WB2a: Corp Config Provisioning
- [ ] `crates/capsem-core/src/net/policy_config/corp_provision.rs` -- CorpSource struct, fetch_corp_config(), validate_corp_toml()
- [ ] `crates/capsem-core/src/net/policy_config/corp_provision.rs` -- install_corp_config(), read_corp_source(), refresh_corp_config_if_stale()
- [ ] `crates/capsem-core/src/net/policy_config/loader.rs` -- corp_config_paths() with ~/.capsem/corp.toml fallback, merge logic
- [ ] `crates/capsem-core/src/net/policy_config/mod.rs` -- pub mod corp_provision
- [ ] Unit tests: 8 validation tests (pure, no I/O)
- [ ] `tests/capsem-install/test_corp_config.py` -- provisioning (4), precedence (2), preset interactions (3), provider gating (6), e2e wizard+corp (5), state file (2)
- [ ] Commit: `feat: corp config provisioning from URL or file path`

### WB2: Setup Wizard
- [ ] Add `inquire = "0.7"` to `crates/capsem/Cargo.toml`
- [ ] `crates/capsem/src/setup.rs` -- SetupState, orchestrator, step functions
- [ ] Step 0: Corp config provisioning (if --corp-config)
- [ ] Step 1: Welcome + background asset download
- [ ] Step 2: Doctor diagnostics (optional)
- [ ] Step 3: Security preset (corp-aware, skip if locked)
- [ ] Step 4: AI Providers (corp-aware, skip locked, pre-fill keys)
- [ ] Step 5: Repositories (corp-aware)
- [ ] Step 6: Summary + await download (10min timeout) + PATH check + install service + VM boot verify
- [ ] `crates/capsem/src/main.rs` -- Setup command with --non-interactive, --preset, --force, --accept-detected, --corp-config
- [ ] Non-interactive mode
- [ ] Re-run logic: skip completed unless --force
- [ ] `tests/capsem-install/test_setup_wizard.py` -- 4 tests
- [ ] Commit: `feat: capsem setup interactive wizard`

### WB4: Self-Update
- [ ] Add `self-replace`, `semver`, `tempfile`, `clap_complete` to `crates/capsem/Cargo.toml`
- [ ] `crates/capsem/src/platform.rs` -- InstallLayout enum + detect_install_layout()
- [ ] `crates/capsem/src/update.rs` -- read_cached_update_notice(), refresh_update_cache_if_stale()
- [ ] `crates/capsem/src/update.rs` -- run_update() with atomic download-all-then-swap sequence
- [ ] `crates/capsem/src/main.rs` -- Update { yes } command, background cache refresh after dispatch
- [ ] Background corp config refresh (tokio::spawn after dispatch)
- [ ] `tests/capsem-install/test_update.py` -- 4 tests (3 happy + partial-failure-preserves-old)
- [ ] Commit: `feat: capsem update with asset vacuum`

### Polish: Completions + Uninstall
- [ ] `crates/capsem/src/completions.rs` -- generate_completions(shell) via clap_complete
- [ ] `crates/capsem/src/uninstall.rs` -- run_uninstall(yes): stop, remove unit, remove binaries, remove ~/.capsem/
- [ ] `crates/capsem/src/main.rs` -- Completions { shell } and Uninstall { yes } commands
- [ ] `tests/capsem-install/test_completions.py` -- bash/zsh/fish validation
- [ ] `tests/capsem-install/test_uninstall.py` -- full cleanup test
- [ ] Commit: `feat: shell completions and capsem uninstall`

### Test Hardening: Lifecycle + Error Paths + Reinstall
- [ ] `tests/capsem-install/test_lifecycle.py` -- full user journey: install -> setup -> list -> service status -> update -> uninstall
- [ ] `tests/capsem-install/test_reinstall.py` -- compile v1, install, recompile v2, install, verify v2 is installed via build hash + file hash
- [ ] `tests/capsem-install/test_reinstall.py` -- all 4 binaries replaced on reinstall, not just capsem
- [ ] `tests/capsem-install/test_error_paths.py` -- 8 failure scenario tests (bad binary, missing manifest, corrupt state, wrong perms, stale socket, etc.)
- [ ] Verify test_lifecycle.py passes end-to-end in Docker
- [ ] Verify test_reinstall.py proves install is not silently a no-op
- [ ] Verify all error path tests produce actionable error messages (not stack traces)

### Skills & Documentation
- [ ] `skills/dev-installation/SKILL.md` -- new skill (14 sections)
- [ ] Update `skills/dev-testing/SKILL.md` -- add install test tier + capsem-install suite
- [ ] Update `skills/dev-capsem/SKILL.md` -- add /dev-installation to skill map
- [ ] Update `CLAUDE.md` -- add /dev-installation to skills table
- [ ] Commit: `docs: dev-installation skill and developer docs updates`

### Testing Gate
- [ ] `just test` passes (unit + cross-compile + frontend)
- [ ] `just test-install` passes (Docker e2e: lifecycle + error paths + all WBs)
- [ ] `just install` works on macOS (local testing)
- [ ] Manual macOS: auto-launch, service install/uninstall, setup wizard, LaunchAgent
- [ ] CI: test-install job passes in ci.yaml
- [ ] Changelog updated

## Notes
- WB6 (CI release pipeline) and WB7 (install.sh) are deferred
- `just install` depends on `_build-host` so it always recompiles before installing
- `build.rs` embeds CAPSEM_BUILD_HASH (git SHA + timestamp) so every build is uniquely identifiable via `capsem version`
- `scripts/simulate-install.sh` is the bridge to WB7 -- when real install.sh lands, swap it in conftest fixture
- `host_config::detect()` already in capsem-core, no porting needed
- `apply_preset()` already writes to user.toml, no porting needed
- `cleanup_old_versions()` integrates with ImageRegistry, already tested
- MCP try_ensure_service() at capsem-mcp/src/main.rs:80 is the reference pattern
- Service resolve_assets_dir() at capsem-service/src/main.rs:456 already works for installed layout
- systemd-in-Docker requires --privileged --cgroupns=host + systemd as PID 1
- LaunchAgent tests are manual-only (can't run launchctl in Docker)
- generate_plist() and generate_systemd_unit() are pure functions -- unit tested on all platforms
- Update uses atomic download-all-then-swap to avoid partial-update bricking
- test_install_sh.py needs update when WB7 lands (marked with TODO)
