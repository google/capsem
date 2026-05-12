# Site Download And Release Verification

## Goal

Make the marketing-site installer work with the v1.1 release format and make
Linux `.deb` payload verification a permanent script and CI gate.

## Decisions

- Keep the public install entry point as `curl -fsSL https://capsem.org/install.sh | sh`.
- Resolve latest package assets through the GitHub release API because package
  filenames include the stamped release version.
- Verify downloaded packages against the signed release manifest when local
  tools are available, and always fail on hash mismatches when Python can parse
  the manifest.
- Centralize `.deb` payload checks in a Python script so CI and local
  post-release checks use the same logic.

## Files

- `site/public/install.sh`
- `tests/test_install_sh.py`
- `scripts/verify_deb_payload.py`
- `tests/test_verify_deb_payload.py`
- `.github/workflows/release.yaml`
- `.github/workflows/ci.yaml`
- `justfile`
- `tests/test_release_workflow_policy.py`
- `tests/test_ci_codesign_runner.py`
- `scripts/run_signed.sh`
- `scripts/create_hash_assets.py`
- `tests/capsem-build-chain/test_create_hash_assets.py`
- `scripts/lib/exec_lock.sh`
- `tests/test_exec_lock.py`
- `skills/release-process/SKILL.md`
- `CHANGELOG.md`

## Done

- Website installer selects the new `Capsem-<version>.pkg` macOS asset and
  `Capsem_<version>_<arch>.deb` Linux assets.
- Installer validates the downloaded package against `manifest.json` when
  Python is available and verifies the manifest signature when `minisign` is
  available.
- macOS install uses the native `installer` command instead of opening a pkg
  from a temporary directory that may be removed immediately.
- `.deb` payload verifier checks control metadata, helper binaries, signed
  manifest files, and optional minisign verification.
- Release CI calls the verifier for Linux release artifacts.
- PR CI preserves the macOS cargo runner build log on failures, and the runner
  serializes ad-hoc codesigning during concurrent `nextest` discovery.
- PR install E2E installs the host tools needed to build and sign missing VM
  assets from a clean checkout, including Node/pnpm for the doctor/frontend
  checks reached by the clean asset rebuild fallback.
- PR CI Rust coverage uses the same 65-line floor as the documented local
  `just test` gate, with a policy test preventing future drift.
- Clean Linux CI can rebuild VM asset hash aliases even when Docker-produced
  source files cannot be hardlinked by the runner user.
- PR install E2E runs pytest inside the Docker/systemd container with the dev
  dependency group available instead of relying on implicit `uv run` behavior.
- macOS PR CI keeps Python schema/coverage collection scoped to top-level
  contract tests instead of accidentally collecting VM integration suites.
- The shared `just` execution lock works on macOS runners without installing a
  separate `flock` binary.

## Testing Matrix

- Unit/contract: focused Python tests for installer helper functions, `.deb`
  verifier archive parsing, CI policy drift, hash-asset hardlink fallback, and
  the macOS-safe execution-lock fallback.
- Functional: marketing site build and release workflow policy tests.
- Adversarial: malformed/missing `.deb` payload tests and missing release asset
  tests.
- E2E/VM: covered by release `test-install`, Linux artifact validation, and
  x86 boot test; no VM boot needed for static site changes.
- Telemetry: not applicable.
- Performance: not applicable.
