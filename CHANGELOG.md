# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
- **capsem-gateway: TCP-to-UDS reverse proxy** -- standalone binary that bridges TCP (default port 19222) to capsem-service UDS. Bearer token auth (64-char random, regenerated on restart, written to `~/.capsem/run/gateway.token` with 0600 permissions). All service endpoints proxied through with method/path/query/body preserved. `GET /` health check (no auth). `GET /status` aggregated VM health with 2s cache TTL for efficient tray polling. CORS permissive for browser access. Graceful shutdown cleans up token/port/pid files. No capsem-core dependency, no VM access -- pure low-privilege proxy.
- **capsem-service: auto-spawn gateway and tray** -- service now spawns capsem-gateway (TCP proxy) and capsem-tray (macOS menu bar) as child processes on startup. Both are killed on graceful shutdown. Tray spawn is macOS-only, gateway spawn is cross-platform. Sibling binary discovery falls back to target/debug/ for development.

### Security
- **capsem-gateway: 10 MB request body size limit** -- proxy now enforces a 10 MB maximum on incoming request bodies via `http_body_util::Limited`, returning 413 Payload Too Large for oversized payloads. Prevents OOM from malicious clients.
- **capsem-gateway: CORS restricted to localhost origins** -- replaced `CorsLayer::permissive()` (allow all origins) with a predicate that only allows `http(s)://localhost`, `http(s)://127.0.0.1`, and `tauri://` origins. Prevents cross-origin requests from external websites.
- **capsem-gateway: auth failure rate limiting** -- after 20 failed auth attempts within 60 seconds, the gateway returns 429 Too Many Requests instead of 401. Prevents brute-force token guessing.
- **capsem-tray: macOS menu bar tray** -- standalone binary that polls the gateway `/status` endpoint and shows per-VM submenus (Connect, Suspend/Resume, Fork, Stop, Delete), global actions (New Temporary VM, New Long-term VM, Open Capsem, Quit), and color-coded status icons (green/grey/red). Uses `tray-icon` + `muda` for native NSStatusItem. No capsem-core dependency, no Tauri.

### Fixed
- **capsem-service: suspend silently reported success on failure** -- `handle_suspend` discarded IPC send errors with `let _` and returned `{"success": true}` even when the VM never confirmed suspended state. Now propagates all IPC errors and returns 500 if the VM does not confirm suspension within 15 seconds.
- **capsem-service: resume did not pass checkpoint path** -- `resume_sandbox` re-spawned `capsem-process` without `--checkpoint-path`, causing suspended VMs to cold-boot instead of warm-restoring. Now passes `--checkpoint-path` when the registry entry has `suspended: true` and the checkpoint file exists.
- **capsem-service: resume did not clear suspended flag** -- after successful resume, `entry.suspended` stayed `true` and `entry.checkpoint_path` retained the stale value. Now clears both and saves the registry.
- **capsem-service: /list and /info did not distinguish Suspended from Stopped** -- persistent VMs with `suspended: true` were reported as "Stopped". Now returns "Suspended" status, and the gateway's `/status` endpoint includes `suspended_count` in `ResourceSummary`.
- **capsem-gateway: terminal WebSocket gave no error on VM unavailable** -- when the per-VM UDS connect failed after WebSocket upgrade, the connection silently dropped. Now sends a Close frame with code 1011 and reason "VM not available".
- **capsem-gateway: proxy timeout too short for suspend** -- 30-second proxy timeout could expire during suspend operations (up to 26s). Increased to 120 seconds. Added 5-minute safety timeout on the background HTTP connection driver.
- **capsem-gateway: terminal UDS path fallback incorrect** -- `terminal_uds_path` used `parent().unwrap_or("/tmp")` which never triggered for bare filenames (parent returns `Some("")`). Now filters empty parents before falling back.
- **capsem-gateway: proxy URI parse could panic** -- `forward()` used `.unwrap()` on `upstream_uri.parse()`, which could panic on malformed URIs. Replaced with error propagation that returns 502 Bad Gateway.
- **capsem-gateway: terminal WebSocket rejected underscores in VM IDs** -- `handle_terminal_ws` validation allowed only `[a-zA-Z0-9-]`, rejecting persistent VMs with underscores (e.g. `my_dev`). Aligned with service's `validate_vm_name()`: `[a-zA-Z0-9_-]`, must start alphanumeric, length 1-64.
- **capsem-gateway: terminal.rs had zero test coverage** -- added 31 unit + integration tests covering ID validation, UDS path construction, WebSocket relay (text, binary, ping/pong, close with reason, process disconnect, client disconnect, missing UDS, invalid ID rejection). Coverage went from 0% to 89%.
- **capsem-gateway: not tracked in CI coverage** -- added `-p capsem-gateway` to CI coverage pipeline and `gateway` component to codecov.yml (80% target).
- **capsem-process: process.log written in text format instead of JSONL** -- tracing subscriber used default text formatter with ANSI colors, making process.log unparseable by integration tests and tooling. Switched to JSON format matching capsem-service. Also changed RUST_LOG from `debug` to `capsem=info` for subprocess to avoid noisy debug entries.
- **capsem run: session not registered in main.db** -- `handle_run` in capsem-service provisioned and destroyed VMs without creating a session record or rolling up telemetry counters. Sessions from `capsem run` were invisible to `capsem sessions` and integration tests.
- **capsem run: missing `--env` support** -- `capsem run` had no way to pass environment variables to the guest, unlike `capsem create -e`. Added `--env`/`-e` to CLI, `env` field to `RunRequest`, and `env` param to `capsem_run` MCP tool. Integration test now passes API key via `--env` instead of relying on process env inheritance.
- **capsem-process: missing boot timeline in process.log** -- state transition events were only emitted in the capsem-app CLI path, not in capsem-process. Boot timeline is now logged after `boot_vm` returns.
- **Test scripts missing `run` subcommand** -- `injection_test.py`, `integration_test.py`, and `doctor_session_test.py` called `capsem <command>` instead of `capsem run <command>`, causing exit 2 on all scenarios. Also improved failure output to show full stdout/stderr instead of just lines matching "FAILED".
- **capsem-init: guest binaries deployed 755 instead of 555** -- `capsem-doctor`, `capsem-bench`, and `snapshots` were deployed with write bits via initrd overlay, violating the read-only binary invariant.
- **Dead code wired into production paths** -- consolidated duplicate path logic between `paths.rs` and `service_install.rs`. `is_service_installed()` now guards `try_ensure_service()` to prevent unmanaged duplicate service spawns. `start_background_download()` wired into setup wizard. `install_bin_dir()` wired into uninstall for layout-aware binary removal. `assets_dir_from_home()` used by `discover_paths()`. Removed `ServiceSpawnArgs` (was identical to `CapsemPaths`). Zero `#[allow(dead_code)]` annotations remain.
- **initrd repack: permission denied on read-only guest binaries** -- `_pack-initrd` now `rm -f` before overwriting 555-permission files (`capsem-doctor`, `capsem-bench`, `snapshots`), matching the pattern already used for agent binaries.
- **Service race condition on exec/write/read after provision** -- `handle_exec`, `handle_write_file`, and `handle_read_file` now wait for the VM socket to be ready before sending IPC commands. Previously, calling these endpoints immediately after `/provision` or `/resume` would fail with "failed to connect to sandbox" because the capsem-process had not yet created its socket. Extracted `wait_for_vm_ready` helper (socket existence + ping) shared by all IPC handlers. This fixes `capsem doctor` and any client that calls exec without polling.
- **pnpm audit: defu prototype pollution and vite file read vulnerabilities** -- added `defu>=6.1.5` and `vite>=6.4.2` overrides to frontend `pnpm.overrides`.
- **capsem-process: reject invalid fd -1 in clone_fd** -- defensive check prevents undefined behavior when an invalid file descriptor is passed.
- **capsem doctor: streaming output** -- doctor now streams test results in real-time via terminal IPC instead of buffering all output until completion. Also adds `--durations=10` to surface the 10 slowest tests.
- **capsem doctor: removed invalid --json flag** -- `capsem-doctor` is a pytest wrapper that doesn't support `--json`. The flag caused pytest to exit with "unrecognized arguments".
- **MCP snapshots_changes: JSON pagination breaks parsing** -- `format=json` output was wrapped in pagination headers (`Content length: ...`), making `json.loads()` fail. JSON format now returns the raw array without pagination headers.
- **Guest binary permissions: snapshots and capsem-bench** -- changed from 755 to 555 in rootfs Dockerfile to match the read-only binary invariant.
- **Rust warnings-as-errors for all crates** -- `RUSTFLAGS="-D warnings" cargo check --workspace` now runs in both `just smoke` and `just test`, blocking on any warning in any crate. Previously only capsem-service and capsem-process were checked, and only in `just test`.

### Added
- **Auto Docker GC** -- `_docker-gc` recipe runs automatically after `build-assets`, `cross-compile`, and `test-install` to prevent unbounded disk growth. Prunes stopped containers, unused images >72h, build cache >72h, and runs `fstrim` on the Colima VM disk to release freed space back to macOS.
- **Doctor: separate CLI vs daemon checks** -- `just doctor` now checks the Docker CLI binary and daemon reachability independently, with platform-specific fix hints (macOS: start Colima, Linux: systemctl start docker).
- **Shell completions and `capsem uninstall`** -- `capsem completions bash|zsh|fish` generates shell completions via clap_complete. `capsem uninstall --yes` stops service, removes unit, binaries, `~/.capsem/`, and logs.
- **`capsem update` self-update** -- checks GitHub for new releases, downloads assets with hash verification, and cleans up old versions. Update notice displayed on every command (24h cached check). `--yes` skips confirmation. Development builds directed to build from source. Install layout detection (MacosPkg, UserDir, Development).
- **`capsem setup` interactive wizard** -- first-time setup with security preset selection, AI provider credential detection, repository access check, service installation, and PATH verification. Supports `--non-interactive`, `--preset`, `--force`, `--accept-detected`, and `--corp-config` flags. Persists state to `~/.capsem/setup-state.json` for incremental re-runs. Corp-aware: skips prompts for corp-locked settings.
- **Corp config provisioning** -- enterprise users can provision corp config from a URL or local file path via `capsem setup --corp-config`. Config installs to `~/.capsem/corp.toml` with source metadata in `corp-source.json`. Background refresh with ETag-based conditional GET. Loader now merges system (`/etc/capsem/corp.toml`) and user-provisioned (`~/.capsem/corp.toml`) corp configs with system taking precedence per-key.
- **Remote manifest fetch and background asset download** -- `fetch_remote_manifest()` and `fetch_latest_manifest()` fetch VM asset manifests from GitHub releases. `start_background_download()` spawns a tokio task that checks and downloads missing assets with progress reporting via an mpsc channel. Reuses existing AssetManager, DownloadProgress, and blake3 verification.
- **`capsem service install/uninstall/status`** -- register capsem as a LaunchAgent (macOS) or systemd user unit (Linux) with `capsem service install`. Pure generator functions produce the plist/unit content; side-effecting functions handle platform registration. Auto-launch prefers the service manager when a unit is installed.
- **CLI auto-launches service on first command** -- `capsem list` (or any command) now auto-starts the service daemon if no socket is found. Tries systemd/LaunchAgent if a unit is installed, falls back to direct spawn. New `paths` module discovers sibling binaries and assets with installed-first resolution (`~/.capsem/assets/`) before dev fallback. MCP server also uses installed-first asset resolution. Consolidated CLI HTTP methods into a single `request()` with retry-on-connect-fail.
- **Native installer e2e test harness** -- Docker-based install test infrastructure with systemd user sessions. `just install` builds and installs to `~/.capsem/` with codesigning on macOS. `just test-install` runs the full install layout tests in a Docker container. `capsem version` now prints a unique build hash (`capsem 0.16.1 (build c37b920.1775464335)`) for binary identity verification. CI runs install tests on every PR; release pipeline gates on them.
- **Fork images** -- snapshot running or stopped VMs into reusable template images (`capsem fork`), boot new VMs from them (`capsem create --image`). Image registry with list/inspect/delete. Flat genealogy model (images depend only on base squashfs, never on each other). Asset cleanup protects referenced squashfs versions. Available via CLI, MCP tools (`capsem_fork`, `capsem_image_list`, `capsem_image_inspect`, `capsem_image_delete`), and service HTTP API.
- **Session DB schema v5** -- adds `source_image` and `persistent` columns. Vacuum skips persistent VM sessions.
- **CLI parity sprint** -- `--timeout` on `exec`, `capsem version`, `-q`/`--quiet` on `list`, `--tail N` on `logs`, `capsem restart` for persistent VMs, `--env KEY=VALUE` / `-e` on `create` for guest environment injection.
- **`--env` plumbing** -- environment variables flow from CLI/MCP through service, process, and into guest boot config (`send_boot_config`). Supports up to 128 env vars per VM.
- **MCP: `capsem_version` tool** -- returns MCP server version and service connectivity status.
- **MCP: `tail` parameter** -- on `capsem_vm_logs` and `capsem_service_logs` tools, limit output to last N lines (applied after grep filter).
- **MCP: `env` parameter** -- on `capsem_create` tool, inject environment variables into the guest.
- **Next-gen daemon architecture (Sprint 1)** -- capsem now runs as a daemon service (`capsem-service`) that spawns isolated per-VM processes (`capsem-process`), mirroring Chrome's multi-process security model. The service manages VM lifecycle over a UDS API, while each process boots and owns exactly one VM.
- **Full CLI client (`capsem`)** -- new subcommands: `start`, `stop`, `shell`, `list`/`ls`, `status`, `exec`, `delete`/`rm`, `info`, `logs`, `doctor`. The CLI communicates with the service daemon over `~/.capsem/service.sock`.
- **`capsem-mcp` crate** -- standalone MCP server (stdio transport via `rmcp`) that bridges AI agent tool calls to the service API. Provides `capsem_create`, `capsem_exec`, `capsem_read_file`, `capsem_write_file`, `capsem_list`, `capsem_delete`, `capsem_info`, `capsem_inspect`, `capsem_inspect_schema`, `capsem_service_logs`, `capsem_vm_logs` tools.
- **Structured IPC protocol** -- `capsem-proto` extended with `Exec`, `WriteFile`, `ReadFile`, `ReloadConfig`, `StartTerminalStream` commands and matching result variants. New `ipc_ext` module in `capsem-core` for framed message helpers.
- **Service-level resource management** -- concurrent VM limit (`max_concurrent_vms`), per-VM CPU/RAM validation (1-8 CPUs, 256MB-16GB), stale instance cleanup, auto-remove flag, socket path length validation.
- **Multi-version asset resolution** -- service resolves assets from `~/.capsem/assets/v{version}/` with arch-specific fallback.
- **Network policy config: builder tests** -- comprehensive unit tests for `settings_to_vm_settings`, `settings_to_domain_rules`, `load_merged_settings`, and preset validation.
- **Session maintenance** -- new cleanup routines in `capsem-core` for session directory housekeeping.
- **Testing sprint Phase 3 complete** -- 11 new test suites (T15-T25) covering build chain E2E, guest validation, cleanup verification, codesign strict, serial console, session.db lifecycle, config runtime, recipe smoke, recovery/crash-resilience, rootfs artifacts, and exhaustive per-table session.db validation. ~84 new Python integration tests across 40+ test files.
- **New just recipes for Phase 3 tests** -- `test-build-chain`, `test-guest`, `test-cleanup`, `test-codesign`, `test-serial`, `test-session-lifecycle`, `test-config-runtime`, `test-recipes`, `test-recovery`, `test-rootfs`, `test-session-exhaustive`, plus a combined `test-vm` recipe.

### Changed
- **`capsem-process` is now the VM owner** -- boot logic moved from `capsem-app` into `capsem-process`, which receives config via CLI args and communicates with the service over a typed IPC channel (`tokio-unix-ipc`). Includes PTY exec with ANSI stripping, file I/O forwarding, and terminal streaming.
- **`capsem-agent` guest binary** -- updated vsock I/O, net proxy, and MCP server modules to match the new host-guest protocol.
- **Justfile overhaul** -- restructured recipes for the daemon workflow (`run-service`, `run-process`), updated build and test targets.

### Fixed
- **Silent epoch on malformed image timestamps** -- `time_format` serde deserializer silently returned `UNIX_EPOCH` for garbage input, corrupting image sort order. Now returns a proper deserialization error.
- **`top_mcp_tools` merged tools from different servers** -- SQL `GROUP BY tool_name` without `server_name` collapsed cross-server tools into one row with an arbitrary server name. Added `server_name` to the GROUP BY clause.
- **Image registry TOCTOU and concurrent write corruption** -- `create_image_from_session` had a TOCTOU race (exists check then create_dir_all). Replaced with atomic `create_dir`. Added `flock`-based file locking around registry insert/remove with atomic write (write-to-temp then rename).
- **`handle_logs` returned 404 for stopped persistent VMs** -- unlike `handle_info`, it only checked running instances. Added persistent registry fallback.
- **Blocking I/O in async context** -- `std::thread::sleep` in CLI shell loop (replaced with `tokio::time::sleep`), `std::process::Command` in MCP service relaunch (replaced with `tokio::process::Command`), blocking file reads in MCP `service_logs` and service `handle_logs` (wrapped in `spawn_blocking`).
- **CLI `SandboxInfo` missing fields** -- CLI struct lacked `ram_mb`, `cpus`, `version` fields that the service returns. Added with `#[serde(default)]` and display in `status` command.
- **Panicking `unwrap()` in MCP service relaunch** -- `Path::parent().unwrap()` replaced with proper error propagation.
- **`snapshots` CLI missing from release rootfs** -- the `snapshots` tool was never copied into the rootfs Docker build context or Dockerfile template, so release builds shipped without it. Added `ROOTFS_ARTIFACTS` constant as single source of truth in `docker.py`, plus 6 validation layers: builder unit tests, builder doctor pre-build check, config validator, rootfs artifacts test suite, CI release workflow validation, and in-VM guest binary assertions (changed from `pytest.skip` to `pytest.fail`).
- **`just doctor-fix` fails on fresh machines** -- `build-assets` triggered `_ensure-setup` which ran `doctor` which failed on missing assets, creating a circular dependency. Fix commands now set `CAPSEM_SKIP_ASSET_CHECK=1` and `touch .dev-setup` to break the cycle. Guest binary checks are also skipped when asset check is skipped (no assets = no binaries). Fixes bail on first failure instead of continuing to run dependent steps.
- **Docker cross-arch builds fail (legacy builder cache poisoning)** -- Docker's legacy builder shared intermediate layer cache across `--platform` values, reusing arm64 layers for x86_64 builds. Fixed by requiring Docker BuildKit (buildx). Added buildx and Colima Rosetta checks to `just doctor` and `scripts/bootstrap.sh`.

## [0.16.1] - 2026-04-02

### Added
- **KVM boot diagnostics** -- when vCPU creation fails on Linux, Capsem now runs automatic diagnostic probes: kernel version, nested KVM status, KVM capabilities, and a fresh-VM-without-IRQCHIP test to isolate the root cause. All results logged at ERROR level so they appear without `RUST_LOG=debug`.
- **`scripts/kvm-diagnostic.py`** -- standalone diagnostic script for manual KVM environment debugging. Tests 7 phases: /dev/kvm basics, capabilities, Capsem boot sequence, no-irqchip mode, reversed ordering, split IRQCHIP, and environment info.

### Fixed
- **KVM boot errors are now actionable** -- `/dev/kvm` missing explains how to enable KVM (modprobe, BIOS). Permission denied suggests `usermod -aG kvm`. EEXIST on vCPU creation explains restricted/nested KVM and points to the diagnostic script.
- **Linux boot failure shows macOS error message** -- `gui.rs` said "unsigned binary or missing entitlement" on all platforms. Now shows platform-specific guidance: KVM troubleshooting on Linux, entitlement info on macOS.
- **LATEST_RELEASE.md stale at v0.15.1** -- boot screen showed wrong version. Regenerated from CHANGELOG.md.

### Changed
- **`just doctor` rewritten as standalone scripts** -- moved from 265-line inline justfile recipe to `scripts/doctor-common.sh` + platform-specific `doctor-macos.sh` and `doctor-linux.sh`. Colored output (green/red/yellow), structured recap table, and auto-fix: detects fixable issues (missing rustup targets, cargo tools, broken symlinks) and prompts to fix them automatically. `--fix` flag for non-interactive auto-fix.

## [0.16.0] - 2026-04-02

### Added
- **`just clean` reports freed space** -- shows per-directory sizes before deletion and total freed at the end. Also cleans `tmp/` and `coverage/` directories.
- **`just clean-all` prunes docker volumes** -- adds `--volumes` to docker prune for full reclaim.
- **Automatic incremental cache trimming** -- `_clean-stale` now checks if `target/` exceeds 20 GB and auto-removes incremental compilation caches (`target/debug/incremental`, `target/release/incremental`, `target/llvm-cov-target`). Prevents unbounded growth that caused 113 GB bloat.
- **`_clean-stale` wired into all build paths** -- added to `build-assets` and `cross-compile` dependency chains (was already in `test` and `_compile`).
- **Revert telemetry** -- `snapshots_revert` now logs a `restored` file event to the session DB, including the source checkpoint (e.g., `"src/main.py (from cp-3)"`). New `FileAction::Restored` variant in capsem-logger, `FileEventStats.restored` counter in reader queries.
- **Boot audit logging** -- comprehensive `[boot-audit]` tracing throughout the GUI and CLI boot paths (main.rs, gui.rs, boot.rs, cli.rs, session_mgmt.rs). Every step from session cleanup through hypervisor boot is timestamped, making hangs immediately diagnosable.
- **Doctor: VM asset and guest binary checks** -- `just doctor` now validates asset manifest version, B3SUM integrity, and guest binary presence/format.
- **Smoke test recipe** -- `just smoke-test` (alias `just smoke`) runs unit tests + repack + sign + capsem-doctor as a fast end-to-end validation without full asset rebuild.
- **Doctor: Docker BuildKit (buildx) and Colima Rosetta checks** -- `just doctor` now validates that buildx is installed and Colima has Rosetta enabled for cross-arch container builds.

### Fixed
- **Cross-arch Docker builds fail on macOS** -- Docker's legacy builder shared intermediate layer cache across `--platform` values, causing arm64 layers to be reused for x86_64 builds. Fixed by requiring Docker BuildKit (buildx), which properly includes platform in cache keys. Added buildx to `just doctor` and `scripts/bootstrap.sh`.
- **Snapshots tab shows nothing during long sessions** -- the tab called `callMcpTool('snapshots_list')` once on mount, never refreshed, and failed silently if the MCP gateway wasn't wired yet. Replaced with SQL queries against a new `snapshot_events` table in `session.db`, consistent with all other stats tabs. Each snapshot event stores a self-contained `(start_fs_event_id, stop_fs_event_id]` range for efficient per-snapshot change counts via `fs_events` cross-reference.
- **Symlink loop hangs app on startup** -- `disk_usage_bytes()` used `is_dir()` / `metadata()` which follow symlinks. A `.venv/lib64 -> lib` relative symlink in session workspaces caused infinite recursion, hanging the app at boot. Fixed to use `symlink_metadata()` throughout. Added regression tests for symlink loops, absolute escapes, and real session timing.
- **Wizard flashes briefly on app launch** -- the setup wizard appeared for one frame before settings finished loading. Added `!settingsStore.loading` guard to prevent the wizard from rendering until settings are fully resolved.
- **KVM boot path compile errors** -- `vm/boot.rs` referenced `rootfs_path()` and `virtiofs_share()` methods that were renamed. Fixed to use `disk_path()` and `virtio_fs_share()`.
- **capsem-cli missing `mut`** -- `socket.read(&resp_buf)` needed `&mut resp_buf`.

### Security
- **Symlink sandbox escape (documented)** -- guest agents can create symlinks through VirtioFS that point to arbitrary host paths (e.g., `host_root -> /`). Host-side code that follows these symlinks escapes the sandbox. `disk_usage_bytes` is fixed; 6 other code paths identified and documented in `tmp/bugs/symlink_escape.md` for hardening.

## [0.15.3] - 2026-04-02

### Fixed
- **x86_64 CI boot test fails on restricted KVM** -- GitHub Actions runners expose `/dev/kvm` but lack full VM support (no CPUID, no PIT). The boot test now probes KVM capability before attempting a VM boot and skips gracefully with a warning annotation when the runner's KVM is insufficient.

## [0.15.2] - 2026-04-02

### Fixed
- **x86_64 boot test fails on CI: KVM_CREATE_PIT2 unsupported** -- GitHub Actions runners use restricted KVM that doesn't support the legacy i8254 PIT timer. Made PIT creation optional with a warning; when unavailable, `no_timer_check` is appended to the kernel cmdline so Linux uses alternative timer sources.
- **`cross-compile` missing boot test** -- CI installs the `.deb` and boot-tests with capsem-doctor but `cross-compile` didn't. Added boot test step that runs when `/dev/kvm` is available and the target matches the native arch; skips on macOS or cross-arch builds.
- **`cross-compile` missing GNU cross-linker config** -- `.cargo/config.toml` only had musl linker entries. Added `x86_64-linux-gnu-gcc` and `aarch64-linux-gnu-gcc` for GNU targets used by the Tauri app build.

## [0.15.1] - 2026-04-01

### Fixed
- **x86_64 Linux build fails: aarch64 boot module not cfg-gated** -- `mod boot` (ARM64 kernel loading, FDT, register setup) was included unconditionally, causing 14 compile errors on x86_64 (`set_one_reg`, `REG_PC`, `KERNEL_TEXT_OFFSET` not found). Gated with `#[cfg(target_arch = "aarch64")]`.
- **Cross-compile linker error on arm64 hosts** -- building `capsem-agent` for `x86_64-unknown-linux-gnu` inside the Docker container used the native `cc` (arm64) which doesn't understand `-m64`. Added `x86_64-linux-gnu-gcc` and `aarch64-linux-gnu-gcc` cross-linker entries to `.cargo/config.toml`.
- **Multiarch dpkg conflict in cross-compile Docker image** -- `libpango1.0-dev` arm64-to-amd64 swap failed on shared `.gir` file. Added `--force-overwrite` to `swap-dev-libs.sh`.

### Changed
- **`build-assets` builds both arm64 and x86_64** -- previously only built for the native architecture, so cross-compile for the other arch always failed locally due to missing VM assets.
- **`full-test` includes `cross-compile`** -- catches platform-gating errors before tagging instead of discovering them in CI.

## [0.15.0] - 2026-04-01

### Added
- **x86_64 KVM backend** -- full KVM support for x86_64 Linux: bzImage boot protocol, identity-mapped page tables, GDT, IRQCHIP/PIT interrupt controller, CPUID passthrough, 16550 UART serial console (PIO), E820 memory map, virtio-mmio device discovery via kernel cmdline. The .deb now boots VMs on both aarch64 and x86_64.
- **Cross-compile Docker image** -- purpose-built `capsem-host-builder` image (Ubuntu 24.04) with all Tauri build deps pre-baked (system libs, Node.js 24, pnpm 10, Rust stable, cargo tools, uv). Replaces the old `rust:bookworm` ad-hoc install approach. Named volumes cache cargo registry and per-arch build artifacts between runs. New recipes: `just build-host-image`, `just clean-host-image`.
- **x86_64 release boot test** -- release pipeline now boot-tests the x86_64 .deb with capsem-doctor before publishing.
- **Compile-time KVM struct size assertions** -- `const _` assertions for all KVM ioctl structs (both aarch64 and x86_64) that fail at compile time, not runtime.
- **Kernel arch-mismatch detection** -- x86_64 boot rejects ARM64 Image kernels, aarch64 boot rejects bzImage kernels, with clear error messages instead of cryptic crashes.

### Changed
- **Container runtime: Podman replaced with Colima + Docker CLI** -- macOS now uses Colima (Apple Virtualization.framework with Rosetta) instead of Podman (libkrun). Rosetta gives near-native x86_64 container performance on Apple Silicon, making cross-arch kernel and rootfs builds much faster. All podman-specific code paths removed; standardized on `docker` CLI everywhere.

### Fixed
- **`just run` blocked on Linux** -- the `_sign` recipe hard-exited on non-macOS, preventing `just run`, `just bench`, and `just full-test` from working on Linux with KVM. Now skips codesigning on Linux.
- **x86_64 KVM boot broken: wrong entry point + missing setup header** -- the 64-bit entry point was `KERNEL_LOAD_ADDR` instead of `KERNEL_LOAD_ADDR + 0x200` (`startup_64`), causing the vCPU to execute 32-bit code in long mode and hang. Fixed by preserving bzImage setup header into boot_params and correcting the entry point.
- **`install.sh` fails on Linux** -- added OS and architecture detection so the same one-liner works on both macOS (arm64 .dmg) and Linux (x86_64/arm64 .deb via `apt install`).
- **Site docs claim macOS-only** -- updated to reflect Linux/KVM support.
- **`.cargo/config.toml` not tracked** -- broke codesigning on fresh clones. Fixed by anchoring the gitignore pattern to root.
- **Boot screen showed "No release notes available"** -- replaced Vite plugin path with `LATEST_RELEASE.md` generated by `cut-release`.
- **No error screen when VM assets fail** -- added proper error state to the boot screen with trigger-specific messages.

## [0.14.20] - 2026-03-30

### Fixed
- **CI release upload collision on per-arch VM assets** -- `gh release upload "$f#${arch}-${base}"` sets the display label, not the filename. Both arches uploaded `initrd.img`, causing a name collision. Fixed by renaming files to `${arch}-${base}` before upload.

## [0.14.19] - 2026-03-30

### Fixed
- **AI CLI version check fails in CI** -- `extract_tool_versions()` runs `gemini --version` and `codex --version` inside the built rootfs image, but `/opt/ai-clis/bin` was not on the container PATH. Added `ENV PATH` to the Dockerfile template after npm CLI install so version extraction finds the binaries.
- **`cut-release` skipped container build** -- `cut-release` depended on `just test` (unit tests only), so Dockerfile and rootfs issues were only caught by CI after tagging. Now `cut-release` depends on `full-test`, which depends on `build-assets`. The full chain (container build + unit tests + capsem-doctor + integration + bench) runs locally before any tag is created.
- **Container agent build fails writing Cargo.lock** -- source mounted `:ro` prevented cargo from generating `Cargo.lock`. Switched to symlinking source into writable `/build` dir so cargo can write the lockfile without modifying the host.

## [0.14.18] - 2026-03-30

### Changed
- **Config-driven tool version extraction** -- `extract_tool_versions()` now builds its shell script from TOML configs (`version_commands` fields) instead of a hardcoded tool list. Covers build tools (node, npm, uv, pip), apt packages (git, python3, gh, tmux, curl), Python packages (pytest, numpy, requests, pandas), and AI CLIs (claude, gemini, codex) with grouped output in tool-versions.txt. Build-time validation catches silent install failures (N/A) for enabled AI CLIs. New W013 diagnostic warns when an AI provider has a CLI but no `version_command`.

### Fixed
- **VM asset download fails with arch-prefixed release names** -- CI uploads per-arch assets as `arm64-rootfs.squashfs` etc., but `AssetManager` constructed download URLs with bare filenames (`rootfs.squashfs`), causing 404s. Added `arch_prefix` to `AssetManager` so download URLs match the release naming convention. Local storage still uses bare filenames.

## [0.14.17] - 2026-03-30

## [0.14.16] - 2026-03-30

### Fixed
- **CI test job: create stub assets for Tauri build.rs** -- the parallelization commit removed asset downloads from test, but `cargo test --workspace` compiles capsem-app whose build.rs needs assets/manifest.json. Was masked by Rust cache until tauri.conf.json change invalidated it.
- **CI create-release cleanup** -- removed stale AppImage/updater references (latest.json merge, tar.gz/sig collection), fixed SBOM attestation to cover both DMG and deb, fixed test summary to parse `cargo llvm-cov` output format, prefix per-arch VM assets (`arm64-vmlinuz`, `x86_64-vmlinuz`) to avoid upload name collisions.

## [0.14.15] - 2026-03-30

## [0.14.14] - 2026-03-30

## [0.14.13] - 2026-03-30

### Improved
- **CI pipeline parallelized (~18 min vs ~45 min)** -- test runs in parallel with build-assets and app builds. Test gates create-release but doesn't block compilation. Removed redundant cross-compile check and asset downloads from test job.

### Fixed
- **Pin Xcode 16.2 on macOS CI runners** -- Xcode 15.4's xcodebuild crashes with `Abort trap: 6` when Tauri tries to locate notarytool. Runner image update broke the default Xcode between v0.14.11 (passed) and v0.14.12 (failed). Explicitly selecting Xcode 16.2 prevents runner drift.
- **Drop AppImage from Linux releases** -- linuxdeploy cannot run on GitHub CI runners (Ubuntu 24.04 lacks FUSE2, and neither `libfuse2` nor `APPIMAGE_EXTRACT_AND_RUN=1` resolves it reliably). Linux ships `.deb` only on both arm64 and x86_64. Root cause of every v0.14.x Linux build failure (14 consecutive failed releases).
- **Container agent build: replace `file` with `ls -l`** -- `file` command is not available in `rust:slim-bookworm`. Binary verification now uses `ls -l` (coreutils); real validation (existence + non-zero size) is done in Python after the container exits.
- **Broken capsem-doctor link in docs** -- getting-started page linked to `/testing/capsem-doctor/` (removed section) instead of `/debugging/capsem-doctor/`.
- **Site description outdated** -- splash page and meta description now mention Linux (KVM) support added in v0.14.
- **Security docs sidebar ordering** -- three security pages lacked `sidebar.order`, causing alphabetical sort instead of logical progression.
- **`.dockerignore` untracked** -- Docker builds on CI or fresh clones were copying `target/`, `node_modules/`, `.venv/` into build context.

## [0.14.12] - 2026-03-29

### Fixed
- **Skip AppImage on arm64 Linux** -- linuxdeploy has no arm64 build. arm64 Linux (Chromebooks) now builds `.deb` only. x86_64 builds both deb + AppImage.

## [0.14.11] - 2026-03-29

### Fixed
- **CI Linux build: add Tauri signing keys** -- `build-app-linux` was missing `TAURI_SIGNING_PRIVATE_KEY`, causing "public key found but no private key" failure. Also collect `.tar.gz` and `.sig` updater artifacts.

### Added
- **`just cross-compile [arch]`** -- build agent binaries + full Linux app (deb + AppImage) inside a container. No host cross-compile toolchain needed. Supports arm64 and x86_64. Clean build every run (no stale volumes).
- **Container-native agent compilation** -- builds natively inside a Linux container, eliminating cross-compile cfg gating issues.
- **Multi-arch Linux release** -- CI now builds deb + AppImage for both arm64 and x86_64 via matrix job. Artifacts validated with `dpkg-deb --info` and `file`.

## [0.14.10] - 2026-03-29

### Fixed
- **CI Linux build: install xdg-utils** -- Tauri's AppImage bundler requires `xdg-open`. Added `xdg-utils` to `apt-get install` in `build-app-linux`.
- **Linux build: gate all macOS-only APIs** -- `ApfsSnapshot` (`libc::clonefile`), `AppleVzHypervisor` import in boot.rs, and `vm_integration.rs` tests were not `cfg`-gated, causing compile failures on Linux app builds. Boot now dispatches to `KvmHypervisor` on Linux.
- **Builder: apt clock skew on macOS** -- Podman/Docker VM clock drift after sleep/wake caused `apt-get update` to reject release files as "not valid yet" (exit 100). Added `Acquire::Check-Date=false` to all apt-get calls in Dockerfile templates and squashfs creation. Also added `sync_container_clock()` to auto-sync the VM clock with the host before builds.

### Added
- **Platform gating static analysis test** -- `cargo test --test platform_gating` scans all `.rs` files for ungated macOS-only and Linux-only symbols. Catches platform API issues before they reach CI.
- **Builder doctor: container clock check** -- `capsem-builder doctor` now detects clock skew between host and container VM, reports direction and magnitude, and suggests a fix.

### Improved
- **Boot timing display** -- formatted table with right-aligned columns and proportional bar chart instead of flat log lines.
- **capsem-bench refactored to package** -- split 897-line single file into `capsem_bench/` Python package with per-category modules (disk, rootfs, startup, http_bench, throughput, snapshot). Shell wrapper at `capsem-bench` preserves the same CLI interface.
- **capsem-bench JSON output** -- saved to `/tmp/capsem-benchmark.json` inside the VM instead of dumped to stdout.

### Docs
- **Site restructuring** -- moved capsem-doctor to new top-level Debugging section (with troubleshooting guide), moved benchmarking methodology to Development, added top-level Benchmarks section with current performance results (boot time, disk I/O, CLI startup, HTTP, throughput, snapshots).

## [0.14.8] - 2026-03-29

### Fixed
- **Linux build: gate all macOS-only APIs** -- `ApfsSnapshot` (`libc::clonefile`) and `AppleVzHypervisor` import in boot.rs were not `cfg`-gated, causing compile failures on Linux app builds. Boot now dispatches to `KvmHypervisor` on Linux.

## [0.14.7] - 2026-03-29

### Fixed
- **Linux build: gate `ApfsSnapshot` behind `cfg(target_os = "macos")`** -- `libc::clonefile` is macOS-only, causing compile failure on Linux app builds.

## [0.14.6] - 2026-03-28

### Fixed
- **CI build-assets restores Rust toolchain** -- v0.14.5 removed `dtolnay/rust-toolchain` when switching to just recipes, but `build-rootfs` cross-compiles the guest agent and needs the musl target installed.
- **CI build-assets builds both kernel and rootfs** -- release workflow only built rootfs, missing vmlinuz and initrd.img. Now uses `just build-kernel` and `just build-rootfs` recipes instead of reimplementing build logic.
- **CI assets/current ordering** -- moved `cp -r` after `generate_checksums` so Tauri's `build.rs` finds real files instead of a stripped symlink.

### Improved
- **`just doctor` codesigning diagnostics** -- new four-step Codesigning section checks Xcode CLTools, codesign binary, entitlements.plist, and runs a real test sign. Every `[FAIL]` line now includes a copy-pasteable fix command.
- **`bootstrap.sh` platform checks** -- macOS: validates Xcode Command Line Tools. Linux: prints informational notice about which recipes work (test, build-assets, audit) vs require macOS (run, dev, bench).
- **`_sign` recipe platform guard** -- fails immediately on Linux with actionable message instead of cryptic "codesign: command not found".
- **`run_signed.sh` error surfacing** -- codesign failures now print to stderr with a hint to run `just doctor`, instead of silently logging to `target/build.log`.
- **Developer getting-started docs** -- added platform requirements table, codesigning section with validation table, and codesign troubleshooting to the site.

## [0.14.2] - 2026-03-28

### Fixed
- **KVM virtio_blk split-borrow** -- `queue_notify` uses `.take()` pattern to avoid split-borrow when processing read/write/get_id operations.
- **CI release uses cp -r for assets/current** -- GitHub Actions artifacts strip symlinks, causing the `ln -s` approach to fail. Switched to `cp -r`.
- **Builder checksums handle current/ as directory** -- `generate_checksums()` now removes `current/` whether it's a symlink or a directory (from a prior `cp -r`).
- **Guest agent `libc::time_t` deprecation** -- replaced deprecated `libc::time_t` with `i64` in vsock_io timeout constant.

### Added
- **Developer getting-started documentation** -- full setup guide at capsem.org/development/getting-started/ covering prerequisites, container runtime setup, cross-compilation, and troubleshooting.
- **Bootstrap script** -- `scripts/bootstrap.sh` checks all required tools, installs Python and frontend deps, and runs `just doctor`.
- **`.dev-setup` sentinel** -- `just doctor` writes a `.dev-setup` file on success. All recipes (`run`, `test`, `dev`, `bench`) auto-run doctor if the sentinel is missing, preventing new developers from skipping setup.
- **`uv` check in `just doctor`** -- doctor now validates that `uv` is installed (previously missing, causing silent `build-assets` failures).
- **README prerequisites** -- "Build from source" section now lists required tools and links to the full development guide.
- **`dev-start` skill** -- quick-start pointer skill for new developers.

## [0.14.1] - 2026-03-28

### Fixed
- **Builder uses Python blake3 for checksums** -- `generate_checksums()` no longer shells out to `b3sum` CLI. Uses the `blake3` Python library directly, making the builder self-contained in CI environments.
- **Site workflow uses pnpm 10** -- pnpm 9 errored with workspace detection issues.

## [0.14.0] - 2026-03-28

### Added
- **Hypervisor abstraction layer** -- `Hypervisor`, `VmHandle`, `SerialConsole` traits in new `hypervisor` module. Platform-agnostic `VsockConnection` with lifetime anchor pattern.
- **KVM backend** -- embedded VMM using rust-vmm crates (`kvm-ioctls`, `vm-memory`, `linux-loader`). Virtio console, block, vsock (vhost-vsock), and VirtioFS (embedded FUSE server) devices. GICv3 interrupt controller, FDT generation, multi-vCPU support. ~5,500 LOC.
- **Linux app builds** -- Tauri deb and AppImage targets. macOS-only dependencies gated behind `cfg(target_os = "macos")`. CFRunLoop pumping replaced with platform-agnostic sleep on Linux.
- **capsem-builder Python package** -- config-driven build system for guest VM images. Pydantic models for all TOML configs, Jinja2 Dockerfile renderer (rootfs + kernel, multi-arch), compiler-style validation linter, Click CLI, scaffolding, BOM manifest, vulnerability audit parsing, MCP stdio server, and build doctor. 408 tests at 97% coverage.
- **capsem-builder CLI** -- `validate`, `build`, `inspect`, `init`, `add`, `audit`, `new`, `mcp`, and `doctor` commands.
- **Docker build execution** -- `capsem-builder build` produces real VM assets (kernel, initrd, rootfs squashfs). Config-driven multi-architecture output to per-arch subdirectories (`assets/arm64/`, `assets/x86_64/`).
- **Guest image TOML configs** -- declarative configs in `guest/config/` replacing hardcoded values: `build.toml` (multi-arch), `ai/*.toml` (3 providers), `packages/*.toml`, `mcp/*.toml`, `security/web.toml`, `vm/resources.toml`, `vm/environment.toml`, `kernel/defconfig.*` (arm64 + x86_64).
- **Jinja2 Dockerfile templates** -- `Dockerfile.rootfs.j2` and `Dockerfile.kernel.j2` render multi-arch Dockerfiles from TOML configs. 51 conformance tests verify parity with hand-authored Dockerfiles.
- **Settings schema (Pydantic)** -- canonical schema source with two-node-type design (GroupNode + SettingNode). JSON Schema generation, cross-language golden fixtures with Python/Rust/TypeScript conformance tests (99 tests).
- **Config-driven settings grammar** -- formalized TOML grammar with Group, Leaf, and Action node types. Settings UI fully data-driven.
- **Batch settings IPC** -- `load_settings` and `save_settings` Tauri commands replace 3 parallel calls with 1.
- **SettingsModel TypeScript class** -- pure TS class with settings logic, fully unit-tested (43 tests).
- **Snapshot benchmarks** -- `capsem-bench snapshot` measures create/list/changes/revert/delete latency at 10/100/500 file workspace sizes.
- **Direct clonefile(2) syscall** -- `ApfsSnapshot` uses `libc::clonefile()` directly. Snapshot create dropped from 50ms to 3.7ms (93% faster).
- **Hardlink-based incremental snapshots** -- `SnapshotBackend` trait with `ApfsSnapshot` (macOS) and `HardlinkSnapshot` (cross-platform) implementations.
- **FUSE ops unit tests** -- 30+ tests covering file I/O, directory operations, metadata, and adversarial cases.
- **Doctor session validation test** -- `scripts/doctor_session_test.py` verifies session.db telemetry after capsem-doctor run.
- **Container runtime resource checks** -- `just doctor` and `capsem-builder doctor` verify podman/Docker have enough memory (min 4GB).
- **Asset resolution test suite** -- 28 new tests across Rust and Python for manifest parsing, hash verification, and per-arch resolution.
- **`manifest_compat` module** -- shared `extract_hashes()` for manifest hash extraction, testable independently from `build.rs`.
- **Multi-arch asset selection** -- host app detects architecture at compile time and loads assets from per-arch subdirectories. Backward compatible with flat layout.
- **Asset pipeline documentation** -- new site page and skill documenting the build-to-boot asset flow.
- **Hypervisor architecture documentation** -- boot sequence, KVM internals, virtio device slots, VirtioFS server. Five mermaid diagrams.
- **Capsem-doctor documentation** -- 11 test categories, test infrastructure, adding new tests.
- **Corporate image support** -- custom guest configs produce different images (6 corporate image tests).
- **Persistent MCP client** -- `snapshots` CLI reuses a single fastmcp Client across all tool calls.

### Changed
- **Multi-arch release pipeline** -- CI builds arm64 and x86_64 VM assets in parallel on native runners. Per-arch attestation. Unified manifest with both architectures.
- **Release workflow adds Linux builds** -- separate `build-app-linux` job produces deb and AppImage alongside macOS DMG.
- **Site deployment fixed** -- workflow switched from npm to pnpm, Node pinned to 24.
- Apple Virtualization.framework code moved to `hypervisor/apple_vz/` behind `cfg(target_os = "macos")` gate. macOS-only dependencies now target-conditional.
- `VsockManager` replaced by `mpsc::UnboundedReceiver<VsockConnection>` returned from `Hypervisor::boot()`.
- `auto_snapshot` uses `SnapshotBackend` trait (APFS clonefile on macOS, recursive copy elsewhere).
- `notify` crate uses default features (cross-platform) instead of macOS-only `macos_fsevent`.
- Claude Code installed via native installer (`curl` instead of `npm`). Binary in `/usr/local/bin/` (chmod 555).
- Builder cleans up container images after extracting assets.
- Guest artifacts moved to `guest/artifacts/` from `images/`.
- `just build-assets` now uses capsem-builder with config-driven Dockerfile generation.
- Multi-arch cross-compilation configured for both `aarch64-unknown-linux-musl` and `x86_64-unknown-linux-musl`.
- Multi-arch diagnostics accept both `aarch64` and `x86_64`.
- Linux KVM backend promoted to Production status.
- CI coverage tracking for Linux KVM backend (`linux-unit` Codecov flag).
- Settings grammar documented with full specification.
- Settings architecture page with 7 mermaid diagrams.
- Side effect dispatch driven by metadata instead of hardcoded checks.
- MCP injection generalized for multiple servers from config.
- Site: mermaid diagram support via `astro-mermaid`.
- Skills table added to CLAUDE.md and GEMINI.md.
- `cut-release` recipe now bumps `pyproject.toml` alongside Cargo.toml and tauri.conf.json.
- Preflight checks add `uv` tool and `x86_64-unknown-linux-musl` target.
- README updated for multi-platform support (macOS + Linux), documentation links point to capsem.org.

### Fixed
- **Asset manifest format bug** -- `gen_manifest.py` produced filenames like `"arm64/vmlinuz"` instead of bare `"vmlinuz"`, causing `build.rs` to silently skip hash verification.
- **Per-arch manifest parsing** -- `Manifest::from_json()` rejected per-arch format. Added `from_json_for_arch()`.
- **apt clock skew in container builds** -- added `Acquire::Check-Valid-Until=false` to all apt calls.
- **Mock data generated from build system** -- settings and MCP data now generated from `config/defaults.json` and Rust `mcp-export` binary instead of hand-crafted mock.
- **`step` metadata field flows to UI** -- was silently dropped from generated JSON.
- **Build log contamination** -- signing and generation scripts now log to `target/build.log`.
- **Snapshot MCP no longer hangs** -- blocking I/O moved to `spawn_blocking` threads.
- **Snapshot panel now displays snapshots** -- frontend now passes `format: "json"`.
- **Vacuum preserves content sessions** -- keeps at least 25 sessions with AI activity.
- **inspect-session shows MCP tool usage** -- per-tool breakdown replaces old view.
- **Integration test Gemini API key handling** -- reads from `~/.capsem/user.toml` as fallback.
- **FS monitor debouncer lost delete events** -- replaced last-write-wins hashmap with event queue.
- **MCP snapshot tools returned unbounded JSON** -- now paginated text tables.
- **Frontend npm audit vulnerabilities** -- pinned transitive deps via pnpm overrides.

### Security
- **Safe FUSE deserialization** -- `read_struct` returns `Option<T>` with hard bounds check in all builds.
- **fsync/flush error propagation** -- returns mapped errno on failure instead of silently succeeding.
- **VirtioFS resource limits** -- file handle cap (4096), read size clamp (1MB), gather buffer limit (2MB).
- **Async VirtioFS worker thread** -- FUSE processing on dedicated thread, irqfd interrupt delivery, virtqueue memory barriers.
- **Security documentation** -- threat model overview and virtualization security pages.

### Removed
- **`images/` directory** -- legacy build files fully replaced by `guest/config/`, `guest/artifacts/`, and `src/capsem/builder/templates/`.

## [0.12.1] - 2026-03-25

### Fixed
- **Files and Snapshots tabs broken in GUI mode** -- `FsMonitor` (file watcher) and `AutoSnapshotScheduler` were only started in CLI mode, never wired into the GUI boot path. Both now start automatically when running the Tauri app.
- **Snapshot API tool name mismatch** -- frontend sent `list_snapshots`/`delete_snapshot` but backend expected `snapshots_list`/`snapshots_delete`, causing all snapshot operations to fail silently.

### Changed
- **Snapshots tab revamped** -- unified table replacing separate manual/auto sections. New columns: total changes, added, modified, deleted per snapshot. Change counts sourced from per-snapshot diffs already computed by the backend.

## [0.12.0] - 2026-03-24

### Changed
- **Decomposed god modules into focused sub-modules** -- split `main.rs` (2,722 LOC) into 7 modules (assets, boot, cli, gui, logging, session_mgmt, vsock_wiring); split `policy_config.rs` (5,999 LOC) into 8 sub-modules (types, registry, loader, presets, resolver, builder, lint, tree); split `session.rs` (1,995 LOC) into 3 sub-modules (types, index, maintenance). All existing import paths preserved via re-exports.
- **Decomposed Tauri commands into domain modules** -- split `commands.rs` (1,425 LOC) into 7 focused modules: terminal, settings, vm_state, session, mcp, logging, utilities. Shared helpers (active_vm_id, reload_all_policies) in mod.rs. All Tauri IPC paths unchanged.
- **Moved AI traffic parsing under `net/`** -- `gateway/` renamed to `net/ai_traffic/` to reflect its role as the MITM proxy's AI parsing layer. All import paths updated.
- **`net_event_counts()` returns a named struct** -- replaced bare `(usize, usize, usize)` tuple with `NetEventCounts { total, allowed, denied }` to prevent field-order bugs.

### Fixed
- **Guest agent vsock I/O no longer hangs on host stall** -- `vsock_connect()` now sets `SO_SNDTIMEO`/`SO_RCVTIMEO` (30s) on all vsock sockets. `write_all_fd` and `read_exact_fd` explicitly handle `EAGAIN` as a fatal timeout, preventing both kernel-level hangs and userspace spin-loops.
- **AsyncVsock double-close bug** -- removed manual `libc::close()` in `Drop` that double-closed the fd already owned by the inner `UnixStream`.

## [0.11.0] - 2026-03-24

### Added
- **`snapshots` CLI tool** -- in-VM command for managing workspace snapshots (`snapshots create/list/changes/history/compact/revert/delete`). Uses FastMCP client to talk to the host MCP gateway. Supports `--json` flag for machine-readable output.
- **`snapshots_history` MCP tool** -- shows all versions of a file across snapshots with sequential status (new/modified/unchanged/deleted). Accepts both relative paths and `/root/` prefixed paths.
- **`snapshots_compact` MCP tool** -- merges multiple snapshots into a single new manual snapshot. Newest-file-wins strategy. Deletes source snapshots after compaction, freeing pool slots.
- **Boot timing via vsock** -- capsem-init records per-stage durations as JSONL, PTY agent sends `BootTiming` message to host after boot. Host logs each stage with tracing and emits `boot-timing` event to frontend. Stages: squashfs, virtiofs, overlayfs, workspace, network, net_proxy, deploy, venv, agent_start.
- **Named snapshots** -- `snapshots_create` MCP tool creates named checkpoints with blake3 workspace hash. Manual snapshots are stored in a separate pool from auto snapshots and are never auto-culled.
- **Snapshot management MCP tools** -- 8 namespaced tools: `snapshots_create`, `snapshots_list`, `snapshots_changes`, `snapshots_revert`, `snapshots_delete`, `snapshots_history`, `snapshots_compact`. All prefixed with `snapshots_` to avoid collisions.
- **Snapshots UI tab** -- new tab in StatsView showing auto and manual snapshots with stat cards (total, auto, manual, available slots), delete button for manual snapshots.
- **`call_mcp_tool` Tauri command** -- generic frontend dispatcher for MCP built-in tools. Prepares for Phase 3 daemon MCP server.
- **Configurable snapshot limits** -- `settings.vm.snapshots.auto_max` (default 10), `settings.vm.snapshots.manual_max` (default 12), `settings.vm.snapshots.auto_interval` (default 300s) in the settings registry.
- **Boot time regression test** -- `test_boot_time_under_1s` fails if guest boot exceeds 1 second, catches regressions like the AI CLI copy stall.
- **XSS sanitization on guest data** -- boot timing stage names validated alphanumeric+underscore at both agent and host layers. File event paths reject NUL bytes, path traversal, control chars.
- **88 capsem-doctor MCP tests** -- comprehensive snapshot scenario coverage: modify/delete/recreate flows, copy/move, same-name-different-dirs, edge cases (deep paths, special chars, rapid snaps, 100 files), per-tool edge cases, belt-and-suspenders (MCP + CLI paths).
- Dual-pool snapshot scheduler: auto slots (ring buffer) + manual slots (named, never auto-culled). `SnapshotOrigin` enum (Auto/Manual).

### Changed
- **`snapshots_list` shows per-snapshot diffs** -- changes computed vs previous snapshot (not current workspace), showing what changed AT each snapshot. Includes `files_count` per entry.
- **`snapshots_revert` checkpoint is optional** -- auto-picks latest snapshot containing the file. Errors on "already current" (content + permissions match). Restores file permissions from snapshot.
- **All snapshots include blake3 hash** -- auto snapshots now compute workspace hash (previously manual-only).
- **Path normalization** -- all snapshot tools accept both `hello.txt` and `/root/hello.txt`.
- **AI CLIs use /opt/ai-clis directly** -- eliminated boot-time `cp -a` of hundreds of MB from squashfs to scratch disk. Boot time dropped from multi-second stall to ~530ms.
- **PATH single source of truth** -- `config/defaults.toml` defines PATH (sent via BootConfig SetEnv). Removed duplicate PATH exports from capsem-init, capsem-bashrc, capsem-doctor, profile.d.

### Fixed
- MCP file tools unavailable in GUI mode -- auto-snapshot scheduler was only wired into MCP config in CLI path, never in GUI boot path. Extracted shared `wire_auto_snapshots()` to eliminate duplication.
- `snapshots_list` changes were computed vs current workspace instead of vs previous snapshot
- `snapshots_history` status was computed vs current instead of sequentially
- `snapshots_revert` silently overwrote identical files
- File monitoring and MCP gateway no longer silently disabled when MITM proxy fails -- session DB decoupled from CA/policy loading
- Host file monitor (`FsMonitor`) was dropped immediately after creation, stopping FSEvents watcher
- `FsMonitor::emit` was not awaiting `db.write()`, so file events were never written to the session DB
- Zombie session vacuum warnings on startup
- `_init_and_call` test helper now surfaces actual MCP error messages instead of crashing with `KeyError`
- Snapshot test pool exhaustion -- autouse cleanup fixture deletes manual snapshots after each test

### Removed
- Guest `capsem-fs-watch` inotify daemon and vsock port 5005 -- host-side FSEvents monitoring fully replaces guest-side file watching

## [0.10.0] - 2026-03-21

### Added
- **VirtioFS storage mode** -- replaces tmpfs overlay + scratch disk with a single VirtioFS shared directory per session. Enables host-side file monitoring, auto-snapshots, and MCP file tools. System packages use an ext4 loopback image; workspace files in `/root` are directly visible on the host.
- **Host-side file monitoring** -- macOS FSEvents watches the VirtioFS workspace directory, replacing the in-guest `capsem-fs-watch` inotify daemon. More secure (no guest cooperation needed).
- **Rolling auto-snapshots** -- 12 APFS clone snapshots at 5-minute intervals (configurable). AI agents can list changed files and revert individual files to any checkpoint via MCP tools.
- **MCP file tools** -- `list_changed_files` (diff workspace against any auto-snapshot checkpoint) and `revert_file` (restore a file from any checkpoint, reflected immediately in guest via VirtioFS). Wired into the MCP gateway as built-in tools.
- **VirtioFS capsem-doctor tests** -- 9 new in-VM tests verifying VirtioFS root mount, ext4 loopback upper, loop device, workspace read/write, pip install, file delete+recreate
- Kernel support for VirtioFS (`CONFIG_FUSE_FS`, `CONFIG_VIRTIO_FS`) and loop devices (`CONFIG_BLK_DEV_LOOP`)
- Session schema v4: `storage_mode`, `rootfs_hash`, `rootfs_version` columns for rootfs lineage tracking
- Code coverage reporting via Codecov on PR and release CI pipelines
- OAuth credential forwarding for Claude Code and Gemini CLI -- auto-detects `~/.claude/.credentials.json` (subscription auth) and `~/.config/gcloud/application_default_credentials.json` (Google Cloud ADC), injects into guest VM at boot so agents work without API keys
- ECDSA SSH key detection (`id_ecdsa.pub`) in addition to ed25519 and RSA
- Boot screen with embedded release notes, download/boot progress, and re-run wizard button -- replaces the bare download progress overlay

### Changed
- Anthropic and OpenAI providers now enabled by default (was disabled) -- all three AI providers are allowed out of the box; corporate lockdown via `corp.toml` still overrides
- Default storage mode is now VirtioFS (block mode preserved for backward compatibility)
- Guest `capsem-fs-watch` daemon no longer launched in VirtioFS mode (host monitors instead)

### Fixed
- Frontend dependencies now auto-install on fresh clone -- `just dev`, `just ui`, `just run`, `just test`, `just doctor`, and all other recipes that need npm packages run `pnpm install --frozen-lockfile` automatically
- Setup wizard re-run now re-detects host configuration (SSH keys, API keys, OAuth credentials, GitHub tokens) instead of keeping stale values from first run

## [0.9.18] - 2026-03-21

### Fixed
- MCP server and filesystem watcher missing from release VM assets -- Claude and Gemini reported MCP as "disconnected" because `capsem-mcp-server` and `capsem-fs-watch` were never included in the release rootfs
- MCP Servers settings page showing "no VM running" permanently -- MCP data now reloads automatically when the VM finishes booting

### Added
- Build pipeline now auto-derives guest binary list from `capsem-agent/Cargo.toml` -- adding a new `[[bin]]` target is automatically picked up by `build.py`
- Rust test and preflight check verify all guest binaries appear in `Dockerfile.rootfs` and `justfile` -- prevents future binary-list drift between dev and release

## [0.9.17] - 2026-03-20

## [0.9.16] - 2026-03-20

## [0.9.15] - 2026-03-20

## [0.9.14] - 2026-03-20

### Fixed
- Download progress screen not shown on first launch: `vmStatus()` poll now returns "downloading" via app-level state, fixing the race where the event fired before the frontend subscribed
- `latest.json` missing from release artifacts, causing auto-updater `update check failed` on every boot

## [0.9.13] - 2026-03-20

### Fixed
- First-launch crash: `gui_boot_vm` called from tokio worker thread after rootfs download caused `EXC_BREAKPOINT` (`dispatch_assert_queue_fail`). VM start/stop now guarded by `is_main_thread()` check, post-download boot dispatched to main thread via `run_on_main_thread`
- Site domain references updated from `capsem.dev` (dead) to `capsem.org`

### Added
- Boot path logging: `resolve_rootfs` and `create_asset_manager` now log each location checked, version, manifest path, release count, and download status
- `cut-release` recipe: one-command version bump, changelog stamp, commit, tag, push, and CI wait

### Changed
- Release pipeline merged from two steps (build on tag push + publish via `workflow_dispatch`) into a single pipeline that builds and publishes on tag push
- `release` recipe simplified: waits for CI build (which now includes publish), no longer triggers a separate workflow
- Consolidated seven 0.9.x news posts into a single page covering 0.9.0 through 0.9.13

## [0.9.12] - 2026-03-19

### Added
- Wizard validates API keys in real-time against provider endpoints (spinner, check/X inline)
- API key detection now checks `~/.config/openai/api_key` and `~/.anthropic/api_key`
- Build verification documentation (SBOM, attestation, manifest signatures)

### Fixed
- `svelte-check` failing on `dist/` build artifacts (excluded from tsconfig)

## [0.9.11] - 2026-03-19

### Fixed
- Download progress now shown in main app view when setup wizard is skipped (returning users with existing config but missing rootfs saw a blank terminal)

### Added
- Frontend test infrastructure (vitest + @testing-library/svelte) with store and component tests

## [0.9.10] - 2026-03-19

### Fixed
- Rootfs removed from DMG bundle (was 463 MB, now ~15 MB) -- rootfs is downloaded on first launch
- Build attestation (SBOM + provenance) restored after CI refactor
- Manifest.json now signed with minisign (same key as updater artifacts)

## [0.9.3] - 2026-03-18

### Fixed
- CI codesign hang: keychain now set as default, explicitly unlocked with 1-hour timeout, and existing keychain search list preserved
- CI Node.js upgraded from 22 to 24
- CI release creation split from build: artifacts uploaded as CI artifacts, release created locally with `gh` CLI (org restricts GITHUB_TOKEN to read-only)

### Changed
- GitHub Actions upgraded to Node 24 (checkout v5, setup-node v5, upload/download-artifact v5, setup-buildx v4)
- CI workflow scoped to PRs only; site deploy scoped to main + site/ changes only

## [0.9.0] - 2026-03-18

### Added
- Persistent logging system: three-layer tracing (stdout, per-launch JSONL file, Tauri UI layer) with per-VM log files in session directories (CLI + GUI)
- Logs view in sidebar with live event stream, boot timeline visualization, session history browser, level filtering, and auto-scroll
- Per-launch log files (`~/.capsem/logs/<timestamp>.jsonl`) with automatic 7-day cleanup
- Per-VM session logs (`~/.capsem/sessions/<id>/capsem.log`) with structured JSONL events for both CLI and GUI modes
- `load_session_log` and `list_log_sessions` Tauri commands for historical log access
- Error messages now included in `vm-state-changed` events for all error states
- Boot timeline state transitions emitted as structured tracing events
- Integration test verifies log file creation, JSONL validity, level filtering, boot timeline events, and timestamp format
- App auto-update: `createUpdaterArtifacts` enabled so CI produces `.tar.gz` + `.sig` updater files and `latest.json` -- the built-in updater now works
- `app.auto_update` setting (default: true) to gate the startup update check, with "Check for Updates" button in Settings > App
- Multi-version asset manifest (`manifest.json`) replaces single-version `B3SUMS` -- supports multiple release versions, merge across releases, and future checkpoint restore
- Version-scoped asset directories (`~/.capsem/assets/v{version}/`) with automatic migration from flat layout and cleanup of old versions
- `pinned.json` support for keeping specific asset versions during cleanup (for future checkpointing)
- `scripts/gen_manifest.py` for manifest generation in justfile and build.py
- First-run setup wizard -- 6-step guided configuration (Welcome, Security, AI Providers, Repositories, MCP Servers, All Set) that runs while the VM image downloads in the background
- Host config auto-detection -- wizard scans ~/.gitconfig, ~/.ssh/*.pub, environment variables, and `gh auth token` to pre-populate settings with detected values
- SSH public key setting (`vm.environment.ssh.public_key`) -- injected as /root/.ssh/authorized_keys in the guest VM at boot
- Re-run setup wizard button in Settings > VM to revisit configuration without overwriting existing settings
- Resumable asset downloads -- partial .tmp files are preserved across app restarts and continued via HTTP Range headers instead of re-downloading from scratch
- Security presets ("Medium" and "High") -- one-click security profiles selectable from Settings > Security
- Automatic migration of old setting IDs (`web.*`, `registry.*`) to new `security.*` namespace -- existing user.toml and corp.toml files work without manual changes
- `fetch_http` now supports `format=markdown` (new default) -- converts HTML to clean markdown preserving headings, links, lists, bold/italic, and code blocks
- Wikipedia (`en.wikipedia.org`, `*.wikipedia.org`) added to default allow list for MCP HTTP tools
- Auto-detect latest stable kernel version from kernel.org during `just build-assets`
- User-editable bashrc and tmux.conf as file settings in Settings > VM > Shell
- Filetype-aware syntax highlighting for file settings (bash, conf, json)
- Documentation URLs for API key settings (links to provider console/settings pages)
- Repositories section in settings with git identity (author name/email) for VM commits
- Personal access token settings for GitHub and GitLab (enables git push over HTTPS via .git-credentials)
- GitLab as a repository provider with domain allow/block and token support
- Added `tmux` and `gh` to the default rootfs for terminal multiplexing and GitHub CLI support
- Token prefix hints in settings UI -- apikey inputs show expected format (e.g., `ghp_...`, `sk-ant-...`) with a warning if the entered value doesn't match
- `GH_TOKEN` / `GITHUB_TOKEN` env vars injected in VM when GitHub token is configured, enabling `gh` CLI without `gh auth login`
- `GITLAB_TOKEN` env var injected in VM when GitLab token is configured

### Changed
- CI release workflow now accumulates manifest.json across releases and uploads it alongside rootfs
- `_pack-initrd` regenerates manifest.json on every `just run` via `scripts/gen_manifest.py`
- `build.rs` reads hashes from manifest.json (preferred) with B3SUMS fallback
- Settings restructured: "Web" and "Package Registries" merged under new "Security" top-level section with "Web", "Services > Search Engines", and "Services > Package Registries" sub-groups
- MCP gateway rewritten to use rmcp (official Rust MCP SDK) -- replaces hand-rolled JSON-RPC/SSE client with proper Streamable HTTP transport, automatic pagination, and typed tool/resource/prompt routing
- Upgraded reqwest from 0.12 to 0.13
- MCP server UI redesigned: collapsible server cards with URL/auth config, "verified"/"definition changed" status labels
- Tool origin telemetry expanded from 2 values (native/mcp) to 3 values (native/mcp_proxy/local)
- Auto-detected stdio MCP servers from Claude/Gemini settings shown with unsupported warning instead of silently dropped
- `just install` now runs validation gates only (doctor + full-test); `.app` bundling is CI-only
- Missing API key warnings now appear in the group header when collapsed, with a "Get key" link
- GitHub moved from "Package Registries" to "Repositories" section
- `registry.github.*` setting IDs renamed to `repository.github.*`
- Package Registries description updated to "Package manager registries"

### Removed
- Stdio bridge for MCP servers (`stdio_bridge.rs`) -- replaced by HTTP client

### Fixed
- MCP server bearer token auth sent double "Bearer" prefix (`Bearer Bearer <token>`), causing 401 from authenticated servers like deps.dev
- Tool calls no longer double-counted in stats -- MCP-proxied tool_calls (origin=mcp_proxy) filtered from native counts across all 6 tool queries
- Native tool response preview now displayed in unified tool list (was hardcoded NULL, now joined from tool_responses via call_id)
- Non-text content blocks (tool_reference, image) in Anthropic tool results now produce meaningful preview instead of empty string
- OpenAI multipart tool result content now extracted correctly
- `check_session.py` tool response matching fixed -- joins on call_id only (tool responses arrive in next model call with different model_call_id)
- MCP server now visible in `claude mcp list` -- was injected into wrong file (`settings.json` instead of `.claude.json`)
- Codex CLI MCP server config added (`~/.codex/config.toml`) -- was missing entirely
- Disabling an AI provider now takes effect immediately on existing keep-alive connections (policy was previously snapshot per-connection, not per-request, so in-flight HTTP/1.1 connections continued to allow requests after the provider was toggled off)
- MCP tool_responses no longer double-counted in multi-turn conversations (request parsers now extract only trailing tool results instead of full history)
- MCP call previews no longer truncated at 200 chars (removed hard truncation; 256KB cap_field safety net remains)
- `fetch_http` paginate now UTF-8 safe -- uses `floor_char_boundary` to avoid panics on multi-byte content (emoji, Cyrillic, CJK, etc.)
- `fetch_http` on subpaths (e.g. `elie.net/about`) now returns full page content -- replaced `tl` HTML parser with `scraper` (html5ever) which correctly handles minified/complex HTML
- `fetch_http` format default changed from `content` to `markdown` for better AI agent consumption
- MCP byte tracking: `bytes_sent`/`bytes_received` columns added to mcp_calls for full I/O auditability
- Builtin MCP tool HTTP requests now emit net_events with `conn_type=mcp_builtin` for network audit visibility
- Guest process_name resolution uses `/proc/{pid}/cmdline` (real binary name) instead of `/proc/{pid}/comm` (thread name), fixing "MainThread" attribution
- Gemini tool call_ids now include a counter suffix to distinguish multiple calls to the same function
- Claude Code no longer warns about missing `/root/.local/bin` directory (created at boot after scratch disk mount)
- tmux now has a clean minimal config: mouse support, no escape delay, proper 256-color/truecolor, high scrollback
- tmux sessions can now find `gemini` and other npm-global binaries (PATH was lost when tmux started a login shell that reset it via `/etc/profile`)
- `gh auth status` injection test no longer fails with fake test tokens (test now verifies token detection, not authentication)
- Git authentication in VM: switched from `.netrc` to `.git-credentials` + `credential.helper=store` so `git push` works out of the box
- "Get one" links in settings now open in host browser via `tauri-plugin-opener` (previously broken in Tauri webview)

### Security
- Kernel hardening: heap zeroing (`INIT_ON_ALLOC`), SLUB freelist hardening, page allocator randomization, KPTI (`UNMAP_KERNEL_AT_EL0`), ARM64 BTI + PAC, `HARDENED_USERCOPY`, seccomp filter, cmdline hardening (`init_on_alloc=1 slab_nomerge page_alloc.shuffle=1`)
- Git credential tokens now reject `@` and `:` characters (in addition to newlines) to prevent URL injection in `.git-credentials`

## [0.8.8] - 2026-03-07

### Added
- Proxy throughput benchmark (`capsem-bench throughput`): downloads 100 MB through the full MITM proxy pipeline and reports MB/s — baseline ~35 MB/s on Apple Silicon
- `capsem-bench` is now repacked into the initrd on every `just run`, so changes to the benchmark script take effect immediately without a full rootfs rebuild
- `ash-speed.hetzner.com` added to the default network allow list and integration test config for the throughput benchmark
- Rust integration test `mitm_proxy_download_throughput` (in `crates/capsem-core/tests/mitm_integration.rs`): validates 100 MB download through the proxy at the host level; marked `#[ignore]` so it runs only on demand
- `test_proxy_download_throughput` in `capsem-doctor` (`test_network.py`): in-VM Layer 7 test verifying end-to-end proxy throughput; skips gracefully if the speed-test domain is not in the allow list
- `docs/performance.md`: documents all benchmark modes, baseline numbers, proxy data path, and domain allow list setup
- `just run` now kills any existing Capsem instance before booting, preventing a stale GUI window from appearing alongside a CLI run
- Notarization credential verification in CI preflight job: validates Apple API key against `notarytool history` before spending time on build-assets and tests
- Notarization preflight check in `scripts/preflight.sh`: verifies `.p8` key, API Key ID, Issuer ID, and runs a live `notarytool history` test

### Fixed
- `capsem-init` now aborts boot (kernel panic) if the tmpfs mount for the overlay upper layer fails, preventing a silent degraded boot where writes land on the initramfs instead of the intended tmpfs
- `capsem-init` now creates `/mnt/b` before mounting tmpfs on it (missing `mkdir -p` caused the tmpfs mount to fail with "No such file or directory" on fresh initrds)
- CI release no longer hangs on first-time notarization: `--skip-stapling` flag submits for notarization without waiting for Apple's response (first-time notarization can take hours)

### Security
- Boot invariant enforcement: `capsem-init` fatal-exits on tmpfs or overlayfs mount failure rather than continuing with a wrong upper layer; preflight check verifies this abort is present

## [0.8.4] - 2026-03-06

### Added
- `apt-get install` support inside the VM: overlayfs mounts with `redirect_dir=on,metacopy=on` (requires `CONFIG_OVERLAY_FS_REDIRECT_DIR`, `CONFIG_OVERLAY_FS_INDEX`, `CONFIG_TMPFS_XATTR` in kernel config), enabling dpkg directory renames without EXDEV errors. Packages installed in a session are gone after shutdown (ephemeral model preserved).
- `apt-packages.txt`: declarative list of system packages baked into the rootfs — edit and `just build-assets` to add/remove packages.
- Debian apt sources switched to HTTPS (`deb.debian.org`, `security.debian.org`) in `Dockerfile.rootfs`; both domains added to the default network allow list so the MITM proxy forwards them.
- Package lists pre-populated at rootfs build time so `apt-get install` works inside a running VM without a prior `apt-get update`.
- `force-unsafe-io` dpkg config in `capsem-init`: skips redundant fsyncs on overlayfs.
- Claude Code installed as a native binary (downloaded directly from Anthropic's GCS release bucket) instead of via npm, removing the Node.js dependency for the Claude CLI.
- Ephemeral model preflight check (`check_ephemeral_model` in `scripts/preflight.sh`): statically verifies `capsem-init` never skips `mke2fs` and never uses the scratch disk as overlay upper layer.
- Ephemeral model end-to-end test (`check_persistence` in `scripts/integration_test.py`): boots two consecutive VMs, writes a sentinel file in the first, and asserts it is absent in the second.

### Changed
- `images/README.md` developer section now documents how to add packages from all sources (apt, pip, npm, runtime) with copy-paste examples.

### Security
- Ephemeral model invariants documented in `CLAUDE.md` and enforced by preflight + integration test to prevent accidental persistence anti-patterns from being introduced.

### Added
- `just doctor` command: checks all required dev tools, container runtime (docker/podman), Rust targets, and cargo tools are installed
- Release preflight checks (`scripts/preflight.sh`): validates Apple certificate format, keychain import, and base64 sync before CI release
- `scripts/fix_p12_legacy.sh`: converts OpenSSL 3.x p12 files to legacy 3DES format macOS Keychain accepts
- CI preflight job in release workflow: fails fast on certificate/credential issues before slow build jobs

### Changed
- Release builds are CI-only (removed `just release`); push a `vX.Y.Z` tag to trigger `.github/workflows/release.yaml`
- `just build-assets`, `just install` now run `just doctor` first to catch missing tools early
- `just run`, `just full-test`, `just bench` now verify VM assets exist before proceeding

### Fixed
- Apple certificate import in CI: re-exported p12 with legacy 3DES/SHA1 encryption (macOS rejects OpenSSL 3.x default PBES2/AES-256-CBC with misleading "wrong password" error)

### Added
- Configuration overrides via `CAPSEM_USER_CONFIG` and `CAPSEM_CORP_CONFIG` environment variables to support isolated testing and CI.
- Dedicated integration test configurations (`config/integration-test-user.toml` and `config/integration-test-corp.toml`) for reproducible end-to-end validation.
- Thin DMG distribution: rootfs excluded from app bundle, downloaded on first launch via asset manager with blake3 hash verification
- Asset manager (`asset_manager.rs`): checks, downloads, and verifies VM assets from GitHub Releases with streaming progress
- Download progress UI: full-screen progress bar shown during first-launch rootfs download
- CLI download support: `capsem "command"` auto-downloads rootfs with stderr progress if missing
- Squashfs support: boot_vm accepts both rootfs.squashfs (new) and rootfs.img (legacy) formats
- Release workflow uploads rootfs.squashfs as separate GitHub Release asset alongside the thin DMG
- Onboarding plan (`docs/onboarding.md`): first-launch wizard scope for credentials, MCP config, and guided setup
- AI stats tab: unified model analytics with stat cards (total calls, tokens, cost, models), model usage chart, token breakdown, cost-over-time, and provider distribution
- `StatCards.svelte` reusable component for stat card rows across all analytics tabs
- Chart color system (`css-var.ts`): provider hue families, model color assignment, file action colors, server palette -- all using oklch() constants (no CSS var lookups)
- LayerChart v2 API documentation (`docs/libs/layercharts.md`) for LLM-friendly chart development

### Changed
- Asset resolution in macOS app bundle now searches multiple paths in `Resources` (including nested Tauri v2 paths) for better reliability.
- Integration test isolated from host user settings and correctly maps `GOOGLE_API_KEY` to `GEMINI_API_KEY` for the internal VM CLI.
- Tauri asset bundling now uses a flat map to prevent deeply nested `_up_/_up_/assets` structures in the final package.
- `just dev` now automatically passes `CAPSEM_ASSETS_DIR` to ensure the VM boots during local development.
- Stats "Models" tab renamed to "Model" (AITab.svelte replaces ModelsTab.svelte)
- Network, Tools, and Files stats tabs rebuilt with LayerChart v2 simplified chart components (BarChart, PieChart) replacing raw D3/Chart.js primitives
- SQL queries expanded: per-model token/cost breakdowns, provider distribution, cost-over-time, tool success rates, file action breakdowns
- Wizard auto-show on first run removed (setup wizard is still accessible from sidebar)

### Fixed
- Integration test SQLite connection robustness improved by using plain paths instead of URI formatting.
- Anthropic API tracking: MITM proxy now strips `accept-encoding` for AI providers so SSE streaming responses arrive uncompressed. This fixes the issue where Anthropic usage and cost were recorded as NULL.
- AI telemetry pollution: `model_call` records are now strictly filtered to valid LLM API paths (e.g., `/v1/messages`), preventing metadata endpoints from generating spurious NULL traces.
- Fallback model extraction: Added regex-based fallback to extract the model name from truncated JSON request bodies when the 64KB preview buffer limit is reached.
- fs-watch telemetry drops: Fixed a race condition during VM boot where early vsock connections (like `fs-watch`) were dropped by the host before the terminal/control handshake completed.
- `scripts/run_signed.sh` now correctly refreshes the binary signature via `touch` after re-signing with entitlements.
- Build prerequisites documentation updated with `b3sum`, `tauri-cli`, and `musl-cross` toolchain requirements.
- capsem-doctor PATH: writable bin dirs (`/root/.npm-global/bin`, `/root/.local/bin`) now included so AI CLIs and npm globals are found
- Gemini CLI settings.json: added `homeDirectoryWarningDismissed` and `sessionRetention` to suppress first-run prompts
- AI provider domain-blocked test now skips when the provider is explicitly enabled by policy
- Integration test handles compressed session DBs (`session.db.gz`) after vacuum
- Integration test accepts `vacuumed` as valid terminal session status

### Changed
- capsem-doctor and diagnostics are now repacked into the initrd, so changes take effect with `just run` instead of requiring `just build-assets`
- `just full-test` now includes initrd repack to ensure latest guest code is deployed

### Added
- `config_lint()` function: validates all settings (JSON files, number ranges, choices, API key format, nul bytes, URL format) with clear human-readable error messages displayed inline in the settings UI
- `SettingsNode` tree API: backend exposes the TOML settings hierarchy as a nested tree with resolved values at leaves, replacing the flat list for UI rendering
- `get_settings_tree` and `lint_config` Tauri commands for the new tree-based settings UI
- UI debug skill (`.claude/skills/UI_debug.md`): comprehensive Chrome DevTools MCP-based visual verification checklist for the settings UI

### Changed
- File settings now store path and content together as `{ path, content }` objects instead of keeping `guest_path` in metadata -- path is the source of truth for MCP injection and guest config generation
- Guest config file permissions tightened from 0o644 to 0o600 (owner-only) since settings files may contain API keys
- JSON validation uses zero-allocation `serde::de::IgnoredAny` instead of parsing into `serde_json::Value`
- Settings UI fully rewritten: left nav and section content are auto-generated from the TOML settings tree. Adding new categories or settings to `defaults.toml` automatically appears in the UI with no frontend code changes. Replaced 6 hardcoded section components (ProvidersSection, McpSection, NetworkPolicySection, EnvironmentSection, ResourcesSection, AppearanceSection) and their icon imports with a single generic recursive renderer (`SettingsSection.svelte`)
- SubMenu component now supports optional icons (icon-less items render label only)

### Security
- File setting paths are validated: must start with `/`, must not contain `..`, warns on unusual paths not under `/root/` or `/etc/`

### Added
- File analytics section: stat cards, action breakdown chart, events-over-time chart, and searchable event table for filesystem activity tracking
- Setup wizard hook: auto-detects first run (no API keys configured) and shows a welcome view with provider setup shortcut
- Reveal/hide toggle for API key and password fields in provider settings
- Range hints (min/max) shown below number inputs in VM resource and appearance settings
- Dropdown rendering for settings with predefined choices

### Changed
- Analytics data separation: Models and MCP analytics sections now exclusively query session.db; cross-session data (sessions over time, avg calls per session) moved to Dashboard
- "Session stats" button in terminal footer now navigates to session-level AI analytics instead of cross-session dashboard
- MCP analytics stat cards expanded from 2 (total + avg/session) to 4 (total, allowed, warned, denied)

### Security
- main.db `query_raw` now enforces `PRAGMA query_only = ON` around user SQL execution, preventing write-through via SQL injection (e.g., `SELECT 1; DROP TABLE sessions`) in the `query_db` IPC command
- Read-only enforcement tests for both session.db (`DbReader`) and main.db (`SessionIndex`) query paths: INSERT, CREATE TABLE, DROP TABLE, and semicolon injection all verified to fail at the SQLite level

### Changed
- Unified SQL gateway: `query_db` IPC command now supports both session.db and main.db via `db` parameter ("session" or "main"), with bind parameter support via `params` array. Replaced 11 per-query Tauri commands (net_events, get_model_calls, get_traces, get_trace_detail, get_mcp_calls, get_file_events, get_session_history, get_global_stats, get_top_providers, get_top_tools, get_top_mcp_tools) with a single `query_db` gateway
- Frontend queries now run through `db.ts` (unified query layer) instead of individual api.ts wrappers, using parameterized SQL from `sql.ts`
- Removed `ModelCallResponse` Rust wrapper struct (was only needed for the deleted `get_model_calls` command)
- Justfile streamlined from 23 recipes to 13 public + 5 internal helpers: `run` now auto-repacks initrd (replaces separate `repack`), `test` includes cross-compile + frontend check (replaces `check`), `full-test` combines capsem-doctor + integration test + bench (replaces `smoke-test`/`integration-test`/`preflight`), `build-assets` replaces `build`, `inspect-session` replaces `check-session`, `release` now produces a DMG at `target/release/Capsem.dmg`
- Removed recipes: `compile`, `sign`, `frontend`, `rebuild`, `repack`, `repack-initrd`, `ensure-tools`, `smoke-test`, `integration-test`, `preflight` (functionality preserved as internal `_`-prefixed helpers or merged into public recipes)

### Fixed
- 12 compilation warnings eliminated across 3 files: dead code warnings in `capsem-fs-watch` cross-platform helpers (blanket `#![cfg_attr(not(target_os = "linux"), allow(dead_code))]`), unused `SessionStats` import in commands.rs, and test-only `close()` method gated with `#[cfg(test)]`
- Test fixture updated from integration test session with full pipeline coverage: denied net events, deleted file events, positive cost estimates, `origin` column on tool_calls
- `fixture_top_domains_non_empty` test assertion fixed: `count >= allowed + denied` accounts for error events that are counted in total but not in allowed/denied buckets
- `query_raw_real_type` test now validates REAL type serialization without requiring positive cost values in the fixture
- Integration test now exercises denied net events (curl to blocked domain), deleted file events (create + rm), cost estimation assertions, and tool origin verification (34 checks, up from 28)

### Added
- Session DB lifecycle management: sessions now progress through running -> stopped -> vacuumed -> terminated states. After a session stops, its DB is checkpointed, vacuumed, and gzip-compressed (`session.db.gz`), then WAL/SHM files are removed. Terminated sessions retain their main.db audit trail record even after disk artifacts are deleted.
- `vm.terminated_retention_days` setting (default 365): controls how long terminated session records are kept in main.db before permanent purging
- Periodic main.db WAL checkpoint every 5 minutes to prevent unbounded WAL growth
- DbWriter now checkpoints WAL on clean shutdown (drop)
- Startup vacuum recovery: any sessions that stopped but were not vacuumed (e.g. due to crash) are automatically compressed on next app launch
- `check-session` script now handles compressed session DBs (auto-decompresses `.gz` files)
- End-to-end integration test (`just integration-test`): boots a real VM, exercises all 6 telemetry pipelines (fs_events, net_events, mcp_calls, model_calls, tool_calls, main.db rollup), runs capsem-doctor MCP tests, asks Gemini to write a poem, and verifies every event type is correctly logged in the session DB
- Release preflight gates (`just preflight`): unit tests, cross-compile, capsem-doctor smoke test, integration test, and benchmarks must all pass before `just release` or `just install` builds the app
- In-VM benchmark recipe (`just bench`): standalone entry point for capsem-bench (disk I/O, rootfs read, CLI startup, HTTP latency)
- Tool origin tracking: `tool_calls` table now records `origin` ("native" or "mcp") and `mcp_call_id` columns to distinguish model built-in tools from MCP gateway tools
- `check-session` data quality warnings: flags model_calls with NULL model, tokens, or request_body_preview
- `check-session` tool lifecycle section: shows origin breakdown and MCP call correlation
- Diagnostic logging when streaming model_calls complete with NULL model, tokens, or preview fields

### Fixed
- Session backfill now looks for `session.db` instead of the old `info.db` filename
- MITM proxy AI telemetry: model name, token counts, and request body preview were NULL for all model_calls when `log_bodies` was disabled. The proxy now always captures up to 64KB of AI provider request/response bodies for metadata parsing regardless of the `log_bodies` setting.
- MITM proxy model resolution: added fallback chain (request body -> SSE stream -> response JSON -> URL path) so model name is extracted even for providers that put it in the URL (e.g. Gemini `/v1beta/models/gemini-2.5-flash:generateContent`)
- MITM proxy stream detection: streaming flag now detected from URL path (`streamGenerateContent` vs `generateContent`) instead of unreliable request body parsing
- MITM proxy non-streaming usage: token counts now parsed from JSON response body when SSE stream parsing yields no usage metadata
- MITM proxy tool origin: tool_calls now use `tool_origin()` for correct "native" vs "mcp" classification instead of hardcoding "native"
- MITM proxy tool responses: tool_result entries from AI request bodies are now correctly extracted (previously always empty when body capture was disabled)
- MITM proxy non-streaming response parsing now handles gzip-compressed response bodies (upstream often sends Content-Encoding: gzip)
- MITM proxy no longer creates model_call records for HEAD requests (connectivity probes from AI CLIs have no body/model/tokens)
- Telemetry event pipeline silently dropping events under burst load: `try_write()` in MITM proxy and fs-watch handler failed without logging when the 256-slot DB channel was full (e.g. during `npm install`). Replaced with async `write().await` via `tokio::spawn` for backpressure, and bumped channel capacity from 256 to 4096.
- MCP builtin tools (`fetch_http`, `grep_http`, `http_headers`) returning empty responses: `capsem-mcp-server` used `SHUT_RDWR` after stdin closed, killing in-flight gateway responses before they could be read back. Changed to `SHUT_WR` (half-close) so the reader thread collects all responses before shutdown.
- MCP `fetch_http` and `grep_http` now reject binary content (images, PDFs, audio, video, etc.) with a clear error instead of returning garbled text or UTF-8 decode errors
- MCP tools now reject non-HTTP schemes (`file://`, `ftp://`, `data:`, etc.) before any network request is made
- MCP `grep_http` now rejects empty patterns instead of matching every line

### Changed
- Settings registry migrated from hardcoded Rust to `config/defaults.toml` (TOML-based, embedded at compile time). Setting definitions use `String` fields instead of `&'static str`. No user-facing behavior change.
- Session culling now marks sessions as "terminated" instead of deleting main.db rows, preserving the audit trail. Old terminated records are purged after `vm.terminated_retention_days` (default 365 days).
- Schema migrated from v2 to v3 (additive: new `compressed_size_bytes` and `vacuumed_at` columns on sessions table)
- MCP built-in tools exposed without `builtin__` prefix: models now see `fetch_http`, `grep_http`, `http_headers` instead of `builtin__fetch_http` etc. -- cleaner tool names for AI agents
- MCP built-in tool descriptions rewritten with full documentation: HTML extraction behavior, output format, pagination, domain policy enforcement, and error conditions
- Per-session analytics (Traffic, AI Models, MCP views) now use `queryDb(sql)` with SQL constants instead of dedicated Tauri commands -- reduces Rust boilerplate and gives the frontend more flexibility
- Network store rewritten: individual SQL queries replace monolithic `getSessionStats()` call, adding SQL-driven avg latency, method distribution, and process distribution
- Dashboard session detail no longer shows file event count (global dashboard should only show global data)
- Rootfs switched from 2GB ext4 to 382MB squashfs (zstd, 64K blocks) -- 81% smaller for DMG distribution
- Boot sequence uses overlayfs (immutable squashfs lower + ephemeral tmpfs upper) -- writes to system paths silently go to tmpfs
- Test fixture (`data/fixtures/test.db`) is now captured from real sessions instead of generated by a Python script
- `just update-fixture <path>` replaces `just gen-test-db`: copies a real session DB, scrubs API keys, and syncs to `frontend/public/fixtures/`

### Removed
- Dead AI gateway server (`gateway/server.rs`, 997 lines): axum HTTP server on vsock:5004 was never wired up in main.rs. All AI traffic goes through the MITM proxy on vsock:5002. `extract_model_from_path`, `parse_non_streaming_usage`, and `tool_origin` helpers moved to `gateway/provider.rs` and `gateway/events.rs` where the MITM proxy can use them.
- `VSOCK_PORT_AI_GATEWAY` constant (port 5004) -- unused, never wired up
- `GatewayConfig` struct -- only used by the dead server
- `gateway_integration.rs` test file -- tests for the dead server
- `axum` dependency from capsem-core
- `get_session_stats`, `get_mcp_stats`, `get_file_stats` Tauri IPC commands -- replaced by frontend SQL via `queryDb()`
- `SessionStatsResponse` struct from commands.rs and `SessionStatsResponse`, `SessionStats`, `McpCallStats`, `FileEventStats` types from frontend
- `SessionsSection.svelte` -- orphan component never imported by AnalyticsView
- `data/fixtures/generate_test_db.py` -- synthetic data generator replaced by real session captures

### Added
- `sql.ts`: centralized SQL query constants for all per-session analytics (13 queries covering net stats, domains, time buckets, provider usage, tool usage, model stats, MCP stats, file stats, latency, method/process distribution)
- `queryOne<T>()` and `queryAll<T>()` typed helpers in `api.ts` for running SQL against the active session's info.db
- Analytics data architecture documented in `docs/architecture.md` (two-database design, data flow, query strategy, polling patterns)
- Frontend development skill file (`.claude/skills/frontend.md`)
- In-VM filesystem watcher (`capsem-fs-watch`): inotify-based daemon streams file create/modify/delete events to the host over vsock:5005 for real-time file activity telemetry
- `fs_events` audit table in `capsem-logger`: records every file operation with timestamp, action, path, and size
- `FileEvent` type with `WriteOp::FileEvent` variant and reader queries (`recent_file_events`, `search_file_events`, `file_event_stats`)
- `get_file_events` and `get_file_stats` Tauri IPC commands for the frontend
- Files view in frontend: summary cards (total/created/modified/deleted), searchable event table with action badges, 2s polling
- Files sidebar navigation item with document icon between Sessions and MCP Tools
- Mock file event data (13 entries) for browser dev mode
- MCP gateway wired to vsock:5003: host now accepts MCP connections from guest agents, fixing Gemini CLI hang on startup
- Built-in HTTP tools: `fetch_http`, `grep_http`, `http_headers` -- AI agents can fetch web content, search pages, and inspect headers from within the sandbox, all checked against domain policy
- MCP domain policy hot-reload: changing network settings in the UI immediately updates which domains built-in HTTP tools can access
- `capsem-doctor` MCP tests: 6 new in-VM diagnostic tests verifying MCP binary, initialize handshake, tools/list, allowed/blocked fetch, and fastmcp availability
- `fastmcp` Python package in guest rootfs for building custom MCP servers inside the VM
- MCP Proxy Gateway: AI agents in the guest VM can now use host-side MCP tools transparently via a unified `capsem-mcp-server` binary injected at boot
- `capsem-mcp-server` guest binary: lightweight NDJSON-over-vsock bridge (~90 lines) relaying MCP JSON-RPC between agents and the host gateway on vsock:5003
- MCP gateway host module (`capsem-core::mcp`): types, policy engine, stdio bridge, server manager, and vsock gateway for routing tool calls to host-side MCP servers
- Namespaced MCP tools: tools from multiple servers are exposed as `{server}__{tool}` to prevent collisions (e.g., `github__search_repos`, `slack__send_message`)
- Per-tool dynamic policy: each MCP tool can be set to allow (forward normally), warn (forward + flag), or block (return JSON-RPC error) with hot-reload via `Arc<RwLock<Arc<McpPolicy>>>`
- MCP server auto-detection: reads existing MCP configs from `~/.claude/settings.json` and `~/.gemini/settings.json` at boot
- `mcp_calls` audit table in `capsem-logger`: full telemetry for every MCP tool call (server, method, tool, decision, duration, error)
- `McpCall` event type with `WriteOp::McpCall` variant and `insert_mcp_call()` writer method
- `DbReader` MCP queries: `recent_mcp_calls(limit, search)` with text search across server/method/tool, `mcp_call_stats()` aggregation (total, allowed, denied, warned, by-server breakdown)
- Schema migration: existing databases automatically gain the `mcp_calls` table on open
- `get_mcp_calls` and `get_mcp_stats` Tauri IPC commands for the frontend
- `inject_capsem_mcp_server()`: automatically merges `{"capsem": {"command": "/run/capsem-mcp-server"}}` into Claude and Gemini settings.json at boot, preserving user-provided MCP server entries
- MCP Tools view in frontend: summary cards (total/warned/denied), per-server breakdown, searchable call log table with decision badges
- MCP sidebar navigation item with layers icon between Sessions and Settings
- Mock MCP data: 6 sample calls across 3 servers (github, filesystem, slack) for browser dev mode
- Generic usage details tracking: token breakdowns (cache_read, thinking) stored as extensible `usage_details` JSON map instead of individual columns -- zero schema changes when adding new token types
- OpenAI Responses API (`/v1/responses`) streaming support: parses `response.created`, `response.output_text.delta`, `response.reasoning_summary_text.delta`, `response.function_call_arguments.delta`, `response.output_item.added/done`, and `response.completed` SSE events
- OpenAI cached token parsing from `prompt_tokens_details.cached_tokens` and reasoning token parsing from `completion_tokens_details.reasoning_tokens`
- Gemini thinking token parsing from `thoughtsTokenCount` (was parsed but unused)
- Non-streaming response parsing: gateway now extracts model, input/output tokens, and usage details from non-streaming JSON responses (all three providers), enabling cost estimation and token tracking for non-streamed API calls
- Cache and thinking token counts shown in session stats and trace detail UI

### Changed
- `capsem-proto` simplified: removed `McpGuestMsg`/`McpHostMsg` enums and encode/decode functions in favor of raw NDJSON passthrough (less code, better performance)
- `capsem-init` deploys `capsem-mcp-server` from initrd (with rootfs fallback)
- `just repack` cross-compiles and bundles `capsem-mcp-server` alongside pty-agent and net-proxy
- Sessions view: trace detail panel now shows MCP tool calls inline with model calls
- Token details stored as flexible `usage_details TEXT` JSON column replacing individual token columns -- single schema handles all current and future token breakdowns
- Cost estimation accounts for cached tokens: `cache_read` tokens subtracted from effective input before pricing calculation
- Pricing function signature simplified: accepts `&BTreeMap<String, u64>` usage details map instead of individual token parameters

### Fixed
- MCP gateway no longer sends a JSON-RPC response for `notifications/initialized` (it's a notification, not a request) -- fixes protocol confusion in some MCP clients
- Token metrics double-counted in trace detail view when a model call had both request and response tool entries -- now only the first row per call shows metrics
- Non-streaming API responses (no `stream: true`) recorded with null tokens and $0.00 cost -- now properly parsed for all providers
- HEAD connectivity checks from AI CLIs (Claude, Gemini) no longer create empty model_call rows -- filtered at the gateway level

## [0.8.0] - 2026-02-28

### Added
- `capsem-logger` crate: unified audit database with dedicated writer thread, replacing three separate SQLite databases (`WebDb`, `GatewayDb`, `AiDb`) with a single `session.db` per VM session
- Dedicated writer thread using `tokio::sync::mpsc` channel with block-then-drain batching (up to 128 ops per transaction), eliminating `spawn_blocking` + `Arc<Mutex<>>` contention
- `DbWriter` / `DbReader` API: async writes via channel, read-only WAL concurrent readers, typed `WriteOp` enum for debuggable operations
- Unified schema: `net_events` (all HTTPS connections), `model_calls` (denormalized request+response), `tool_calls`, `tool_responses` tables in a single DB file
- Inline SSE event parsing in the MITM proxy for AI provider traffic (Anthropic, OpenAI, Google Gemini)
- Provider-agnostic LLM event types (`LlmEvent`, `StreamSummary`) with `collect_summary()` for structured audit logging
- Hand-rolled SSE wire-format parser with chunk-boundary-safe state machine (no crate dependency)
- Provider-specific SSE stream parsers: Anthropic (interleaved content blocks, thinking), OpenAI Chat Completions (tool calls, content filter), Google Gemini (complete events, synthetic call IDs)
- Request body parser extracting model, stream flag, system prompt preview, message/tool counts, and tool_result entries for tool call lifecycle linking
- `AiResponseBody`: hyper Body wrapper that does SSE parsing inline during `poll_frame` with zero added latency
- AI provider domain detection (`api.anthropic.com`, `api.openai.com`, `generativelanguage.googleapis.com`) in the MITM proxy
- API key suffix extraction (last 4 chars, Stripe-style) from `x-api-key` and `Authorization: Bearer` headers
- Per-call cost tracking: gateway estimates USD cost using bundled model pricing data from pydantic/genai-prices
- Fuzzy model name matching for pricing: unknown model variants (date-stamped, custom-suffixed) now resolve to the correct pricing via progressive suffix stripping and longest-prefix fallback instead of silently returning $0.00
- Trace ID assignment in MITM proxy: multi-turn tool-use conversations are linked by shared trace IDs, enabling the Sessions view to render conversation spans
- SQL-driven session statistics: counts, token usage, cost, domain distribution, and time-bucketed charts all computed via SQLite queries
- New Tauri IPC commands: `get_session_stats` (full aggregate dashboard data), `get_model_calls` (model call history with search)
- LLM Usage section in Sessions view: API call count, input/output tokens, estimated cost, per-provider breakdown, model calls table, tool usage badges
- SQL-powered search in Network view: debounced search queries hit SQLite LIKE instead of client-side filtering
- `just update_prices` recipe to refresh bundled model pricing data
- `capsem-bench` in-VM performance benchmark tool: disk I/O (sequential read/write, random 4K IOPS) and HTTP throughput (ab-style concurrent requests with latency percentiles)
- `capsem-bench rootfs` benchmark: sequential and random 4K read performance on the read-only rootfs
- `capsem-bench startup` benchmark: cold-start latency for python3, node, claude, gemini, and codex CLIs (3 runs, min/mean/max)
- Rich table formatting for all capsem-bench output (replaces manual text formatting)
- Configurable VM CPU cores via `vm.cpu_count` setting (1-8, default 4)
- Configurable VM RAM via `vm.ram_gb` setting (1-16 GB, default 4 GB)
- 1 GB swap file on scratch disk for better memory pressure handling
- Search category in settings: Google Search (on by default), Perplexity, and Firecrawl toggles with domain-level policy
- Custom allow/block domain lists (`network.custom_allow`, `network.custom_block`) for user-defined domain rules
- Active Policy debug panel in Network view: collapsible section showing allowed/blocked domain lists, default action, corp managed status, and policy conflicts
- Policy conflict detection: domains appearing in both allow and block lists are flagged in the Network view

### Changed
- Terminal UI overhaul: borderless look with 10px padding, thin styled scrollbar, theme-matching background (full black in dark mode)
- Removed bottom status bar; session stats (tokens, tools, cost, VM status) now displayed inline below the terminal
- Sidebar reorganized: Console + Sessions in nav, Settings/theme/collapse in footer
- Network view moved into Settings as a collapsible "Network Statistics" section
- Sessions panel (charts, spans, analytics) now accessible from sidebar nav
- Session Statistics section added to bottom of Settings view
- MITM proxy and gateway server use `DbWriter` channel instead of `spawn_blocking` + `Arc<Mutex<>>` for all database writes
- Session telemetry stored in `session.db` (was `info.db`)
- VM Disk Performance Overhaul: 2M+ IOPS for random 4K reads (~8 GB/s) and ~20x speedup in random write throughput
- Network Proxy Overhaul: replaced synchronous thread-per-connection guest proxy with Tokio-based async implementation
- Structural Latency Elimination: `TCP_NODELAY` on both guest and host proxies, reducing proxy overhead to the physical network floor (~40ms median RTT)
- VM CPU default increased from 2 to 4 cores
- VM RAM default increased from 512 MB to 4 GB
- Scratch disk default increased from 8 GB to 16 GB
- Node.js V8 heap cap raised from 512 MB to 2 GB to match higher RAM
- Network store is now SQL-driven: counts and charts read from `get_session_stats` instead of counting JS arrays
- Session info response expanded with LLM metrics (model call count, tokens, tool calls, estimated cost)
- `net_events` command accepts optional `search` parameter for SQL-backed filtering
- `get_session_info` is now async with `spawn_blocking` for proper non-blocking DB access
- Rootfs disk caching mode changed from `Automatic` to `Cached` for aggressive host page cache retention on the read-only disk
- Host-side disk settings: enabled host-level caching (`VZDiskImageCachingMode::Cached`) and disabled synchronization barriers (`VZDiskImageSynchronizationMode::None`)
- Guest-side kernel tuning: `capsem-init` now sets I/O scheduler to `none`, `read_ahead_kb` to 4096, and `nr_requests` to 256 for all VirtIO devices
- Filesystem optimizations: `noatime,nodiratime,noload` mount options for rootfs and scratch disks
- Scratch disk format optimization: `mke2fs -m 0` to reclaim reserved root blocks
- `elie.net` moved from a Package Registry toggle to the default custom allowed domains list
- `network.log_bodies` and `network.max_body_capture` moved from Network to VM category
- Session settings (`session.retention_days`, `session.max_sessions`, `session.max_disk_gb`) moved from Session to VM category
- Mock data now mirrors the full backend settings registry (~35 settings across 7 categories)
- Settings view categories displayed in fixed order: AI Providers, Search, Package Registries, Network, Guest Environment, Appearance, VM
- Settings view categories collapsed by default (click to expand)
- Network view: allowed/blocked domain lists are now separate collapsible groups within Active Policy

### Fixed
- VM status indicator now shows correct color (blue for running, yellow for booting) instead of defaulting to no color due to state casing mismatch between Rust and frontend
- MITM proxy now assigns trace IDs and estimates costs for AI model calls, enabling Sessions view to display LLM statistics
- Fixture-dependent test assertions in capsem-logger replaced with data-agnostic checks to prevent breakage on fixture regeneration
- Benign "error shutting down connection" warnings in the host proxy logs are now filtered

### Removed
- Dead `gateway/audit.rs` module (839 lines, never compiled) superseded by capsem-logger
- `GatewayDb` (redundant flat table, replaced by `model_calls` in unified schema)
- `AiDb` (normalized 4-table schema, merged into `capsem-logger`)
- `WebDb` (replaced by `net_events` table in unified schema)
- `StreamAccumulator` (unused since `AiResponseBody` replaced it)
- `registry.elie.allow` setting (replaced by `network.custom_allow` default)
- `registry.debian.allow` setting (rootfs is read-only, packages cannot be installed at runtime)
- `domainlist` setting type from frontend (custom allow/block use standard `text` type with ID-based chip rendering)

### Security
- Terminal input batching thread now caps coalesced buffer at 64 KB, preventing unbounded memory growth if the IPC channel is flooded faster than the inner try_recv loop can drain
- Sanitize HTTP headers in telemetry logs: allowlisted headers (content-type, host, server, etc.) stored verbatim; all others (authorization, x-api-key, cookies) have values replaced with BLAKE3 hash prefix (`hash:<12-char-hex>`) to prevent credential leakage while preserving header presence and enabling correlation

## [0.7.0] - 2026-02-26

### Changed
- Terminal output uses poll-based binary IPC (`terminal_poll`) instead of JSON event emission, eliminating ~4x serialization overhead
- Terminal input batched with 5ms window (up to 4KB) to reduce IPC round-trips per keystroke
- Vsock read buffer increased from 8KB to 64KB and mpsc channel from 256 to 8192 entries
- CoalesceBuffer defaults changed from 10ms/64KB to 5ms/10MB for higher throughput
- Terminal output queue with 64-entry backpressure cap prevents OOM when frontend stops polling

## [0.6.0] - 2026-02-26

### Added
- Guest dev environment: `pip install`, `uv pip install`, `npm install -g` all work out of the box on the read-only rootfs
- Python venv auto-activated at boot with `--system-site-packages` (packages install to `/root/.venv`)
- `pip` and `python` aliased to `uv pip` and `uv run python` (faster, no root warning)
- AI CLIs (claude, gemini, codex) installed to writable scratch disk at boot so auto-update works
- npm global prefix redirected to writable `/root/.npm-global` for `npm install -g`
- Pre-installed Python packages declared in `images/requirements.txt`: numpy, requests, httpx, pandas, scipy, scikit-learn, matplotlib, pillow, pyyaml, beautifulsoup4, lxml, tqdm, rich
- Pre-installed npm globals declared in `images/npm-globals.txt` (AI CLIs)
- Login banner shows AI tool status: ready (blue), no API key (purple), disabled by policy (purple)
- Host injects `CAPSEM_ANTHROPIC_ALLOWED`, `CAPSEM_OPENAI_ALLOWED`, `CAPSEM_GOOGLE_ALLOWED` env vars at boot
- Configurable login banner (`images/banner.txt`) and random developer tips (`images/tips.txt`)
- Removed PEP 668 EXTERNALLY-MANAGED marker from rootfs
- `just build` upgrades all tools to latest: apt packages, pip, npm, node, nvm, uv
- Claude Code yolo mode: `~/.claude/settings.json` with `bypassPermissions` + `CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC=1`, and `~/.claude.json` state file to skip onboarding, trust dialogs, and keybinding prompts
- Gemini CLI yolo mode: `~/.gemini/settings.json` with `approvalMode: "yolo"`, telemetry/auto-updates disabled, folder trust disabled, and Gemini's own sandbox disabled (capsem provides the sandbox)
- Metadata-driven env var injection: settings declare `env_vars` in metadata instead of hardcoded mappings
- Built-in guest environment settings (`guest.shell.term`, `guest.shell.home`, `guest.shell.path`, `guest.shell.lang`, `guest.tls.ca_bundle`) configurable via user.toml and corp.toml
- Individual vsock boot messages (`SetEnv`, `FileWrite`, `BootConfigDone`) replacing single `BootConfig` frame, eliminating the 8KB frame size limit for boot configuration
- Guest boot log at `/var/log/capsem-boot.log` recording clock sync, env vars, file writes, and handshake status
- Per-service domain settings (`ai.*.domains`) with user-editable comma-separated domain patterns
- AI provider API key injection into guest VM environment variables (`ANTHROPIC_API_KEY`, `OPENAI_API_KEY`, `GEMINI_API_KEY`)
- Google AI (`ai.google.allow`) enabled by default for out-of-the-box Gemini CLI support
- Per-session unique IDs (`YYYYMMDD-HHMMSS-XXXX`) replacing hardcoded "default"/"cli" VM IDs
- Session index database (`~/.capsem/sessions/main.db`) tracking metadata across sessions
- `get_session_info` and `get_session_history` Tauri IPC commands for the Sessions view
- Session retention settings: `session.retention_days`, `session.max_sessions`, `session.max_disk_gb`
- Age-based, count-based, and disk-based session culling at startup
- Migration from legacy `session.json` files to `main.db` on startup
- Request count snapshotting (`count_by_decision`) when sessions stop
- Svelte 5 + Tailwind v4 + DaisyUI v5 frontend framework replacing vanilla JS
- Single Svelte island architecture: `<App client:only="svelte" />` in Astro shell
- Sidebar navigation with collapsible icon rail (Console, Sessions, Network, Settings)
- Network events view with filterable table, expandable rows showing headers/body
- Settings view with categorized editor, type-aware inputs, corp lock indicators
- Sessions view with VM state timeline from state machine history
- Terminal view wrapping existing xterm.js web component with Tauri event wiring
- Status bar showing VM state indicator, HTTPS call count, allowed/denied stats
- Light/dark theme toggle with localStorage persistence and system preference fallback
- Svelte 5 rune stores for VM state, network events, settings, theme, and sidebar
- TypeScript IPC layer (`types.ts` + `api.ts`) with typed wrappers for all Tauri commands
- `svelte-check` added to `just check` and `pnpm run check` pipelines
- Generic typed settings system replacing TOML-based policy config -- each setting has ID, type, category, default, metadata, and optional `enabled_by` parent toggle
- Per-setting corp override: corporate settings (`/etc/capsem/corp.toml`) lock individual settings, not entire sections
- Setting metadata with domain patterns, HTTP method permissions, numeric bounds, and text choices
- `get_settings` and `update_setting` Tauri IPC commands for the settings UI
- Settings architecture documentation in `docs/architecture.md`
- Policy override security documentation in `docs/security.md`

### Changed
- Increased vsock MAX_FRAME_SIZE from 8KB to 256KB for generous boot payloads
- Boot handshake protocol now sends env vars and files as individual messages instead of a single `BootConfig` payload
- Sessions view redesigned: current session info cards, network analytics, session history table (replaced CPU/memory/binary stats that VZ doesn't expose)
- Per-session telemetry renamed from `web.db` to `info.db` (legacy `web.db` still read for backward compatibility)
- Each VM boot creates a fresh telemetry database, eliminating stale request carryover between sessions
- Network policy replaced with simplified rule-based system: per-domain read/write verb control with defaults (GET allowed, POST denied)
- Configuration format changed from section-based TOML (`[network]`, `[guest]`, `[vm]`) to flat settings map (`[settings]` with dotted keys like `"registry.github.allow"`)
- Domain allow/block lists now derived from setting toggles and their metadata (e.g., toggling `registry.github.allow` controls `github.com`, `*.github.com`, `*.githubusercontent.com`)
- AI provider domains moved from explicit block-list to disabled-by-default toggles with domain metadata
- Guest environment variables stored as `guest.env.*` settings instead of `[guest].env` table
- VM settings (scratch disk size) stored as `vm.scratch_disk_size_gb` setting instead of `[vm]` section
- Removed SNI-based pre-TLS policy check; all policy enforcement at HTTP level
- Removed generativelanguage.googleapis.com from block-list (Gemini API testing)
- MITM proxy streams request and response bodies instead of buffering in memory
- Upstream TLS config cached per-VM instead of recreated per-request
- Default `log_bodies` changed from false to true

### Fixed
- Denied domains now record HTTP method, path, and status in telemetry (TLS handshake completes, denial at HTTP 403 level)
- Guest receives proper HTTP 403 response with reason for denied requests instead of cryptic TLS connection error
- "Invalid Date" in Session/Network views: timestamps now serialize as epoch seconds instead of SystemTime objects
- Legacy "default"/"cli" sessions migrated as "crashed" instead of carrying over stale "running" status
- web.db now records query string, matched rule, and 403 status for denied requests
- Upstream connection failures record error reason in telemetry

### Removed
- `get_vm_stats` command and `VmStats`/`BinaryCall` types (VZ framework doesn't expose guest metrics)
- Hardcoded `DEFAULT_VM_ID` constant -- replaced by dynamic session IDs
- `session.json` files -- replaced by `main.db` session index (migrated automatically)
- SNI parser module (`sni_parser.rs`) -- domain extracted from TLS handshake instead

### Security
- Env var sanitization: reject keys containing `=` or NUL bytes, values containing NUL (prevents agent crash / kernel panic)
- Blocked env var list: LD_PRELOAD, LD_LIBRARY_PATH, IFS, BASH_ENV, and other dangerous variables rejected during boot
- Boot allocation caps: max 128 env vars, 64 files, 10MB total file data
- FileWrite path traversal protection: reject paths containing `..`
- Defense-in-depth: guest agent validates env vars and file paths independently of host
- Body size limit (100MB) prevents OOM from malicious guest payloads
- Replaced unsafe borrow_fd with safe fd cloning
- Corp-locked settings cannot be modified by user, enforced at the merge level

## [0.5.0] - 2026-02-25

### Added
- Ephemeral scratch disk for `/root` workspace (8GB default, configurable via `[vm].scratch_disk_size_gb` in `~/.capsem/user.toml`)
- Per-session directory structure (`~/.capsem/sessions/<vm_id>/`) with session metadata (`session.json`)
- Stale session cleanup on startup: leftover scratch images deleted, orphaned "running" sessions marked as "crashed"
- Block device identifiers (`rootfs`, `scratch`) for stable device naming in the guest (`/dev/disk/by-id/virtio-*`)
- uv fast Python package installer available to guest AI agents

### Changed
- Guest `/root` workspace now uses ext4 on a virtio block device instead of RAM-backed tmpfs, increasing usable space from ~512MB to 8GB+
- Upgraded Node.js from Debian's v18 to v24 LTS via nvm
- Replaced pip3 with uv for in-VM Python package management (certifi, pytest)

### Fixed
- gemini CLI crashing with `SyntaxError: Invalid regular expression flags` due to Node.js 18 lacking the 'v' regex flag
- AI CLI smoke test was too lenient -- now verifies `--help` runs without JS runtime errors instead of only checking for signal crashes

## [0.4.0] - 2026-02-25

### Added
- Host-side state machine (`HostState`) with validated transitions, timing history, and structured perf logging
- Per-state message validation: host validates both outbound and inbound vsock control messages against lifecycle stage
- New Tauri IPC commands for Svelte UI: `get_guest_config`, `get_network_policy`, `set_guest_env`, `remove_guest_env`, `get_vm_state`
- Structured `vm-state-changed` events with JSON payloads (state + trigger) instead of plain strings
- Protocol documentation (`docs/protocol.md`): wire format, message reference, state machine diagrams, boot handshake, security invariants
- Zero-trust guest binary security rule documented in `docs/security.md`
- `write_policy_file()` for TOML serialization of user.toml changes from the UI
- MITM transparent proxy: full HTTP inspection (method, path, status code, headers, body preview) for all HTTPS traffic from the guest VM
- Static Capsem MITM CA certificate (ECDSA P-256, 100-year validity) baked into the guest rootfs trust store
- On-demand domain certificate minting with RwLock cache for TLS termination
- HTTP-level policy engine: method+path rules on top of domain allow/block lists (`[[network.rules]]` in user.toml)
- Extended telemetry: `web.db` now records HTTP method, path, status code, request/response headers, and body previews
- CA trust environment variables (`REQUESTS_CA_BUNDLE`, `NODE_EXTRA_CA_CERTS`, `SSL_CERT_FILE`) injected via BootConfig
- certifi CA bundle patching in rootfs for Python SDK compatibility (requests, openai, anthropic)
- Schema migration for existing `web.db` databases (adds new columns without data loss)
- Clock synchronization -- guest VM clock is set from host at boot time (fixes TLS cert validation, git, curl)
- Environment variable injection via vsock boot config (`BootConfig`/`BootReady` handshake)
- `[guest]` section in `user.toml` for custom guest environment variables
- `--env KEY=VALUE` CLI flag for one-off env injection (`capsem --env FOO=bar echo $FOO`)
- `capsem-proto` crate -- shared protocol types for host/guest communication
- Clock sync diagnostic test in `capsem-doctor`
- In-VM diagnostic test suite expanded: MITM CA trust chain tests (system store, certifi, curl without -k, Python urllib), network edge cases (HTTP port 80, non-443 ports, direct IP, AI provider blocking, multi-domain DNS), process integrity (pty-agent, dnsmasq, no systemd/sshd/cron), deeper kernel hardening (no modules loaded, no debugfs, no IPv6, no swap, no kallsyms, ro cmdline), environment validation (TERM, HOME, PATH, arch, kernel version, mount points), and 14 additional unix utility checks
- `just test` recipe runs workspace tests with coverage summary via `cargo-llvm-cov`
- `just ensure-tools` auto-installs `cargo-llvm-cov` and `llvm-tools-preview` on fresh clones
- Air-gapped networking: `curl https://elie.net` now works from inside the guest VM
- Host-side SNI proxy inspects TLS ClientHello, enforces domain allow-list, and bridges to the real internet
- Domain policy engine with allow-list, block-list, and wildcard pattern matching (`*.github.com`)
- Configurable domain policy via `~/.capsem/user.toml` and `/etc/capsem/corp.toml` (corp overrides user)
- Per-session `web.db` (SQLite) recording every HTTPS connection attempt for auditing
- Guest-side `capsem-net-proxy` binary: TCP-to-vsock relay for transparent HTTPS proxying
- Default developer allow-list: GitHub, npm, PyPI, crates.io, Debian repos, elie.net
- AI provider domain blocking at SNI level (api.anthropic.com, api.openai.com, googleapis.com)
- `net_events` Tauri command for querying recent network events from the frontend
- Per-VM network isolation: each VM gets its own policy, web.db, and connection handlers

### Changed
- SNI proxy replaced by MITM transparent proxy for full HTTP-level traffic inspection and policy enforcement
- Domain policy (`DomainPolicy`) wrapped by `HttpPolicy` which adds method+path rules while preserving backward compatibility
- `load_merged_policy()` now returns `HttpPolicy` instead of `DomainPolicy`
- HTTPS proxy connections spawn as async tokio tasks instead of blocking threads
- Control protocol split into disjoint `HostToGuest`/`GuestToHost` enums with reserved variants for file operations and lifecycle management
- Guest agent boot sequence restructured: vsock connects first, receives clock + env from host before forking bash
- Max control frame size bumped from 4KB to 8KB to accommodate env var payloads
- `just build`, `just repack`, and `just check` now run tests with coverage as a gate before proceeding
- Kernel now includes IP stack + netfilter (CONFIG_INET=y, iptables REDIRECT) for air-gapped networking
- Rootfs includes iproute2, iptables, and dnsmasq for guest network setup
- capsem-init sets up dummy0 NIC, fake DNS, and iptables rules at boot
- `just repack` now includes `capsem-net-proxy` alongside `capsem-pty-agent`
- Refactored VM smoke test into pytest-based diagnostic suite (`capsem-doctor`)
- Split tests into focused modules: sandbox security, utilities, runtimes, AI CLIs, workflows
- Added sandbox security tests (rootfs read-only, no kernel modules, no /dev/mem, network isolation, no setuid/setgid)
- Added Python and Node.js execution tests (actual code runs, not just version checks)
- Added AI CLI sandbox verification (binaries execute without crashing)
- Network sandbox tests updated: verify air-gapped proxy (allowed/denied domains) instead of raw network block

### Fixed
- MITM proxy TLS handshake failure: rustls crypto provider was not initialized, causing silent panics on every proxy connection
- MITM proxy now uses explicit `builder_with_provider()` instead of relying on global crypto state, eliminating the class of bug entirely
- `just build` failure: Dockerfile.rootfs could not find CA cert (build context was `images/`, cert was in `config/`)
- `just build` failure: certifi not installed when CA bundle patching step runs
- Kernel `CONFIG_KALLSYMS=n` was silently ignored because the option requires `CONFIG_EXPERT=y` to be configurable
- Kernel cmdline now includes `ro` for read-only rootfs mount
- `just smoke-test` now returns non-zero exit code on test failures
- In-VM diagnostic test fixes: `/proc/modules` absent is valid (CONFIG_MODULES=n), bash test checks availability not current shell, CA bundle tests grep base64 instead of DER-encoded CN, Python TLS test verifies handshake not HTTP status

### Deprecated
- `sni_proxy::handle_connection` -- use `mitm_proxy::handle_connection` for full HTTP inspection

### Security
- `CONFIG_EXPERT=y` in kernel defconfig ensures all hardening options (KALLSYMS=n, MODULES=n, etc.) are respected by `make olddefconfig`
- Kernel symbol table (`/proc/kallsyms`) now empty -- eliminates kernel ASLR bypass vector
- MITM proxy enables full HTTP audit trail: every request method, path, status code, and headers are logged to web.db
- HTTP-level policy rules allow fine-grained control (e.g., allow GET but deny POST to specific paths)
- Default-deny domain policy: only explicitly allowed domains are reachable from the guest
- No DNS leaves the VM: all resolution is faked to a local IP
- Corporate policy (`/etc/capsem/corp.toml`) overrides user settings for enterprise lockdown
- Per-VM isolation prevents cross-VM network interference

## [0.3.0] - 2026-02-24

### Added
- PTY-over-vsock terminal communication replacing serial broadcast channel
- Guest PTY agent (`capsem-pty-agent`) for high-throughput terminal I/O with full PTY support
- Terminal resize support (`stty size` reflects window dimensions)
- vsock control channel with MessagePack framing for structured commands (resize, heartbeat)
- Kernel vsock support (`CONFIG_VSOCKETS`, `CONFIG_VIRTIO_VSOCKETS`)
- Multi-VM-ready app state architecture (`vm_id`-keyed `HashMap`)
- Output coalescing (10ms/64KB) to prevent frontend IPC saturation
- Boot-time command execution via vsock (`Exec`/`ExecDone` control messages)
- CLI mode (`capsem "command"`) routes commands through vsock PTY agent with exit code propagation

### Changed
- Terminal input now routes through vsock when connected, falling back to serial
- Guest init script (`capsem-init`) launches PTY agent instead of direct bash/setsid
- CLI mode rewritten from serial I/O to vsock-based execution with proper exit codes
- `just repack` now cross-compiles and bundles the PTY agent into the initrd for fast iteration
- Serial forwarding stops once vsock connects, eliminating duplicate output
- M5 redesigned: zero-trust network boundaries with SNI proxy domain filtering, AI provider domain blocking, and real-time file telemetry via fanotify
- M6 redesigned: active AI audit gateway with 9-stage event lifecycle (PII scrubbing, tool call interception, secret scanning), replaces passive proxy approach
- M7 redesigned: hybrid MCP architecture -- local tools run sandboxed in-VM, remote tools route through host gateway with credential injection
- M8 redesigned: per-session audit databases with zstd-compressed blobs, OverlayFS config write-back, enterprise observability (Prometheus, OTLP, corporate policy via MDM)

### Fixed
- Shell prompt not appearing after command execution (stderr was redirected to /dev/hvc0, sending readline prompt through buffered serial path instead of vsock PTY)

### Security
- Removed serial console fallback: missing PTY agent halts boot instead of opening an unprotected shell
- Replaced scattered `unsafe { File::from_raw_fd }` + `mem::forget` with centralized `borrow_fd` helper using `ManuallyDrop`
- Added T13 threat (AI Traffic Audit Bypass) documenting the enforcement chain: iptables -> vsock bridge -> SNI proxy -> audit gateway
- Updated T3 (Data Exfiltration) with fswatch telemetry, PII engine, and secret scanning mitigations
- Updated T5 (Credential Theft) with gateway key injection and PII scrubbing on model calls
- Updated T11 (Network Exfiltration) with AI domain blocking at SNI proxy and 9-stage lifecycle enforcement
- Added Corporate Security Profile section with MDM-distributable policy.toml for enterprise deployments

## [0.2.0] - 2026-02-24

### Added
- blake3 integrity checking of VM assets (B3SUMS)
- Kernel hardening configuration for guest VM
- Proper terminal signal handling (setsid for controlling tty)
- Boot-up tracing spans for timing diagnostics
- Utility helpers for VM lifecycle

### Fixed
- Utility module fixes

## [0.1.0] - 2026-02-23

### Added
- Native macOS app using Tauri 2.0 with Astro frontend
- Linux VM sandboxing via Apple Virtualization.framework
- Virtio serial console with bidirectional I/O (xterm.js <-> guest /dev/hvc0)
- Custom capsem-init (PID 1) with chroot and setsid
- Docker/Podman-based VM asset build pipeline (kernel, initrd, rootfs)
- `just` task runner workflows (build, repack, dev, run, release, install)
- Codesigning with com.apple.security.virtualization entitlement
- xterm.js terminal web component
- Tauri auto-updater plugin integration

### Changed
- Complete rewrite from Python proxy architecture (v1) to native Rust/Tauri VM app
