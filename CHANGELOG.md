# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Changed
- **Inline `#[cfg(test)] mod tests { ... }` blocks extracted to sibling
  `tests.rs` files across four hot-path modules.** Pure mechanical code
  motion -- each block moved verbatim (dedented one level) into a new
  `tests.rs` alongside its parent, and the parent now declares
  `#[cfg(test)] mod tests;`. Test visibility is identical to inline;
  zero behavior change. Before / after line counts:
  `capsem-core/src/net/policy_config/mod.rs` 4,364 → 38
  (tests.rs 4,325); `capsem-core/src/session/mod.rs` 1,230 → 12
  (tests.rs 1,217); `capsem-proto/src/lib.rs` 1,722 → 403
  (tests.rs 1,318); `capsem-core/src/hypervisor/kvm/virtio_fs/mod.rs`
  1,218 → 313 (tests.rs 904). Net: ~7,800 lines of test code no longer
  sits between file open and production definitions, which was the
  single biggest friction when agents or humans navigate these files.
  Full suite green: `cargo test -p capsem-core` 1,464 pass;
  `cargo test -p capsem-proto` 144 pass; clippy clean on every
  touched crate.

### Changed
- **`capsem-service`: `PersistentRegistry` extracted from `main.rs` into its
  own `capsem_service::registry` module.** Pure code motion: `PersistentVmEntry`,
  `PersistentRegistryData`, and `PersistentRegistry` with its eight methods
  (`load`, `save`, `register`, `unregister`, `get`, `get_mut`, `list`,
  `contains`) now live in `crates/capsem-service/src/registry.rs` and are
  re-imported by `main.rs`. Seven registry-only tests move with the types;
  seven new tests drive the module to 100% line coverage (corrupt-JSON
  load, missing-file load, `get` / `get_mut` / `contains` miss paths,
  `list` iteration, atomic temp-rename on save). Moved tests switch from
  ad-hoc `env::temp_dir()` + manual cleanup to `tempfile::TempDir` to
  eliminate cross-run path collisions. `main.rs` drops from 4,855 to
  4,563 lines. No behavior change; first step of the
  `capsem-service-split-followup` sprint (T1).

### Added
- **Unit-coverage lifts on six files to recover the `unit` codecov flag.**
  Workspace line coverage had regressed below the 80% unit target after
  several service/CLI mains grew. Added 55 new tests across
  `capsem-core/src/vm/terminal.rs` (was 0%, now ~100%),
  `capsem-core/src/net/policy_config/types.rs` (was 45%),
  `capsem-core/src/net/policy_config/corp_provision.rs` (was 40%; tests
  exercise install/read/refresh paths against a tempdir plus the
  stale-TTL guard), `capsem-core/src/net/policy_config/loader.rs` (was
  79%, new coverage on `parse_mcp_section` / `parse_mcp_section_json` /
  `validate_setting_value`), `capsem-logger/src/writer.rs` (ExecEvent,
  McpCall, AuditEvent roundtrips plus `try_write` and `:memory:` reader
  rejection), and `capsem-process/src/helpers.rs`
  (`query_max_fs_event_id`). Workspace unit coverage moved from 76.79%
  to 77.49% lines / 80.78% regions / 78.85% functions; the remaining
  gap to 80% lines is concentrated in `capsem-service/src/main.rs`,
  `capsem/src/main.rs`, and `capsem-process/src/vsock.rs` and is tracked
  by `sprints/capsem-service-split-followup/`.

### Fixed
- **Flaky env-var race in `policy_config/loader.rs` tests.**
  `user_config_path_override_via_env`, `corp_config_path_override_via_env`,
  and `corp_config_path_default` each mutated `CAPSEM_USER_CONFIG` /
  `CAPSEM_CORP_CONFIG` at process scope, so parallel cargo-test execution
  could observe one test's `set_var` while another asserted the env was
  unset. Merged the three into one `env_var_path_resolution` test that
  snapshots and restores prior values. No production behavior change --
  these env vars are set once at startup in prod.

### Changed
- **Marketing tagline updated to "The fastest way to ship with AI securely."**
  Replaces the previous "Native AI Agent Security" / "Sandbox AI coding agents..."
  phrasing across the marketing site hero, footer, and meta tags; the docs site
  splash and description; the workspace `Cargo.toml` package description; the
  `capsem --help` about line; the `capsem setup` welcome; the macOS `.pkg`
  installer welcome page; and the README header.

### Added
- **Integration-test fixtures archive their tmp_dir on failure.** When any
  test that spins up a capsem-service via `tests/helpers/service.py::ServiceInstance`,
  the e2e `RealService`, or the MCP conftest's `_start_capsem_service`
  fails, the fixture teardown now copies its
  `/var/folders/.../capsem-test-*` directory into
  `test-artifacts/<timestamp>-<worker>-<nodeid>/<tmp-basename>/` before
  the usual `shutil.rmtree`, so `service.log`, `logs/gateway.log`,
  `sessions/<vm>/process.log`, `sessions/<vm>/serial.log`, and
  `sessions/<vm>/session.db` all survive for post-mortem. The failing
  test's stderr prints `ARTIFACT: preserved <src> -> <dest>`; Unix
  sockets/FIFOs are skipped because `shutil.copy2` can't read them.
  `test-artifacts/` is gitignored. `skills/dev-debugging/SKILL.md` and
  `skills/dev-bug-review/SKILL.md` document the layout and when to read
  it -- first stop for "VM didn't boot" and "exec timed out" failures
  in the integration suite, where log availability used to depend on
  whether macOS had culled `/var/folders` yet.

### Changed
- **Temp VM names now suffix `-tmp` and never collide on the first word.**
  Auto-generated names went from `tmp-<adj>-<noun>` to `<adj>-<noun>-tmp`
  (e.g. `brave-falcon-tmp`) so every tab/list entry leads with a
  distinctive adjective instead of the same `tmp-` prefix. The generator
  also consults the live instance table and skips any adjective that
  matches the leading segment of an existing VM name, so two concurrent
  temp VMs never share a first word. The adjective and noun rosters were
  expanded (68 adjectives, 85 nouns) to keep the avoid-set useful even
  under heavy concurrency, and the generator falls back to a random
  adjective if every one is already claimed. `scripts/integration_test.py`
  was updated to match on the `-tmp` suffix instead of the prefix.

### Changed
- **Throughput benchmark target moved off `ash-speed.hetzner.com`.** The
  `capsem-bench throughput` command, the in-VM `test_proxy_download_throughput`
  diagnostic, and the host-side `mitm_proxy_download_throughput` integration
  test all pointed at `ash-speed.hetzner.com/{1,10,100}MB.bin`, which has
  been 404ing silently (the integration-test swap in `bdc8c12` already
  noticed -- curl reported 146 bytes of nginx error page while every
  test asserted only "request logged + decision=allowed"). Swapped all
  three to `https://cdn.elie.net/static/files/i-am-a-legend/i-am-a-legend-slides.pdf`
  (~9.5 MB via Cloudflare, 301-redirects to `elie.net`). Size constants
  dropped to a conservative 9 MiB floor and the curl invocations gained
  `-L` so the proxy's 301-follow is exercised; the Rust test hits
  `elie.net` directly because raw hyper does not follow redirects.
  Dropped `ash-speed.hetzner.com` from the default web allow list
  (`config/defaults.toml`, `guest/config/security/web.toml`, and the
  hand-written `frontend/src/lib/mock-settings.ts`) since no live test
  or config still needs it; regenerated `config/defaults.json` and
  `frontend/src/lib/mock-settings.generated.ts` from the TOML. Docs
  page `docs/src/content/docs/development/benchmarking.md` updated to
  match.

### Added
- **`tests/test_repack_deb.py` -- 6 pytests that exercise
  `scripts/repack-deb.sh` directly in under a second.** Previously the
  repack step was only validated through `just test-install`, which
  takes minutes (Tauri build + systemd container + pnpm install)
  before any repack-related bug surfaces. The new harness builds a
  minimal fixture `.deb` with `dpkg-deb -b`, seeds fake companion
  binaries, and invokes the script end-to-end; coverage includes the
  happy path (all six companion binaries land at `/usr/bin/<name>`
  with mode 0755), `DEBIAN/postinst` copy fidelity, loud failure when
  a companion binary is missing, loud failure when the input path
  contains an embedded newline (regression for the `ls *.deb`
  multi-match bug), the build-timestamp stamp on `Version:`, and
  output-defaults-to-overwriting-input semantics. Skipped with a
  clear message when `dpkg-deb` is not on PATH (macOS default); runs
  in Linux CI and inside the `capsem-install-test` container.
  Verified in-container: 6 passed in 0.17s.
- **`just test` now records an in-VM capsem-bench baseline on every
  run.** The stage-6 "Benchmarks" step used to call
  `{{binary}} "capsem-bench"`, which clap parsed as a host-side
  subcommand and aborted with `unrecognized subcommand 'capsem-bench'`.
  Replaced with a new pytest
  (`tests/capsem-serial/test_capsem_bench_baseline.py`) that provisions
  a fresh VM, runs `capsem-bench all` inside it, pulls
  `/tmp/capsem-benchmark.json` out via `/exec cat`, and archives it to
  `benchmarks/capsem-bench/data_<version>_<arch>.json` with host-side
  timestamp + arch stamp. Mirrors the `_save_benchmark` pattern used by
  the existing `test_lifecycle_benchmark.py` host-side archives
  (`benchmarks/lifecycle/`, `benchmarks/fork/`). No regression gate
  yet -- once ~5-10 clean archives land per arch, per-category
  tolerances can be picked and promoted to pytest asserts, mirroring
  `OP_GATE_MS` / `FORK_GATE_MS` / `IMAGE_SIZE_GATE_MB` in the
  lifecycle benchmark. Host-side lifecycle/fork regressions remain
  gated today.
### Fixed
- **Service reaps `capsem-process` orphans on startup when reusing a run_dir.**
  A SIGKILL to capsem-service (crash, OOM, or `svc.proc.kill()` in the
  recovery test suite) does not propagate to its per-VM children. The
  children kept running with their `--session-dir` still pointing at the
  dead service's run_dir, holding Apple VZ VMs, vsock ports, and sockets
  indefinitely. When a replacement service started on the same run_dir it
  only removed stale socket files -- the orphan processes themselves
  persisted across the entire test session.
  Added `find_orphan_capsem_pids` + `reap_orphan_capsem_processes` in
  `crates/capsem-service/src/main.rs`: on startup (after creating
  `instances/`, before socket cleanup), shell out to `ps`, filter
  `capsem-process` lines whose cmdline contains `--session-dir <run_dir>`,
  SIGTERM them, poll up to 2s, SIGKILL survivors. The matcher is a pure
  function with four unit tests in `#[cfg(test)]` covering happy path,
  unrelated run_dir, non-capsem-process binaries that happen to mention
  the run_dir, and empty input. After the fix, the recovery tests
  (`tests/capsem-recovery/test_orphaned_process.py`,
  `test_service_health_after_recovery.py`) leave zero surviving
  `capsem-process` children.
- **Leak detector: controller-only gate + cross-process attribution.**
  Under `-n 4`, each xdist worker ran its own `pytest_sessionfinish` and
  flagged every other worker's session-scoped fixture processes as a
  "leak", because workers cannot distinguish their own children from
  peers' on the shared host. The in-worker gate also fired mid-teardown
  against processes that would have exited a second later via
  capsem-guard. Restructured: workers now only record first-seen
  attribution to `tests/leak-attribution.jsonl` (shared append log) and
  do NOT fail their session; the controller / single-process runner does
  the real gate at `pytest_sessionfinish`, after every worker has
  finished, when the host is the source of truth. The controller settles
  suspects with an exponential-backoff poll (50 ms -> 500 ms, 15 s
  budget) mirroring `capsem_core::poll::poll_until`, filters by the
  conftest-import-time baseline, merges worker attribution from the
  jsonl, and writes a deduped report to `tests/leak-report.log`. Verified
  `tests/capsem-mcp/ + tests/capsem-recovery/ -n 4` now finishes with
  zero reported leaks and zero surviving `capsem-*` processes on the host.
- **Leak detector: eliminate false positives and xdist-controller double-reporting.**
  `tests/conftest.py`'s `get_capsem_processes` was matching `'capsem-' in arg`
  across every process's full cmdline, so `cargo build -p capsem-*`, `rustc`
  driving a capsem crate, and every unrelated tool invoked from a path
  containing `capsem-next/` showed up as a "leak". The per-test check_leaks
  fixture also logged a line for every session-scoped fixture process on every
  test it outlived, so a single shared_vm in a 20-test file produced 20 false
  leak entries. On top of that, under `-n 4` the xdist controller process --
  which never runs session-scoped fixtures or tests -- also ran
  `pytest_sessionfinish` with an empty baseline and re-reported every capsem
  process as `<unknown>`. Rewrote the detector: match on `psutil` process
  name starting with `capsem-` (no cmdline scanning); snapshot the baseline
  at conftest import time so the xdist controller sees one too; per-test
  fixture now only records first-seen attribution; real leak check fires
  once at `pytest_sessionfinish` against processes still alive not in the
  baseline; skip the check entirely in the xdist controller and let each
  worker report its own leaks with real attribution. Verified
  `tests/capsem-build-chain/` and `tests/capsem-mcp/ -n 4` now produce no
  false-positive leak entries.
- **Stop logging routine VM lifecycle transitions at WARN.** Two `tracing::warn!` lines in capsem-service were firing on every normal shutdown -- "shutdown_vm_process removing instance" and "provision_sandbox child exit handler removing instance" -- which made the warn channel useless for actual problems. The first is now `debug!`. The second was further wrong: it fired *before* checking whether the child died unexpectedly vs after an explicit shutdown. Moved the warn inside the `if let Some(info) = removed` branch so it only fires for the genuinely surprising case (and reworded to say so), with a `debug!` for the expected post-shutdown path.
- **`shutdown_vm_process` is now synchronous: awaits actual exit + cleans the UDS socket inline, no background reaper.** Previously it spawned a fire-and-forget `tokio::spawn` to wait for the process and remove the socket, which left every caller racing the reaper. `handle_delete`, `handle_run`, and `handle_stop` were each working around this by calling `wait_for_process_exit` themselves (or hand-rolling the same loop), and `handle_purge` -- which fan-outs via `join_all` -- was the only one *not* working around it, so its parallel shutdowns relied on the reaper to clean up. Collapsed all of this: `shutdown_vm_process` now blocks on `wait_for_process_exit(pid, 5s)` and removes `*.sock` / `*.ready` itself, dropping ~50 lines of duplicate poll/SIGKILL/cleanup code from the four call sites and giving every caller a single clean contract -- when this returns, the process is gone, the socket is removed, and the session DB has flushed.
- **VM process cleanup now uses `poll_until` instead of hand-rolled fixed-interval loops, and `handle_run`/`handle_delete` synchronously await process exit before responding.** `wait_for_process_exit` was polling at fixed 100ms intervals and `handle_delete` was reinventing the same loop inline (with a different timeout, racing the background reaper from `shutdown_vm_process`). Switched the helper to `capsem_core::poll::poll_until` (50ms initial, exponential backoff to 500ms cap) and routed `handle_delete` + `handle_run` through it, which (a) removes the duplication, (b) cuts common-case latency since most processes exit in <50ms, (c) gives both endpoints the SIGKILL fallback for free, and (d) eliminates the `handle_run` race where the response could be returned before the VM process was actually gone (root cause of leak-detector false positives in `tests/capsem-mcp/`).
- **Fixed clippy break in `kill_all_vm_processes`** introduced by the prior service-shutdown cleanup change. The for-loop was switched to borrow `pids_and_sockets` (so `uds_path`/`session_dir` became references), but the existing `&uds_path`/`&session_dir` calls weren't updated, producing two `clippy::needless_borrows_for_generic_args` errors that broke `just test` Stage 1.
- **Improved VM process cleanup in delete handler.** Replaced fixed wait loops with bounded polling and SIGKILL fallback in `handle_delete` to ensure robust cleanup of `capsem-process` instances during deletion.
- **Fixed zombie process leak in service test helper.** Added `wait()` after `kill()` in `ServiceInstance.stop` to ensure child processes are fully reaped.
- **Wired capsem-guard into MCP subprocesses.** Added `capsem-guard` to `capsem-mcp-aggregator` and `capsem-mcp-builtin` to ensure they exit when their parent process dies, eliminating leaks.
- **Improved service-side VM process cleanup.** Replaced fixed 500ms sleep with a bounded polling loop (up to 2s) and SIGKILL fallback in `kill_all_vm_processes` to ensure robust cleanup of `capsem-process` instances.
- **`_clean-stale` now caps each cargo kind directory by size, so
  target/ stops growing unbounded during active dev.** The age-only
  prune (remove entries older than 2-3 days) never fired in practice
  because every build touches every `deps/`, `incremental/`, `build/`,
  and `.fingerprint/` entry -- nothing ever crossed the age threshold
  and the recipe's report said `cargo removed=0` while `target/` sat
  at 72 GB on `/System/Volumes/Data` (23 GB of that in
  `target/debug/incremental/` alone; push to 100% full triggered
  ENOSPC in several integration tests). Added a second pass to
  `scripts/clean_stale.py::clean_cargo_artifacts` that, for each
  profile (debug/release/llvm-cov-target), enforces a per-kind size
  budget (`deps` 12 GB, `incremental` 3 GB, `build` 1 GB,
  `.fingerprint` 500 MB) by deleting oldest-mtime entries until the
  total drops under cap. Newest entries survive so a warm build cache
  is preserved. `deps/` pruning scopes to cargo-generated extensions
  (`.rlib`, `.o`, `.rmeta`, `.d`) -- test binaries are left alone.
  Added 3 tests (budget evicts oldest, no-op under cap, deps filter
  scopes by extension); existing 16 clean_stale tests still pass.
  Measured on this machine: 72 GB -> 30 GB, 110,400 entries evicted
  in 21 s.
- **Artifact capture no longer fills the disk with rootfs.img copies.**
  `tests/helpers/service.py::preserve_tmp_dir_on_failure` recursively
  copied every file from a failing test's `/var/folders/.../capsem-test-*`
  tmpdir into `test-artifacts/`, including per-VM `sessions/<id>/system/rootfs.img`
  (~2 GB each, plus the `auto_snapshots/0/system/rootfs.img` clones).
  29 failure dirs consumed 18 GB apparent / ~9 GB real (APFS clone
  sharing) on /System/Volumes/Data -- enough to push the host to 100%
  and cause downstream ENOSPC failures in other tests. Taught the
  `shutil.copytree` ignore callback to skip (a) files named `rootfs.img`
  / `rootfs.img.backing`, (b) any regular file larger than
  `ARTIFACT_MAX_FILE_BYTES` (25 MB), and (c) sockets/FIFOs (pre-existing).
  Added a rotation pass: after every preserve, only the
  `ARTIFACT_MAX_KEPT_DIRS` most-recent subdirs under `test-artifacts/`
  survive (default 20). Landed `tests/test_preserve_artifacts.py` with
  5 pytests pinning these invariants (rootfs skipped, oversize skipped,
  logs/session.db preserved, no-op when no failures, rotation keeps N).
- **`tests/capsem-security/test_binary_perms.py::test_agent_binaries_555`
  is green on macOS again.** `capsem-builder`'s container agent build
  runs `chmod 555 /output/<binary>` inside the build container
  (`src/capsem/builder/docker.py::container_compile_agent` line 444),
  but Docker-for-Mac bind-mount semantics let the 0o755 executable
  bits survive on the host side for `capsem-pty-agent` and
  `capsem-net-proxy` (capsem-mcp-server and capsem-sysutil came out
  0o555 cleanly -- same chmod, different result, macOS Docker
  filesystem weirdness). The initrd-pack recipe already re-applied
  `chmod 555` to its copies, but the on-disk `target/linux-agent/<arch>/`
  files remained 0o755, tripping the invariant check. Added an
  explicit `chmod 555 "$RELEASE_DIR"/{capsem-pty-agent,...}` step
  right after the `uv run capsem-builder agent` invocation in the
  `_pack-initrd` recipe so the invariant is enforced every time the
  build runs, regardless of what the container filesystem decides to
  preserve.
- **`just test-install` no longer passes dpkg-deb a two-path mess
  after a version bump.** The repack step did
  `DEB=$(ls /cargo-target/debug/bundle/deb/*.deb)` -- when the persistent
  `capsem-install-target` volume still held a previous version's `.deb`
  (e.g. today's `0.16.1` -> `1.0.1776688771` bump left the old file
  sitting next to the new one), the glob matched both and `$()`
  captured them joined by a newline. `scripts/repack-deb.sh` then got
  one path-with-embedded-newline, which `dpkg-deb` tried to open as a
  single file and bailed with `No such file or directory`. Added
  `rm -f /cargo-target/debug/bundle/deb/*.deb` before the Tauri build
  so the bundle dir always starts empty, and switched the lookup to
  `ls -t ... | head -1` as belt-and-braces for the same class of
  bug.
- **Linux builds of `capsem-process` / `capsem-service` compile again.**
  Two sites in `crates/capsem-process/src/vsock.rs` called
  `capsem_core::hypervisor::apple_vz::run_on_main_thread(...)` inside
  the Stop and Suspend command handlers. The `apple_vz` module is
  gated on `#[cfg(target_os = "macos")]` (see
  `capsem-core/src/hypervisor/mod.rs:7`), so both sites broke
  `cargo build` on Linux with `cannot find 'apple_vz' in 'hypervisor'`
  -- surfaced by `just test-install`'s in-container
  `cargo build {{host_crates}}` step. Wrapped each call in
  `#[cfg(target_os = "macos")]` with a non-macOS branch that invokes
  the `VmHandle` methods directly; Apple VZ has a main-thread
  constraint (CFRunLoop) that KVM does not, and KVM's trait default
  returns "not supported" for `pause`/`save_state`, which `?`
  propagates -- the correct behaviour for a backend without
  checkpoint support. Also silenced `-D warnings` on
  `capsem-service::spawn_companions`'s `tray_bin` parameter, which is
  consumed only by the `#[cfg(target_os = "macos")]` tray-spawn block
  and therefore unused on Linux. Added
  `#[cfg(not(target_os = "macos"))] let _ = tray_bin;` to mark the
  intent without changing the cross-platform signature.
- **`just test-install` no longer dies with `Permission denied`
  when rustup tries to self-update.** Same root class as the
  `just cross-compile` EXDEV fix: the `capsem-install-test` image
  extends `capsem-host-builder` and inherits its `/usr/local/rustup`
  (root-owned from image build). The test-install recipe runs `cargo
  build` as the non-root `capsem` user, so rustup's
  channel-sync-on-first-cargo attempt to write
  `/usr/local/rustup/tmp/` is denied (`os error 13`). Added a
  dedicated `capsem-install-rustup:/usr/local/rustup` named-volume
  mount to the systemd-container `docker run`, added
  `/usr/local/rustup` to the chown pass, and added the volume to the
  `_clean-host-image` cleanup list. Mirrors the `capsem-rustup`
  pattern introduced for cross-compile; using a separate volume keeps
  the two images' rustup states from cross-contaminating if they ever
  drift to different stable channels.
- **`just test` / `just smoke` no longer hang on a pnpm interactive
  prompt.** The `_pnpm-install` helper ran `pnpm install
  --frozen-lockfile` with no `CI` env var, so whenever the on-disk
  `node_modules` store drifted from the lockfile (version bump, pnpm
  upgrade, stale npm artifacts, manual edits), pnpm asked `The
  modules directory at ... will be removed and reinstalled from
  scratch. Proceed? (Y/n)` on stdin and sat there forever in a
  non-interactive just-test run. Added `CI=true` to the invocation --
  same idiom already used in the cross-compile docker bash and the
  test-install container at lines 494 / 792 of the justfile -- which
  tells pnpm to auto-accept defaults instead of prompting.
- **`just cross-compile` no longer requires the release Tauri signing
  keys for dev builds.** The recipe read `private/tauri/capsem.key`
  and `private/tauri/password.txt` on the host and passed them to the
  container unconditionally. For any dev who doesn't have those files
  (everyone outside release CI), both env vars became empty strings,
  which Tauri 2 treats as "try to sign with an empty key" and aborts
  with `failed to decode secret key: incorrect updater private key
  password: Missing comment in secret key`. The real release keys are
  injected via GitHub Actions secrets (`TAURI_SIGNING_PRIVATE_KEY` +
  `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` in
  `.github/workflows/release.yaml`); dev builds only need *a* valid
  key so `cargo tauri build` completes. Now the host only passes the
  signing env vars when both `private/tauri/capsem.key` and
  `private/tauri/password.txt` actually exist; otherwise the container
  generates a throwaway dev key into
  `/cargo-target/dev-tauri-private` (persistent across runs via the
  existing `capsem-host-target-<arch>` volume) with
  `cargo tauri signer generate --ci --force`. The generated key has a
  fixed password of `dev` -- its signatures are worthless for
  release-updater verification, but the bundle builds.
- **`just cross-compile` no longer dies with `Invalid cross-device link`
  when rustup self-updates inside the host-builder container.**
  `rust-toolchain.toml` pins `channel = "stable"`, so every time a new
  stable drops, the first cargo invocation inside the pre-built
  `capsem-host-builder` image triggers a rustup channel sync. The sync
  tries to `rename(2)` a toolchain directory
  (`toolchains/stable-.../lib/rustlib/.../self-contained`) from the
  image's lower overlay layer into `/usr/local/rustup/tmp/` on the
  container's upper layer; Docker-for-Mac's overlayfs bounces that
  specific cross-layer rename with `os error 18` and rustup aborts.
  Added a persistent `capsem-rustup:/usr/local/rustup` named-volume
  mount to the `docker run` in `cross-compile`, matching the same
  pattern used for `capsem-cargo-registry` / `capsem-cargo-git` /
  `capsem-host-target-<arch>`. First run copies the image's baked-in
  rustup tree into the volume; subsequent runs put all of rustup on
  one filesystem, so the rename stays within a single mount and the
  EXDEV class of bug is eliminated whether or not rustup self-updates.
  Updated `_clean-host-image` to rm the new volume and drop the
  never-wired `capsem-rustup-{arm64,x86_64}` placeholders.
- **`just test` and `just smoke` execution lock actually blocks
  concurrent runs now.** The lockfile lived at
  `$CAPSEM_RUN_DIR/execution.lock` under `$CAPSEM_HOME`, but the recipe
  ran `rm -rf "$CAPSEM_HOME"` *before* the `flock`. A second invocation
  therefore nuked the first's lockfile and created a new one; `flock -n
  3` on the new inode succeeded unchallenged (the first invocation's
  fd was pinned to the unlinked inode), so two `just test` or `just
  smoke` runs could race through the same `$CAPSEM_HOME`/shared-service
  path and trample each other's VMs. Moved the lockfile to
  `target/capsem-test-execution.lock` (outside `$CAPSEM_HOME`, survives
  the wipe) and acquired it *before* `rm -rf`. Extracted the
  mkdir/exec/flock dance into a single shell helper
  (`scripts/lib/exec_lock.sh::acquire_exec_lock`) and replaced all 8
  inline copies in the justfile (`dev`, `shell`, `run`, `test`,
  `smoke`, `build-gateway`, `bench`, `release`) with two-line
  `source + acquire_exec_lock <path>` calls. Added
  `tests/test_exec_lock.py` (3 tests: concurrent blocker, reacquire
  after release, parent-dir creation) so this regression can't sneak
  back in.
- **`cargo test -p capsem-guard --lib` is deterministic again.** The
  `install_happy_path_returns_guards_and_creates_lock` test did
  `install -> drop -> install` in-process, which reliably failed under
  parallel `cargo test` because a sibling test
  (`singleton_reacquires_after_ungraceful_holder_exit`) calls
  `Command::spawn`, and the forked child briefly inherits our flock fd
  before exec'ing. `O_CLOEXEC` only closes on exec, not on fork; that
  window is enough for the kernel-level flock to survive our drop, so
  the second `install()` returns `Ok(None)` instead of `Ok(Some(_))`.
  This trap is already called out on
  `singleton_reacquires_after_drop_in_isolated_process`, which solves
  it by forking a clean subprocess for the drop-then-reacquire check;
  the new install_happy_path test quietly regressed that workaround.
  Removed the drop+re-install portion of the test -- its stated
  purpose (cover `install()`'s `Ok(Some(_))` arm and assert the
  lockfile exists) is preserved. llvm-cov on
  `capsem-guard/src/lib.rs` is unchanged to the region (714 / 37
  missed, 94.82%); the deleted lines duplicated coverage already
  carried by the isolated subprocess test.
- **Excluded `tests/capsem-build-chain/` from parallel pytest execution.** The suite runs `cargo build` and `codesign` via session-scoped fixtures, which caused races and failures on codesigning (`replacing existing signature` errors) when run concurrently with other tests. Now run in serial after the parallel block.
- **`capsem-process` now exits on `SIGTERM` on macOS.** Previously, the process blocked on `CFRunLoopRun()` and the signal handler task only logged the signal without stopping the run loop. Now, the signal handler calls `CFRunLoopStop` to allow the process to exit cleanly, fixing race conditions in VM cleanup tests.
- **MCP `shared_vm` consumers no longer intermittently 404 after
  `test_purge_all` runs on the same xdist worker.** `test_purge_all` was
  calling `capsem_purge { all: true }` on the session-scoped
  `capsem_service`, which also hosts the session-scoped `shared_vm`
  (persistent, named `shared-<worker>-<hex>`). Because `all=true`
  destroys every sandbox on the service -- persistent included -- any
  subsequent test on the same worker that used `shared_vm`
  (`test_sql_query`, `test_exec.*`, `test_file_io.*`, `test_lifecycle.*`,
  `test_mcp_call.*`) got `404 Not Found: sandbox not found` whenever
  pytest happened to schedule `test_purge_all` first. Fix: extracted the
  MCP conftest's service-startup into `_start_capsem_service()` so the
  `--gateway-port 0`, `--foreground`, `sign_binary`, and log-dumping
  invariants live in one place, added an `isolated_mcp_session`
  function-scoped fixture that spins up its own transient service for
  globally destructive tests, and migrated `test_purge_all` onto it.
  Added `test_isolated_mcp_session_does_not_affect_shared_service` to
  pin the isolation invariant so a future destructive test can't quietly
  regrow the same bug.
- **`capsem_mcp_call` no longer hangs for 60s on every invocation.** The
  service -> capsem-process IPC channel is `tokio-unix-ipc`, which uses
  bincode as its wire format. Bincode is not self-describing, and
  `serde_json::Value::deserialize` calls `deserialize_any`, which bincode
  explicitly rejects. `ServiceToProcess::McpCallTool { arguments:
  serde_json::Value }` therefore serialized fine on the service side and
  then failed to deserialize inside capsem-process the moment the message
  hit the wire -- the per-connection handler returned silently and the
  service's 60s `send_ipc_command` timeout fired. End result: every
  `tests/capsem-mcp/test_mcp_call.py` test spent exactly 60s hanging
  (120s combined, 75% of the 160s MCP parallel group), and the entire
  `capsem_mcp_call` feature path was dead on arrival on any non-stub
  aggregator. Fix: changed the IPC payload to JSON-stringified forms --
  `McpCallTool { arguments_json: String }` and
  `McpCallToolResult { result_json: Option<String> }` -- so the payload
  is opaque to bincode. The service and capsem-process now
  `serde_json::to_string` / `from_str` at the boundary. Added
  `mcp_call_tool_roundtrip_bincode` / `mcp_call_tool_result_roundtrip_bincode`
  tests in `capsem-proto` that exercise the real bincode path (the old
  tests only roundtripped through `serde_json::to_vec`, which is
  self-describing and missed the bug). MCP pytest group: 160s -> ~40s.
- **`capsem install` and `just install` can no longer bake a
  `target/test-home` path into the installed LaunchAgent / systemd unit.**
  `install_service()` resolves `--assets-dir` via
  `capsem_core::paths::capsem_assets_dir()`, which honors `CAPSEM_HOME` /
  `CAPSEM_RUN_DIR` / `CAPSEM_ASSETS_DIR`. If the installer inherited any
  of those from a prior `just test` session, the resulting LaunchAgent
  permanently referenced a directory that `just test` wipes on every
  run -- and with `KeepAlive=true`, launchd kept respawning it against a
  dead path, racing against `_ensure-service` during subsequent tests.
  Two-layer fix:
  - `install_service()` now bails with a clear message if any of the three
    isolation vars are set, telling the caller to `unset` them.
  - The `just install` recipe explicitly `unset`s them before running, so
    shells that accidentally still have them exported install cleanly.
  `scripts/integration_test.py::_kill_dev_service` also switched from
  `pkill -f capsem-service.*--foreground` (which catches any installed
  LaunchAgent/systemd unit on the box) to a strict pidfile-based kill,
  mirroring the discipline `_ensure-service` already follows.
- **`capsem run` auto-launch now honors `CAPSEM_HOME`.** When the client
  couldn't reach the service socket it fell back to
  `launchctl kickstart` / `systemctl --user start` whenever a
  LaunchAgent / systemd unit existed. Those units point at the default
  `$HOME/.capsem` layout, so under an isolated test run
  (`CAPSEM_HOME=target/test-home/.capsem`) the kicked service bound a
  socket in the *real* home while the client kept polling the test home
  until the 5s `AwaitStartup` budget expired -- `scripts/integration_test.py`'s
  ephemeral-model check always failed on machines with capsem installed.
  `UdsClient::try_ensure_service` now skips the service-manager branch
  whenever `CAPSEM_HOME` is set and goes straight to direct-spawn, so the
  child service inherits `CAPSEM_HOME` and binds the socket the client is
  watching. Production `~/.capsem` flow is unchanged.
- **Direct-spawn auto-launch no longer hangs the CLI's stdout/stderr
  pipes.** `UdsClient::try_ensure_service`'s fallback path spawned the
  service with inherited stdio, so when the CLI was invoked from Python
  under `subprocess.run(capture_output=True)`, the detached service
  kept stdout/stderr open long after the CLI returned. Python's
  `communicate()` waited for EOF on those pipes and always timed out at
  its outer 120s deadline -- the same symptom
  `scripts/integration_test.py::check_persistence` hit under a test
  harness without an existing running service. The spawn now redirects
  all three fds to `/dev/null`; service logs still land in
  `<run_dir>/service.log` as before.
- **`_ensure-service` no longer leaks the execution-lock fd.** The
  backgrounded capsem-service inherited fd 3 (which holds `flock -n 3` on
  `$CAPSEM_RUN_DIR/execution.lock`) from its parent shell. If `just smoke`
  or `just test` aborted after starting the service, the service kept fd 3
  open and the flock stayed held after the outer shell exited, bricking
  subsequent runs with "another agent holds the test execution lock". The
  service is now launched with `3>&-` so fd 3 is closed before exec.
- **`just install` now leaves `~/.capsem/assets/` in the layout the service's
  resolver actually reads.** The .pkg/.deb ships only `manifest.json` (binaries
  and assets are on independent shipping cadences), and `capsem setup` was a
  stub with a TODO, so a fresh install left the UI banner stuck on "VM assets
  are missing" and every VM boot failed asset resolution. Added
  `scripts/sync-dev-assets.sh`, invoked by the `install` recipe after the
  installer runs, which mirrors the locally built `assets/$arch/*` hash-named
  files into `~/.capsem/assets/$arch/` (the exact paths
  `ManifestV2::resolve()` looks up) and removes the legacy `v1.0.*/`
  directories that accumulated from the old v1 layout. Also updated
  `scripts/simulate-install.sh` to honor the same layout so
  `tests/capsem-install/` agrees with production.

### Added
- **`capsem setup` actually downloads VM assets, and `capsem update --assets`
  re-fetches them on their own cadence.** New
  `capsem_core::asset_manager::download_missing_assets()` streams each arch's
  asset files from the GitHub release URL (per-arch upload names:
  `arm64-vmlinuz` / `arm64-initrd.img` / `arm64-rootfs.squashfs`),
  blake3-verifies the bytes, and places them at
  `$base/$arch/{hash_filename}` with 0o444 perms. `step_welcome` in the setup
  wizard, and a new `capsem update --assets` subcommand, both call into it.
  `CAPSEM_RELEASE_URL` env override lets integration tests redirect the
  download target.

### Tests
- **`tests/capsem-install/` is now safe to run bare-metal.** The module-level
  `CAPSEM_DIR` previously hardcoded `$HOME/.capsem`, so running
  `pytest tests/capsem-install/` clobbered the developer's real install
  (`simulate-install.sh` overwrote binaries; `test_full_uninstall` literally
  asserted `~/.capsem` was removed). `conftest.py` now provisions a temp
  `CAPSEM_HOME` for the session and auto-skips the `live_system` tier
  bare-metal unless `CAPSEM_ALLOW_DESTRUCTIVE=1`, because those tests invoke
  `capsem setup` / `capsem uninstall` which touch the system-level
  LaunchAgent / systemd unit outside any `CAPSEM_HOME` override.
  `test_installed_layout` was rewritten to assert the v2 layout
  (`$ASSETS/$arch/{hash_filename}`) instead of the legacy
  `$ASSETS/v$VERSION/` the resolver no longer reads.
  New `test_asset_download.py` covers the happy path, 404, hash mismatch,
  and idempotent rerun for `capsem update --assets` against a local HTTP
  fixture.

### Changed
- **`just test` and `just smoke` reordered for fail-fast feedback.** Audits,
  Rust lint, and the frontend suite now run in a single parallel block at the
  top of each recipe, so a bad Svelte type, a broken clippy lint, or a
  dependency advisory surfaces in under two minutes instead of after 5-10
  minutes of `cargo llvm-cov` and cross-compile. The lint gate switched from
  `cargo check --workspace` to
  `cargo clippy --workspace --all-targets -- -D warnings`, enforcing the
  project's stated bar (`CLAUDE.md`: "treat clippy and rustc warnings as
  build failures") with no duplicate compile (clippy is a strict superset of
  check). Smoke additionally gained `pnpm run check` in its parallel block --
  previously a Svelte/TS type error only surfaced under `just test`.
- **`just test` ignores `tests/capsem-recipes/` and `tests/capsem-install/`
  in its parallel pytest stage.** Both directories contain tests that
  `subprocess.run(["cargo", "build", ...])` from inside pytest; under `-n 4`
  this atomically replaced the codesigned `capsem-service` / `capsem-process`
  binaries while other xdist workers were booting VMs against them, hanging
  `just test` at 99%. The recipe tests are redundant inside `just test`
  (clippy + `cargo llvm-cov` + `_build-host` already cover their assertions)
  and remain runnable standalone via `uv run pytest -m recipe`. The install
  suite is fully covered by `just test-install` inside Docker.
- **Every Shiki grammar and theme is now a lazy chunk fetched on first
  use, and the heavy app views are code-split.** The app was importing
  `'shiki'` (the default `bundle-full` export), which references all
  235 Shiki languages -- Vite code-split every one, shipping >600 KB
  chunks for grammars we never use (emacs-lisp, wolfram, wasm,
  vue-vine, ...). Moved to `shiki/core` and swapped the Oniguruma WASM
  regex engine for `createJavaScriptRegexEngine()` (removes a 608 KB
  WASM-as-JS chunk; the JS engine covers every grammar we ship).
  `shiki.ts` now creates the highlighter with empty `langs` and
  `themes` arrays and exposes a single `highlightCode(code, lang,
  theme)` entry point plus `ensureShikiLang` / `ensureShikiTheme`
  helpers. Each of the 31 supported languages and 21 themes is a
  `() => import('@shikijs/langs/<name>')` entry, so Vite emits one
  chunk per grammar/theme; they are fetched the first time a matching
  file is rendered, then retained for the session. A `shikiTick`
  pattern in StatsView upgrades the plaintext fallback to highlighted
  HTML once the prewarm promise for its langs/theme resolves. In
  `App.svelte` the heavy views (Settings, Stats, Logs, ServiceLogs,
  Files, Inspector, OnboardingWizard, CreateSandboxDialog) are now
  loaded via `{#await import()}` so they're fetched only on first use.
  App chunk drops from 582 KB to 142 KB. `chunkSizeWarningLimit` is
  raised to 700 KB to accommodate the inherent ~620 KB cpp grammar
  chunk (loaded only for `.cpp`/`.hpp`/`.cc`/`.cxx`); every other
  chunk stays under 200 KB. `@shikijs/langs` and `@shikijs/themes` are
  now direct deps (previously transitive) so Vite can resolve the
  subpath imports.

### Fixed
- **`just test` no longer kills or mutates a locally installed capsem.**
  Previously the test harness (`scripts/integration_test.py`,
  `_ensure-service`, and every Rust site that computed `$HOME/.capsem/...`
  directly) ran against the shared `~/.capsem/` directory, so a pkill-by-name
  on `capsem-service --foreground` took down the user's installed daemon,
  `~/.capsem/run/service.{sock,pid}` were deleted, and `~/.capsem/assets`
  was swapped for a symlink. Added a `CAPSEM_HOME` env var honored by a new
  `capsem_core::paths` module (with `capsem_run_dir`, `capsem_assets_dir`,
  `capsem_sessions_dir`, `capsem_bin_dir`, `capsem_logs_dir`,
  `service_socket_path`, `service_pidfile_path`) and routed every
  `$HOME/.capsem/...` site across `capsem`, `capsem-service`,
  `capsem-mcp`, `capsem-gateway`, `capsem-tray`, `capsem-app`, and
  `capsem-core` through it. `just test` / `just smoke` now export
  `CAPSEM_HOME=target/test-home/.capsem` (cleaned each run, swept by
  `just clean`). `_ensure-service` no longer uses pkill-by-name --
  it kills only the service tracked by its own pidfile, so an isolated
  test run never touches an installed daemon. The execution-lock flock
  moves into the test home alongside its socket.
- **Dev-build tray icon now renders orange** so the menu-bar icon is
  visually distinct from an installed release build. Grey pixels are
  recoloured to a `#FF8800` ramp at icon-load time under
  `cfg!(debug_assertions)`; anti-aliased edges remap by luminance so the
  icon stays smooth instead of banding. Release builds are untouched.
- **External links ("Get a key", API key docs, onboarding "Learn more")
  now open in the system browser from the Tauri desktop app.** Previously
  `<a target="_blank">` did nothing in the Tauri webview because
  `window.open` is a no-op there, and the `open_url` IPC handler already
  wired up in `capsem-app` was never called from the frontend. `openUrl()`
  in `api.ts` now detects the Tauri shell via `__TAURI_INTERNALS__` and
  invokes the `open_url` command; a document-level click interceptor in
  `App.svelte` routes every `<a target="_blank">` and `http(s):`/`mailto:`
  link through it, so existing call sites keep working unchanged. Browser
  dev mode still falls back to `window.open`.
- **Dark-mode warning banners no longer render with a white strip and
  unreadable text.** Two compounding issues: `html`/`body` had no
  theme-aware `background-color`, so the browser's default white canvas
  showed through any transparent element; and `--warning` /
  `--warning-foreground` were referenced across the frontend (install-
  incomplete banner, "VM assets are missing" alert, password-required
  badges, MCP section warnings) but never defined, so `bg-warning/10`
  resolved to fully transparent. Set the canvas to
  `var(--background)`/`var(--foreground)` on `html, body` and defined
  `--warning` (amber-600 light / amber-400 dark) plus
  `--warning-foreground` in `:root` and `.dark` so the amber tint and
  legible contrast appear on every warning surface.
- **`just install` no longer re-shows the GUI onboarding wizard on every
  reinstall.** The single `onboarding_completed` flag conflated "CLI install
  finished" with "user dismissed the welcome wizard", so dev reinstalls
  re-triggered the full-screen wizard even when the user had already clicked
  through it. Split into two flags: `install_completed` (set by `capsem setup`
  on success) and `onboarding_completed` (set only by the GUI wizard's "Get
  Started" button). Added `onboarding_version` to let a future release force
  re-onboarding by bumping `CURRENT_ONBOARDING_VERSION`. The frontend now reads
  a server-computed `needs_onboarding` instead of mirroring the version
  constant. Added `capsem setup --force-onboarding` to reset the wizard flags
  without wiping install state. Existing state files missing `install_completed`
  are migrated on load: if the `summary` step is present, install is inferred
  complete so upgraded users don't see a spurious "install didn't finish"
  banner. The app renders that banner only when `install_completed=false`,
  with a "Retry install" button that hits a new `POST /setup/retry` endpoint
  -- the service spawns `capsem setup --non-interactive --accept-detected` so
  users can recover from a broken install without opening a terminal.
- **`just smoke` now passes on dev machines whose user config opts
  into `security.web.allow_read=true`.** Three unrelated failures came
  out in the same wash:
  - Doctor `test_denied_domain_rejected` (test_network.py) and
    `test_denied_domain` (test_sandbox.py) hard-coded the default-deny
    posture. They now skip when `CAPSEM_WEB_ALLOW_READ=1` -- surfaced
    by injecting the `security.web.allow_{read,write}` toggles into
    the guest as `CAPSEM_WEB_ALLOW_{READ,WRITE}` env vars, the same
    pattern already used for `CAPSEM_OPENAI_ALLOWED` / etc. The
    policy's actual read/write denial is still exercised by
    `test_post_to_random_domain_denied`, which doesn't depend on the
    user's toggles.
  - `scripts/integration_test.py` looked for `run-*` session
    directories; current `capsem-service` generates
    `tmp-<adj>-<noun>` IDs (see `generate_tmp_name`). Also relaxed
    the `~/.capsem/logs` check -- that directory only exists after
    the Tauri desktop shell has been launched, which integration_test
    never does. The script now only validates the logs if they're
    present.
  - `config/integration-test-user.toml` used the stale
    `network.custom_{allow,block}` / `network.default_action`
    setting IDs; migrated to current `security.web.*` keys so the
    deny list (`deny.example.com`) actually takes effect.
- **`scripts/integration_test.py` restarts `capsem-service` with
  `CAPSEM_{USER,CORP}_CONFIG` in its env before booting the test VM**,
  then tears it down on exit. Required because the dev service
  (started by `_ensure-service`) inherits no test config, and
  `capsem run` talks to whatever service is already listening, so the
  per-VM policy previously fell back silently to `~/.capsem/user.toml`.
  Complements the service-side env passthrough in `refactor(service):
  extract pure helpers into lib + submodules`.

### Added
- **Failed-session log preservation for post-mortem.** Three host-side
  loss paths used to silently `remove_dir_all` the session directory
  when capsem-process died unexpectedly, taking `process.log`,
  `mcp-aggregator.stderr.log`, `serial.log`, and `session.db` with
  them -- exactly when those logs are most useful. All three now
  funnel through a single `ServiceState::preserve_failed_session_dir`
  helper that renames the dir to a `-failed-<ts>-<rand>` sibling
  (via `capsem_core::session::generate_session_id`) and calls
  `cull_failed_sessions` to cap the surviving count at
  `MAX_FAILED_SESSIONS = 5`. If rename fails (EEXIST, permission,
  cross-filesystem), `warn!` with the specific error and fall back
  to `remove_dir_all` so disk isn't leaked when the filesystem is
  already unhappy. Paths wired through the helper:
  (a) `handle_run`'s `wait_for_vm_ready` timeout -- now also awaits
  `wait_for_process_exit` before rename so the child has finished
  flushing session.db and log files (avoids the path-based-reopen
  ENOENT hazard during shutdown);
  (b) `scrub_evicted_instance` (promoted from free fn to
  `ServiceState` method) when `cleanup_stale_instances` detects a
  dead PID -- the loss path the last service commit introduced;
  (c) `provision_sandbox`'s child-exit handler, which fires only
  when the child died outside the explicit teardown path
  (`shutdown_vm_process` removes the map entry first, so the
  `removed = Some(info)` branch is by definition the "died
  unexpectedly" case). Four new unit tests pin the contract: rename
  preserves file contents, cull keeps newest and prunes oldest, cull
  is a no-op under the cap, cull never touches non-`-failed-` dirs.
- **Multi-agent execution lock on heavy `just` recipes.** `smoke`,
  `test`, `bench`, `shell`, `exec`, `ui`, `install`, and
  `test-gateway-e2e` now acquire a non-blocking `flock(1)` on
  `~/.capsem/run/execution.lock` before doing anything that touches
  the shared `capsem-service`. A second agent attempting a heavy
  recipe while one is in flight gets an immediate
  `"another agent holds the capsem execution lock ..."` error instead
  of silently restarting the service under the first agent's VMs.
  The kernel releases the lock when the holding process exits, so
  there are no stale lockfiles on crash/SIGKILL. `flock` is now
  checked by `just doctor` (hints point at `brew install flock` on
  macOS, `util-linux` on Linux) and auto-installed by
  `scripts/bootstrap.sh` on macOS when Homebrew is available.

### Changed
- **`UdsClient::connect_with_timeout` now uses
  `capsem_core::poll::poll_until`** instead of a hand-rolled
  exponential-backoff loop. New `ConnectMode { FailFast, AwaitStartup }`
  parameter makes the retryable-vs-permanent classification explicit
  at every call site: the initial probe in `request()` stays
  `FailFast` so CLI calls don't sit for 5 s when the service is
  definitively down; post-launch retries in `try_ensure_service` are
  `AwaitStartup` so a just-started service's `ENOENT`/`ConnectionRefused`
  are treated as "socket not bound yet" rather than "service dead."
  Also folded: `try_ensure_service` now returns the connected
  `UnixStream` so `request()` no longer does a third redundant
  connect. Net effect on the code is smaller than the diff suggests --
  mostly deletes the hand-rolled state machine and replaces it with
  the shared primitive. See `/dev-rust-patterns` lesson 19 and
  `/dev-bug-review` (the skill now explicitly calls out
  "grep for existing primitives, don't hand-roll" as a first-class
  step of the workflow).
- **`capsem-service` split into `lib + bin`** -- new `crates/capsem-service/src/lib.rs` exposes the `api`, `errors`, `fs_utils`, and `naming` submodules. Pure helpers (`AppError`, `sanitize_file_path`, `extract_magika_info`, `identify_file_sync`, `validate_vm_name`, `generate_tmp_name`) move out of `main.rs` into their own files with their own `#[cfg(test)] mod tests`. `ServiceState`, `PersistentRegistry`, `resolve_workspace_path`, and every axum handler stay in `main.rs` (their move is a follow-up sprint). `api.rs` content is unchanged -- `errors.rs` re-exports `ErrorResponse` via `pub use`. +14 net new unit tests; `errors.rs`/`fs_utils.rs`/`naming.rs` each at 100% line, region, and function coverage. Unblocks future `crates/capsem-service/tests/` integration tests now that `lib.rs` exists.
- **Workspace MSRV bumped from Rust 1.82 to 1.91.** `capsem-core`'s
  `mcp::builtin_tools` relies on `str::floor_char_boundary`, stable in
  1.91, which clippy's `incompatible_msrv` lint correctly flagged.
  Raising the floor clears the lint (no downgrade path), matches the
  toolchain the tree is actually built with, and unblocks
  `cargo clippy -- -D warnings` across the workspace.

### Fixed
- **`capsem doctor` (and any other auto-launch path) no longer
  spuriously fails with "Service manager started capsem but socket not
  ready."** Root cause: `UdsClient::connect_with_timeout` fast-failed
  on `ENOENT`/`ConnectionRefused` from its very first attempt, breaking
  out of its own retry loop before the just-requested service could
  bind its socket. The obvious symptom was the misleading error
  message; the less obvious consequence was that the auto-launch
  path became racy under load and flaky in tests. Fix is the
  `ConnectMode`-aware refactor above plus preserving the inner error
  via `Context` instead of the old `.map_err(|_| anyhow!(...))` which
  threw the real `io::Error::kind` away. Pre-existing clippy cleared
  in the same file: 3 `print_literal` on table-header printlns
  (intentional literal labels -- allowed locally with a comment),
  3 `field_reassign_with_default` in `setup.rs` test fixtures.
  Regression tests in `client.rs`: FailFast short-circuits in under
  500 ms on a missing socket; AwaitStartup sees a `UnixListener`
  bound 400 ms after the connect call starts; AwaitStartup times out
  cleanly with a preserved error chain when nothing ever binds.

### Added
- **Host-side logs now carry `vm_id` and `trace_id` as structured
  fields for cross-process correlation.** `capsem-process` generates a
  16-hex-char `trace_id` at startup and enters a root
  `info_span!("vm", vm_id, trace_id)` that every subsequent log line
  inherits. The same pair is propagated to the aggregator subprocess
  via `CAPSEM_VM_ID` / `CAPSEM_TRACE_ID` env vars, and
  `capsem-mcp-aggregator` enters a matching
  `info_span!("aggregator", vm_id, trace_id)`. Grep for a `trace_id` to
  follow a single VM's execution across `process.log`,
  `mcp-aggregator.stderr.log`, and `session.db` in the same session
  directory. First step toward broader log correlation -- other
  binaries (service, gateway, app) will pick up the same pair in
  follow-ups. OpenTelemetry export was proposed alongside this and
  explicitly deferred to a sprint proposal: it's a feature, it adds
  a new outbound channel to an air-gapped product, and the
  correlation problem that motivated it is solved by `trace_id`
  alone.

### Fixed
- **`capsem-mcp-aggregator` stderr no longer pollutes `process.log`.**
  `capsem-process` spawned the aggregator with
  `Stdio::inherit()` for stderr, so the aggregator's plain-text
  tracing merged into the parent's JSON tracing stream and made
  `process.log` effectively unparseable with `jq` / log pipelines.
  Two coupled fixes: (a) the aggregator's subscriber now uses
  `.json()`, matching `capsem-process` and `capsem-service`; (b) the
  aggregator's stderr is now redirected to a dedicated
  `mcp-aggregator.stderr.log` in the VM's session directory, opened
  with `0o600` under `#[cfg(unix)]` per
  `/dev-rust-patterns` lesson 14. End state: `process.log` is pure
  parent JSON, `mcp-aggregator.stderr.log` is pure aggregator JSON.
  Also elevated a small set of lifecycle events from `debug` to
  `info` (aggregator reader/writer/monitor task start/stop,
  `mcp::gateway::serve_mcp_session_inner` EOF) so critical
  lifecycle transitions are always visible in the default filter.
  Cleared 4 pre-existing clippy errors in `capsem-process` that the
  gate surfaced: one `too_many_arguments` on `ipc::handle_ipc_connection`
  (8 > 7 -- `#[allow(...)]` with no behavior change), three
  `useless_vec` in unit tests. Three new unit tests pin the
  `trace_id` contract (16 hex chars, no collisions over 64 calls)
  and the aggregator-log path (lives in session dir). The JSON
  format switch and the root-span wiring are not cleanly unit
  testable without a live subscriber harness; validated via compile
  + clippy + existing suites.
- **`capsem-app`'s update-prompt no longer blocks a tauri/tokio worker
  thread while the user decides.** `check_for_update_with_prompt` in
  `crates/capsem-app/src/main.rs` used `tauri_plugin_dialog`'s
  `.blocking_show()` from inside an `async fn` spawned on the runtime.
  Because the user can leave the dialog sitting for seconds to minutes,
  the blocked thread effectively holds a runtime worker for human time
  -- same anti-pattern we just fixed in the tray (`std::process::Command`
  in async). The fix is NOT `spawn_blocking` (its bounded pool is sized
  for short I/O, not human waits); it's bridging the plugin's
  callback-based `.show(|accepted| ...)` to async via
  `tokio::sync::oneshot`. See `/dev-rust-patterns` "Blocking-in-async
  anti-pattern" and `/dev-bug-review`.
- **`capsem-app` session log now created with mode `0o600`.** The
  per-launch log at `~/.capsem/logs/<timestamp>.jsonl` was opened via
  `File::create`, which applies the user's umask (typically `0644`) and
  leaves the file readable by every local user. The log contains
  tracing spans with VM ids, filesystem paths, provider API metadata,
  and tool-call arguments -- on a shared box that is a user-to-user
  information leak. Factored an `open_log_file(path)` helper that uses
  `OpenOptions::mode(0o600)` under `#[cfg(unix)]` with a plain-options
  fallback elsewhere, matching the established pattern already used by
  `pty_log.rs`, the gateway auth token, per-VM sockets, and
  `capsem-core`'s key helpers. Two new unit tests pin the behavior
  (file round-trips content; mode is exactly `0o600` on Unix). Also
  cleared a pre-existing `needless_borrows_for_generic_args` clippy on
  the deep-link `window.eval` call in the same file. See
  `/dev-rust-patterns` lesson 14.
- **`provision_sandbox` no longer holds the `instances` mutex across
  blocking filesystem work, and no longer probes for stale records on
  every successful provision.** `cleanup_stale_instances` previously
  held the std::sync::Mutex from the `kill(pid, 0)` probe loop all the
  way through the `remove_dir_all` + `remove_file` sweep for every
  evicted ephemeral session -- hundreds of ms of blocking I/O under
  which every other `instances.lock()` caller (~30 sites: list /
  status / stop / delete / suspend / resume / fork / exec handlers)
  stalled. Split into a two-phase contract: `drain_dead_instances`
  probes and evicts under the lock (microseconds), and the caller
  scrubs each evicted entry's filesystem artifacts via the free
  `scrub_evicted_instance` with the lock released. Additionally gated
  the probe itself: `provision_sandbox` now only runs it when
  `instances.contains_key(id)` or the map is already at
  `max_concurrent_vms` -- the two conditions under which stale
  reclamation could unblock the caller. Three regression tests pin
  the drain contract (dead-only eviction, no-op when all alive, mutex
  released on return). Follow-up to commit 34d0e3f.
- **`POST /run` no longer blocks the tokio reactor on provision.**
  `handle_run` at `crates/capsem-service/src/main.rs:2484` was calling
  `state.provision_sandbox(...)` directly from the axum async handler,
  missed by commit 34d0e3f's spawn_blocking sweep that covered
  `handle_provision` and `handle_fork`. Same blocking I/O
  (APFS clonefile, `rootfs.img` fsync, walkdir, subprocess spawn),
  same fix -- wrap in `tokio::task::spawn_blocking` with the runtime-
  handle thread-local preserved for the inner
  `tokio::process::Command::spawn`.
- **Cleared 18 pre-existing clippy errors surfaced by running
  `-D warnings` across the provision path's dependency graph:** 3 in
  `capsem-service/main.rs` (two `u64 as u64` casts in
  `attach_summary_telemetry`, one `iter_kv_map` in the MCP refresh
  broadcast), 6 in `capsem-core/asset_manager.rs` (five `iter_kv_map`,
  one `collapsible_if` in the asset-version resolver), 1 in
  `capsem-core/setup_state.rs` (field reassignment after
  `Default::default()` in a unit test), 1 `incompatible_msrv`
  (addressed via the MSRV bump above), 4 redundant closures + 3
  redundant `+ 0` operands in `capsem-logger/reader.rs` test fixtures.
  None were behavioral; the redundant-closure fixes convert
  `|row| read_*_row(row)` to `read_*_row`.
- **`just test` no longer self-destructs across parallel workers from a
  broad `pkill`.** Four sites fired `pkill -9 -x capsem-service` (or
  `-f capsem-service`) which matched every `capsem-service` on the box,
  including every other pytest-xdist worker's test service. A single
  install-tests fixture running `simulate-install.sh` took the whole suite
  down -- reproducibly -- pushing ~148 tests into "service refused
  connection" / "VM never exec-ready" cascades. Each site now scopes the
  match to its own install prefix:
  - `scripts/simulate-install.sh` matches `$INSTALL_DIR/<name>`.
  - `tests/capsem-install/conftest.py::_kill_service` matches
    `$INSTALL_DIR/<name>`.
  - `tests/capsem-install/test_service_install.py` matches
    `$INSTALL_DIR/capsem-service`.
  - `crates/capsem/src/uninstall.rs` and
    `crates/capsem/src/service_install.rs` use `current_exe().parent()` to
    scope `pkill` to the binary's own install directory -- semantically
    also correct in production: `capsem uninstall` from `~/.capsem/bin`
    only affects processes launched from `~/.capsem/bin`, leaving dev
    services under `target/debug/` alone.
- **Gateway tests now pass their parent PID.** `tests/helpers/gateway.py`
  spawned `capsem-gateway` without `--parent-pid`, so `capsem-guard`
  returned `Err(NoParent)` and the gateway exited 0 immediately. Every
  gateway fixture then failed its 10s readiness wait and every gateway
  test (60+ errors, ~10 failures) cascade-failed at setup. Helper now
  passes `--parent-pid=os.getpid()` and `--run-dir` so the per-test
  singleton lock lands in the test tmp dir.
- **Built-in MCP snapshot tools: `snapshots_history` dropped its `path`
  argument and `snapshots_compact` dropped its `name` argument** because
  `SnapshotPaginationParams` / `SnapshotCompactParams` in
  `crates/capsem-mcp-builtin/src/main.rs` didn't declare those fields.
  rmcp's typed-parameter deserialiser silently discarded the unknown
  keys, so every `snapshots_history` call returned
  `-32602 missing 'path' argument`. Added a dedicated
  `SnapshotHistoryParams` struct and extended `SnapshotCompactParams`.
  Also renamed `SnapshotCheckpointParams` → `SnapshotRevertParams` with
  `path: String` required and `checkpoint: Option<String>` optional
  (matches `handle_revert_file`'s auto-pick-newest behaviour).
- **Built-in MCP tool failures are now `isError: true` on the result
  instead of a success-shaped result containing error text.**
  `extract_text` in `capsem-mcp-builtin` returned
  `Ok(text)` for every tool response, including the ones where
  `call_builtin_tool` had set `isError: true`. rmcp maps `Err(String)`
  to the wire-level `isError` result, so blocked-domain and
  invalid-URL rejections from `fetch_http` / `grep_http` / `http_headers`
  went through as regular successes. Now propagated correctly.
- **Three built-in HTTP tools carry MCP annotations.** Added
  `annotations(title, read_only_hint, destructive_hint, idempotent_hint,
  open_world_hint)` to `fetch_http`, `grep_http`, `http_headers` so their
  `tools/list` output matches the file-tool annotations the MCP spec
  expects clients to surface.
- **Guest `snapshots` CLI calls now use namespaced tool names.** The
  in-VM `snapshots` helper in `guest/artifacts/snapshots` called
  `snapshots_create`/`_list`/`_revert`/`_delete`/`_history`/`_compact`
  against the host MCP gateway, which namespaces aggregator tools as
  `{server}__{tool}` -- so every bare call returned
  `-32603 tool call failed` and every `snapshots …` command inside the
  VM died with the capsem-mcp-server stderr bleeding into the CLI
  error. Prefixed each call with `local__`. See
  `crates/capsem-core/src/mcp/types.rs::namespace_name` and the `local`
  key in `config/defaults.json`.
- **Tray no longer flashes the menu bar during tests.** Added
  `CAPSEM_TRAY_HEADLESS` env var to `capsem-tray` -- when set, the
  binary still arms parent-watch and acquires the singleton flock but
  skips `NSStatusItem` / `TrayIconBuilder` creation and idles. The
  integration test helpers (`tests/helpers/service.py`,
  `tests/capsem-mcp/conftest.py`) no longer pass `--tray-binary` at all;
  the tray-focused `tests/capsem-service/test_companion_lifecycle.py`
  keeps spawning the tray but in headless mode. Full-suite runs now
  create zero menu-bar icons.
- **In-VM diagnostic test suite realigned to current product behaviour.**
  `guest/artifacts/diagnostics/test_mcp.py` and `test_network.py` had a
  cluster of stale assertions: tool names without the `local__`
  namespace prefix that the gateway applies, the four
  `pytest.raises(AssertionError)` blocks that were only catching the
  "tool not found" protocol error (now that the tools are found and
  exercise real error paths), `mcpServers["capsem"]` instead of the
  canonical `["local"]` key from `config/defaults.json`, `fetch_http` /
  `grep_http` / `http_headers` expecting `{"isError": true}` where the
  host code was returning a success result containing error text,
  `list_changed_files` expected in `tools/list` (renamed to
  `snapshots_changes`), and `test_denied_domain_rejected` using
  `api.openai.com` which is policy-gated by `CAPSEM_OPENAI_ALLOWED`
  (returns 401 when enabled, not the 403 the test wanted).
  Introduced `ns()` / `_init_and_call` auto-prefix + `_assert_tool_error`
  helpers; `_init_and_call` now collapses JSON-RPC errors into
  `isError: true` tool results so callers see a single shape regardless
  of where the failure originated.
- **`tests/capsem-stress/test_rapid_exec.py::test_rapid_file_io` hit a
  404 on every iteration.** The test POSTed to `/write-file/{id}` and
  `/read-file/{id}` (dashes) but the service routes are `/write_file/`
  and `/read_file/` (underscores); it also sent `data: list[int]` bytes
  where the endpoint expects `content: str`. Fixed.

### Added
- **`capsem-guard` crate** -- new tiny library (`crates/capsem-guard/`) with
  parent-watch + singleton flock primitives. Used by `capsem-gateway` and
  `capsem-tray` to make them non-standalone companions of `capsem-service`:
  they refuse to start without a valid `--parent-pid`, acquire a system-wide
  singleton lock, and self-exit within 100 ms when the parent dies. Works
  under SIGKILL, OOM, and pytest-xdist worker death -- scenarios where
  `tokio::process::Command::kill_on_drop(true)` silently does nothing.
  Implementation details: `getppid()`-based watcher (immune to zombie state),
  `O_CLOEXEC`-atomic `flock(2)` with a process-local registry to cover the
  fork-to-exec window, global tray lock at `~/.capsem/run/tray.lock` (one
  menu-bar icon system-wide). 31 Rust unit tests + 15 adversarial Python
  integration tests in `tests/capsem-service/test_companion_lifecycle.py`
  (refuse-standalone × 4, singleton × 3 incl. 20-way hammer, dies-with-parent
  × 2, service-SIGKILL end-to-end × 1, timing-budget regression guards × 5).
  See `/dev-rust-patterns` lesson 18.

### Fixed
- **Tray action dispatch no longer stalls its tokio worker on fork/exec.**
  `launch_ui` and `launch_ui_action` in `crates/capsem-tray/src/main.rs`
  called `std::process::Command::spawn` synchronously from the async
  `dispatch_action` path. Because the tray runs a `new_current_thread`
  tokio runtime (one worker), each Connect / New Session / Save / Fork
  click briefly froze status polling and further action dispatch during
  the `posix_spawn`/`fork+exec` syscall. Swapping to
  `tokio::process::Command` would not have helped -- its `spawn()` still
  invokes the same blocking syscall. Both launches now run on a
  dedicated `std::thread::spawn` (not `tokio::task::spawn_blocking`, whose
  bounded worker pool is the wrong fit for the reaper's long
  `Child::wait()`) and the child is now reaped, eliminating zombie
  accumulation on the long-lived tray process. Deduped the two
  near-identical launch bodies behind `find_capsem_app_binary` + a pure
  `build_launch_invocation` helper, covered by 6 new unit tests pinning
  deep-link construction for the direct-binary and `open -a Capsem`
  fallback paths. Also fixed a `clippy::redundant_closure` around
  `tray_lock_path`. See `/dev-rust-patterns` "Blocking-in-async
  anti-pattern" and `/dev-bug-review`.
- **`POST /fork/{id}` and `POST /sandboxes` (provision) no longer block
  the tokio reactor during heavy filesystem work.** `handle_fork` called
  `capsem_core::auto_snapshot::clone_sandbox_state` directly from the axum
  handler; `handle_provision` called the synchronous
  `ServiceState::provision_sandbox`, which wraps the same clone plus a
  `sync_all()` flush of `rootfs.img` and a walkdir-based `disk_usage_bytes`.
  Under concurrent fork/provision load these could exhaust axum worker
  threads and stall unrelated requests. Both call sites are now wrapped in
  `tokio::task::spawn_blocking`, matching the established pattern in the
  same file (`handle_upload`, `list_dir_recursive`, `handle_detect_host_config`,
  the `remove_dir_all` cleanups). The sync-to-spawn_blocking handoff
  preserves the tokio runtime handle via thread-locals, so the
  `tokio::process::Command::spawn` call inside `provision_sandbox` still
  works. All 116 capsem-service tests remain green.
- **AI traffic parsers no longer build a full JSON DOM for tool call args
  and responses.** Three places in `crates/capsem-core/src/net/ai_traffic/`
  parsed LLM SSE payloads into `serde_json::Value` only to stringify them
  (Gemini `functionCall.args` in `google.rs`, Gemini `functionResponse.response`
  in `request_parser.rs`) or not use them at all (OpenAI Responses API
  `ResponseInfo.output`). Switched the two stringified sites to
  `Box<serde_json::value::RawValue>` so the fragment is kept as a lazy byte
  slice and re-emitted verbatim without an intermediate `BTreeMap`/`Vec` DOM
  allocation; deleted the unused OpenAI `output` field entirely. Enabled the
  workspace `serde_json` `raw_value` feature. Added two regression tests
  (`stream_function_call_preserves_arg_bytes_verbatim`,
  `google_function_response_preserves_bytes_verbatim`) pinning the byte-
  verbatim preservation behavior -- RawValue keeps whitespace and key order
  as-sent, where `Value` would re-serialize to canonical-compact form. See
  `/dev-rust-patterns` lesson 6.
- **Companion processes no longer leak across interrupted test runs.**
  `just test -n 4` under ctrl-C / pytest-xdist worker death / SIGKILL left
  `capsem-gateway` and `capsem-tray` reparented to PID 1 because their only
  cleanup hook was `kill_on_drop(true)`, which does not fire on ungraceful
  exit. Accumulated orphans caused downstream "vm-ready never asserted"
  poll spins, UDS connection refusals, and the suspend/resume regression.
  Fixed by wiring all companions through the new `capsem-guard` library;
  the contract is enforced on the companion side so the spawner can't get
  it wrong.

### Changed
- **Linux CI now measures coverage for every portable host crate.** The
  `llvm-cov nextest --codecov` invocation on the KVM runner previously
  tested only 8 of 14 workspace members. Added `capsem-agent` (118 tests),
  `capsem-gateway` (128), `capsem-process` (72), and `capsem-guard` (31)
  -- none of which had macOS-only code paths gating their Linux build.
  The only crates still excluded from Linux CI are `capsem-app` (Tauri
  shell) and `capsem-tray` (`muda` menu-bar), both genuinely macOS-only.
  Net effect: ~349 additional Rust tests now contribute to the Codecov
  dashboard from the Linux side, catching Linux-specific regressions the
  macOS run cannot. Added a new `Guard` component to `codecov.yml` so the
  crate shows up alongside Gateway, Service, CLI, etc.

### Fixed
- **`just ui` / `just shell` re-invocation no longer leaves the dev service
  without a gateway.** Three related bugs in the companion-shutdown path
  collectively caused the new gateway to hit `EADDRINUSE` on port 19222
  whenever the prior dev service was killed (SIGTERM or SIGKILL) and a
  new one spawned within `_ensure-service`'s 500 ms restart budget. User
  symptom: the frontend WebSocket connected briefly (served by the
  orphan gateway), then dropped when the orphan's parent-watch fired,
  after which every reconnect hit "connection refused". Each bug has a
  dedicated regression test in `tests/capsem-service/test_companion_lifecycle.py`.
  - `capsem-service` killed VMs *before* companions on graceful shutdown.
    `kill_all_vm_processes` includes an unconditional 500 ms SIGTERM-grace
    `thread::sleep`, so companion-kill didn't run until at least 500 ms
    after SIGTERM -- exactly when the new service was spawning its own
    gateway. Fixed by reordering graceful_shutdown to kill companions
    first. Guard: `TestServiceSigtermReapsCompanionsPromptly` (300 ms
    budget).
  - `kill_all_vm_processes` slept 500 ms *even when zero VMs were
    running*, inflating every shutdown by half the `_ensure-service`
    budget. Fixed by early-returning when the VM list is empty and
    skipping the grace sleep when no VM was actually signalled. Guard:
    `TestServiceShutdownIsFastWithoutVMs` (300 ms full shutdown budget).
  - `capsem-guard`'s parent-watch polled every 500 ms, so a SIGKILL'd
    service's companions could remain alive up to a full poll interval --
    the full `_ensure-service` budget by itself. Tightened
    `PARENT_POLL_INTERVAL` from 500 ms to 100 ms (`getppid()` is a vDSO
    call; cost is negligible). Guard: `TestCompanionsDieFastAfterServiceSigkill`
    (300 ms budget on SIGKILL path). Also documents an end-to-end
    restart contract in `TestServiceRestartSequenceKeepsGatewayHealthy`.

- **`just _clean-stale` no longer hangs for minutes.** The bash body called
  `lsof -tU "$s"` once per socket in `/tmp/capsem/*.sock`. On macOS each call
  scans every process's FD table (~200 ms), so after ~1700 dead sockets
  accumulated the loop took ~6 minutes and made `just test` / `just smoke` /
  `just install` / `just build-assets` look stuck. Replaced the entire recipe
  with `scripts/clean_stale.py`, which probes socket liveness via
  `socket.connect()` (~4 us per socket, ~50000x faster) and ports the other
  stages (stale rootfs/`_up_` dirs, stale test fixtures, cargo artifact
  age-prune) to Python. Measured: 1772 orphan sockets + 926 stale cargo dirs
  cleaned in 3.2 s total; steady-state second run 1.3 s. Covered by 16 pytest
  cases in `tests/capsem-cleanup-script/` including a 2000-socket perf guard
  that fails if the regression ever returns.

- **`just test -n 4` concurrency cascade** -- four independent bugs surfaced as
  "flaky tests" whenever pytest ran with parallel workers. Collapsed the cascade
  from ~130 test failures down to ~5.
  - **`capsem-service` is now self-idempotent on startup.** New
    `crates/capsem-service/src/startup.rs` probes `/version` on the target UDS
    and an adjacent advisory flock serialises the probe→remove-stale→bind
    critical section. Four parallel `capsem-service --uds-path X` invocations
    converge on exactly one running service; losers exit 0 when the version
    matches, exit non-zero on mismatch (never auto-kill).
  - **`capsem-gateway` honours the service's `run_dir`.** New `--run-dir` flag
    (plus `CAPSEM_RUN_DIR` env fallback) replaces the `$HOME/.capsem/run`
    hardcode. The service passes it when spawning the gateway child, so
    `gateway.{token,port,pid}` land where the service polls for them. The
    gateway also writes `gateway.port` *after* `TcpListener::bind` so
    OS-assigned ports (`--gateway-port 0`) are recorded correctly instead of
    persisting the configured `0`.
  - **`axum::serve` no longer blocks on `gateway-ready`.** `spawn_companions`
    ran inline before `axum::serve`, delaying UDS accept by up to 5 s per
    startup while polling for `gateway.token`. Companion spawning is now
    detached via `tokio::spawn`, so the UDS accepts the instant it binds.
    Companion children are parked in a `Mutex<Vec<Child>>` and explicitly
    killed on graceful shutdown (kill_on_drop handles crash paths).
  - **CLI `run_dir` derives from `--uds-path`.** When `--uds-path` is explicit
    (tests, custom deployments), `crates/capsem/src/main.rs` now takes the
    parent directory as `run_dir` instead of falling back to
    `CAPSEM_RUN_DIR`/`$HOME`. Keeps doctor logs and inherited paths consistent
    with wherever the service actually writes.
- **`ProvisionResponse.uds_path` is the source of truth for instance sockets.**
  Clients were recomputing `<run_dir>/instances/{id}.sock`, but the service
  falls back to `/tmp/capsem/<hash>.sock` when the preferred path exceeds
  macOS's 104-byte `SUN_LEN`. The fallback hash uses process-randomised
  `DefaultHasher`, so clients *cannot* reliably recompute. The provision
  response now includes the server-chosen path; `capsem doctor` uses it
  directly (fixes "Session did not become ready within 30s" on e2e tests
  rooted under `/var/folders/...`).

### Changed
- **Shared `capsem_core::uds` module** -- extracted `SUN_PATH_MAX` and
  `instance_socket_path` into `crates/capsem-core/src/uds.rs` so the
  SUN-length workaround lives in exactly one place. Service delegates; clients
  use it as a fallback only when talking to a pre-`uds_path` service.
- **`capsem doctor` uses the shared poll helper** -- the hand-rolled
  `loop { if sock.exists() ... sleep 200ms }` waiting for the per-VM IPC
  channel was replaced with `capsem_core::poll::poll_until`, same primitive
  already used by CLI/service/MCP.

### Changed
- **Crate `capsem-ui` renamed to `capsem-app`** -- crate and binary name now match the directory. Tauri identifier (`com.capsem.capsem`), productName (`Capsem`), and code-signing/notarization are unaffected. `justfile`, CI workflow, `capsem-tray` binary path lookups, `capsem-build-chain` tests, and the relevant skills were updated. localStorage keys `capsem-ui-mode` / `capsem-ui-font-size` were intentionally left unchanged to preserve user preferences across upgrades.
- **Workspace Cargo metadata** -- `[workspace.package]` now carries `description`, `license = "Apache-2.0"`, `repository`, `homepage`, `rust-version`, and `authors`; every per-crate `Cargo.toml` inherits via `.workspace = true`. `cargo metadata` consumers (SBOM, GitHub dep graph, cargo-deny) now see canonical values for all 13 crates.
- **Skills drift reconciled** -- `/dev-capsem` crate map no longer has the duplicate `capsem-gateway` row and now lists `capsem-mcp-aggregator` and `capsem-mcp-builtin`. `/dev-mcp` tool table drops three tools that no longer exist (`capsem_image_list/inspect/delete`), adds `capsem_mcp_servers`, `capsem_mcp_tools`, `capsem_mcp_call`, and documents the three-crate MCP subprocess architecture. CLAUDE.md project layout now lists all 13 crates and the skills table matches `skills/` on disk.

### Added
- **`SECURITY.md`** -- vulnerability reporting policy (GitHub Security Advisories), supported versions, disclosure timeline, scope (sandbox escape, MITM bypass, supply-chain integrity) and explicit out-of-scope (anything inside the guest VM by design).
- **`RELEASE.md`** -- human-facing pre/post-release checklist that points back to `/release-process` for depth. Captures the `just cut-release` path, CI pipeline shape, and what to check after the tag is pushed.
- **`rust-toolchain.toml`** -- pins the stable channel + `aarch64-unknown-linux-musl` / `x86_64-unknown-linux-musl` targets so local and CI builds resolve the same toolchain.
- **`docs/usage/mcp-tools.md`** -- user-facing reference for the 22 MCP tools exposed by `capsem-mcp`, grouped by session lifecycle, exec/file, telemetry, MCP aggregator, and diagnostics. Source of truth remains `crates/capsem-mcp/src/main.rs`.
- **`docs/usage/shell-completions.md`** -- how to generate and install bash/zsh/fish/PowerShell completions via `capsem completions <shell>`.
- **Pointer READMEs at `crates/capsem/README.md` and `crates/capsem-proto/README.md`** -- ~10-line README each for the two externally-visible crates, linking to capsem.org.

### Added
- **capsem/setup.rs tests + small DI refactor** -- helpers (`load_state`, `save_state`, each `step_*`) now take `capsem_dir: &Path` explicitly instead of reading it from `$HOME` at call time. `run_setup` still computes the real dir once and threads it through, so the public contract is unchanged. 11 new unit tests cover state-file roundtrip (including atomic overwrite + parent-dir creation), corrupt-state recovery, and `step_corp_config` success / invalid-TOML / missing-file paths against a `tempdir()`. `setup.rs` coverage 0% → 47%.
- **Unit tests for capsem-app helpers** -- `parse_flag`, `cleanup_old_logs`, `format_log_filename` (extracted from `log_filename` for testability). 12 new tests covering the deep-link argument parser and log housekeeping.
- **HTTP-level tests for capsem-tray gateway client** -- new `spawn_http_probe` test helper spins up a single-connection `tokio::net::TcpListener` so `status`, `stop_vm`, `delete_vm`, `suspend_vm`, `resume_vm`, `provision_temp` are exercised end-to-end (happy path + 4xx/5xx + dead host). `GatewayClient::new`/`new_with_base_url` added for injection. `capsem-tray/src/gateway.rs` jumps from 36% to 94% coverage. Also added `parse_port_file` tests against malformed `gateway.port` contents.
- **capsem-gateway Args + event-ws tests** -- clap default/override tests and a `handle_events_ws`-without-Upgrade test. `main.rs` coverage 69% → 75%.
- **capsem-logger reader fixture-based aggregate tests** -- populates net/model/tool/mcp/fs tables and asserts `session_stats`, `top_domains`, `search_net_events`, `net_event_counts`, `recent_net_events`, `tool_calls_for`, `tool_responses_for`. 24 new tests, reader.rs coverage 75% → 79%.
- **Coverage reporting for capsem-ui, capsem-mcp-aggregator, capsem-mcp-builtin** -- these three crates were invisible to Codecov (never in the `-p` list passed to `cargo llvm-cov`). Added to both macOS and Linux CI runs (capsem-ui macOS-only since Tauri). The `tooling` component in `codecov.yml` now includes the MCP subprocess crates; new `systray` component covers `capsem-tray`; `crates/capsem-app/gen/**` added to ignore list so Tauri-generated code doesn't pollute coverage.
- **PTY ring buffer on host for banner replay** -- `capsem-process` now fronts the terminal broadcast channel with a `TerminalRelay` that retains the last 64 KiB of PTY output. Newly-subscribing WebSocket or IPC clients receive the buffered snapshot atomically (snapshot + subscribe under one mutex) before the live stream, so a fresh browser tab sees the shell's login banner even though the shell printed it before the client connected. Covers both `/terminal` WS (frontend) and `StartTerminalStream` IPC (`capsem shell`).
- **Tray menu split by VM kind** -- persistent running: Connect + Stop + Fork + Delete. Persistent stopped/suspended: Resume + Fork + Delete (or just Fork + Delete when fully stopped). Ephemeral running: Connect + Save + Delete (no Stop, since stopping an ephemeral == destroying it). Save and Fork open the desktop app with `--action save|fork` and dispatch a `capsem:tab-action` event that the Toolbar picks up to open the matching dialog -- the tray can't prompt for a name, so the UI owns it.
- **Tray deep-link uses direct binary path** -- `open -a Capsem --args` only forwards args to a *new* launch on macOS; it drops args when the app is already running. `capsem-tray` now invokes `/Applications/Capsem.app/Contents/MacOS/capsem-ui` (or `~/Applications/...`) directly, so `tauri-plugin-single-instance` sees the second launch and forwards `--connect` / `--action` to the running instance.
- **Session-boot overlay in the terminal** -- three pulsing dots + "Setting up session..." shown while the WS is reconnecting but has never received a byte. Overlay inherits the terminal theme background (no flash), switches to "Reconnecting..." once a real byte has been seen (tracked on first `onmessage`, not on `onopen`, so spurious gateway-initiated closes during VM boot don't trigger the wrong label).
- **`ProvisionRequest.ram_mb` / `cpus` now optional** -- service fills missing fields from merged VM settings (`vm.resources.ram_gb`, `vm.resources.cpu_count`). Lets callers without a settings round-trip (the tray's "New Session") honor the user's configured defaults instead of hardcoding.
- **`capsem-app` reverted to thin webview shell** -- 578-line `main.rs` + 9 helper files (`assets.rs`, `boot.rs`, `cli.rs`, `commands/`, `gui.rs`, `logging.rs`, `session_mgmt.rs`, `state.rs`, `vsock_wiring.rs`) collapsed to a 185-line `main.rs` with 3 IPC commands: `log_frontend`, `open_url`, `check_for_app_update`. Drops `capsem-core`, `capsem-logger`, `anyhow`, `reqwest`, `rmp-serde`, and the macOS `objc2-*` deps from the app. All VM/MCP/MITM logic stays in the service daemon; the app only hosts the webview and deep-link handling.
- **Terminal iframe owns its WebSocket lifecycle** -- replaces the parent/iframe `ready`/`vm-id`/`ws-ticket` postMessage handshake with URL-param init (`/vm/terminal/index.html?vm=…&theme=…&mode=…&fontSize=…&fontFamily=…`). The iframe fetches its own gateway token, manages WebSocket lifecycle + exponential-backoff reconnect with fresh tokens. Parent→iframe postMessage now covers only runtime signals (`theme-change`, `focus`, `clipboard-paste`). Removes `MsgReady`, `MsgVmId`, `MsgWsTicket`, `MsgWsConnected` from the contract.
- **Frontend logging to Rust tracing** -- new `frontend/src/lib/tauri-log.ts` patches `console.*` + `window.onerror` + `onunhandledrejection` to forward via `invoke('log_frontend')` from `@tauri-apps/api/core`. Webview logs now land in `~/.capsem/logs/<timestamp>.jsonl` alongside backend events, target `frontend`. No-op outside the Tauri webview (detects via `__TAURI_INTERNALS__`, not the opt-in `window.isTauri` global).
- **Frontend build timestamp in toolbar** -- `__BUILD_TS__` set at Vite build time, displayed right-side of toolbar. Makes stale-bundle issues obvious at a glance.
- **`just build-ui [release]` recipe** -- frontend build + `cargo build -p capsem-ui` in lockstep. Required because `tauri::generate_context!()` embeds the frontend bundle at cargo compile time; rebuilding only the frontend has no effect on an already-compiled binary. Documented in `CLAUDE.md`, `/dev-just`, and `/frontend-design`.
- **`just run-ui -- [args]`** -- `build-ui` then launch `./target/debug/capsem-ui` with passthrough args (e.g., `just run-ui -- --connect <vm-id>`).

### Removed
- **`POST /setup/assets/download`** -- zero callers anywhere (no frontend, no CLI, no MCP tool wraps it). The handler was a stub that always returned `{"started": false, "reason": "asset pipeline not yet wired -- run \`capsem update\` from the terminal"}`. The real asset download path is the `capsem update` CLI. Removing the route and the `handle_trigger_download` handler; if/when an in-service asset pipeline is added later, add it back under the name that matches its behavior.

### Changed
- **`capsem_service_logs` now routes through the service's `/service-logs` endpoint** instead of opening `$CAPSEM_RUN_DIR/service.log` directly. The direct-file read was an inherited shortcut and left two parallel implementations of the same logic (MCP tool + HTTP handler) that could drift. The MCP tool now has a single code path on par with `capsem_vm_logs`; grep/tail filtering is still applied locally on the returned text. Post-mortem reads when capsem-service has crashed are no longer covered by this tool -- use `tail -f ~/.capsem/run/service.log` from the shell, same as every other tool that can't reach a dead service.

### Fixed
- **Suspend/resume: /root reads now survive a VM restore** -- two bugs compounded. (a) `AppleVzSerialConsole::spawn_reader` started the pipe reader inside `machine.start()` before the capsem-process tokio broadcast subscriber attached, so the first ~100ms of post-resume serial output was dropped by `tokio::broadcast::send` (no receivers, message discarded). The serial log showed no reconnect/rebind activity even though the guest agent was running it, which masked the next bug for months. Now `AppleVzHypervisor::boot` attaches a file-writer subscriber *before* `machine.start()` spawns the reader; log path flows through a new `serial_log_path` field on `VmConfig` / `BootOptions`. The duplicate subscriber in `capsem-process/src/main.rs` is removed. (b) After resume the guest agent has to rebind `/root` onto a fresh virtiofs mount because the old connection is gone, but the chroot's `/mnt/shared` path wasn't created in the rootfs, and `mount --bind` was firing before the new virtiofs had completed its FUSE init handshake with the new host virtiofsd -- so the bind captured a stale-empty subtree and `/root` stayed ENOENT even though the mount reported success. Agent now does `mkdir -p /mnt/shared`, mounts the virtiofs, polls `/mnt/shared/workspace` until `exists()` succeeds (20ms x 50 attempts), then binds to `/root`. Plus the supporting plumbing: persists `VZGenericMachineIdentifier` across save/restore (else `restoreMachineStateFromURL` fails with `VZErrorRestore(12)`), dispatches VZ `pause`/`save_state`/`stop` via `CFRunLoopPerformBlock` (NOT `dispatch_async(main_queue)` -- that deadlocks on VZ's own completions), `fsync`s the checkpoint before process exit, clears stale `.ready` + UDS on resume so `wait_for_vm_ready` doesn't match the prior boot, subscribes every IPC connection to the state broadcast (was only `TerminalOutput`), tolerates `fsfreeze` ENOTSUP on the VirtioFS root, and the agent reconnects via a 3s heartbeat + `POLLHUP` on the vsock fd after the host process disappears. The MCP `test_suspend_and_resume_persistent` xfail is removed; the lifecycle test's `marker in str(read_resp)` substring check is tightened to require `"content" in read_resp` since the ENOENT error message echoes the path (the old assertion was a false-positive on the failing path).
- **`capsem-mcp` now respects HTTP status codes when talking to capsem-service** -- `UdsClient::request` used to discard the response status and try to deserialize every body as success, so a non-2xx response with a JSON body (e.g. the `{"error": "..."}` payload the service returns on 502/503/400) was handed back to the tool layer as `Ok(value)` with an embedded error field. `capsem_mcp_call` printed the raw 502 body as a successful tool result; other tools only avoided this because they happen to run `format_service_response` which catches the embedded `error` key. The client now reads `status()` first and returns `Err("502 Bad Gateway: ...")` on non-success, preferring the `error` field from the JSON body when present.
- **`capsem_core::setup_state::load_state` now warns on corrupt files** -- previously a malformed or truncated `~/.capsem/setup-state.json` was silently swallowed and the function returned `SetupState::default()`, so `capsem setup` would quietly report "no steps done" and re-run the whole wizard with no indication anything was wrong. Now logs a `warn!` with the path and parse error before falling back; behavior on a missing file (the first-run case) is unchanged.
- **`DbReader::query_raw` now validates SQL up front** -- previously it relied on `SQLITE_OPEN_READ_ONLY` at the connection level, which made in-memory readers accept writes and produced cryptic "attempt to write a readonly database" errors on the file-backed path. Now calls `validate_select_only` first, returning a clear `<KEYWORD> statements are not allowed` message consistently. Defense-in-depth; no behavior change for valid SELECT queries.
- **Terminal iframe src must end in `index.html`** -- Tauri's custom protocol on macOS does not auto-append `index.html` for trailing-slash paths the way Vite/Astro dev server does. `/vm/terminal/` silently 404'd in the Tauri webview while working in Chrome dev mode.
- **CSP re-enabled without blocking Astro hydration** -- production CSP on the terminal iframe now includes `'unsafe-inline'` for `script-src` (Astro emits inline hydration scripts in prod). `connect-src` stays locked to gateway + localhost, which is the meaningful defense against a compromised terminal exfiltrating data.

### Added (existing work, continued)
- **Files API: path sanitization and Magika init** -- allowlist-based `sanitize_file_path` (strips XSS, null bytes, unicode, rejects `..` traversal), `resolve_workspace_path` (canonicalize + starts_with check), and shared `Mutex<magika::Session>` in `ServiceState` for AI-powered file type detection.
- **GET /files/{id} directory listing** -- recursive host-side VirtioFS directory listing with file metadata (size, mtime), Magika file-type detection (label, MIME, is_text) at all depths, hidden file filtering, configurable depth (1-6).
- **GET/POST /files/{id}/content** -- binary-safe file download (raw bytes + Magika MIME type + Content-Disposition) and upload (raw bytes, create_dir_all, mode 0644) via host-side VirtioFS. 10MB limit enforced server-side.
- **Files tab UI** -- host-side file tree replaces vsock `find` command (real sizes, Magika labels, no frame limit), syntax-highlighted file viewer with copy-to-clipboard and download buttons, inline image/SVG preview, binary file handling, drag-and-drop upload with visual overlay and status feedback. Shiki language detection expanded to 30+ languages with content-sniffing fallback.
- **Orthogonal asset versioning** -- binary version (`1.0.{timestamp}`) and asset version (`YYYY.MMDD.patch`) are fully independent. The v2 manifest has separate `assets` and `binaries` sections with `min_binary`/`min_assets` compatibility ranges, deprecation tracking, and release dates. Assets use hash-based filenames (`rootfs-{hash16}.squashfs`) via hardlinks for zero-cost dedup.
- **`capsem status` shows full system health** -- version, service/gateway connectivity and version sync (catches stale processes), asset version with per-file ok/MISSING status.
- **Service `/version` endpoint** -- returns the running service binary version for staleness detection.
- **`/setup/assets` uses resolved paths** -- returns hash-named file paths and asset version instead of hardcoded logical names.

### Changed
- **MCP builtin tools refactored to standalone server** -- HTTP tools (fetch_http, grep_http, http_headers) and snapshot tools (snapshots_changes, snapshots_list, snapshots_revert, etc.) extracted from gateway into `capsem-mcp-builtin`, a stdio MCP server subprocess managed by the aggregator like any external server. Gateway dispatch simplified to route all tool calls uniformly through the aggregator.
- **MCP aggregator IPC switched to MessagePack** -- NDJSON protocol replaced with length-prefixed msgpack frames for better performance and binary safety.
- **MCP server definitions support stdio transport** -- `McpServerDef` gains `command`, `args`, `env` fields. Auto-detected stdio servers from Claude/Gemini configs are now connectable (previously display-only). `unsupported_stdio` field removed.
- **MCP server renamed from "Capsem" to "local"** -- the builtin server is now named "local" in both the settings tree and runtime API for consistency.
- **frontend: MCP section with collapsible server cards** -- each server card expands to show its tools with per-tool allow/ask/block permission selectors. Runtime status badges (running/stopped, tool count) from mcpStore. Refresh button in header.
- **frontend: MCP settings wired to gateway** -- MCP server add/remove/toggle and policy now persist via the settings API. Config reload broadcasts to running VMs immediately.
- **frontend: toolbar redesign** -- hamburger menu on left with view switcher, VM actions moved to dropdown menu, live stats (tokens, tool calls, cost) on the right. Shell OSC title shows in center.
- **frontend: settings page loading states** -- spinner while loading, error banner with retry on failure.
- **frontend: restart button** -- toolbar restart now stops then resumes the VM.
- **frontend: fork auto-opens tab** -- forking a VM automatically opens it in a new tab.

### Added
- **Host-side command recording (3 layers)** -- records all shell commands from the host for tamper-proof auditing:
  - Layer 1 (exec_events): structured API-path commands logged to session.db at dispatch time
  - Layer 2 (pty.log): raw PTY transcript with timestamps and direction tags, 20MB rotation
  - Layer 3 (audit_events): kernel execve syscalls via auditd, streamed over vsock:5006 to session.db
- **`capsem history` CLI** -- `capsem history <session>` with `--layer`, `--search`, `--tail`, `--json` flags
- **History API endpoints** -- `GET /history/{id}`, `/history/{id}/processes`, `/history/{id}/counts`, `/history/{id}/transcript`
- **Cross-session history index** -- `exec_count` and `audit_event_count` columns in main.db sessions table
- **Kernel audit support** -- CONFIG_AUDIT + CONFIG_AUDITSYSCALL in guest kernel, auditd started in capsem-init with immutable rules
- **`capsem-mcp-builtin` crate** -- standalone stdio MCP server binary for local tools (HTTP + snapshot). Spawned by the aggregator as "local" server, tools discovered and cached like any external server.
- **MCP aggregator subprocess** -- external MCP server connections now run in an isolated `capsem-mcp-aggregator` subprocess with only network access, no VM/DB/filesystem privileges. Spawned by capsem-process at boot.
- **service MCP API endpoints** -- `GET /mcp/servers`, `GET /mcp/tools`, `GET /mcp/policy`, `POST /mcp/tools/refresh`, `POST /mcp/tools/{name}/approve`, `POST /mcp/tools/{name}/call` unblock the frontend and CLI.
- **CLI `capsem mcp` subcommands** -- `capsem mcp servers`, `capsem mcp tools`, `capsem mcp policy`, `capsem mcp refresh`, `capsem mcp call`.
- **debug MCP tools** -- `capsem_mcp_servers`, `capsem_mcp_tools`, `capsem_mcp_call` in capsem-mcp for AI agent MCP management.
- **MCP IPC protocol** -- `McpListServers`, `McpListTools`, `McpRefreshTools`, `McpCallTool` service-to-process messages with corresponding result types.

### Changed
- **frontend: MCP settings wired to gateway** -- MCP server add/remove/toggle and default tool policy now persist via the settings API instead of local-only state. Servers, tools, and policy load from the gateway on mount.
- **frontend: restart button works** -- toolbar restart button now stops then resumes the VM (was previously identical to stop).
- **frontend: fork auto-opens tab** -- forking a VM from the toolbar now automatically opens the forked VM in a new tab.
- **frontend: settings loading/error states** -- settings page shows a spinner while loading and an error banner with retry on failure.

### Fixed
- **Stale update cache suggests downgrade** -- `read_cached_update_notice` now re-validates with `is_newer` before displaying, preventing bogus "Update available: 1.0.x -> 0.16.x" notices after a version scheme change.
- **Install leaves stale gateway token** -- `just install` now unloads the LaunchAgent before killing processes, preventing macOS from respawning the old service. Cleans stale `gateway.token` and `gateway.port` files.
- **Asset resolution in arch subdirs** -- `ManifestV2::resolve` checks both `base_dir/{hash}` and `base_dir/{arch}/{hash}`, fixing installed service asset lookup.
- **`_pack-initrd` skips docker when binaries are current** -- avoids unnecessary container cross-compile on every `just shell`.
- **v1 asset code removed** -- `asset_manager.rs` reduced from 1947 to ~400 lines. All v1 types, download infra, and legacy cleanup deleted. Download stubs point to `sprints/orthogonal-ci/plan.md`.
- **MITM cert "not yet valid" after Mac sleep** -- leaf certificates now use a fixed `notBefore` of 2026-01-01 instead of `now - 1h`, preventing cert validation failures when the guest clock drifts. Ping messages now carry `epoch_secs` so the guest clock resyncs every 10s heartbeat, covering Mac sleep/wake and long-running VMs.
- **frontend: tab names use VM name** -- provisioning and deep-link flows now show the VM's fun name (e.g. "tmp-agile-blaze") instead of the raw ID.
- **frontend: snapshot stats query real VM** -- Snapshots tab in Stats view now queries the VM's session.db via `/inspect` instead of the local mock database.
- **frontend: VM logs and service logs wired** -- VM Logs view parses NDJSON process logs into structured table with level/source/message columns and Process/Serial toggle. Service Logs view fetches from new `/service-logs` endpoint.
- **frontend: detail panel restored** -- click any tool call, network request, or file event row in Stats to open a slide-out detail panel with Shiki syntax-highlighted JSON, headers, and request/response bodies.

### Changed
- **frontend: removed URL bar** -- toolbar no longer shows the address/search bar; cleaner layout with just VM actions and view switcher.
- **frontend: removed Inspector tab** -- Inspector view removed from the toolbar view switcher; available via hamburger menu if needed.
- **frontend: removed status dot** -- connection indicator dot removed from toolbar.
- **service: added /service-logs endpoint** -- returns last 100KB of service.log as plain text for the frontend Service Logs view.

### Changed
- **build: auto-prune stale cargo artifacts** -- `_clean-stale` now removes orphaned `.o`/`.rlib`/`.rmeta` files and incremental dirs older than 3 days when `target/` exceeds 10 GB. Runs automatically after `test`, `smoke`, and `install` to prevent unbounded growth (previously hit 72 GB from accumulated hash variants).
- **CLI: simplified command structure** -- removed `service` subcommand group; `install`, `status`, `start`, `stop` are now top-level commands. Removed session-level `stop` (use `suspend` or `delete`) and `status` (use `info`). Removed `start` alias from `create`. Renamed all "sandbox" terminology to "session". Session identifier parameter shows as `<SESSION>` in help.
- **CLI: enriched `list` output** -- table now shows NAME, STATUS, RAM, CPUs, and UPTIME columns instead of the old ID/STATUS/PERSIST/PID.
- **CLI: enriched `info` output** -- shows formatted session details with telemetry (tokens, cost, tool calls, requests) instead of raw JSON. Use `--json` for machine-readable output.
- **CLI: service start/stop** -- new `capsem start` and `capsem stop` commands to start/stop the background daemon via launchctl (macOS) or systemctl (Linux).
- **MCP: tool descriptions updated** -- all tool descriptions now use "session" instead of "VM" or "sandbox".

### Fixed
- **tray: icon stays white template** -- tray icon no longer switches to a dark non-template icon when VMs are running. Always uses the template icon so macOS adapts it to menu bar appearance.
- **tray: VM names no longer truncated** -- VM labels in the tray menu now show the full name or ID instead of truncating to 8 characters.
- **tray: unified "New Session" action** -- replaced "New Temporary" and "New Permanent..." menu items with a single "New Session" that creates a session (save it to make it permanent).

### Added
- **VM identity: fun temporary names** -- ephemeral VMs get memorable names like `tmp-brave-falcon` instead of opaque `vm-1712345678`. Persistent VMs keep user-chosen names. Shell prompt now shows the VM name (hostname) instead of static "capsem".
- **VM identity: host timezone injection** -- guest VMs inherit the host's timezone at boot via `TZ` env var and `/etc/localtime`. `date` inside the VM now shows local time instead of UTC. Clock and timezone are also resynced on resume from suspend.
- **service: settings endpoints** -- `GET /settings` returns the merged settings tree (user + corp + defaults) with issues and presets. `POST /settings` batch-updates settings atomically. `GET /settings/presets` lists security presets. `POST /settings/presets/{id}` applies a preset. `POST /settings/lint` validates config. All endpoints are thin wrappers around existing `capsem-core` functions.
- **service: telemetry-enriched `/list`** -- running VMs in `GET /list` now include live telemetry (tokens, cost, tool calls, requests, file events) read from session.db. Shared `enrich_telemetry()` function used by both `/list` and `/info/{id}`.
- **gateway: telemetry pass-through in `/status`** -- `VmSummary` now includes 11 optional telemetry fields forwarded from the service. Frontend gets per-VM stats in a single poll without per-VM API calls.
- **frontend: dashboard global stats** -- NewTabPage shows 4 summary cards (sessions, total tokens, total cost, requests) from `GET /stats` cross-session aggregation.
- **frontend: VM table telemetry columns** -- Uptime, Tokens, Cost columns in the sandbox table. Shows "--" for stopped VMs, live values for running.
- **frontend: `getStats()` API** -- new API function with graceful offline handling (returns empty stats when disconnected).
- **frontend: shared formatters** -- `format.ts` with `formatUptime`, `formatTokens`, `formatCost`, `formatDuration`, `formatBytes`, `formatTime`, `truncate`, `fmtAge`. StatsView refactored to use shared module.
- **standalone installer: macOS .pkg build** -- `scripts/build-pkg.sh` assembles a .pkg from the Tauri .app, all 6 companion binaries, VM assets, and a postinstall script that copies to `~/.capsem/bin/`, codesigns, registers LaunchAgent, and runs setup. CI pipeline updated to build .pkg alongside .dmg.
- **`just install` builds and installs the platform package** -- builds release binaries, frontend, and Tauri app, then assembles and installs the native package: .pkg with macOS Installer GUI on macOS, .deb via `dpkg -i` on Linux. The postinstall script handles codesign, PATH, service registration, and setup. Replaces the old `simulate-install.sh` bypass.
- **`just test-install` exercises the real .deb path** -- Docker e2e tests now build a real .deb (Tauri + `repack-deb.sh`), install with `dpkg -i` (exercising `deb-postinst.sh` with systemd registration and setup), then run the pytest suite against the installed layout. Named volumes cache cargo builds across runs. Tests split into packaging (run in Docker) and `live_system` (need VM assets, run on real systems).
- **standalone installer: Linux .deb repack** -- `scripts/repack-deb.sh` injects companion binaries and a postinst script into the Tauri .deb. Postinst symlinks system binaries to `~/.capsem/bin/`, registers systemd user unit, and runs setup.
- **CLI: auto-setup on first use** -- running any sandbox command without prior `capsem setup` triggers non-interactive setup automatically (service registration, credential detection, asset download). Skipped when `--uds-path` is explicit.
- **`just install`: graceful stop + health check** -- stops existing service before overwriting binaries, verifies service health after registration, auto-runs setup on first install.

### Fixed
- **CLI: delete nonexistent sandbox now returns error** -- HTTP status code was not checked before deserializing response body, causing 404 errors to be silently swallowed when `T = serde_json::Value` in the untagged `ApiResponse` enum.
- **uninstall: kill and remove all 6 binaries** -- capsem-gateway and capsem-tray were missing from the CAPSEM_BINARIES list and pkill commands.
- **install: .pkg and .deb packaging scripts fail on missing binaries** -- `build-pkg.sh` and `repack-deb.sh` printed a WARNING and continued when a companion binary was missing, potentially producing broken packages in CI. Now exit with error, matching `simulate-install.sh` behavior.
- **install: setup-state.json atomic write** -- `save_state()` used `fs::write()` directly; a crash mid-write could corrupt the setup state. Now uses temp file + `fs::rename` for atomic updates.
- **install: macOS .pkg postinstall user detection** -- postinstall script assumed `$USER` was always set correctly by macOS Installer.app. When installed via `sudo installer -pkg` (CLI), `$USER` is root. Now checks `$SUDO_USER` first, falls back to console owner via `stat /dev/console`.
- **install: Linux .deb postinst XDG_RUNTIME_DIR** -- `deb-postinst.sh` ran `su $TARGET_USER -c "capsem service install"` without propagating `XDG_RUNTIME_DIR`, causing `systemctl --user` to fail. Now passes `XDG_RUNTIME_DIR=/run/user/$UID` explicitly.

### Changed
- **image elimination: everything is a sandbox** -- removed the "image" concept entirely. `fork` now creates a stopped persistent sandbox instead of an image. `create --from <sandbox>` replaces `create --image`. Image registry, image CLI commands, and image MCP tools are all removed. `--image` remains as a hidden alias for `--from`. `SandboxInfo` API now includes `forked_from` and `description` fields. Session DB schema bumped to v6 (renames `source_image` to `forked_from`). Net reduction: ~500 lines and one abstraction layer.
- **CI: test 6 additional Rust crates** -- capsem-service, capsem (CLI), capsem-mcp, capsem-tray, capsem-process now run in CI (422 tests were previously local-only). capsem-app gets a compile check.
- **CI: run non-VM Python integration tests** -- capsem-bootstrap, capsem-codesign, capsem-rootfs-artifacts suites now execute in CI. All 25 integration suites are collect-only verified.
- **CI: Rust coverage floor** -- `--fail-under-lines 70` enforced on both macOS and Linux CI jobs. Codecov unit upload now fails CI on error.
- **capsem-process: module decomposition** -- split 1,522-line main.rs monolith into 6 modules (helpers, job_store, vsock, ipc, terminal + main). Tests grew from 24 to 62.
- **dev-testing skill: test matrix** -- added Rust crate CI matrix, Python integration suite tier map, and coverage targets documentation.
- **integration tests: suite expansion** -- capsem-recovery (4->9 tests: stale sentinels, partial sessions, post-recovery health), capsem-stress (3->7: rapid exec, file I/O, name reuse, mass delete), capsem-config-runtime (5->10: env injection, python3, arch match, workspace write, rootfs readonly), capsem-session-lifecycle (6->10: WAL cleanup, ordered events, domain fields, live DB reads). Fixed two `>= 0` assertions that always passed.

### Added
- **service: `GET /stats` endpoint** -- returns full main.db aggregation in one call: global stats (tokens, cost, tool calls, network counts), recent sessions with all telemetry columns, top providers, top tools, and top MCP tools. Replaces the need for raw SQL on `_main`.
- **service: `/inspect/_main` support** -- the `/inspect/{id}` endpoint now recognizes `_main` as a sentinel, routing raw SQL queries to the global session index (main.db) instead of a per-VM session.db. Unblocks `queryDbMain()` in the frontend.
- **service: `SandboxInfo` telemetry fields** -- `/info/{id}` now returns live session telemetry for running VMs: input/output tokens, estimated cost, tool calls, MCP calls, network request counts, file events, model call count, and uptime. `/list` includes uptime for running VMs. All new fields are optional and omitted when absent for backwards compatibility.
- **gateway: token endpoint for browser auth** -- `GET /token` returns the auth token, restricted to loopback IP (127.0.0.1/::1) via hardcoded peer IP check. Allows browser-based frontends to authenticate without filesystem access.
- **gateway: WebSocket query-param auth** -- `/terminal/{id}` paths accept `?token=` query parameter as auth fallback for browser WebSocket connections (which cannot set custom headers). Only the `token` param is recognized; all others are silently dropped. Non-terminal paths ignore query params entirely.
- **frontend: settings export/import** -- export all settings to JSON file, import from previously exported file. Import stages changes for review before saving. Validates version, skips corp-locked and unchanged settings.
- **frontend: MCP server management UI** -- add/remove/enable/disable external MCP servers from the settings page. Form with name, URL, bearer token, and custom headers. Replaces the "edit config.toml" placeholder.
- **smoke: per-step timing and log file** -- smoke recipe now logs to `target/smoke.log` with elapsed time per step and total. `capsem doctor --fast` skips the 64s throughput download test.
- **smoke: parallel test groups** -- Python integration tests run MCP, service/CLI, and gateway groups concurrently (122s -> 58s). Pre-signs binaries to avoid codesign races.
- **bench: host-side lifecycle and fork benchmarks** -- `just bench` now runs both in-VM benchmarks and host-side lifecycle/fork benchmarks from `test_lifecycle_benchmark.py`.

### Fixed
- **tray: invisible menu bar icon** -- tray main loop used `thread::sleep(16ms)` which does not pump the macOS Cocoa run loop. `NSStatusItem` requires an active run loop to render. Replaced with `CFRunLoopRunInMode` which processes AppKit events at the same 60 Hz cadence.
- **service: companion logs to files** -- gateway and tray child processes had stdout/stderr routed to `/dev/null`, making debugging impossible. Now logs to `~/Library/Logs/capsem/gateway.log` and `tray.log`, falling back to null if the file can't be opened.
- **install: remove stale pgrep wait loop** -- service install no longer polls for capsem-tray death after `bootout` + `pkill -9`, removing up to 3s of unnecessary latency.
- **agent: venv activation race** -- capsem-pty-agent now waits up to 3s for capsem-init's background venv creation before checking `/root/.venv/bin/activate`. Previously the agent checked once and missed it, leaving `VIRTUAL_ENV` unset for the shell and all exec commands.
- **agent: write_nofollow missing parent dirs** -- runtime `FileWrite` via `write_nofollow()` now creates parent directories before opening the file. Previously, writing to `/root/project/main.py` failed with "No such file or directory" if `/root/project/` didn't exist.
- **service: handle_delete now waits for process death** -- `handle_delete` gives the VM process 500ms to flush its session DB, then SIGKILLs if still alive. Previously it was fire-and-forget: the HTTP 200 returned before the process died, leaving orphans when the service was restarted.
- **service: handle_stop now waits for process death** -- same fix as delete. Prevents resume from racing the old process on the same socket (old process's shutdown timer would kill the new one).
- **service: handle_run rollup race** -- session DB rollup (file events, net events, MCP calls) now completes before the HTTP response returns. Previously it was `tokio::spawn`'d fire-and-forget, so callers reading `main.db` immediately after `capsem run` saw empty counters.
- **service: wait_for_process_exit verifies SIGKILL** -- after sending SIGKILL, now polls for up to 2s to confirm the process actually died. Logs a warning/error if it survives.
- **service: socket path fallback for long run_dir paths** -- `instance_socket_path()` falls back to `/tmp/capsem/{hash}.sock` when the preferred `{run_dir}/instances/{id}.sock` path exceeds 90 bytes. Fixes "path must be shorter than SUN_LEN" crashes when tests use `/var/folders/...` temp dirs.
- **install: dev symlink conflict** -- `simulate-install.sh` now removes the `~/.capsem/assets` dev symlink before copying real assets. Previously `cp` failed with "identical file" when the symlink pointed at the source.
- **install: PATH added to correct shell profile** -- install now writes to `~/.bash_profile` (if it exists) instead of `~/.bashrc` on bash, since macOS Terminal opens login shells that don't source `~/.bashrc`.
- **justfile: _ensure-service kills orphaned VM processes** -- kills `capsem-process` instances before killing the service, with a SIGKILL follow-up. Previously, service restart orphaned running VMs.
- **test: codesign race in parallel test groups** -- `sign_binary()` now uses file locking to prevent concurrent test processes from corrupting binaries during `codesign --force`.
- **test: fork timing gate relaxed** -- `test_winter_is_coming` fork gate raised from 0.5s to 2.0s. The proper gate (500ms over 3 runs) is in the dedicated fork benchmark.

### Fixed
- **frontend: syntax highlighting race condition** -- file editor controls (settings.json, state.json, etc.) now reliably get Shiki syntax coloring. Previously the first FileEditorControl to mount could miss highlighting due to async Shiki init not re-triggering the render effect.
- **frontend: design system color violations** -- MCP section badges and status indicators now use semantic tokens (blue=positive, purple=negative) instead of raw green/red Tailwind colors.
- **perf: cold boot 6x faster (6.2s -> 1.0s)** -- first VM boot was wasting 5 seconds on a silent IPC Ping timeout. When `vm_ready` was false, capsem-process silently dropped the Ping and the service waited the full 5s before retrying. Fixed: process now closes the connection immediately on not-ready Ping, and readiness is signaled via a `.ready` sentinel file (stat() check, 5ms poll) instead of IPC round trips. Also deduplicated three hand-rolled exponential backoff implementations (vsock_connect_retry, reconnect loop, CLI connect_with_timeout) into a shared `capsem_proto::poll` module with `RetryOpts` + `retry_with_backoff`, reused by the async `capsem_core::poll::poll_until` via type alias.
- **perf: async VM delete (5s -> 20ms)** -- `shutdown_vm_process` now sends the shutdown signal and returns immediately. Process teardown (wait + force-kill + socket cleanup) runs in a background task. Telemetry rollup in `handle_run` waits for process exit in background before reading session.db, ensuring DbWriter has flushed. Also: consolidated redundant mutex acquisitions in `handle_fork` (4 locks -> 1-2) and `handle_persist` (2 locks -> 1), parallelized IPC fan-out in `handle_reload_config` and `handle_purge` with `join_all`, moved `remove_dir_all` to `spawn_blocking`, added periodic cleanup timer, logged registry save failures.
- **perf: capsem-process self-exit on shutdown** -- after forwarding `HostToGuest::Shutdown` to the guest agent, capsem-process now waits `SHUTDOWN_GRACE_SECS + 500ms` then calls `vm.stop()` and `exit(0)`. Previously, `CFRunLoopRun` kept the process alive indefinitely after guest shutdown, requiring SIGKILL from the service.
- **fork: full VM state preservation** -- fork now captures both rootfs overlay and workspace files. Previously, `create_session_from_image()` cloned data to `session_dir/system/` and `session_dir/workspace/` (real directories), but the VM only sees `session_dir/guest/` via VirtioFS, so cloned data was invisible to the guest. Fixed to clone into `guest/` subdirectories with compat symlinks, matching the VirtioFS share layout.
- **disk usage: report actual blocks, not logical size** -- `disk_usage_bytes()` now uses `blocks * 512` instead of `meta.len()`, so sparse files (e.g. a 2GB rootfs.img overlay with 9MB of actual changes) report their true disk footprint. Fixes inflated image sizes in `capsem_image_inspect`.
- **benchmark: fork performance and size regression gates** -- `test_fork_benchmark` in `test_lifecycle_benchmark.py` profiles fork speed (< 500ms gate), image size (< 12MB gate), boot-from-image speed, and data survival (packages + workspace). Runs 3 cycles with per-run and summary output + JSON.
- **CLI: connect timeout with exponential backoff** -- CLI no longer hangs when the service socket is unreachable. `connect_with_timeout()` retries with exponential backoff (100ms, 200ms, ..., up to 10 attempts). Fails immediately on ENOENT or connection refused. Explicit `--uds-path` skips auto-launch entirely for instant failure.
- **test: DRY shared constants and helpers across integration tests** -- extracted `DEFAULT_RAM_MB`, `DEFAULT_CPUS`, `EXEC_READY_TIMEOUT`, `EXEC_TIMEOUT_SECS`, `HTTP_TIMEOUT`, and `GUEST_WORKSPACE` into `tests/helpers/constants.py`. Moved duplicated `parse_content`, `content_text`, `wait_exec_ready`, and `wait_file_ready` helpers into `tests/helpers/mcp.py`. Updated 49 test files to use shared constants and helpers instead of hardcoded values.
- **test: fix file I/O paths rejected by workspace sandbox** -- tests were writing to `/tmp/` which the agent now correctly rejects as outside the workspace root (`/root/`). Fixed all service, gateway, isolation, session, and E2E test paths to use `/root/`.

### Added
- **VM lifecycle: guest-initiated shutdown** -- `shutdown`, `halt`, `poweroff`, and `reboot` commands work inside the VM via `capsem-sysutil`, a multi-call binary deployed to `/run/capsem-sysutil` with symlinks in `/sbin/`. Opens a dedicated vsock:5004 lifecycle channel (independent of the PTY agent) to send `ShutdownRequest` to the host. Reboot prints an error (not supported in sandbox). Includes countdown timer matching `SHUTDOWN_GRACE_SECS`.
- **VM lifecycle: suspend and warm resume** -- persistent VMs can be suspended via `capsem suspend <name>` (CLI), `capsem_suspend` (MCP), or `suspend` inside the guest. Uses Apple VZ `saveMachineStateTo` (macOS 14+) with a quiescence protocol: agent freezes filesystem (`fsfreeze -f /`), host pauses VM, saves checkpoint, stops. Resume detects checkpoint file and uses `restoreMachineStateFrom` for warm restore. Agent reconnects with exponential backoff, re-sends Ready, and thaws filesystem.
- **VM identity** -- service injects `CAPSEM_VM_ID` (UUID) and `CAPSEM_VM_NAME` (user-chosen name or UUID for ephemeral) as environment variables. Agent calls `sethostname(CAPSEM_VM_NAME)` after boot so the shell prompt reflects the VM name.
- **VmHandle trait: pause/resume/save/restore** -- hypervisor abstraction extended with `pause()`, `resume()`, `save_state(path)`, `restore_state(path)`, and `supports_checkpoint()`. Apple VZ implementation dispatches to main thread. KVM defaults return errors.
- **capsem-doctor: lifecycle diagnostics** -- new `test_lifecycle.py` category verifying sysutil symlinks (`/sbin/shutdown`, `/sbin/halt`, `/sbin/poweroff`, `/sbin/reboot`, `/usr/local/bin/suspend`), `CAPSEM_VM_ID`/`CAPSEM_VM_NAME` env vars, and hostname matching VM name.
- **capsem-gateway: TCP-to-UDS reverse proxy** -- standalone binary that bridges TCP (default port 19222) to capsem-service UDS. Bearer token auth (64-char random, regenerated on restart, written to `~/.capsem/run/gateway.token` with 0600 permissions). All service endpoints proxied through with method/path/query/body preserved. `GET /` health check (no auth). `GET /status` aggregated VM health with 2s cache TTL for efficient tray polling. CORS permissive for browser access. Graceful shutdown cleans up token/port/pid files. No capsem-core dependency, no VM access -- pure low-privilege proxy.
- **capsem-service: auto-spawn gateway and tray** -- service now spawns capsem-gateway (TCP proxy) and capsem-tray (macOS menu bar) as child processes on startup. Both are killed on graceful shutdown. Tray spawn is macOS-only, gateway spawn is cross-platform. Sibling binary discovery falls back to target/debug/ for development.

### Changed
- **exec: separated from interactive PTY** -- exec commands now spawn a direct child process with piped stdout instead of injecting into the shared PTY and scanning for a magic ESC sentinel. Output flows on a dedicated vsock port 5005 (`VSOCK_PORT_EXEC`) with an `ExecStarted { id }` handshake, while `ExecDone` continues on the control channel. Eliminates sentinel spoofing risk, removes `strip_ansi()` post-processing, and keeps control_loop responsive to heartbeats during long-running commands. The interactive terminal (vsock:5001) is no longer contaminated by exec output.
- **removed capsem-app direct CLI mode** -- deleted `run_cli()` and `cli.rs` from capsem-app. All VM operations now go through capsem-service (single path). The Tauri GUI uses the service API like every other client.

### Security
- **image name path traversal** -- image names (fork, inspect, delete) are now validated with the same rules as VM names (alphanumeric, hyphens, underscores only). Previously, a name like `../../etc` could escape the images directory during fork or trigger `remove_dir_all` outside the sandbox on delete. Defense-in-depth assertion added to `ImageRegistry::image_dir()`.
- **persistent registry atomic writes** -- `PersistentRegistry::save()` now uses write-to-tmp + fsync + rename instead of direct `std::fs::write`. Prevents a crash mid-write from producing a zero-byte or partial JSON file, which would lose all persistent VM state on next startup.
- **symlink sandbox escape hardening** -- guest agent FileWrite/FileRead/FileDelete handlers now validate paths with `validate_file_path_safe()` (canonicalize + workspace containment check) and use `O_NOFOLLOW` on actual file operations to eliminate TOCTOU window. Previously, FileWrite had no validation at all, and FileRead/FileDelete only checked for `..` and NUL bytes. A compromised guest could create a symlink pointing outside `/root/` and read/write/delete arbitrary files through it. Snapshot system now preserves symlinks instead of silently dropping them, includes them in workspace hashes and file counts, and surfaces `is_symlink` in MCP snapshot listings.
- **capsem-gateway: 10 MB request body size limit** -- proxy now enforces a 10 MB maximum on incoming request bodies via `http_body_util::Limited`, returning 413 Payload Too Large for oversized payloads. Prevents OOM from malicious clients.
- **capsem-gateway: CORS restricted to localhost origins** -- replaced `CorsLayer::permissive()` (allow all origins) with a predicate that only allows `http(s)://localhost`, `http(s)://127.0.0.1`, and `tauri://` origins. Prevents cross-origin requests from external websites.
- **capsem-gateway: auth failure rate limiting** -- after 20 failed auth attempts within 60 seconds, the gateway returns 429 Too Many Requests instead of 401. Prevents brute-force token guessing.
- **capsem-process: UDS sockets hardened to 0600** -- IPC and terminal WebSocket sockets now have chmod 0600 after bind. Previously inherited umask (0755), allowing any local user to connect to a VM's terminal or send exec commands with no auth.
- **capsem-process: environment cleared on spawn** -- service now uses `env_clear()` before spawning capsem-process, passing only `HOME`, `PATH`, `USER`, `TMPDIR`, `RUST_LOG`. Prevents API keys, tokens, and secrets from the user's shell leaking into per-VM processes.
- **capsem-process: serial.log permissions 0600** -- serial log files now created with explicit 0600 mode. Previously world-readable via umask default, potentially exposing terminal output containing secrets.
- **capsem-process: guest cannot force process exit** -- control channel read error on vsock:5000 now breaks the read loop instead of calling `process::exit(1)`. A compromised guest can no longer DoS its host process by closing the vsock fd.
- **capsem-tray: macOS menu bar tray** -- standalone binary that polls the gateway `/status` endpoint. VMs split into Permanent and Temporary sections. Permanent VMs get Connect/Resume, Fork, Stop, Delete; temporary VMs get Connect/Resume, Delete. Connect/Resume is a single context-sensitive button (shows Resume when suspended). "New Permanent..." opens UI with name dialog. Color-coded icons: purple (active), black template (idle, auto light/dark), red (error). Uses `tray-icon` + `muda` for native NSStatusItem. No capsem-core dependency, no Tauri.

### Fixed
- **capsem doctor: streaming output and VM cleanup** -- doctor now types the command into the shell (TerminalInput) instead of using Exec, so test output streams in real-time instead of buffering until completion. VM is always deleted on exit, including Ctrl-C. Full output written to `~/.capsem/run/doctor-latest.log`.
- **capsem-sysutil: help output to stdout** -- `print_help()` wrote to stderr (`eprintln!`), so `shutdown --help` appeared empty to callers checking stdout. Now uses `println!`.
- **snapshot revert: symlink support** -- revert used `.exists()` (follows symlinks) and `fs::copy` (dereferences), so reverting a symlink failed with "file does not exist". Now uses `symlink_metadata()` to detect symlinks and restores them with `read_link` + `symlink()`. Auto-select also uses `symlink_metadata` so snapshots containing only symlinks are found.
- **snapshot revert: VirtioFS stale cache** -- overwriting a file via `fs::copy` could leave VirtioFS with a stale cached size, causing truncated reads in the guest. Now removes the file first and fsyncs after write to force cache invalidation.
- **venv activation in exec** -- agent now adds `VIRTUAL_ENV` and prepends venv bin to `PATH` in boot_env after capsem-init creates the venv. Both PTY shell and exec commands see the venv. Removed duplicate activation from capsem-bashrc.
- **capsem-process: service-initiated suspend was silently dropped** -- the IPC handler in `handle_ipc_connection` matched `ServiceToProcess::Suspend` but logged "not yet implemented" instead of forwarding to the ctrl channel where the actual suspend logic lives. `capsem suspend <id>` and the MCP `capsem_suspend` tool were non-functional. Now forwards to the ctrl channel like `Shutdown` does.
- **capsem-agent: reconnect timer never reset** -- `start_reconnect` was set once at first disconnect and never updated after successful reconnect. A second suspend/resume cycle >30s into the VM's lifetime caused the agent to immediately timeout and exit. Now resets timer and backoff delay after each successful reconnect.
- **capsem-sysutil: operator precedence bug in --help guard** -- `a == "--help" || a == "-h" && cmd != "shutdown"` parsed as `"--help" || ("-h" && ...)` due to `&&` binding tighter. Added explicit parentheses.
- **capsem-sysutil: fd leak on write failure** -- `send_lifecycle_msg` did not close the vsock fd if `write_all_fd` or `encode_guest_msg` failed. Now closes fd on all paths.
- **capsem-service: suspend silently reported success on failure** -- `handle_suspend` discarded IPC send errors with `let _` and returned `{"success": true}` even when the VM never confirmed suspended state. Now propagates all IPC errors and returns 500 if the VM does not confirm suspension within 15 seconds.
- **capsem-service: resume did not pass checkpoint path** -- `resume_sandbox` re-spawned `capsem-process` without `--checkpoint-path`, causing suspended VMs to cold-boot instead of warm-restoring. Now passes `--checkpoint-path` when the registry entry has `suspended: true` and the checkpoint file exists.
- **capsem-service: resume did not clear suspended flag** -- after successful resume, `entry.suspended` stayed `true` and `entry.checkpoint_path` retained the stale value. Now clears both and saves the registry.
- **capsem-service: /list and /info did not distinguish Suspended from Stopped** -- persistent VMs with `suspended: true` were reported as "Stopped". Now returns "Suspended" status, and the gateway's `/status` endpoint includes `suspended_count` in `ResourceSummary`.
- **capsem-gateway: terminal WebSocket gave no error on VM unavailable** -- when the per-VM UDS connect failed after WebSocket upgrade, the connection silently dropped. Now sends a Close frame with code 1011 and reason "VM not available".
- **capsem-gateway: proxy timeout too short for suspend** -- 30-second proxy timeout could expire during suspend operations (up to 26s). Increased to 120 seconds. Added 5-minute safety timeout on the background HTTP connection driver.
- **capsem-gateway: terminal UDS path fallback incorrect** -- `terminal_uds_path` used `parent().unwrap_or("/tmp")` which never triggered for bare filenames (parent returns `Some("")`). Now filters empty parents before falling back.
- **capsem-process: VirtioFS share narrowed to guest/ subtree** -- VirtioFS previously shared the full `session_dir`, exposing `session.db`, `serial.log`, and `auto_snapshots/` to the guest. Now only `session_dir/guest/` (containing `system/` and `workspace/`) is shared. Host-only files are outside the share boundary. Compat symlinks preserve existing code paths.
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

- **Settings system** -- dynamic settings UI rendered from the backend tree structure (not mocked). Recursive `SettingsSection` renderer handles all setting types: bool toggles (immediate save), text/number/select inputs (staged batch save), password fields with reveal + prefix validation + "required" badge, file editors with Shiki syntax highlighting (theme-aware, shared singleton), domain chip lists (add/remove). Toggle-gated provider cards with slide transitions, chevron animation, and collapsed warning summaries. Settings store (`settings.svelte.ts`) with `load/stage/save/discard/updateImmediate`. Settings model (`settings-model.ts`) with tree indexing, widget resolution, preset matching, pending changes tracking. MCP section with policy, built-in tools, and external servers. Security preset selector. Dirty bar with unsaved change count + Save/Discard. WCAG AA contrast tests for all warning/error/status colors (11 checks, amber-700/red-700 light, amber-400/red-300 dark). 206 tests passing.

- **Gateway wiring (Sprint 05)** -- frontend now connects to capsem-gateway for real data instead of mock. API client (`api.ts`) with Bearer auth token fetched from `GET /token` (hardcoded 127.0.0.1 IP check). Reactive gateway store with automatic health check and reconnection. VM store polls `GET /status` every 2s with visibility-aware pause. All view components (Stats, Logs, Files, Inspector) wired to gateway API with transparent mock fallback on network error. Terminal WebSocket connection via iframe postMessage handshake with `?token=` query-param auth (allowlisted, other params dropped). VM lifecycle actions (stop/delete/fork/resume) wired in both toolbar and new-tab page. Connection status dot in toolbar. Gateway-side: added `GET /token` endpoint with loopback IP restriction, added query-param auth fallback for WebSocket paths only. Settings remains mock (service has no settings CRUD API yet). 115 gateway tests, 303 frontend tests passing.

### Added
- **Theme system** -- three independent axes: UI mode (auto/light/dark), accent color (9 options), and terminal theme (12 families with dark/light variants). All persisted in localStorage. Terminal themes sourced from canonical iTerm2-Color-Schemes palettes. Accent colors are primary-only CSS overrides on a single consistent dark/light base (removed ~2000 lines of per-theme Preline CSS). Settings page with Interface and Terminal subsections, color swatches, and live terminal preview.
- **Bundled fonts** -- Google Sans Flex for UI chrome, Google Sans Code as default terminal font, plus JetBrains Mono, Fira Code, Cascadia Code, Inconsolata, Hack, Space Mono, Ubuntu Mono. All local TTF in `public/fonts/`, zero external loads. Terminal font and font size configurable in Settings with localStorage persistence and iframe propagation via postMessage.
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
