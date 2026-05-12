# Sprint: Site Download And Release Verification

## Tasks

- [x] Plan installer and release verification hardening.
- [x] Update marketing installer for v1.1 package names and manifest checks.
- [x] Add permanent `.deb` payload verifier.
- [x] Wire verifier into release CI.
- [x] Update tests and changelog.
- [x] Run focused verification gates.
- [x] Commit local FAQ/release-skill work plus this fix.
- [x] Check live `capsem.org` and confirm the deployed site was still stale.
- [x] Open PR #37 from a fresh `origin/main` branch.
- [x] Fix Linux KVM test compile errors surfaced by PR CI.
- [x] Fix macOS PR CI clean-checkout frontend dist ordering for Tauri tests.
- [x] Fix macOS PR CI codesigning race/diagnostics in the cargo runner.

## Notes

- The existing uncommitted FAQ page and release-process skill changes are
  intentional local work and should be committed, not discarded.
- The previous one-off local `.deb` inspection failed because the macOS host did
  not have `dpkg-deb`/`zstd` on PATH. The checked-in verifier should make this
  explicit and reusable instead of relying on ad hoc shell.
- Live `https://capsem.org/`, `/faq`, and `/install.sh` returned 200, but the
  deployed content was still old: `install.sh` lacked manifest signature/checksum
  verification and the pages still showed the old Linux roadmap copy. The fix
  must merge to `main` to trigger the Cloudflare Pages deploy.
- PR #37 initially failed `test-linux` on mainline Linux-only test compile
  errors: wrong `memory` path in a KVM MMIO test, missing `Debug` derives for
  `unwrap_err()`, missing `PermissionsExt`, and immutable test harness bindings
  around mutable queue notification.
- PR #37 then failed macOS `test` before frontend build because
  `capsem-app`'s Tauri test build checks `frontendDist`. CI now creates a
  minimal `frontend/dist/index.html` before Rust unit coverage; the real
  frontend check/test/build step still runs later.
- PR #37 then failed macOS `test` during `nextest` test discovery because the
  cargo runner signs binaries on demand and concurrent discovery can race
  `codesign`. `scripts/run_signed.sh` now serializes signing, skips already
  valid signatures, and CI uploads `target/build.log` on failure so the real
  signing error is not lost behind Codecov fallout.

## Coverage Ledger

- Unit/contract: `UV_CACHE_DIR=/private/tmp/capsem-uv-cache PYTHONPYCACHEPREFIX=/private/tmp/capsem-pycache uv run --offline pytest tests/test_install_sh.py tests/test_verify_deb_payload.py tests/test_release_workflow_policy.py -q` passed with 36 tests; `UV_CACHE_DIR=/private/tmp/capsem-uv-cache PYTHONPYCACHEPREFIX=/private/tmp/capsem-pycache uv run --offline pytest tests/test_ci_codesign_runner.py tests/test_release_workflow_policy.py -q` passed with 18 tests; `cargo check -p capsem-core --tests` passed.
- Functional: `pnpm -C site run build` passed and generated `/index.html` plus `/faq/index.html`; `sh -n site/public/install.sh` passed; `bash -n scripts/run_signed.sh` passed; disposable local `scripts/run_signed.sh` codesign smoke passed; `PYTHONPYCACHEPREFIX=/private/tmp/capsem-pycache python3 -m py_compile scripts/verify_deb_payload.py` passed.
- Adversarial: `.deb` verifier tests reject missing helper payloads and mismatched architecture; install script tests reject missing release assets.
- E2E/VM:
- Telemetry: not applicable.
- Performance: not applicable.
- Missing/deferred: local Linux-target `cargo check --target aarch64-unknown-linux-musl --tests` could not complete on macOS because `aarch64-linux-musl-gcc` is absent for C dependencies; GitHub `test-linux` remains the authoritative proof for that path. No live install was executed from the website script in this sprint; release CI covers package install and boot paths.
