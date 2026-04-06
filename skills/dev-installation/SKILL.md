---
name: dev-installation
description: Capsem native CLI installer -- setup wizard, service registration, self-update, background asset download, corp config provisioning, and the Docker-based install test harness. Use when working on capsem setup/update/uninstall commands, service install/uninstall, asset management, corp config, install test infrastructure, or the installed layout (~/.capsem/).
---

# Native CLI Installer

## Installed layout

```
~/.capsem/
  bin/capsem, capsem-service, capsem-process, capsem-mcp
  assets/manifest.json, v{ver}/
  run/service.sock, service.pid, instances/, persistent/
  setup-state.json
  update-check.json
  user.toml
  corp.toml               (CLI-provisioned corp config)
  corp-source.json         (corp config source metadata)
```

## CLI commands (no service required)

These commands dispatch before UdsClient creation -- they work without the service running:

| Command | Module | What |
|---------|--------|------|
| `capsem version` | main.rs | Print version + build hash |
| `capsem setup` | setup.rs | First-time setup wizard |
| `capsem update` | update.rs | Self-update from GitHub |
| `capsem service install\|uninstall\|status` | service_install.rs | Service registration |
| `capsem completions bash\|zsh\|fish` | completions.rs | Shell completions |
| `capsem uninstall --yes` | uninstall.rs | Full removal |

## Path discovery (paths.rs)

`discover_paths()` finds sibling binaries and assets:

1. `current_exe().parent()` -> bin_dir -> capsem-service, capsem-process
2. Assets: `~/.capsem/assets/` (the only layout -- no dev fallback, use `just install` or symlink)

## Auto-launch (main.rs UdsClient)

`try_ensure_service()` runs on every service-dependent command:

1. Check socket connectivity
2. Try systemd/LaunchAgent if unit installed (via `try_start_via_service_manager()`)
3. Fall back to direct spawn with `--foreground --assets-dir --process-binary`
4. Poll socket for 5s

The `request()` method wraps all HTTP calls with retry-on-connect-fail.

## Service registration (service_install.rs)

Pure generators (unit-testable on all platforms):
- `generate_plist()` -> macOS LaunchAgent XML
- `generate_systemd_unit()` -> systemd user unit

Side-effecting:
- `install_service()` -> write + `launchctl bootstrap` / `systemctl --user enable --now`
- `uninstall_service()` -> `launchctl bootout` / `systemctl --user disable --now` + delete
- `service_status()` -> installed + running + pid + unit_path

## Setup wizard (setup.rs)

6 steps, corp-aware, state persisted to `setup-state.json`:

0. Corp config provisioning (if `--corp-config`)
1. Welcome
2. (Doctor -- deferred)
3. Security preset (skips corp-locked)
4. AI providers (auto-detect credentials)
5. Repositories (detect git/SSH/GitHub)
6. Summary + PATH check + service install

Flags: `--non-interactive`, `--preset`, `--force`, `--accept-detected`, `--corp-config`

## Self-update (update.rs)

- `read_cached_update_notice()` -> sync file read on every command
- `refresh_update_cache_if_stale()` -> background 24h-cached GitHub check
- `run_update()` -> fetch manifest, download assets, cleanup old versions
- Layout detection: MacosPkg, UserDir, Development (bails with "build from source")

## Corp config provisioning (capsem-core: corp_provision.rs)

- `fetch_corp_config(url)` -> GET + validate + return content + ETag
- `validate_corp_toml(content)` -> parse as SettingsFile
- `install_corp_config(dir, content, source)` -> write corp.toml + corp-source.json
- `refresh_corp_config_if_stale(dir)` -> background conditional GET with ETag

Loader changes: `corp_config_paths()` returns [/etc, ~/.capsem/] with first-wins merge.

## Remote manifest + background download (capsem-core: asset_manager.rs)

- `fetch_remote_manifest(client, version)` -> GET release manifest.json
- `fetch_latest_manifest(client)` -> GitHub API latest release -> manifest
- `start_background_download(manifest, version, dir, arch)` -> tokio task + mpsc progress

## Test harness

Docker-based e2e tests in `tests/capsem-install/`:

| File | Tests |
|------|-------|
| test_smoke.py | Harness works (systemd, binaries, build hash) |
| test_auto_launch.py | Auto-launch, path discovery, asset resolution, error cases |
| test_service_install.py | Install/uninstall/status, idempotent, systemd integration |
| test_setup_wizard.py | Non-interactive, rerun skip, --force, user.toml |
| test_corp_config.py | Provisioning, validation, precedence |
| test_update.py | Dev build bail, layout detection, cache, preserve-on-fail |
| test_completions.py | bash/zsh/fish output |
| test_uninstall.py | Full cleanup |
| test_lifecycle.py | End-to-end user journey |
| test_reinstall.py | Binary replacement verification |
| test_error_paths.py | Failure scenarios with actionable errors |

Run: `just test-install` (Docker with systemd PID 1)

## Key files

```
crates/capsem/src/
  main.rs              CLI entry, command dispatch, UdsClient with auto-launch
  paths.rs             Binary + asset path discovery
  platform.rs          Install layout detection
  setup.rs             Setup wizard orchestrator
  update.rs            Self-update + cache
  service_install.rs   LaunchAgent + systemd unit generation + registration
  completions.rs       Shell completions via clap_complete
  uninstall.rs         Full removal
  build.rs             Build hash embedding (CAPSEM_BUILD_HASH)

crates/capsem-core/src/
  asset_manager.rs     Remote manifest, background download, cleanup
  net/policy_config/
    corp_provision.rs  Corp config fetch, validate, install, refresh
    loader.rs          corp_config_paths() with merge
```
