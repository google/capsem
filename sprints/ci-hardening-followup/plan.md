# CI Hardening Follow-Up

## Goal

Beef up ordinary PR/main CI after the release verification work exposed warnings
that could still look green in GitHub Actions. This sprint turns the observed
weak spots into workflow behavior plus policy tests so they do not drift back.

## Scope

- `.github/workflows/ci.yaml`
  - Keep Linux PR CI compile-only for hosted KVM, but make the diagnostics step
    non-red without relying on `continue-on-error`.
  - Make Rust integration coverage release-blocking by removing the `|| true`
    mask.
  - Make the coverage summary pipe fail when `cargo llvm-cov report` fails.
  - Move Codecov test analytics from the deprecated test-results action to the
    supported `codecov/codecov-action@v5` path.
  - Opt the workflow into Node 24 action runtime to avoid late Node 20 action
    deprecation surprises.
- `.github/workflows/release.yaml`, `.github/workflows/docs.yaml`,
  `.github/workflows/site.yaml`
  - Opt remaining workflows into Node 24 action runtime.
- `tests/test_ci_codesign_runner.py`
  - Add policy tests that fail if these ordinary CI invariants regress.
- `skills/release-process/SKILL.md`
  - Capture the hard-won invariant in the release skill.
- `CHANGELOG.md`
  - Record the user-facing CI reliability fix under Unreleased.

## Decisions

- Keep KVM live execution out of PR Linux CI. Hosted ARM runners are still not
  the right place for real KVM exercise; release CI owns that.
- Do not use `continue-on-error` for diagnostic-only steps. Make the diagnostic
  command explicitly non-fatal so a green job does not carry a red annotation.
- Do not mask test commands with `|| true`. If a test lane is intentionally
  informational, it should be named and tested as such, not silently ignored.
- Use `set -o pipefail` on coverage summary pipes so `tee` cannot hide a failed
  coverage command.

## Done

- Focused CI policy tests pass locally.
- Release workflow policy tests still pass.
- `git diff --check` is clean.
- Branch is pushed and PR CI proves the workflow changes on GitHub.

## Coverage Matrix

- Unit/contract: `tests/test_ci_codesign_runner.py` workflow policy tests.
- Functional: GitHub Actions PR/main CI run exercises the edited workflow.
- Adversarial: Policy tests assert masks and deprecated action wiring are absent.
- E2E/VM: Not applicable; this change only edits CI orchestration.
- Telemetry: Not applicable; no session data changes.
- Performance: Not applicable; no runtime product path changes.
- Missing/deferred: Full `just test` is not required for YAML-only CI policy
  hardening; the focused Python policy tests and GitHub Actions run are the
  relevant gates.
