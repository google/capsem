---
name: dev-installation
description: The native package installer: install, update, uninstall, service registration, asset download, corp config. Use when working on those or the installed ~/.capsem/ layout.
---

# Native Package Installer

## Installed layout

```
~/.capsem/
  bin/capsem, capsem-service, capsem-process, capsem-tui,
      capsem-mcp, capsem-mcp-aggregator, capsem-mcp-builtin,
      capsem-gateway, capsem-tray, capsem-admin, capsem-mock-server
  assets/manifest.json, manifest-metadata.json, {asset-name}-{hash16}.{ext}
  run/service.sock, service.pid, instances/, persistent/
  corp.toml               (CLI-provisioned corp config)
  corp-source.json         (corp config source metadata)
```

## CLI commands (no service required)

These commands dispatch before UdsClient creation -- they work without the service running:

| Command | Module | What |
|---------|--------|------|
| `capsem version` | main.rs | Print version + build hash |
| `capsem update` | update.rs | Check the selected release manifest URL and report the matching binary installer |
| `capsem service install\|uninstall\|status` | service_install.rs | Service registration |
| `capsem completions bash\|zsh\|fish` | completions.rs | Shell completions |
| `capsem uninstall --yes` | uninstall.rs | Full removal |

## Path discovery (paths.rs)

`discover_paths()` finds sibling binaries and assets:

1. `current_exe().parent()` -> bin_dir -> the packaged host binary cohort:
   `capsem`, `capsem-service`, `capsem-process`, `capsem-tui`,
   `capsem-mcp`, `capsem-mcp-aggregator`, `capsem-mcp-builtin`,
   `capsem-gateway`, `capsem-tray`, `capsem-admin`, `capsem-mock-server`
2. Assets: `~/.capsem/assets/` (the only installed layout -- packages install
   manifest URL provenance, then postinstall hydrates the live manifest and
   assets through `capsem update --assets --manifest <URL>`)

## Auto-launch (main.rs UdsClient)

`try_ensure_service()` runs on every service-dependent command:

1. Check socket connectivity
2. Try systemd/LaunchAgent if unit installed (via `try_start_via_service_manager()`)
3. Fall back to direct spawn only for explicit development commands; installed
   package paths are otherwise authoritative
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

## Package install

The package is the install unit. It may accept a manifest URL override for corp
and development installs, records that URL in packaged
`manifest-metadata.json`, hydrates the live manifest through
`capsem update --assets --manifest <URL>` during postinstall, installs/restarts
service files, and writes timestamped install logs. Packages do not carry an
`assets/manifest.json` payload. They do not run an AI-provider setup wizard and
they do not create a user policy file.

Postinstall writes the selected verified release document unchanged to
`assets/manifest.json`. The boot resolver derives its compact runtime view in
memory; it must never serialize that view over the installed release graph.
`assets/manifest-metadata.json`, with schema
`capsem.manifest_metadata.v1`, is the only sidecar for provenance and update
state. CLI status and About Capsem both read the service's canonical
`GET /system/status` response, which includes those exact two JSON documents.

The public installer fetches the selected manifest once and hands postinstall
two identities: its logical URL and root-owned exact payload bytes. The hidden
preactivation rail consumes those bytes through stdin, uses the logical URL for
relative assets and `checked_url`, and preserves the package-owned polling URL,
channel, and origin. The request and payload remain paired for package-manager
retry after failure; postinstall removes both only after success.

## Package maintainer scripts

- macOS `.pkg`: `scripts/pkg-scripts/preinstall` unloads the LaunchAgent, kills
  stale package-owned helpers, removes old app/share payloads, then
  `scripts/pkg-scripts/postinstall` copies binaries, hydrates assets, registers
  the service, waits for service/gateway readiness, and opens the app.
- Linux `.deb`: `scripts/deb-preinst.sh` is packaged as `DEBIAN/preinst`. It
  runs `systemctl --user stop capsem.service` when a user systemd session is
  available, then kills the stale helper cohort before package replacement so
  old service/gateway/tray/process binaries cannot survive from old inodes.
  `scripts/deb-postinst.sh` symlinks the packaged binaries into
  `~/.capsem/bin`, hydrates assets, and invokes `capsem install` to register or
  restart the user service.

## Self-update (update.rs)

- `read_cached_update_notice()` -> sync file read on every command
- `refresh_update_cache_if_stale()` -> background 24h-cached check merged atomically into `assets/manifest-metadata.json`
- `run_update()` -> check the selected manifest URL and stage one complete compatible release transaction before mutating the installation
- `capsem update --yes` -> verifies every changed package/profile artifact, prints the tested package-manager apply command for audit, executes it through `sudo` when the native package changes, and atomically activates the selected profile graph; this is the one ordinary update path used by the installed service
- `capsem update --assets` -> low-level diagnostic/repair rail for hydrating the locally installed manifest or an explicit `--manifest` URL; normal product surfaces never direct users to apply assets separately
- Corporate VM asset channels use `capsem update --assets --manifest <URL>`; `--corp <URL>` provisions policy config and must not be combined with `--assets`
- `--manifest` and `--corp` are URL-only inputs. Local files must use `file:///absolute/path`, while hosted release and corporate channels use `https://...` or `http://...`; bare paths are rejected so update checks share one URL-based mechanism.
- Stable/nightly switching uses the one complete installed transaction: `capsem update --yes --channel <stable|nightly>`. Explicit channel transitions may downgrade; Linux uses `apt-get --allow-downgrades`. The single metadata file records the installed manifest URL separately from the most recently checked URL.
- An explicit corporate asset manifest moves the installation into a one-way locked channel. Persist `channel_kind=corporate` and `channel_locked=true`; later public-channel or different-manifest selections must fail before fetch or mutation.
- Profile-owned images/configs/evidence belong to the selected channel/profile. Updating the co-work nightly profile can refresh only nightly co-work image/config refs and matching digests; it must not mutate stable, packages, per-binary inventory, or other profiles.
- Profiles may set `min_capsem_version` when a profile requires newer client behavior. That is the compatibility hook; profiles must not point at the selected Capsem binary.
- Layout detection: MacosPkg, LinuxDeb, UserDir, Development (development bails with "build from source")
- Pre-updater installed binaries cannot be retrofitted through the release
  channel. If a shipped binary prints "Binary self-update is not yet wired up",
  that install needs one manual `.pkg` or `.deb` bootstrap into a version that
  contains the package apply path; only then can later binary releases move
  through `capsem update --yes`.
- Installed update smokes require that all packaged host binaries expose a version surface and report the same installed Capsem package version after replacement.

## Corp config provisioning (capsem-core: corp_provision.rs)

- `fetch_corp_config(url)` -> GET + validate + return content + ETag
- `validate_corp_toml(content)` -> parse as SettingsFile
- `install_corp_config(dir, content, source)` -> write corp.toml + corp-source.json
- `refresh_corp_config_if_stale(dir)` -> background conditional GET with ETag

Loader changes: `corp_config_paths()` returns [/etc, ~/.capsem/] with first-wins merge.

## Remote manifest + asset download (capsem-core: asset_manager.rs)

- `download_missing_assets(manifest, binary_version, arch, dir, progress)` -> hydrate missing or corrupt assets from the manifest's release-channel URLs
- `copy_missing_local_assets(...)` -> same contract for `file://` corporate/local manifests
- `cleanup_unused_assets(base_dir, manifest)` -> remove hash-named files no longer referenced by non-deprecated releases

## Test harness

Docker-based e2e tests in `tests/capsem-install/`:

| File | Tests |
|------|-------|
| test_smoke.py | Harness works (systemd, binaries, build hash) |
| test_auto_launch.py | Auto-launch, path discovery, asset resolution, error cases |
| test_service_install.py | Install/uninstall/status, idempotent, systemd integration |
| test_corp_config.py | Provisioning, validation, precedence |
| test_update.py | Dev build bail, layout detection, cache, preserve-on-fail |
| test_completions.py | bash/zsh/fish output |
| test_uninstall.py | Full cleanup |
| test_lifecycle.py | End-to-end user journey |
| test_reinstall.py | Binary replacement verification |
| test_error_paths.py | Failure scenarios with actionable errors |

Run `just _gate-install` for the Linux Docker/systemd boundary. On Apple Silicon
macOS, run `python3 scripts/macos_release_glowup.py` for the exact `.pkg` build, clean Tart
install, receipt/app/binary verification, and service health. Because Tart
macOS guests cannot expose nested virtualization, the recipe then extracts the
same package on the physical Mac and boots a real Capsem guest from its exact
binary/profile payload to a shell marker. Both focused scripts remain
debugging tools; `just test-clean` is the release gate that owns them.

The Linux qualification image is a visible four-step graph: capacity,
dependency materialization, sealed source build, and sealed smoke. Only the
input-keyed materializer may use BuildKit's ordinary network; it consumes the
exact host-platform parent, immutable Ubuntu snapshot, locked uv environment,
and frozen pnpm store. Source build, smoke, and the privileged systemd runtime
all use networking disabled. They bind the runnable input-keyed tag to the
exact platform-child ID before use; the child ID is evidence but is not itself
runnable on every containerd store. Do not repair a failure with runtime apt,
pnpm, uv sync, a second build, or an unverified image tag.

Manifest-selected profile content includes a verified immutable input subtree
under the same `ProfileContent` root as assets and materialized config. Mount
that root read-only, reverify it inside the container, and use the extracted
`capsem-admin` from the exact package to author the checked local package/profile
graph before the single `dpkg -i`. The narrower Debian proof uses the same graph
primitive and secure handoff; neither proof may fall back to a public URL.

The local `.pkg` is intentionally unsigned. Its installer postinstall applies
ad-hoc signatures and the required entitlements to executable payloads, which
the Tart guest verifies before service and gateway checks. Local installation
tests must not import or unlock an Apple Developer certificate or mutate the
user's keychain. Developer ID signing, notarization, and stapling are
publication-workflow responsibilities.

## Key files

```
crates/capsem/src/
  main.rs              CLI entry, command dispatch, UdsClient with auto-launch
  paths.rs             Binary + asset path discovery
  platform.rs          Install layout detection
  package.rs           Package install orchestration and manifest placement
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
