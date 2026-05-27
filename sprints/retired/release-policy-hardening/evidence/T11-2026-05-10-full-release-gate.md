# T11 Evidence: Full Local Release Gate

Date: 2026-05-10, updated 2026-05-11

## Scope

This note records the unblocked T11 local release-candidate gates completed
after T10. It is not a PR, commit, tag, or release sign-off. Host package,
installed CLI, installed app, and tray relaunch proof are captured; Elie visual
sign-off remains the explicit Gate C/Gate D manual stop.

## Passing Proofs

- `just test`: final pass on 2026-05-11 after the installed-tray healing and
  Linux dead-code cfg fixes.
  - Frontend check/build/test: 19 Vitest files and 388 tests passed; Astro
    built 2 pages.
  - Rust coverage: pass, total coverage 68.07%.
  - Python xdist suite: 1344 passed, 69 skipped; Python coverage 91.24%.
  - Build-chain serial tests: 22 passed.
  - Injection test: 5 passed, 0 failed.
  - Local manifest signature gate: `manifest signature verifies with dev key`.
  - Integration: in-VM diagnostics 94 passed, 2 skipped; host integration
    47 passed, 0 failed, 0 warnings.
  - Ephemeral model check: sentinel absent across fresh invocations.
  - Benchmark gate: 1 passed.
  - Linux release cross-compile: produced and validated one fresh
    `Capsem_1.1.1778456247_arm64.deb`; no stale `.deb` artifact was accepted.
  - Install e2e inside Docker/systemd: 33 passed, 31 skipped.
- `cd assets && b3sum --check B3SUMS`: pass for all 9 arch/current asset
  entries.
- `bash scripts/verify-local-manifest-signature.sh assets config/manifest-sign.pub`:
  pass, manifest verifies with the dev key.
- `just doctor`: pass, 42 passed, 0 skipped, 0 warnings. This includes
  `Manifest Signing Tools: minisign`, Colima/Docker runtime checks, VM asset
  B3SUMS, and local asset manifest signature verification.
- `just exec "capsem-doctor"`: pass, 308 passed and 4 skipped; result
  `PASS -- all diagnostics passed`. The command repacked initrd first and
  verified the local manifest signature with the dev key.
- Final post-`just exec` checks:
  - `cd assets && b3sum --check B3SUMS`: pass for all 9 entries.
  - `bash scripts/verify-local-manifest-signature.sh assets config/manifest-sign.pub`:
    pass.
  - `just doctor`: pass, 42 passed, 0 skipped, 0 warnings.
- `just build-ui`: pass after rerunning outside the sandbox when pnpm optional
  native package restore hit sandboxed DNS. Frontend built 2 pages and
  `target/debug/capsem-app` was rebuilt.
- Restored-private release preflight:
  - `env UV_CACHE_DIR=target/uv-cache scripts/preflight.sh`: pass, 40 passed
    and 0 failed. This includes Apple certificate import, legacy p12 format,
    base64 sync, notarization credential history, restored manifest key
    signing, and verification with `config/manifest-sign.pub`.
  - `scripts/check-release-workflow.sh`: pass, 13 passed and 0 failed. The
    local check now auto-discovers `private/minisign/manifest.key` and proves
    the passwordless minisign flow used by CI.
- Host package/install smoke:
  - `just install` built `packages/Capsem-1.1.1778456247.pkg`, opened
    Installer.app, installed `~/.capsem/bin/capsem` and companion binaries,
    and reached `Service is responding.`
  - The recipe then exposed a postinstall bug: an existing
    `~/.capsem/assets -> /Users/elie/git/capsem/assets` symlink let root-owned
    package manifest files land in the repo, causing the final dev-asset sync
    to fail with `Permission denied`.
  - Repair applied: `scripts/pkg-scripts/postinstall` now removes a symlinked
    `~/.capsem/assets` and creates a real per-user asset directory before
    seeding package assets. The current install was repaired by replacing the
    symlink with a user-owned directory and re-syncing local assets.
  - Package artifact refresh: `bash scripts/build-pkg.sh
    target/release/bundle/macos/Capsem.app target/release assets
    1.1.1778456247` rebuilt `packages/Capsem-1.1.1778456247.pkg` after the
    postinstall fix. `pkgutil --expand-full` confirmed the packaged
    `Scripts/postinstall` contains the symlink replacement before
    `seed_asset_manifests`.
  - Installer app-location fix: `scripts/build-pkg.sh` now also stages
    `Capsem.app` under `/usr/local/share/capsem/Capsem.app`, and
    `scripts/pkg-scripts/postinstall` explicitly materializes it into
    `/Applications/Capsem.app` with `ditto`. `pkgutil --expand-full` confirmed
    the rebuilt package contains both the normal `/Applications/Capsem.app`
    payload and the postinstall fallback app copy, plus the packaged
    `install_app_bundle` fallback. The current host has
    `/Applications/Capsem.app` and the running UI process is
    `/Applications/Capsem.app/Contents/MacOS/capsem-app`.
  - Installed asset proof: `~/.capsem/assets/manifest.json` verifies with the
    dev key and service `/list` reports `asset_health.ready=true`,
    `version=2026.0510.20`, and no missing assets.
  - Installed CLI proof: `~/.capsem/bin/capsem --version` reports
    `capsem 1.1.1778456247`.
  - Installed doctor proof: `~/.capsem/bin/capsem doctor` passed with
    308 passed, 4 skipped, and `PASS -- all diagnostics passed`.
  - Installed VM proof: `~/.capsem/bin/capsem run "echo installed-demo-ok"`
    printed `installed-demo-ok`.
  - Demo UI proof: `/Applications/Capsem.app` is present and launched; process
    list shows `/Applications/Capsem.app/Contents/MacOS/capsem-app` running
    alongside `capsem-service`, `capsem-gateway`, and `capsem-tray`.
  - Tray relaunch proof: after adding the service-owned
    `/companions/tray/ensure` endpoint and app launch/focus/periodic heal,
    the installed app restored a killed tray from `/Applications/Capsem.app`.
    Fresh-launch proof had no live tray before launch and spawned tray PID
    `79261` as child of service PID `78909`. Running-app proof killed tray PID
    `79960`; the app's periodic heal spawned replacement tray PID `79981`
    under the same installed service.
- Gate C dev desktop proof:
  - `just run-ui --` launched the dev desktop app; process proof showed the app
    running with the service/gateway/tray, and `/version` responded.
  - The dev app was intentionally stopped after proof, so the launcher session
    exited 143 from our termination.
  - Screenshot capture was blocked by macOS display permission with
    `could not create image from display`; Elie visual sign-off remains open.
  - After screen capture permission was fixed, release-polish removed the
    visible `build {__BUILD_TS__}` timestamp from the toolbar/tab area.
    `pnpm -C frontend run check`, `pnpm -C frontend run build`, and
    `just build-ui` passed. Browser proof confirmed `hasBuildStamp=false` and
    the durable screenshot is
    `sprints/release-policy-hardening/evidence/T11-gate-c-no-build-stamp.png`.
- Focused regressions added during the T11 gate:
  - `env UV_CACHE_DIR=target/uv-cache uv run pytest tests/test_leak_detection.py -q`:
    15 passed.
  - `env UV_CACHE_DIR=target/uv-cache uv run pytest tests/capsem-mcp/test_exec.py::test_stderr -q`:
    1 passed.
  - `env UV_CACHE_DIR=target/uv-cache uv run pytest tests/test_release_workflow_policy.py::test_local_dev_manifest_signing_is_bootstrap_and_doctor_prereq -q`:
    1 passed.
  - `env UV_CACHE_DIR=target/uv-cache uv run pytest tests/test_package_scripts.py tests/test_release_workflow_policy.py -q`:
    18 passed after adding the macOS postinstall symlink regression.
  - `bash -n scripts/doctor-macos.sh scripts/doctor-common.sh`: pass.
- Release hygiene:
  - `env UV_CACHE_DIR=target/uv-cache scripts/preflight.sh`: pass after
    `private/` restore.
  - `scripts/check-release-workflow.sh`: pass after `private/` restore.
  - `git diff --check`: pass.
  - `git status --short`: reviewed; dirty tree intentionally not staged.

## Fixes Landed During T11 Gate

- Local manifest signing is now a hard local prerequisite:
  `scripts/verify-local-manifest-signature.sh` exists, `just _pack-initrd` and
  `just test` run it, `bootstrap.sh` requires `minisign`, and
  `capsem-doctor` can auto-fix/install `minisign` on macOS.
- Asset cleanup now preserves metadata files that are not boot assets:
  `manifest.json`, `manifest.json.minisig`, `manifest-sign.dev.pub`, and
  `B3SUMS`.
- `scripts/integration_test.py` no longer lets a stale sparse desktop launch
  log fail a service-only integration run.
- `just cross-compile` now clears stale `.deb` artifacts before building,
  requires exactly one fresh package, validates only that package, and removes
  stale same-arch copies from `dist/`.
- `just cross-compile` now regenerates `B3SUMS`, `manifest.json`, hash aliases,
  local dev signatures, and signature verification after refreshing
  `assets/current`.
- The pytest leak detector now avoids `psutil.process_iter` attr prefetch so a
  protected macOS process cannot fail unrelated test teardown.
- `scripts/doctor-macos.sh` now captures `colima status` output before grep, so
  `grep -q` cannot trip `pipefail` via a SIGPIPE from the verbose Colima
  status command.
- `scripts/preflight.sh` and `scripts/check-release-workflow.sh` now support
  the restored `private/minisign/manifest.key` layout and passwordless signing,
  matching the CI release workflow instead of requiring a local password file.
- `scripts/pkg-scripts/postinstall` now prevents root-owned package asset
  manifests from being written through a developer symlink into the repo.
- `crates/capsem-app/src/main.rs` macOS/test-gates the service-socket tray
  helper functions so the Linux Tauri release build stays warning-clean.
- `crates/capsem-service/src/main.rs` macOS-gates the tray companion variant
  and fields so Linux Docker/systemd install E2E builds warning-clean.

## Remaining Local Holds

- Gate C desktop dev launch has command/process proof, but Elie visual
  sign-off remains open because screenshot capture was blocked by macOS display
  permission.
- Gate D host package visual sign-off remains open: installed app launch and
  tray relaunch proof are captured, but Elie still needs to visually confirm
  the packaged app path for the demo.
- No staging, commit, tag, push, or PR was performed.
