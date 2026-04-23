version: 1.0.1776980020
---
### Security
- **Verify manifest signatures at boot before trusting asset hashes.**
  The previous commit wired asset hash verification to the on-disk
  `manifest.json`, but an attacker with write access to `assets/` could
  swap both the rootfs and the manifest to match. Closed the gap with
  minisign signature verification: the release pubkey
  (`config/manifest-sign.pub`, key id `93A070CBB288AC9B`) is now baked
  into `capsem-core` via `include_str!`, and
  `asset_manager::load_verified_manifest_for_assets` rejects any
  manifest whose sibling `.minisig` is missing or invalid. Release
  builds (`cfg!(debug_assertions) == false`) hard-fail on a manifest
  without a valid signature; debug builds allow unsigned manifests so
  local dev loops with locally built assets keep working. Added the
  `minisign-verify = "0.2"` crate; covered by 9 new unit tests
  including verify-accepts/rejects-tampered-manifest/rejects-mangled-
  signature/rejects-wrong-pubkey/bails-when-sig-required-but-missing/
  accepts-unsigned-when-allowed/bails-on-bad-signature and a regression
  guard that the baked pubkey file parses as valid minisign. Updated
  `docs/src/content/docs/architecture/asset-pipeline.md` to describe
  the full tamper-resistance chain.

- **Asset hash verification at boot was silently disabled on every release.**
  `crates/capsem-core/src/vm/boot.rs` read three expected hashes via
  `option_env!("VMLINUZ_HASH")` / `"INITRD_HASH"` / `"ROOTFS_HASH"`, but
  nothing in the build chain ever set those env vars -- no `rustc-env=`
  emit from any `build.rs`, no shell-level pre-seed in CI, no `capsem-core
  /build.rs` at all. Every shipped binary therefore reached
  `VmConfig::build()` with `expected_*_hash: None`, skipping the hash
  check on kernel, initrd, and rootfs. Casual corruption, asset/manifest
  drift, or an attacker with write access to `assets/` all went
  undetected at boot. The compile-time embedding approach is also
  incompatible with the project's independent binary/asset release
  model (`min_binary`/`min_assets` compatibility ranges) -- baking
  specific hashes into a binary would tie every binary release to one
  asset release.
  Replaced the `option_env!` path with runtime manifest lookup. New
  `asset_manager::load_manifest_for_assets(assets)` reads `manifest.json`
  from the assets dir or its parent; new `ManifestV2::
  expected_hashes_current(arch)` returns the kernel/initrd/rootfs hashes
  for the current release on the host arch. `boot_vm` now feeds those
  to `VmConfig::builder`, so the hash check fires on every boot that has
  a manifest. Missing or malformed manifest falls back to disabled
  verification with an explicit `[boot-audit] asset hash verification
  disabled` log line, keeping dev loops without a manifest working.
  Tamper resistance for release environments now depends on manifest
  signature verification in the asset-download path; that path is a
  separate, tracked gap.
  Updated `docs/src/content/docs/architecture/asset-pipeline.md` to
  describe the runtime-lookup flow (replacing the old "Compile-Time
  Hash Embedding" section) and fixed the mermaid diagram to match.
  Covered by 8 new unit tests in `crates/capsem-core/src/asset_manager.rs`
  covering `expected_hashes_current`, `load_manifest_for_assets`, and
  the `aarch64 -> arm64` arch mapping.

### Fixed
- **Signal-driven explicit cleanup for capsem-process background-thread
  owners.** Companion fix to the `shutdown_lock` host-serialization
  landed earlier on this branch: even with one teardown at a time, the
  previous code relied on tokio-runtime-drop ordering to run
  `DbWriter::Drop` (join writer thread + `PRAGMA
  wal_checkpoint(TRUNCATE)`) and `FsMonitor` quiescence inside the
  service's 1s SIGTERM-to-SIGKILL budget. Non-deterministic under any
  unrelated slowdown (APFS fsync spike, busy writer queue, slow VZ
  teardown) and still flaky on the observed
  `test_wal_absent_after_clean_shutdown` failure (428512-byte WAL).
  Fixed by hoisting the background-thread owners into a `Shutdown`
  struct owned by `main()` so the SIGTERM handler can drain them
  synchronously before calling `CFRunLoopStop`. New primitives:
  `capsem_logger::DbWriter::shutdown_blocking(&self)` (Arc-safe,
  idempotent -- switches `tx`/`join_handle` to
  `std::sync::Mutex<Option<...>>` so callers holding any `Arc<DbWriter>`
  can deterministically drain the writer thread; existing `Drop`
  delegates to it), and
  `capsem_core::fs_monitor::FsMonitor::shutdown_and_join(&self)` (signals
  the event loop to flush and joins its worker thread). The handler
  drains `FsMonitor` first (fs_events fan into DbWriter), then DbWriter,
  both inside `tokio::task::spawn_blocking` so we don't stall a tokio
  worker on the thread joins; `CFRunLoopStop` runs only after the drain
  completes. Added `CAPSEM_TEST_SLOW_CHECKPOINT_MS` test-only env var in
  `writer_loop` that inserts a sleep before the final checkpoint --
  proves that explicit cleanup waits for the checkpoint where an
  implicit Drop path would race the SIGKILL budget. Documented the
  pattern in `/dev-rust-patterns` next to the host-serialization pattern.
  Covered by four new `capsem-logger::writer` tests
  (`shutdown_blocking_through_arc_flushes_wal`,
  `shutdown_blocking_is_idempotent`, `write_after_shutdown_is_noop`,
  `slow_checkpoint_hook_delays_shutdown`); the
  `test_wal_absent_after_clean_shutdown` integration test now passes
  clean under `-n 4` alongside the rest of `capsem-session-lifecycle`,
  `capsem-cleanup`, `capsem-recovery`, `capsem-stress`, and the full
  `capsem-service` suite.

- **VM teardown races under load left `session.db-wal` non-empty after
  `capsem delete`.** `handle_delete`'s fast path SIGTERMs
  capsem-process and waits 1s for exit before escalating to SIGKILL.
  Under N concurrent deletes on one host, each capsem-process's exit
  path -- Apple VZ guest teardown on the main thread, virtiofs drain,
  `DbWriter::Drop`'s writer-thread join + `PRAGMA
  wal_checkpoint(TRUNCATE)` -- compete for the same main-thread + I/O
  bandwidth, and one teardown can blow the 1s budget. SIGKILL then
  fires mid-checkpoint, leaving a large WAL file on disk (395 kB in
  the failing `test_wal_absent_after_clean_shutdown` artifact).
  Fixed by serializing VM teardown at the service layer: added
  `ServiceState::shutdown_lock: tokio::sync::Mutex<()>`, acquired at
  the top of `shutdown_vm_process` and held through the entire
  `SIGTERM` + `wait_for_process_exit` window. Same pattern as the
  existing `save_restore_lock`: one critical-section operation in
  flight per host at a time, in-process tokio mutex since production
  runs exactly one service per user-host. `handle_purge`'s
  `join_all` of concurrent teardowns now effectively serializes
  through the lock -- intentional trade of concurrency for
  correctness; purge is an admin operation, not latency-sensitive.
  Documented in `skills/dev-rust-patterns/SKILL.md` alongside
  `save_restore_lock` as the "host-serialization locks" pattern, and
  the follow-up refactor (signal-driven explicit cleanup in
  capsem-process so cleanup-correctness doesn't depend on the
  SIGKILL budget) is scoped in
  `sprints/explicit-shutdown-cleanup/ISSUE.md`.

- **Gateway auth rejections were invisible in the log, so
  curl-returns-`000` under load was untriaged.** The gateway's
  `auth_middleware` silently returned 401/429 with no structured log
  line, and the default env filter was `capsem_gateway=info` -- so
  tower_http's per-request spans and hyper's connection-level
  complaints (malformed header, RST during read) also never made it
  into `gateway.log`. When a concurrent `just test` run surfaced
  `test_empty_bearer_returns_401` as `000` instead of `401`, there
  was nothing in the preserved test artifacts to diagnose from.
  Fixed by (a) broadening the default filter to
  `capsem_gateway=info,tower_http=debug,hyper=info` so connection-
  level events land in the log, (b) logging auth rejections at
  `info!`/`warn!` (401 / 429 respectively) with `method`, `path`,
  and a `shape` field that classifies the Authorization header
  without leaking its value (e.g. `bearer-empty`, `bearer-no-space`,
  `basic`, `unknown-scheme`, `non-ascii`), and (c) installing a
  panic hook so any panicked request handler surfaces an
  `ERROR gateway panic` rather than vanishing into a dropped
  connection. No behaviour change on the happy path; diagnostic-only
  for the flaky load-time failure mode. Covered by
  `classify_auth_header` unit tests (absent/empty/non-ascii/bearer
  shape matrix).

- **`ExecDone` always stalled 500ms on no-output commands, taxing every
  fork and every internal `sync`.** `handle_guest_msg(ExecDone)` in
  `crates/capsem-process/src/vsock.rs` used `captured.is_empty()` as a
  heuristic for "EXEC-reader thread hasn't finished depositing yet" and
  unconditionally slept 500ms on that branch. The heuristic cannot
  distinguish "deposit still in flight" from "command legitimately
  produced no stdout", so `true`, `sleep`, `exit`, and the
  `fsfreeze -f /; sync; fsfreeze -u /` pipeline `handle_fork` uses to
  quiesce the guest filesystem each paid 500ms of dead time per call.
  Visible as `test_fork_benchmark` (fork_ms mean ~110ms -> ~621ms,
  blowing the 500ms gate) and a broader regression: any command with
  no stdout took 520-570ms instead of 20-50ms.
  Replaced the heuristic with a proper deposit signal. `JobStore::
  active_exec` now holds an `ActiveExec { id, captured, deposited:
  Arc<tokio::sync::Notify> }`; the EXEC-port reader thread calls
  `notify_one()` after writing `captured` under the active_exec lock,
  and the `ExecDone` handler awaits `.notified()` with a 100ms bound
  (short safety net for guest never opening the EXEC port). Common
  path: deposit lands first, permit stored, `notified()` resolves
  immediately -- no sleep. Racy path: deposit arrives while ExecDone
  is parked, Notify wakes it; ExecDone reads the real captured bytes.
  Covered by `crates/capsem-process/src/vsock/tests.rs::
  exec_done_with_empty_stdout_resolves_without_500ms_stall`, which
  pre-deposits an empty `ActiveExec`, notifies, and asserts ExecDone
  returns under 100ms; fails at 503ms on the old code. `fail_all`
  also wakes any parked ExecDone on the deposit notifier so control-
  channel close doesn't leave the handler stuck.

- **`wait_for_vm_ready` backoff overshot VM ready-time by ~500ms,
  regressing every `provision -> exec-ready` wait.** A recent alignment
  of `wait_for_vm_ready` onto `PollOpts::new`'s project-wide defaults
  (50ms initial / 500ms max) was correct for peer pollers that wait on
  remote processes with seconds-scale startup, but wrong for this hot
  path: the ready-sentinel is a cheap local `stat` on a sub-second
  latency gate, so with max_delay=500ms the exponential curve lands
  attempts at t=50/150/350/750/1250ms and misses a VM that becomes
  ready at t~=550ms until the next 500ms boundary. Visible as
  `test_avg_exec_latency_3_concurrent_vms` / `test_lifecycle_benchmark`
  (exec_ready mean ~570ms -> ~1287ms) and `test_fork_benchmark`
  boot_ready mean (~680ms -> ~784ms). Restored the tight backoff
  (5ms/50ms) inline on this one call site, documenting why it diverges
  from `PollOpts::new` defaults. Covered by
  `crates/capsem-service/src/tests.rs::
  wait_for_vm_ready_detects_ready_within_tight_overshoot`, which
  creates a `.ready` file after 200ms and asserts detection under
  300ms.

- **Suspend/resume: sibling-VM save_state overlap corrupted the
  persistent overlay.** Apple's Virtualization.framework does not
  tolerate overlapping `saveMachineStateToURL` /
  `restoreMachineStateFromURL` calls across sibling VMs on the same
  host: the VirtioFS ring state captured inside the vzsave ends up
  referencing FUSE descriptors the host has torn down or re-keyed on
  behalf of another VM mid-operation. On the unlucky VM, resume
  surfaces as cascading `I/O error, dev loop0` plus
  `EXT4-fs (loop0): failed to convert unwritten extents to written
  extents -- potential data loss!` in the guest, and
  `initial handshake failed: BootReady read failed: failed to fill
  whole buffer` on the host. The 8% tail from
  `sprints/loop-device-io-after-resume/` was this. Added
  `ServiceState::save_restore_lock` (a `tokio::sync::Mutex<()>`) held
  across the full body of `handle_suspend` (until the per-VM
  `capsem-process` has exited and the checkpoint is durable) and
  across `handle_resume` (until `wait_for_vm_ready` confirms the
  new process's `.ready` sentinel). Production runs exactly one
  `capsem-service` per host per user, so per-service serialization is
  sufficient there. Stress harness
  `tests/capsem-mcp/test_stress_suspend_resume.py` now documents that
  it must run at `-n 1`: multiple xdist workers spawn multiple
  services and the in-service lock cannot coordinate across them,
  re-exposing the bug in a state that never occurs in production.
  With the lock, `CAPSEM_STRESS=1 ... -n 1` runs 50/50 (was noisy
  around 46-50/50 before). Scoped the pre-existing MutexGuard in
  `with_graceful_shutdown` into its own block so the compiler's Send
  analysis survives the new tokio mutex in `ServiceState`. Full
  gotcha writeup at `docs/src/content/docs/gotchas/
  concurrent-suspend-resume.md`; skills updated in
  `skills/dev-testing/SKILL.md` to call out the one legitimate `-n 1`
  test.
- **Test infra: capsem-service leaked across aborted pytest runs.** The
  companion reaper in `capsem-guard` only bounds tray and gateway to
  their parent service -- the service itself had no parent-watch. When
  pytest exited abnormally (Ctrl-C, xdist worker crash, hang followed
  by SIGKILL) the session-scoped fixture teardown never fired, and
  `capsem-service` plus its tray+gateway sat around until manually
  killed. `capsem-service` now accepts an optional `--parent-pid` flag
  that wires `capsem_guard::watch_parent_or_exit` into startup,
  symmetric with the existing companion behaviour: on parent death the
  service exits within ~100 ms, which lets the companion reaper take
  the tray and gateway down with it. Real daemon launches that omit
  `--parent-pid` are unaffected. Wired into the three pytest fixtures
  that spawn their own service (`tests/helpers/service.py`,
  `tests/capsem-mcp/conftest.py`, `tests/capsem-e2e/conftest.py`) so
  each one pins service lifetime to its worker. Verified end-to-end by
  spawning the service under a bash wrapper, killing the wrapper, and
  confirming `capsem-service` exits within ~100 ms; and by running the
  `test_stress_suspend_resume.py -n 8` harness and observing
  `pgrep -lf target/debug/capsem` return empty after teardown.
- **Suspend/resume: VZErrorDomain Code=12 "permission denied" on restore
  from a `/var/folders/...` path.** Apple VZ's
  `restoreMachineStateFromURL` enforces strict path matching between
  `saveMachineStateToURL` and restore -- the VirtioFS share paths
  (and any path referenced by the preserved VM state) must resolve
  identically. Under pytest the tmp_dir lands at
  `/var/folders/lv/.../capsem-test-xxx` which is a symlink chain
  through `/var -> /private/var`. If the save path was the symlink
  form and the restore resolved to `/private/var/...` (or vice versa),
  VZ rejected the restore with a security error and the VM entered
  an unrecoverable state (guest kernel came back up on a wedged loop
  device, stress harness showed 21-100+ `permission denied` entries
  per failing `process.log`). Both `capsem-service` and
  `capsem-process` now call `std::fs::canonicalize()` on their
  respective root paths (`run_dir` / `session_dir`) immediately after
  `create_dir_all`, so every downstream derivation (checkpoint path,
  VirtioFS share host_path, machine identifier, session.db, workspace
  dir, `CAPSEM_SESSION_DIR` env for guest MCP, auto-snapshot
  scheduler, MCP aggregator) uses the canonical
  `/private/var/...` form from both the pre-suspend and post-resume
  process. A reproduction outside pytest (using `~/.capsem/...`,
  which doesn't cross the `/var` symlink) passed first try -- the bug
  was pytest-path-specific. Stress harness (50 iters × 8 workers)
  goes from 4.4% VZ-permission-denied failures to 0, with the
  remaining 8% tail being the unrelated loop-device I/O error on
  the persistent overlay (tracked separately in
  `sprints/vsock-resume-reconnect/plan.md`).
- **Suspend: resume-too-soon race where the old `capsem-process`
  still held the checkpoint file.** `capsem-service::handle_suspend`
  previously returned as soon as the child emitted
  `StateChanged { state: "Suspended" }`, but the child broadcasts that
  event *before* its `save_state` finalizer syncs and the process
  exits. A quick subsequent `capsem_resume` could therefore race the
  outgoing process's `.vzsave` fsync / exit, and VZ would see either a
  partially-written checkpoint or contention over the backing file.
  `handle_suspend` now drains the broadcast channel until it closes
  (the child has exited) or a 15s timeout fires, guaranteeing the old
  process is fully gone before returning to the caller.
- **Suspend/resume: VM survives Apple VZ post-resume vsock half-opens
  and post-handshake connection resets.** The host's vsock layer now
  runs a continuous accept loop for the VM's lifetime and hot-swaps
  the underlying fd into stable terminal/control reader-writer bridges
  via dedicated re-key channels. When a connection resets
  (`BrokenPipe` / `ConnectionReset` pre-handshake, any read/write
  error mid-session) the bridges drop the dead fd, clear all
  framing buffers (no `0x81A08329` "control frame too large" misread
  of a MessagePack map header as a length header), and block on the
  rekey channel for a fresh fd produced by the guest's own reconnect
  loop. The initial handshake retries up to 3× on narrow retryable
  errors only (`BrokenPipe` / `ConnectionReset` at any level of the
  `anyhow::Error` source chain — `UnexpectedEof` and decode errors
  fail fast because they indicate a genuinely wedged guest, not the
  half-open-vsock race). Errors in `perform_handshake` now propagate
  with `.context()` so the underlying `std::io::Error` stays in the
  source chain and classification works without string matching.
  All 11 pre-refactor invariants survive: 10s heartbeat, terminal
  resize, lifecycle port for guest shutdown/suspend, audit port,
  exec duration tracking, VZ main-thread dispatch for pause/save/stop,
  fsync-after-save, error-path Unfreeze, deferred_conns processing,
  handshake on spawn_blocking, and reader-break `JobStore::fail_all`
  poisoning when the rekey channel itself closes. Stress harness
  (`test_stress_suspend_resume.py`, 50 iterations × 8 workers) goes
  from 45-48/50 to 47/50; the remaining 3 failures are an independent
  loop-device I/O error on the persistent overlay after restore (see
  `sprints/vsock-resume-reconnect/plan.md` for the handoff). Plan
  and tracker for the sprint live at
  `sprints/vsock-resume-reconnect/{plan,tracker}.md`.
- **capsem-process kept running after `setup_vsock` returned Err,
  turning every handshake failure into a 30-second service-side poll
  timeout.** The tokio task at `capsem-process/src/main.rs:424` just
  logged the error and exited, leaving the parent process alive with
  no `.ready` sentinel and no working control channel. The service
  polled `.ready` until its 30s deadline then reported a generic
  "exec-ready timeout" with no specific diagnosis. Now `std::process
  ::exit(1)` on vsock-setup failure so the service's child-exit
  handler reclaims the instance in <1s and callers (tests, CLI, MCP)
  see the failure promptly. Residual 4% tail failure seen under
  xdist stress is tracked in `sprints/vsock-resume-reconnect/ISSUE.md`
  (the real root cause is an Apple VZ half-open vsock after resume;
  the fix here just makes it surface cleanly).
- **`wait_for_vm_ready` poll hammered the sentinel 600× per 30s window
  while every other caller used 10× fewer polls.** `main.rs::wait_for_vm_ready`
  was the only site in the codebase constructing `PollOpts { max_delay:
  50ms }` directly; every peer (`service-connect`, `service-socket`,
  `gateway-ready`, `shell-socket`, guest `vsock-connect`, `reconnect`)
  uses `PollOpts::new` with the project-standard 500ms max_delay.
  Aligned this one site to the convention. Cuts sentinel-check traffic
  per second by 10× under contention without changing the 30s overall
  timeout.
- **Control-channel reader could silently wedge a VM for 30 seconds
  per command and kept the `.ready` sentinel fresh the whole time.**
  When `capsem-process`'s `ctrl_f_read` loop hit any decode/read error
  (e.g. desync, short-read, oversize frame), it logged and `break`ed
  without cleaning up. In-flight `Exec`/`ReadFile`/`WriteFile` oneshots
  registered in `job_store.jobs` never resolved, so the `ipc.rs` tasks
  awaiting them hung indefinitely; meanwhile `.ready` stayed on disk
  and `vm_ready` stayed `true`, so every subsequent `POST /exec` passed
  `wait_for_vm_ready` and then timed out at 30s too. Added
  `JobStore::fail_all(message)` which drains pending oneshots with
  `JobResult::Error`; the reader's error path now calls it and also
  removes `.ready`, clears `vm_ready`, so in-flight callers get an
  immediate error and new callers fail fast at the readiness check.
- **Handshake reads/writes ran sync on the async runtime, and every
  failure was silently swallowed.** `setup_vsock` did blocking
  `read_control_msg`/`write_control_msg` directly inside the async fn,
  so under contention (N VMs booting at once) all tokio workers could
  block on vsock I/O simultaneously -- runtime starvation slowed every
  handshake and gave guests enough time to hit their own timeouts,
  leading to protocol desync. Worse, `let _ = read_control_msg(...)`
  at the Ready and BootReady reads plus every `let _ =
  write_control_msg(...)` in the restore branch meant a half-failed
  handshake still reached `vm_ready.store(true)` and `.ready` sentinel
  creation, so callers sent commands into a broken vsock. Moved the
  handshake into `tokio::task::spawn_blocking`, propagated every read
  and write error with context, and gated `vm_ready`/`.ready` on the
  handshake actually succeeding.
- **Artifact preserver left `sessions/` and `persistent/` empty in the
  archive when tests failed under contention.** The helper used
  `shutil.copytree` with an `ignore` filter. When capsem-process was
  still alive during teardown (SIGKILL hadn't reaped it yet) and was
  writing/unlinking files concurrently, copytree's error-accumulation
  model created the destination subdirectories but silently failed to
  populate them -- exactly the `persistent/<vm>/` directories that
  hold `process.log` / `serial.log` / `session.db` needed to debug
  suspend/resume failures. Replaced the `copytree`+`ignore` pattern
  with a manual `os.walk` + per-file copy loop so a single flaky file
  no longer takes out its whole parent subdir, with a stderr summary
  (`copied=N skipped=... errors=N`) and the first 10 error reasons
  surfaced so future regressions don't debug in the dark. Added
  regression tests for the concurrent-unlink race and for a >25 MB
  sibling file coexisting with small log files.
- **`just test-install` had no durable cushion against Colima disk/cache
  exhaustion.** The recipe relied on `_docker-gc`'s `until=72h` filters
  (too conservative to recover recent images / build cache) and on the
  persistent `capsem-install-target` cargo volume never going out of
  bounds. In practice the volume grew to 18.7 GB across sprint version
  bumps and images accumulated until Colima disk pressure compounded
  any OOM already in play. Added two self-healing preflight checks to
  `test-install`: (a) if Colima's `/var/lib/docker` has <10 GB free,
  run `docker image prune -af` + `docker builder prune -af` (no until=
  filter); (b) if the `capsem-install-target` volume has passed 25 GB,
  `docker volume rm` it. Both are no-ops in the common case so they
  don't thrash the cache every run. Linux hosts skip (a) since they
  don't use Colima. Guarded `colima ssh` with `</dev/null` so callers
  that pipe stdin into `just` can't stall the check.
- **`just test-install` leaked a systemd container on every failed run,
  eventually SIGTERM-killing the next build with exit 143.**
  The `test-install` recipe gave each run a unique container name
  (`capsem-install-test-$$`) and only cleaned it up on the happy path.
  Any failed `docker exec` (cargo build, Tauri build, dpkg, pytest)
  short-circuited the script under `set -euo pipefail` before the
  `docker stop`/`docker rm` at the end, leaving the privileged systemd
  container running. Stacked containers squatted Colima's 8 GiB VM
  across runs, and the next build's parallel rustc processes OOM-killed
  mid-compile -- visible as `error: Recipe test-install failed with
  exit code 143` with no pytest output. Also removed dead `EXIT_CODE=$?
  ... exit $EXIT_CODE` bookkeeping that `set -e` had made unreachable
  on the failure path. Fixed by switching to a stable container name,
  preemptively `docker rm -f`ing it at the top of the recipe, and
  installing an `EXIT` trap so cleanup runs on any exit path.
- **Docs described a fictional manifest schema.**
  `docs/src/content/docs/architecture/custom-images.md` claimed every build
  produced `assets/{arch}/manifest.json` with a bill-of-materials schema
  containing `packages[]` and `vulnerabilities[]` arrays -- none of which
  ever existed. `docs/src/content/docs/architecture/asset-pipeline.md`
  showed a different wrong schema (`{"latest", "releases": {<ver>: {<arch>:
  {"assets": []}}}}`) and mentioned legacy flat-format compatibility that
  `asset_manager.rs` no longer accepts. Both pages now document the real
  `assets/manifest.json` format 2 schema (top-level `format`, `assets.
  {current, releases.<ver>.{date, deprecated, min_binary, arches.<arch>.
  <filename>.{hash, size}}}`, `binaries.{current, releases}`) and the
  `min_binary`/`min_assets` compatibility contract. Docs site builds
  green.

- **`tests/capsem-build-chain/test_manifest_regen.py` was testing a ghost
  layout and had been silently skipping every assertion.** The fixture read
  `assets/<arch>/manifest.json` (per-arch) and the tests iterated a flat
  `{filename: hash-hex-string}` schema, but the real manifest is top-level
  `assets/manifest.json` with a nested v2 schema (`assets.releases.<ver>
  .arches.<arch>.<filename>.{hash,size}`). Both the path and the schema
  predate a refactor that was never propagated here, so the fixture's
  `pytest.skip()` fired unconditionally and all four tests reported as
  `s` in build-chain runs -- meaning the suite never actually verified
  manifest/asset consistency. Rewrote the fixture to read the real
  manifest and scope to the current release + host arch. Rewrote every
  test against the nested schema: shape check, per-file existence, b3sum
  match, and a strict `test_no_extra_assets` that allows manifest-listed
  names plus their `<stem>-<hex16>.<ext>` hash-tagged aliases and rejects
  everything else. Verified live on the current tree (4 passed) and
  proved the stale-alias gate with a planted `initrd-deadbeef12345678.img`
  that correctly fails the check.

- **`scripts/create_hash_assets.py` left stale hash-tagged aliases that lied
  about their content.** The script creates `<stem>-<hex16>.<ext>` hardlinks
  mirroring manifest entries so the dev layout matches the installed layout.
  It unconditionally unlinked-and-relinked each expected destination, but
  never swept hash-tagged files left over from prior builds -- and because
  `_pack-initrd` replaces `initrd.img` with fresh content on every run, the
  re-link step kept re-pointing those stale names at the new inode. End
  state in this repo: `assets/arm64/` held five `initrd-<hex>.img` names
  all hardlinked to one inode, but only one hex prefix matched the current
  content hash; the other four names claimed hashes they no longer had.
  Nothing in production reads the stale names (`asset_manager.rs` derives
  the filename from the manifest hash), but the content-addressable naming
  contract was quietly broken and any downgrade/rollback path that
  resurrected an older manifest would have served wrong bytes behind the
  right name. Rewrote the script to enumerate every `<stem>-<hex16>(.ext)?`
  filename in each arch dir and delete those not in the expected set before
  (re)creating current hardlinks. Covered by three new unit tests in
  `tests/capsem-build-chain/test_create_hash_assets.py`.

- **CAPSEM_REQUIRE_ARTIFACTS pre-flight falsely failed `just test` Stage 5
  on a successful build.** `tests/conftest.py::_REQUIRED_ARTIFACTS` declared
  the manifest at `assets/<arch>/manifest.json`, but the canonical layout
  is flat top-level (`assets/manifest.json`). Every production reader --
  `capsem-service` boot at `crates/capsem-service/src/main.rs:2740`,
  `capsem setup` at `crates/capsem/src/setup.rs:187`, `scripts/gen_manifest.py`,
  `scripts/check-release-workflow.sh` -- and the builder's
  `generate_checksums` writer at `src/capsem/builder/docker.py:700` all agree
  on the flat path. The per-arch entry was introduced with the gate itself
  in this release cycle and never resolved on a real build, so the pre-flight
  exited with a confusing "missing: ['assets/<arch>/manifest.json']" right
  after Stage 1-4 had produced the actual manifest. Fixed by correcting the
  path in `_REQUIRED_ARTIFACTS` and adding
  `test_required_artifacts_manifest_path_is_flat` in
  `tests/test_leak_detection.py` to pin the canonical location so this can't
  drift again.

### Security
- **Bumped Astro to 6.1.8 across frontend, docs, and site packages** to clear
  advisory GHSA-j687-52p2-xcff (moderate XSS in `define:vars` via incomplete
  `</script>` tag sanitization; patched in Astro >=6.1.6). `just test` Stage 1
  runs `cd frontend && pnpm audit` and was failing because
  `frontend/pnpm-lock.yaml` had locked Astro to 6.1.4 despite the caret range.
  Grepped the tree for `define:vars` and found zero usages -- exploitability
  in this codebase was nil, but `pnpm audit` gates on version, not usage, so
  the `test` recipe couldn't pass until the lockfiles refreshed. `docs/` and
  `site/` were bumped in the same commit because they were also on affected
  Astro versions.

### Fixed
- **Guest binaries landed on the host with 0o755 instead of 0o555 after
  container-native agent builds.** `capsem-builder agent` on macOS cross-
  compiles inside a Linux container and `chmod 555`s the binaries before
  copying them to the bind-mounted `target/linux-agent/<arch>/` output.
  Docker-for-Mac bind-mount semantics non-deterministically dropped the
  host-side mode, so `capsem-pty-agent` and `capsem-net-proxy` could
  surface as `0o755` while `capsem-mcp-server` and `capsem-sysutil`
  stayed `0o555`. The guest-binary read-only invariant (CLAUDE.md) then
  only held when the `_pack-initrd` justfile recipe ran its compensating
  chmod downstream; any caller invoking the builder directly or running
  `tests/capsem-security/test_binary_perms.py::test_agent_binaries_555`
  before repack saw the bad modes. Added
  `enforce_guest_binary_perms(paths)` in `src/capsem/builder/docker.py`
  and called it at the end of both `container_compile_agent` and
  `cross_compile_agent`, so the invariant is applied at the source by
  the builder itself. Removed the now-redundant compensating `chmod 555`
  in the justfile's `_pack-initrd` recipe. Covered by three new unit
  tests in `tests/capsem-build-chain/test_agent_perms.py`.

- **Every `tests/capsem-mcp/` and `tests/capsem-e2e/` MCP test errored
  under `filterwarnings = ["error"]`.** Both dirs spawn `capsem-mcp`
  with `stdin=PIPE, stdout=PIPE` to speak JSON-RPC, then tore the proc
  down with `proc.terminate() + proc.wait()` -- Popen does not close
  PIPE fds on its own, so each test leaked two
  `_io.FileIO` / `_io.TextIOWrapper` handles. pytest's strict mode
  surfaced them as `ExceptionGroup: multiple unraisable exception
  warnings (2 sub-exceptions)` at setup-teardown boundaries, turning
  69 capsem-mcp tests into `ERROR` and 4 capsem-e2e tests into
  `FAILED`. Added `kill_mcp_proc(proc, timeout=5)` in
  `tests/helpers/mcp.py` -- terminates (or kills), waits, then closes
  `proc.stdin / stdout / stderr` if non-None and not already closed.
  Rewired `tests/capsem-mcp/conftest.py::_kill_proc` through it and
  replaced four inline `proc.terminate(); proc.wait()` pairs in
  `tests/capsem-e2e/test_e2e_mcp.py`. Post-fix: 116 passed, 0 errors
  across both dirs. Covered by a unit test in
  `tests/test_leak_detection.py` that spawns a
  `sys.executable -c "sys.stdin.read()"` child with all three pipes,
  calls `kill_mcp_proc`, and asserts `.closed` on each.

- **Missing built artifacts silently skipped tests instead of failing.**
  Tests that depend on `assets/<arch>/manifest.json`,
  `assets/<arch>/initrd.img`, `entitlements.plist`, or
  `target/linux-agent/<arch>/` use `pytest.skip()` when the artifact is
  absent so a fresh local checkout doesn't fail the suite. In CI, where
  earlier `just test` stages are expected to produce those artifacts, a
  skip means an earlier stage silently dropped its output -- and the
  skipped tests dropping out of the gate disguises the breakage as a
  green run. Added `pytest_sessionstart` pre-flight in
  `tests/conftest.py`: when `CAPSEM_REQUIRE_ARTIFACTS=1` is set
  (justfile's `test` recipe now sets it for both the parallel stage-5a
  and serial stage-5b pytest invocations), the hook fails the session
  before collection if any required artifact is missing, with a
  specific message pointing at the build command needed. Local runs
  without the env var are unchanged -- skips still work. Covered by
  two new unit tests in `tests/test_leak_detection.py` pinning both
  branches of `_missing_required_artifacts`.

- **Python test warnings were never promoted to errors.** `pyproject.toml`
  `[tool.pytest.ini_options]` had no `filterwarnings`, so
  `DeprecationWarning`, `ResourceWarning`, and (critically)
  `PytestUnraisableExceptionWarning` were reported but never gated. Real
  fd / socket / thread-resource leaks in both tests and production
  scripts therefore shipped green. Set `filterwarnings = ["error"]` and
  fixed every leak surfaced:
  - `scripts/clean_stale.py` -- all six `os.scandir(...)` call sites
    were either unbracketed (iterator GC'd eventually, but not
    deterministically) or, in `_target_release_has_old_content` and
    `_dir_has_no_recent`, returned early mid-iteration leaving the
    iterator open. Wrapped each in `with os.scandir(...) as entries:`
    so the underlying fd is released on scope exit regardless of
    return path.
  - `tests/test_exec_lock.py` -- the `_spawn_holder` helper returned
    a `subprocess.Popen` with `stdout=PIPE, stderr=PIPE`; tests
    `.wait()`'d but never closed the pipe fds. Hoisted the two
    callers into `with ... as holder:` blocks so Popen's own
    `__exit__` closes the pipes.
  - `tests/capsem-gateway/test_gw_terminal.py` -- `ws_env` fixture
    teardown called `svc_server.shutdown()` (stops
    `serve_forever`) but never `svc_server.server_close()` (releases
    the UDS listen socket). Added the close plus
    `svc_thread.join()`.
  - `tests/capsem-gateway/test_gw_lifecycle.py` -- the SIGTERM /
    SIGINT lifecycle tests called `gw.start()` but never
    `gw.stop()`, so the gateway log-file handle leaked even though
    the gateway process was killed by signal. Wrapped the asserts
    in `try/finally: gw.stop()`.
  - `tests/capsem-service/test_companion_lifecycle.py` -- the
    restart-with-same-run-dir test needed svc_a's log fd closed
    without destroying the shared tmp_dir (svc_b reuses it);
    added an explicit `svc_a._log_file.close()` between the two
    services. `_spawn_service_on_fixed_port` opened its own log
    file anonymously (`stdout=open(log_path, "w")`) so the
    six-rapid-restarts test could not reach it; it now stashes
    the handle on `proc._log_file` and the test closes every
    spawned service's log file in its finally block.

- **Unhandled exceptions in daemon threads were not failing the test
  suite.** Python surfaces thread exceptions as
  `PytestUnhandledThreadExceptionWarning`, which is reported but has
  never been gating in `pyproject.toml`. Real races (e.g. today's
  `MockWsProcess` teardown hitting `loop.stop()` while
  `run_until_complete` was awaiting) shipped green until someone
  eyeballed the warning in a test run. `tests/conftest.py` now installs
  a process-wide `threading.excepthook` at import time (covers
  collection, fixture setup, and every test) that records each caught
  exception in `_CAUGHT_THREAD_EXCEPTIONS` and prints the traceback to
  stderr in real time. `pytest_sessionfinish` fails the session if that
  list is non-empty. Per-process (each xdist worker gates its own;
  thread exceptions are process-local, unlike process leaks which need
  cross-worker visibility). Covered by two new tests in
  `tests/test_leak_detection.py`: hook-is-installed, and
  captures-real-daemon-thread-exception. Also removed the stale
  `tests/capsem-build-chain/conftest.py.bak` (orphaned after the
  `capsem-cli` -> `capsem` / `capsem-ui` -> `capsem-app` rename).

- **Leak detector false-positived sibling `capsem-mcp` processes.** The
  per-test `check_leaks` fixture and the `pytest_sessionfinish` gate in
  `tests/conftest.py` defined "leak" as any `capsem-*` PID on the host
  not present in the import-time baseline. That caught sibling tools
  sharing the host with pytest -- notably Claude Code's own
  `capsem-mcp` stdio subprocess (spawned by the `claude` CLI, not
  pytest) -- and attributed them to whichever test happened to run
  first. Example report: `[master] tests/capsem-build-chain/test_
  cargo_build.py::test_all_binaries_exist 49423 capsem-mcp
  target/debug/capsem-mcp` (PID 49423's PPID chain: `claude` ->
  terminal shell; pytest never in the chain). Added `_ancestry(pid)` +
  `_is_pytest_descendant(pid)` (walk `psutil.Process.parent()` up to
  init) and gated both sites: `check_leaks` only records first-seen
  when the suspect is actually descended from this pytest process,
  and `pytest_sessionfinish` only flags suspects with either
  attribution (recorded by a worker's `check_leaks`) or a live
  ancestry link to the controller. Sibling processes pass neither
  gate and are silently ignored. Covered by three new unit tests in
  `tests/test_leak_detection.py` (ancestry of init excludes self;
  ancestry of own subprocess includes self; ancestry of nonexistent
  PID is empty).

- **`PytestUnhandledThreadExceptionWarning` from `test_gw_terminal.py`
  module teardown.** The `MockWsProcess` daemon thread in
  `tests/capsem-gateway/test_gw_terminal.py` ran
  `loop.run_until_complete(server.serve_forever())`, and `stop()` tore
  the loop down with `loop.call_soon_threadsafe(loop.stop)`. Stopping a
  running loop while `run_until_complete` is awaiting a pending future
  is the exact case that raises `RuntimeError: Event loop stopped
  before Future completed.` on the worker thread; pytest picked it up
  at module teardown (visible on the last test, e.g.
  `test_ws_nonexistent_vm_closes`). Replaced `serve_forever()` with an
  `asyncio.Event`-based shutdown: `_serve` parks on the event and closes
  the server in its `finally`; `stop()` just sets the event via
  `call_soon_threadsafe`, letting `run_until_complete` return cleanly
  and the worker thread exit. Added a direct regression test
  (`test_mock_ws_process_stop_does_not_leak_thread_exception`) that
  installs a `threading.excepthook` and fails if any exception escapes
  the worker thread.

- **`capsem-agent` failed to compile under `clippy::manual-strip`.** The
  `extract_field` audit-log parser in `crates/capsem-agent/src/main.rs`
  hand-rolled a `starts_with('"')` + `rest[1..]` prefix strip that clippy
  1.93's `manual_strip` lint (denied via `-D warnings`) refused. Rewrote
  to `rest.strip_prefix('"')`; semantics unchanged (`stripped.find('"') + 2`
  still yields the same end offset into `rest`).

- **`capsem_read_file` returned ENOENT on real files after `capsem_resume`
  under concurrent load.** The guest agent's post-resume rebind polled
  `/mnt/shared/workspace` with `Path::exists`, which only drives a FUSE
  `GETATTR`. Under `pytest -n 4 --dist=loadfile` (4 concurrent VMs sharing
  one host's virtiofsd pool) virtiofsd could answer GETATTR on the
  workspace dir before it had populated its child-inode map, so `exists()`
  returned true, the agent `mount --bind /mnt/shared/workspace /root`'d an
  empty view, and every subsequent `/root/<file>` read returned ENOENT
  even though the host file was durably on disk (604319f already made
  write_file flush to host). Fix in `rebind_workspace_after_resume`
  (`crates/capsem-agent/src/main.rs`): warm-poll now calls
  `std::fs::read_dir(...).next()`, forcing a FUSE `READDIR` round-trip
  and proving virtiofsd has enumerated child inodes. Warming timeout
  (1 s total, 50 × 20 ms) is unchanged. If warming never completes the
  rebind now aborts instead of binding against an empty subtree, so the
  failure surfaces loudly in `read_file` rather than silently corrupting
  `/root`. Verified against the previously-flaky
  `tests/capsem-mcp/test_state_transitions.py::test_suspend_and_resume_persistent`
  under `-n 4 --dist=loadfile` (1/1 fail pre-fix, 5/5 pass post-fix).

- **`PytestUnknownMarkWarning` on `benchmark` marker.** Registered
  `benchmark` in `pyproject.toml [tool.pytest.ini_options].markers` so
  `tests/capsem-serial/test_parallel_benchmark.py`'s
  `pytest.mark.benchmark` no longer emits the warning. Warnings are
  errors per CLAUDE.md.

- **Stage-5 flake: pytest `check_leaks` fixture crashing at teardown.** Under
  concurrent load, macOS `sysctl(KERN_PROCARGS2)` can deny cmdline access for
  an unrelated host process; psutil surfaces that as an uncaught `SystemError`
  / `PermissionError` that dropped out of `process_iter(['pid','name','cmdline'])`
  before the existing per-iteration `try/except` could run, taking down the
  teardown of whichever test held the turn (observed on
  `test_cors_on_authenticated_endpoint`). Fix: `tests/conftest.py`'s
  `get_capsem_processes` now iterates without attr-prefetching cmdline and
  fetches cmdline lazily with a per-proc `try/except (psutil.Error, OSError,
  SystemError)`. Unit coverage in `tests/test_leak_detection.py`.

### Performance
- **`capsem delete` and `capsem purge` no longer pay the 2.7s graceful
  shutdown floor.** Previously `shutdown_vm_process` unconditionally sent
  `ServiceToProcess::Shutdown` via IPC, which armed capsem-process's 2.5s
  self-timer (giving the guest agent `SHUTDOWN_GRACE_SECS` to SIGTERM bash
  gracefully before SIGKILL) before the caller could observe process
  exit. Delete/purge don't need that grace because the session dir (with
  its workspace and bash history) is about to be removed anyway. Added a
  `graceful: bool` parameter to `shutdown_vm_process`; `handle_delete` and
  `handle_purge` now pass `false`, which SIGTERMs capsem-process directly
  (its `CFRunLoopStop` handler from 9b14618 makes this a clean exit) with
  a 1s poll before escalating to SIGKILL. `handle_stop` and `handle_run`
  keep graceful=true (persistent VMs need bash history preserved;
  handle_run reads session.db after teardown). Observed delete mean
  dropped from 2782 ms to ~70 ms across 3 benchmark runs, unblocking
  `tests/capsem-serial/test_lifecycle_benchmark.py::test_lifecycle_benchmark`
  under `just test` stage 5.

### Changed
- **Convention: Rust unit tests live in a sibling `tests.rs`, not an inline
  `mod tests { ... }` block.** Documented in `CLAUDE.md` (Code Style) and
  `skills/dev-testing/SKILL.md` (with extraction recipe and rationale).
  Codifies the pattern just applied across `policy_config`, `session`,
  `capsem-proto`, and `virtio_fs`. Agents writing new Rust modules should
  default to the sibling pattern; reviewers should push back on new inline
  test blocks.

### Fixed
- **`capsem-agent` `write_nofollow`: fsync before returning so
  `capsem_write_file` is durable across an immediate `capsem_suspend`.**
  Previously the agent did open+write+close without fsync. On VirtioFS, close
  only triggers FUSE_FLUSH (which virtiofsd is free to no-op), so the write
  could still be buffered inside Apple VZ's in-process virtiofsd when a
  caller immediately suspended the VM. VZ tore down virtiofsd before the
  data reached the host backing store, and the resumed VM (with a fresh
  virtiofsd) saw ENOENT. Surfaced as a concurrency flake of
  `tests/capsem-mcp/test_state_transitions.py::test_suspend_and_resume_persistent`
  under `just test` stage 5 -- 2 ms between write_file returning and the
  suspend request is not enough time for Apple's virtiofsd to drain.
  `file.sync_all()` sends FUSE_FSYNC, a core FUSE opcode virtiofsd must
  honor, giving write_file a real durability contract.
- **`capsem-logger/src/writer.rs`: `clippy::type_complexity` in
  `exec_event_insert_populates_row` test.** The test declared an
  8-element tuple type on the destructuring binding for a
  `Connection::query_row` call; clippy flagged it under
  `--all-targets -- -D warnings`. Replaced the outer type annotation
  with per-column `let` bindings inside the closure, which also makes
  each column's expected type read at the call site rather than in a
  parallel tuple. No behavior change; `cargo test -p capsem-logger`
  still 210 pass.

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
