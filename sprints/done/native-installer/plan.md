# Sprint: Native CLI Installer

## What

Standalone CLI distribution for capsem: setup wizard, service registration, self-update, background asset download, and corp config provisioning -- all without the GUI. WB6 (CI release pipeline) and WB7 (install.sh) are deferred.

## Why

- Daemon architecture is functionally complete but only distributable via Tauri bundles
- CLI + service + MCP are the primary interface
- External testers and enterprise users need a standalone install path
- Enterprise users need corp config provisioning + non-interactive setup for MDM/CI

## Installed Layout

```
~/.capsem/
  bin/capsem, capsem-service, capsem-process, capsem-mcp
  assets/manifest.json, v{ver}/
  run/service.sock
  setup-state.json
  update-check.json
  user.toml
  corp.toml               (CLI-provisioned corp config)
  corp-source.json         (corp config source metadata)
```

## Implementation Order

```
Phase 0: E2E Harness
  |
  v
WB1: Auto-launch + paths
  |
  v
WB3: Service install/uninstall --> WB5: Remote manifest + download
  |                                      |
  v                                      v
WB2a: Corp config provisioning --> WB2: Setup wizard
                                         |
                                         v
                                   WB4: Self-update
                                         |
                                         v
                                   Polish: Completions + uninstall
                                         |
                                         v
                                   Skills & docs
```

Each WB adds Docker e2e test cases incrementally.

## Key Decisions

- **`inquire`** for interactive prompts, `clap` for non-interactive flags (already a dep)
- **`self-replace`** for atomic binary replacement during self-update on Linux
- **Background asset download** -- wizard proceeds while VM images download
- **Setup state persistence** -- `~/.capsem/setup-state.json` with schema version for future questions
- **Service registration** -- LaunchAgent (macOS) / systemd user unit (Linux), auto-start on login
- **Update check cache** -- `~/.capsem/update-check.json`, 24h TTL, one-liner notice
- **Asset vacuum on update** -- `cleanup_old_versions()` respects pinned.json + ImageRegistry
- **Corp config** -- URL or local file provisioning, two-tier merge (/etc wins), background refresh with ETag

## Existing Code to Reuse (all in capsem-core)

| What | File | Function |
|------|------|----------|
| Credential detection | `host_config.rs` | `detect()` -- git, SSH, API keys, OAuth, ADC |
| Key validation | `host_config.rs:217` | `validate_api_key()` -- Anthropic, Google, OpenAI, GitHub |
| Security presets | `net/policy_config/presets.rs` | `apply_preset()` -- writes medium/high to user.toml |
| Settings write | `net/policy_config/loader.rs:113` | `write_user_settings()` |
| MCP config | `net/policy_config/loader.rs:141` | `save_mcp_user_config()` |
| Asset download | `asset_manager.rs` | `AssetManager` -- resume, Blake3, progress callbacks |
| Asset vacuum | `asset_manager.rs:639` | `cleanup_old_versions()` -- respects pinned + ImageRegistry |
| Image registry | `image.rs` | Protects base versions used by forked images |
| Service auto-launch | `capsem-mcp/src/main.rs:79` | `try_ensure_service()` pattern |
| Corp config loader | `net/policy_config/loader.rs` | `load_settings_files()`, `corp_config_path()` |
| Corp lock check | `net/policy_config/resolver.rs:6` | Corp settings override user settings per-key |
| Preset skip logic | `net/policy_config/presets.rs:72` | `apply_preset()` skips corp-locked keys |

---

## Phase 0: E2E Install Test Harness

**Goal:** Docker-based e2e test infrastructure exercising the installed layout on Linux. Tests must exercise the *actual install flow*, not just binaries dropped into a directory.

### Files to create

| File | Purpose |
|------|---------|
| `docker/Dockerfile.install-test` | Extends capsem-host-builder, adds systemd + non-root user |
| `scripts/simulate-install.sh` | Reproduces what future install.sh will do: extract, place, link |
| `tests/capsem-install/conftest.py` | Shared fixtures using simulate-install.sh |
| justfile recipe `install` | Build + install to `~/.capsem/` for local testing (replaces old gate) |
| justfile recipe `test-install` | Build Linux binaries, install via simulate-install.sh, run pytest in Docker |

### Build hash for binary identity

Currently `capsem version` prints only `CARGO_PKG_VERSION` (e.g. `capsem 0.13.0`), which is the same across every build from the same tree. We need a unique per-build fingerprint so tests can verify "I installed build A, then installed build B, and the installed binary is actually B."

Add `crates/capsem/build.rs`:

```rust
fn main() {
    // Embed a unique build hash: git short SHA + build timestamp.
    // Changes on every recompile, even from the same commit.
    let git_hash = std::process::Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_else(|_| "unknown".into());
    let build_ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    println!("cargo:rustc-env=CAPSEM_BUILD_HASH={git_hash}.{build_ts}");
    // Rebuild when git HEAD changes or any source changes
    println!("cargo:rerun-if-changed=../../.git/HEAD");
}
```

Update version output in `main.rs`:

```rust
Commands::Version => {
    println!("capsem {} (build {})", env!("CARGO_PKG_VERSION"), env!("CAPSEM_BUILD_HASH"));
    // ...
}
```

Now `capsem version` prints e.g. `capsem 0.13.0 (build c37b920.1743984000)` -- unique per compilation. Tests can compare this between installs.

### just install (local install for macOS dev testing)

Replaces the old `install: doctor test` gate recipe. Depends on `_build-host` so it **always recompiles** before installing.

```
install: _build-host
    #!/bin/bash
    set -euo pipefail
    INSTALL_DIR="$HOME/.capsem/bin"
    ASSETS_SRC="assets"
    ASSETS_DST="$HOME/.capsem/assets"
    mkdir -p "$INSTALL_DIR" "$ASSETS_DST" "$HOME/.capsem/run"
    # Copy binaries
    for bin in capsem capsem-service capsem-process capsem-mcp; do
        cp "target/debug/$bin" "$INSTALL_DIR/$bin"
    done
    # Sign on macOS
    if [[ "$(uname -s)" == "Darwin" ]]; then
        for bin in "$INSTALL_DIR"/capsem*; do
            codesign --sign - --entitlements entitlements.plist --force "$bin"
        done
    fi
    # Copy assets (manifest + arch dir)
    if [[ -f "$ASSETS_SRC/manifest.json" ]]; then
        cp "$ASSETS_SRC/manifest.json" "$ASSETS_DST/"
    fi
    ARCH=$(uname -m); [[ "$ARCH" == "aarch64" ]] && ARCH="arm64"
    if [[ -d "$ASSETS_SRC/$ARCH" ]]; then
        # Versioned layout: assets/v{version}/
        VERSION=$(grep '^version' Cargo.toml | head -1 | sed 's/.*"\(.*\)"/\1/')
        mkdir -p "$ASSETS_DST/v$VERSION"
        cp -r "$ASSETS_SRC/$ARCH"/* "$ASSETS_DST/v$VERSION/"
    fi
    echo "Installed to ~/.capsem/bin/ ($(ls "$INSTALL_DIR" | wc -l | tr -d ' ') binaries)"
    echo "Assets at ~/.capsem/assets/"
    # Print build hash for verification
    "$INSTALL_DIR/capsem" version 2>/dev/null | head -1 || true
    # PATH check
    if [[ ":$PATH:" != *":$INSTALL_DIR:"* ]]; then
        echo ""
        echo "WARNING: $INSTALL_DIR is not in your PATH"
        echo "  Add to your shell profile: export PATH=\"\$HOME/.capsem/bin:\$PATH\""
    fi
```

This lets a developer do `just install && capsem setup` on macOS to test the full installed flow. The `_build-host` dependency means `cargo build` runs first -- incremental compilation handles the fast path when nothing changed.

### scripts/simulate-install.sh

Simulates what the real install.sh (WB7, deferred) will do. This is the **single source of truth** for how binaries land in `~/.capsem/`. Both `just install` and Docker e2e harness call this script.

```bash
#!/bin/bash
# simulate-install.sh -- Reproduce the installed layout for testing.
# Usage: simulate-install.sh <bin_dir_src> <assets_dir_src>
# Installs to ~/.capsem/{bin,assets,run}
set -euo pipefail
BIN_SRC="${1:?usage: simulate-install.sh <bin_dir> <assets_dir>}"
ASSETS_SRC="${2:?}"
INSTALL_DIR="$HOME/.capsem/bin"
ASSETS_DST="$HOME/.capsem/assets"
# ... same logic as just install, but sourcing from args not target/debug
```

When WB7 lands, the real install.sh replaces this and the test harness swaps the fixture -- same tests, real script.

### docker/Dockerfile.install-test

Extends `capsem-host-builder:latest`. Adds:
- `systemd-container`, `dbus-user-session`, `dbus` for systemd user session
- Non-root `capsem` user with sudo
- `loginctl enable-linger capsem` (requires dbus running during build)
- Pre-created `~/.capsem/{bin,assets,run}` dirs owned by capsem user
- `ENV XDG_RUNTIME_DIR=/run/user/1000` (required for `systemctl --user`)

**systemd-in-Docker specifics:** The container must boot systemd as PID 1. The `just test-install` recipe runs it with:

```bash
docker run --privileged --cgroupns=host \
  -v /sys/fs/cgroup:/sys/fs/cgroup:rw \
  --tmpfs /run --tmpfs /tmp \
  -v "$PWD":/src:ro \
  capsem-install-test /usr/lib/systemd/systemd
```

Tests exec into the container as the `capsem` user via `docker exec -u capsem`. Phase 0 includes a **systemd smoke test** (`test_systemd_works`) that verifies `systemctl --user status` succeeds before any real tests run -- if this fails, the entire suite skips with a clear error instead of cascading confusing failures.

### tests/capsem-install/conftest.py

Key fixtures and helpers:
- `INSTALL_DIR = ~/.capsem/bin`, `ASSETS_DIR = ~/.capsem/assets`, `RUN_DIR = ~/.capsem/run`
- `installed_layout` (session-scoped): runs `simulate-install.sh`, asserts all 4 binaries + manifest.json exist. **This is the fixture that all install tests depend on** -- it exercises the real install flow.
- `clean_state`: kills any running service, clears RUN_DIR, yields, kills again
- `run_capsem(*args)` helper: runs `~/.capsem/bin/capsem` with capture + timeout (default 30s)
- `get_build_hash()` helper: runs `capsem version`, parses the build hash from `(build ...)` output
- `systemd_available`: fixture that checks `systemctl --user status` works, used to skip/xfail tests that need systemd

### just test-install

Docker run with:
- Boots systemd as PID 1 (see Dockerfile specifics above)
- Mounts source as `:ro`, reuses existing cargo cache volumes
- Inside container: `cargo build -p capsem -p capsem-service -p capsem-process -p capsem-mcp --target x86_64-unknown-linux-gnu` (or detected target)
- Runs `scripts/simulate-install.sh target/debug/ assets/` as capsem user
- Runs `uv run pytest tests/capsem-install/ -v --tb=short`
- Passes `/dev/kvm` if available (for VM boot tests, otherwise those tests xfail)

### CI integration

Add `test-install` job to `ci.yaml` (runs on every PR):

```yaml
test-install:
  runs-on: ubuntu-24.04-arm
  steps:
    - uses: actions/checkout@v5
    - uses: extractions/setup-just@v3
    - name: Build Docker image
      run: just build-host-image
    - name: Run install tests
      run: just test-install
```

Add as a gate in `release.yaml` (must pass before release).

**Commit:** `feat: Docker-based e2e install test harness with just install`

---

## WB1: CLI Auto-Launch + Asset Path Fix

**Goal:** `capsem list` (or any command) works without manually starting the service.

### Files

| File | Change |
|------|--------|
| `crates/capsem/src/paths.rs` | New. `discover_paths()` -> (service_bin, process_bin, assets_dir). Algorithm: `current_exe().parent()` for sibling bins; assets from `~/.capsem/assets/manifest.json` (installed) then `bin_dir/../../assets/{arch}` (dev) |
| `crates/capsem/src/main.rs` | 1. Add `mod paths;`. 2. Add `try_ensure_service()` to UdsClient -- check socket, try systemd/launchctl if unit exists, else direct spawn, poll 5s. 3. Consolidate `post()/get()/delete()` into `request()` with retry-on-connect-fail calling `try_ensure_service()`. 4. Route commands that don't need service (Version) before creating UdsClient. |
| `crates/capsem-mcp/src/main.rs:96-100` | Replace hardcoded dev-layout path with installed-first fallback: check `~/.capsem/assets/manifest.json`, fall back to `bin_dir/../../assets/{arch}` |

### Key details

- The `try_ensure_service()` in MCP (`main.rs:80`) already has the pattern. The CLI version will also check for installed systemd/LaunchAgent units and prefer `systemctl --user start` / `launchctl kickstart` when present.
- Service `resolve_assets_dir()` at `capsem-service/src/main.rs:456`: already works for installed layout (resolves `v{version}/` under assets_dir). No change needed. Default `assets_base_dir` at line 1242 resolves to `~/.capsem/assets` when `run_dir` is `~/.capsem/run`.

### E2E tests: `tests/capsem-install/test_auto_launch.py`

- `test_auto_launch_from_installed_layout`: `capsem list` auto-starts service
- `test_path_discovery_installed_layout`: binaries discover siblings
- `test_asset_resolution_installed_layout`: service finds `~/.capsem/assets/`
- `test_auto_launch_bad_service_binary`: replace service binary with `exit 1` stub, verify CLI gives clear error within timeout (not hang forever)
- `test_auto_launch_missing_assets`: assets dir empty, verify service starts but reports missing assets in error (not silent failure)

**Commit:** `feat: CLI auto-launches service on first command`

---

## WB3: Service Installation Commands

**Goal:** `capsem service install/uninstall/status` for LaunchAgent (macOS) and systemd (Linux).

### Files

| File | Change |
|------|--------|
| `crates/capsem/src/service_install.rs` | New. Pure functions: `generate_plist()`, `generate_systemd_unit()`. Side-effecting: `is_service_installed()`, `install_service()`, `uninstall_service()`, `service_status()` |
| `crates/capsem/src/main.rs` | Add `mod service_install;`, `Service(ServiceCommands)` variant with Install/Uninstall/Status subcommands. Dispatch before UdsClient (these don't need the service running). |

### generate_plist() key details

- Uses `discover_paths()` for all binary paths (absolute, no `~`)
- `KeepAlive=true`, `RunAtLoad=true`
- `ProgramArguments`: `[capsem-service, --foreground, --assets-dir, {home}/.capsem/assets, --process-binary, {bin_dir}/capsem-process]`
- `StandardOutPath`/`StandardErrorPath` -> `{home}/Library/Logs/capsem/service.log`
- Target: `~/Library/LaunchAgents/com.capsem.service.plist`

### generate_systemd_unit() key details

- `[Unit]` Description=Capsem sandbox service
- `[Service]` ExecStart={service_bin} --foreground --assets-dir {home}/.capsem/assets --process-binary {process_bin}
- Restart=always, RestartSec=2
- `[Install]` WantedBy=default.target
- Target: `~/.config/systemd/user/capsem.service`

### install_service()

- macOS: write plist, `launchctl bootstrap gui/{uid}` (fallback `launchctl load`)
- Linux: write unit, `systemctl --user daemon-reload && enable --now capsem`

### uninstall_service()

- macOS: `launchctl bootout gui/{uid}/com.capsem.service` (fallback `unload`), delete plist
- Linux: `systemctl --user disable --now capsem`, daemon-reload, delete unit

### service_status() returns

`{ installed: bool, running: bool, pid: Option<u32>, uptime: Option<String> }`

### WB1 auto-launch integration

Update `try_ensure_service()` in CLI: if `is_service_installed()`, use `launchctl kickstart` / `systemctl --user start` instead of direct spawn.

### Rust unit tests: `crates/capsem/src/service_install.rs` #[cfg(test)]

Pure-function tests that run on all platforms (macOS CI + Linux CI):
- `test_generate_plist_absolute_paths`: all paths in plist are absolute, no `~`
- `test_generate_plist_valid_xml`: output parses as valid XML plist
- `test_generate_plist_has_keep_alive`: KeepAlive and RunAtLoad set
- `test_generate_systemd_unit_absolute_paths`: ExecStart uses absolute binary path
- `test_generate_systemd_unit_restart_policy`: Restart=always, RestartSec=2
- `test_generate_systemd_unit_wanted_by`: WantedBy=default.target

### E2E tests: `tests/capsem-install/test_service_install.py`

- `test_service_install_creates_systemd_unit`: verify unit file content, absolute paths
- `test_service_status_after_install`: installed + running
- `test_service_uninstall_removes_unit`: clean removal
- `test_auto_launch_uses_systemd_when_installed`: after install, auto-launch restarts via systemd
- `test_service_install_idempotent`: running `capsem service install` twice succeeds without error or duplicate units
- `test_service_uninstall_when_not_installed`: running uninstall with no unit gives clean "not installed" message

**Commit:** `feat: capsem service install/uninstall/status`

---

## WB5: Remote Manifest + Background Download

**Goal:** Fetch VM asset manifests from GitHub releases, download assets in background.

### Files

| File | Change |
|------|--------|
| `crates/capsem-core/src/asset_manager.rs` | Add `fetch_remote_manifest()`, `fetch_latest_manifest()`, `start_background_download()` |

### New functions in asset_manager.rs

Reuses existing types (`Manifest`, `AssetManager`, `DownloadProgress`, `AssetStatus`):

```rust
pub async fn fetch_remote_manifest(client: &reqwest::Client, version: &str) -> Result<Manifest>
// GET {release_url(version)}/manifest.json -> Manifest::from_json_for_arch()

pub async fn fetch_latest_manifest(client: &reqwest::Client) -> Result<(String, Manifest)>
// GET github.com/repos/google/capsem/releases/latest -> tag_name -> fetch_remote_manifest

pub fn start_background_download(
    manifest: Manifest, version: String, assets_base_dir: PathBuf, arch: Option<String>,
) -> (JoinHandle<Result<BackgroundDownloadResult>>, mpsc::Receiver<BackgroundProgress>)
// Spawns tokio task: AssetManager::from_manifest() -> check_all() -> download NeedsDownload assets
```

`BackgroundProgress` enum: `Starting`, `Progress(DownloadProgress)`, `AssetComplete`, `AllComplete`, `Error(String)`.

**Commit:** `feat: remote manifest fetch and background asset download`

---

## WB2a: Corp Config Provisioning

**Goal:** Support corp settings from a remote URL or local file path. Periodic background refresh. The setup wizard reads corp config first and gates options accordingly.

### Context: Existing corp config system

Corp config today (`loader.rs:20-25`):
- Static file at `/etc/capsem/corp.toml` (MDM-distributed, root-owned)
- Overridable via `CAPSEM_CORP_CONFIG` env var
- Corp settings override user settings per-key (`resolver.rs:6-7`)
- `apply_preset()` already skips corp-locked keys (`presets.rs:72`)

**Problem:** Enterprise users installing via CLI can't easily get corp config. MDM deployment of `/etc/capsem/corp.toml` requires root. We need a way to provision corp config from a URL or local file path during setup, and keep it fresh.

### Files

| File | Change |
|------|--------|
| `crates/capsem-core/src/net/policy_config/corp_provision.rs` | New. `fetch_corp_config(url)`, `validate_corp_toml(content)`, `install_corp_config(content, source_meta)`, `refresh_corp_config_if_stale(capsem_dir)`, `read_corp_source()` |
| `crates/capsem-core/src/net/policy_config/loader.rs` | Modify `corp_config_path()` to also check `~/.capsem/corp.toml` as fallback. Modify `load_settings_files()` to merge both if both exist (/etc wins per-key). |
| `crates/capsem-core/src/net/policy_config/mod.rs` | Add `pub mod corp_provision;` |

### corp_provision.rs details

Corp source metadata: `~/.capsem/corp-source.json`

```rust
struct CorpSource {
    url: Option<String>,       // None if provisioned from local file
    file_path: Option<String>, // None if provisioned from URL
    fetched_at: String,        // ISO timestamp
    etag: Option<String>,      // HTTP ETag for conditional refresh
    content_hash: String,      // blake3 of corp.toml content
    refresh_interval_hours: u32, // from corp.toml, cached here (default 24)
}
```

Functions:

```rust
/// Fetch corp config from URL, validate TOML, return content.
pub async fn fetch_corp_config(client: &reqwest::Client, url: &str) -> Result<String>

/// Validate that a string is valid corp TOML (parseable as SettingsFile).
pub fn validate_corp_toml(content: &str) -> Result<SettingsFile>

/// Install corp config: write to ~/.capsem/corp.toml + corp-source.json.
pub fn install_corp_config(capsem_dir: &Path, content: &str, source: &CorpSource) -> Result<()>

/// Read corp source metadata (returns None if no corp-source.json).
pub fn read_corp_source(capsem_dir: &Path) -> Option<CorpSource>

/// Background refresh: if corp was provisioned from URL and TTL expired, re-fetch.
/// Conditional GET with If-None-Match (ETag) to avoid unnecessary downloads.
pub async fn refresh_corp_config_if_stale(capsem_dir: PathBuf)
```

TTL defined in the corp config itself:

```toml
# corp.toml
refresh_interval_hours = 12  # check every 12h (default 24 if omitted)

[settings]
ai.anthropic.allow = true
ai.anthropic.api_key = "sk-ant-corp-..."
```

`refresh_corp_config_if_stale()` reads `refresh_interval_hours` from the currently-installed corp.toml to determine TTL.

### Loader changes (loader.rs)

Current `corp_config_path()` returns only `/etc/capsem/corp.toml`. Change to:

```rust
/// Corporate config paths, in priority order.
/// /etc/capsem/corp.toml (system-level, MDM) takes precedence.
/// ~/.capsem/corp.toml (user-level, CLI-provisioned) is fallback.
pub fn corp_config_paths() -> Vec<PathBuf> {
    let mut paths = vec![];
    if let Ok(path) = std::env::var("CAPSEM_CORP_CONFIG") {
        paths.push(PathBuf::from(path));
        return paths; // env override is exclusive
    }
    let system = PathBuf::from("/etc/capsem/corp.toml");
    if system.exists() { paths.push(system); }
    if let Some(home) = std::env::var("HOME").ok() {
        let user_corp = PathBuf::from(home).join(".capsem").join("corp.toml");
        if user_corp.exists() { paths.push(user_corp); }
    }
    paths
}
```

`load_settings_files()` merges: load each corp file, merge per-key (first path wins = /etc wins).

Backward compatibility: `corp_config_path()` still exists, returns the first path.

### Integration with setup wizard (WB2)

The setup command gains `--corp-config <URL_OR_PATH>`:
- If URL: fetch, validate, install to `~/.capsem/corp.toml`, save source metadata
- If file path: read, validate, install to `~/.capsem/corp.toml`
- All subsequent wizard steps read the merged corp config and gate options accordingly

### Integration with background refresh

In `main.rs`, after dispatching any command (same place as `refresh_update_cache_if_stale`):

```rust
tokio::spawn(refresh_corp_config_if_stale(capsem_dir.clone()));
```

Fire-and-forget background conditional GET if corp was provisioned from URL and TTL expired.

### Unit tests: `crates/capsem-core/src/net/policy_config/corp_provision.rs` #[cfg(test)]

Validation tests (pure functions, no I/O):
- `test_validate_valid_corp_toml`: valid TOML with settings section parses
- `test_validate_empty_corp_toml`: empty file parses (no settings, valid)
- `test_validate_invalid_toml_syntax`: garbage content rejected
- `test_validate_toml_with_unknown_keys`: unknown setting IDs accepted (forward-compatible)
- `test_validate_toml_wrong_types`: e.g. `ai.anthropic.allow = "yes"` instead of `true` rejected
- `test_refresh_interval_parsing`: `refresh_interval_hours` parsed, defaults to 24 if absent
- `test_refresh_interval_zero_means_no_refresh`: TTL of 0 disables periodic refresh
- `test_corp_source_roundtrip`: serialize + deserialize CorpSource JSON

### E2E tests: `tests/capsem-install/test_corp_config.py`

Provisioning tests:
- `test_corp_config_from_local_file`: `capsem setup --corp-config /path/to/corp.toml` installs to `~/.capsem/corp.toml`
- `test_corp_config_validates_toml`: invalid TOML rejected with clear error
- `test_corp_source_metadata_written`: corp-source.json written with correct source path
- `test_corp_config_overwrites_previous`: re-provisioning replaces existing corp.toml

Precedence tests:
- `test_system_corp_takes_precedence`: `/etc/capsem/corp.toml` overrides `~/.capsem/corp.toml` per-key
- `test_user_corp_used_when_no_system_corp`: `~/.capsem/corp.toml` used as fallback

Corp-locked settings -- preset interactions:
- `test_corp_locks_preset_choice`: corp sets `security.preset = "high"` -> `apply_preset("medium")` returns preset as skipped
- `test_corp_locked_settings_respected_in_preset`: corp locks individual settings -> preset skips those specific keys but applies the rest
- `test_corp_locks_all_settings`: corp locks every setting in a preset -> preset is effectively no-op

Corp-locked settings -- provider/repo gating:
- `test_corp_locks_anthropic_provider`: corp sets `ai.anthropic.allow = false` -> wizard skips Anthropic prompt
- `test_corp_prefills_api_key`: corp provides `ai.anthropic.api_key` -> wizard shows key as pre-configured
- `test_corp_locks_and_prefills_multiple_providers`: corp locks Anthropic + Google with keys -> wizard only prompts for unlocked providers
- `test_corp_disables_all_providers`: corp sets all `ai.*.allow = false` -> providers step skipped
- `test_corp_locks_github_token`: corp provides GitHub token -> repositories step shows as pre-configured
- `test_corp_partial_lock`: corp locks some settings, leaves others -> wizard prompts only for unlocked

E2E wizard with corp config variations:
- `test_setup_with_corp_medium_preset_locked`: corp forces medium preset + Anthropic key -> wizard skips security + Anthropic steps
- `test_setup_with_corp_all_locked`: corp provides everything -> every step shows "configured by your organization"
- `test_setup_non_interactive_with_corp`: `--non-interactive --corp-config /path` provisions then runs silently
- `test_setup_with_corp_invalid_api_key`: corp provides bad key -> wizard warns but proceeds (corp is authoritative)
- `test_setup_rerun_after_corp_change`: first run with corp A, second with more restrictive corp B -> corp-locked values noted

State file interactions:
- `test_setup_state_records_corp_source`: setup-state.json includes `corp_config_source` field
- `test_corp_change_forces_reeval`: changed corp config triggers step re-evaluation even without `--force`

**Commit:** `feat: corp config provisioning from URL or file path`

---

## WB2: Setup Wizard

**Goal:** `capsem setup` -- interactive wizard that respects corp policy. Corp config is loaded first (from /etc, ~/.capsem/, or freshly provisioned via `--corp-config`) and gates all options.

### Files

| File | Change |
|------|--------|
| `crates/capsem/Cargo.toml` | Add `inquire = "0.7"` |
| `crates/capsem/src/setup.rs` | New. Setup wizard orchestrator + step functions |
| `crates/capsem/src/main.rs` | Add `mod setup;`, Setup command variant with `--non-interactive`, `--preset`, `--force`, `--accept-detected`, `--corp-config` flags. Dispatch before UdsClient. |

### State persistence: `~/.capsem/setup-state.json`

```rust
struct SetupState {
    schema_version: u32,
    completed_steps: Vec<String>,
    security_preset: Option<String>,
    providers_done: bool,
    repositories_done: bool,
    service_installed: bool,
    vm_verified: bool,
    corp_config_source: Option<String>, // URL or path used
}
```

### Steps

| # | Step | Corp-awareness | Reuses from capsem-core |
|---|------|----------------|------------------------|
| 0 | Corp config (if `--corp-config`) | Provision corp config first | `fetch_corp_config()`, `install_corp_config()` from WB2a |
| 1 | Welcome + start background download | -- | `fetch_latest_manifest()`, `start_background_download()` from WB5 |
| 2 | Doctor (optional) | skip if service not running | -- |
| 3 | Security preset | Skip if corp locks the preset; show which settings are corp-locked | `apply_preset()` from presets.rs:72, `is_setting_corp_locked()` from resolver.rs:6 |
| 4 | Providers | Only show providers not corp-locked; pre-fill corp-provided keys | `host_config::detect()` at host_config.rs:37, `validate_api_key()` at host_config.rs:217 |
| 5 | Repositories | Only show options not corp-locked | `detect()` for git/SSH/GitHub, `validate_api_key("github", ...)` |
| 6 | Summary | Show which settings came from corp vs user choice | Await background download, `install_service()` from WB3, VM boot verify |

**Step 6 details:**
- Await background download with **10-minute timeout**. Show progress from `BackgroundProgress` channel. On timeout or error: print "Assets not yet available. VM boot test skipped. Run `capsem setup --force` to retry." Mark `vm_verified = false` in state.
- **PATH check** (Linux only): if `~/.capsem/bin` is not in `$PATH`, print warning with the export line to add. Critical for users who manually installed or when WB7 (install.sh) is not yet available.
- Install service via `install_service()`.
- VM boot verify: only if assets downloaded successfully and service running.

**Corp gating logic:** Each step calls `load_settings_files()` to get the merged (user, corp) pair. For settings where `is_setting_corp_locked()` returns true, the wizard displays the locked value and skips the prompt. In non-interactive mode, corp-locked settings are silently accepted.

**Non-interactive:** `--non-interactive --preset medium --accept-detected` skips all prompts. `--corp-config` can be combined.

Orchestrator calls steps sequentially, skips completed unless `--force`, saves state after each step.

### E2E tests: `tests/capsem-install/test_setup_wizard.py`

- `test_non_interactive_setup`: completes without prompts, state file written
- `test_setup_rerun_skips_completed`: second run skips done steps
- `test_setup_force_reruns_all`: `--force` re-runs everything
- `test_setup_writes_user_toml`: security preset writes user.toml

**Commit:** `feat: capsem setup interactive wizard`

---

## WB4: Self-Update

**Goal:** `capsem update` checks GitHub, downloads new binaries + assets, restarts service.

### Files

| File | Change |
|------|--------|
| `crates/capsem/Cargo.toml` | Add `self-replace = "1"`, `semver = "1"`, `tempfile = "3"`, `clap_complete = "4"` |
| `crates/capsem/src/platform.rs` | New. `InstallLayout` enum + `detect_install_layout()` |
| `crates/capsem/src/update.rs` | New. Update check cache, update flow |
| `crates/capsem/src/main.rs` | Add `mod platform; mod update;`, `Update { yes }` command. Dispatch before UdsClient. Background cache refresh after every command. |

### platform.rs

```rust
pub enum InstallLayout { MacosPkg, LinuxUserDir, Development }
pub fn detect_install_layout() -> InstallLayout
// MacosPkg if exe in /usr/local/bin, LinuxUserDir if exe in ~/.capsem/bin, else Development
```

### update.rs

Update check cache: `~/.capsem/update-check.json` (24h TTL)

- `read_cached_update_notice()` -- sync file read, called before command dispatch (no latency)
- `refresh_update_cache_if_stale()` -- fire-and-forget `tokio::spawn` after command dispatch

Update flow: `run_update(yes: bool) -> Result<()>`

**Atomic update sequence** -- download everything first, then swap. A partial failure leaves the old version intact:

1. `detect_install_layout()` -- Development -> bail "build from source"
2. `fetch_latest_manifest()` -> compare with semver
3. Confirm with `inquire::Confirm` (unless `--yes`)
4. **Download phase** (nothing replaced yet):
   - Download new binaries to temp dir (`tempfile::tempdir()`)
   - Download new assets via `fetch_remote_manifest()` + `AssetManager::download_asset()` to `~/.capsem/assets/v{new}/`
   - Verify all downloads (hash check). If any fail, abort -- system unchanged.
5. **Swap phase** (point of no return):
   - Stop service (`capsem service uninstall` or `systemctl --user stop capsem`)
   - Platform-specific binary replacement:
     - macOS: `sudo installer -pkg` (pkg handles atomicity)
     - Linux: `self_replace::self_replace()` for running binary, `fs::rename()` for others (same filesystem = atomic)
   - Start service (`capsem service install` or `systemctl --user start capsem`)
6. **Cleanup phase**:
   - `cleanup_old_versions()` (reuse from `asset_manager.rs:639`)
   - Check setup `schema_version`, prompt if new questions

### E2E tests: `tests/capsem-install/test_update.py`

- `test_update_dev_build_bails`: non-installed layout prints "build from source"
- `test_installed_layout_detection`: installed binaries detect LinuxUserDir
- `test_update_cache_write_and_read`: cache file written with 24h TTL
- `test_update_preserves_old_on_download_failure`: simulate asset download failure (write bad manifest), verify binaries unchanged

**Commit:** `feat: capsem update with asset vacuum`

---

## Polish: Completions + Uninstall

### Files

| File | Change |
|------|--------|
| `crates/capsem/src/completions.rs` | New. `generate_completions(shell)` using `clap_complete::generate()` |
| `crates/capsem/src/uninstall.rs` | New. `run_uninstall(yes)`: stop service, remove unit, remove binaries, remove `~/.capsem/`, remove logs |
| `crates/capsem/src/main.rs` | Add `Completions { shell }` and `Uninstall { yes }` commands. Dispatch before UdsClient. |

Commands that dispatch before UdsClient: Setup, Update, Completions, Uninstall, Version.

### E2E tests

- `test_completions.py`: bash/zsh/fish output validation
- `test_uninstall.py`: full uninstall removes binaries, unit, data

**Commit:** `feat: shell completions and capsem uninstall`

---

## Test Hardening: Lifecycle + Error Paths

**Goal:** Cross-WB integration test and failure mode coverage. These tests catch the bugs that individual WB tests miss -- the ones that turn into "debug hell" on real machines.

### tests/capsem-install/test_lifecycle.py

Full user journey in a single test, exercising every WB in sequence:

```python
def test_full_lifecycle(installed_layout, systemd_available):
    """Simulate a real user: install -> setup -> use -> service mgmt -> uninstall."""
    # 1. Fresh install (handled by installed_layout fixture via simulate-install.sh)

    # 2. Setup wizard (non-interactive)
    r = run_capsem("setup", "--non-interactive", "--preset", "medium", "--accept-detected")
    assert r.returncode == 0
    assert Path("~/.capsem/setup-state.json").expanduser().exists()

    # 3. First command triggers auto-launch
    r = run_capsem("list")
    assert r.returncode == 0

    # 4. Service management
    r = run_capsem("service", "status")
    assert r.returncode == 0
    assert "running" in r.stdout

    # 5. Update check (dev build bails)
    r = run_capsem("update")
    assert "build from source" in r.stdout or r.returncode != 0

    # 6. Uninstall
    r = run_capsem("uninstall", "--yes")
    assert r.returncode == 0
    assert not Path("~/.capsem/bin/capsem").expanduser().exists()
```

This single test would have caught every cross-WB socket-path mismatch, state-file conflict, and service-restart race we've seen in past sprints.

### tests/capsem-install/test_reinstall.py

Verifies that `just install` (via `simulate-install.sh`) actually replaces binaries with the new build. Uses the build hash embedded by `build.rs` to distinguish builds.

```python
def test_reinstall_replaces_binaries(clean_state):
    """Compile v1, install, compile v2, install -- verify v2 is installed."""
    capsem_bin = Path("~/.capsem/bin/capsem").expanduser()

    # Build 1: compile and install
    subprocess.run(["cargo", "build", "-p", "capsem"], check=True)
    subprocess.run(["bash", "scripts/simulate-install.sh", "target/debug", "assets"], check=True)
    hash_1 = get_build_hash()  # parses "capsem 0.13.0 (build c37b920.1743984000)"
    file_hash_1 = hashlib.sha256(capsem_bin.read_bytes()).hexdigest()

    # Force recompile: clean the capsem crate specifically
    subprocess.run(["cargo", "clean", "-p", "capsem"], check=True)

    # Build 2: compile and install
    subprocess.run(["cargo", "build", "-p", "capsem"], check=True)
    subprocess.run(["bash", "scripts/simulate-install.sh", "target/debug", "assets"], check=True)
    hash_2 = get_build_hash()
    file_hash_2 = hashlib.sha256(capsem_bin.read_bytes()).hexdigest()

    # The installed binary must be the NEW build, not the old one
    assert hash_1 != hash_2, "Build hashes should differ after recompile"
    assert file_hash_1 != file_hash_2, "Binary file hashes should differ after recompile"


def test_install_all_four_binaries_updated(clean_state):
    """All 4 binaries must be replaced, not just capsem."""
    bins = ["capsem", "capsem-service", "capsem-process", "capsem-mcp"]
    install_dir = Path("~/.capsem/bin").expanduser()

    # Install once, record file hashes
    subprocess.run(["cargo", "build"] + [f"-p {b}" for b in bins], check=True)
    subprocess.run(["bash", "scripts/simulate-install.sh", "target/debug", "assets"], check=True)
    hashes_before = {b: hashlib.sha256((install_dir / b).read_bytes()).hexdigest() for b in bins}

    # Force full recompile
    for b in bins:
        subprocess.run(["cargo", "clean", "-p", b], check=True)

    # Install again
    subprocess.run(["cargo", "build"] + [f"-p {b}" for b in bins], check=True)
    subprocess.run(["bash", "scripts/simulate-install.sh", "target/debug", "assets"], check=True)
    hashes_after = {b: hashlib.sha256((install_dir / b).read_bytes()).hexdigest() for b in bins}

    for b in bins:
        assert hashes_before[b] != hashes_after[b], f"{b} was not replaced by reinstall"
```

The build hash test uses `capsem version` output (semantic check), the file hash test uses SHA-256 of the binary bytes (physical check). Together they prove that `just install` isn't silently a no-op.

### tests/capsem-install/test_error_paths.py

Failure mode tests. Each one targets a specific "works in dev, breaks in prod" scenario:

| Test | What it does | Why it matters |
|------|-------------|----------------|
| `test_auto_launch_timeout_with_bad_binary` | Replace capsem-service with `#!/bin/sh; exit 1`, run `capsem list` | Verify timeout + clear error, not infinite hang |
| `test_missing_manifest_clean_error` | Delete manifest.json, run `capsem list` | Service start fails, CLI shows actionable message |
| `test_corrupt_setup_state_recovery` | Write `{garbage` to setup-state.json, run `capsem setup` | Setup re-creates state, doesn't crash |
| `test_wrong_permissions_on_capsem_dir` | `chmod 000 ~/.capsem/run`, run `capsem list` | Clear "permission denied" error, not cryptic hyper error |
| `test_service_install_idempotent` | Run `capsem service install` twice | Second call succeeds (or says "already installed") |
| `test_stale_socket_recovery` | Create fake service.sock file, run `capsem list` | Auto-launch detects stale socket, replaces it |
| `test_service_crash_during_command` | Start service, kill -9 it mid-command, retry | CLI detects dead service, re-launches |
| `test_setup_ctrl_c_leaves_clean_state` | Run setup, simulate SIGINT after step 3 | Completed steps saved, incomplete step not marked done |

### tests/capsem-install/test_install_sh.py (note for WB7)

The existing `tests/test_install_sh.py` tests the current Tauri-focused install.sh (DMG/deb). When WB7 lands:
- That file must be updated for .pkg (macOS) + tar.gz (Linux) paths
- The `find_asset_url()` tests need new patterns
- `install_linux()` tests change from apt to tar.gz extraction
- `simulate-install.sh` gets replaced by the real `install.sh` in conftest fixtures

Add a `# TODO(WB7): update for native installer` comment at the top of `test_install_sh.py` during Phase 0.

**Commit:** included in each WB's commit (lifecycle test grows incrementally; error path tests added alongside their WB)

---

## Skills & Documentation Updates

### New skill: `skills/dev-installation/SKILL.md`

Holistic skill covering the entire native CLI installer subsystem. Sections:

**Architecture & Layout**
1. Overview: purpose, relationship to Tauri bundle, why it's its own subsystem
2. Installed layout: `~/.capsem/bin/`, `assets/`, `run/`, state files. Contrast with dev layout.
3. Path discovery: `paths.rs` algorithm (sibling discovery, installed-first asset resolution, dev fallback)

**Subsystem Components**
4. Auto-launch: CLI auto-starts service (systemd/LaunchAgent if installed, direct spawn otherwise). MCP shares pattern. Retry-on-connect-fail in `UdsClient::request()`.
5. Service management: `capsem service install/uninstall/status`, plist/systemd generation, pure vs side-effecting split
6. Corp config provisioning: URL/file -> `~/.capsem/corp.toml`, two-tier resolution, background refresh with ETag, corp-source.json
7. Setup wizard: Corp-aware 7-step flow, state persistence, `--non-interactive`, resume-safe
8. Self-update + background refresh: layout detection, version check cache, corp config refresh, platform-specific binary replacement, asset download + vacuum
9. Background asset download: `start_background_download()`, `BackgroundProgress` channel pattern
10. Shell completions & uninstall

**Development Workflow**
11. Dev iteration loop: `cargo build -p capsem` locally for macOS, `just test-install` for Linux/systemd in Docker
12. Testing in detail: Docker harness, capsem-install test suite, fixtures, test tier placement
13. Adding new installer features: checklist

**Reference**
14. Key files table with descriptions and which WB introduced them

### Updates to existing skills/docs

- `skills/dev-testing/SKILL.md`: Add install testing as fourth test tier, add capsem-install to integration test suites table
- `skills/dev-capsem/SKILL.md`: Add `/dev-installation` to skill map and directory map
- `CLAUDE.md`: Add `/dev-installation` to "Skills -- LOAD BEFORE CODING" table

**Commit:** `docs: dev-installation skill and developer docs updates`

---

## Verification

### Automated (runs on every PR via CI)

```bash
just test              # Unit + cross-compile + frontend (nothing breaks)
just test-install      # Docker e2e: lifecycle, error paths, all WBs (Linux + systemd)
```

Both are CI-gated. `just test-install` also gates release.yaml.

### Local macOS testing via just install

```bash
just install           # Build + install to ~/.capsem/ + codesign
~/.capsem/bin/capsem list  # Or add to PATH first
```

This exercises the real installed layout on macOS (LaunchAgent, Apple VZ, codesigned binaries). The Docker e2e tests cover Linux/systemd; `just install` covers macOS/LaunchAgent.

### Manual verification on macOS after WB1:

```bash
just install
# Kill any running service, then:
~/.capsem/bin/capsem list   # Should auto-launch and return empty list
```

After WB3 on macOS:

```bash
capsem service install    # Creates LaunchAgent
capsem service status     # Shows installed + running
capsem service uninstall  # Removes LaunchAgent
capsem service install    # Idempotent -- succeeds again
```

After WB2:

```bash
capsem setup --non-interactive --preset medium --accept-detected  # Headless
capsem setup              # Interactive (manual macOS)
```

After WB4:

```bash
capsem update             # Dev build should bail with "build from source"
```

After Polish:

```bash
capsem completions bash   # Outputs completion script
capsem completions zsh
capsem completions fish
```

### Test confidence matrix

| Scenario | Automated | Platform |
|----------|-----------|----------|
| Binary discovery + sibling resolution | `test-install` | Linux |
| Auto-launch from installed layout | `test-install` | Linux |
| Auto-launch with bad binary / missing assets | `test-install` | Linux |
| systemd unit install/uninstall/status | `test-install` | Linux |
| systemd unit idempotency | `test-install` | Linux |
| LaunchAgent plist generation (pure) | `just test` (Rust unit) | All |
| LaunchAgent plist install/uninstall | **manual** | macOS |
| Setup wizard non-interactive | `test-install` | Linux |
| Corp config provisioning + precedence | `test-install` | Linux |
| Self-update dev-build bail | `test-install` | Linux |
| Update atomicity (partial failure) | `test-install` | Linux |
| Full lifecycle (install->setup->use->uninstall) | `test-install` | Linux |
| Error recovery (corrupt state, stale socket, etc.) | `test-install` | Linux |
| Shell completions output | `test-install` | Linux |
| Full uninstall cleanup | `test-install` | Linux |
| simulate-install.sh matches real layout | `test-install` | Linux |
| VM boot in installed layout | `test-install` (if /dev/kvm) | Linux |
| install.sh rewrite (WB7) | **deferred** | -- |

## What "done" looks like

```bash
# Install on clean Mac (deferred -- WB6/WB7)
# curl -fsSL https://capsem.org/install.sh | sh

# Wizard: corp config, security, providers, repos, service install, VM verification
capsem setup
capsem setup --corp-config https://corp.example.com/capsem.toml

# Enterprise: non-interactive with corp policy
capsem setup --non-interactive --preset high --accept-detected --corp-config /etc/corp-capsem.toml

# Use it
capsem shell

# Update later
capsem update

# Service management
capsem service install
capsem service status
capsem service uninstall

# Shell completions
capsem completions bash > ~/.local/share/bash-completion/completions/capsem

# Full uninstall
capsem uninstall
```
