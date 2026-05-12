# Sprint: Site Download And Release Verification

## Tasks

- [x] Plan installer and release verification hardening.
- [x] Update marketing installer for v1.1 package names and manifest checks.
- [x] Add permanent `.deb` payload verifier.
- [x] Wire verifier into release CI.
- [x] Update tests and changelog.
- [x] Run focused verification gates.
- [x] Commit local FAQ/release-skill work plus this fix.

## Notes

- The existing uncommitted FAQ page and release-process skill changes are
  intentional local work and should be committed, not discarded.
- The previous one-off local `.deb` inspection failed because the macOS host did
  not have `dpkg-deb`/`zstd` on PATH. The checked-in verifier should make this
  explicit and reusable instead of relying on ad hoc shell.

## Coverage Ledger

- Unit/contract: `UV_CACHE_DIR=/private/tmp/capsem-uv-cache uv run --offline pytest tests/test_install_sh.py tests/test_verify_deb_payload.py tests/test_release_workflow_policy.py -q` passed with 34 tests.
- Functional: `pnpm -C site run build` passed and generated `/index.html` plus `/faq/index.html`; `sh -n site/public/install.sh` passed.
- Adversarial: `.deb` verifier tests reject missing helper payloads and mismatched architecture; install script tests reject missing release assets.
- E2E/VM:
- Telemetry: not applicable.
- Performance: not applicable.
- Missing/deferred: no live install was executed from the website script in this sprint; release CI covers package install and boot paths.
