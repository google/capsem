# Sprint: CI Hardening Follow-Up

## Tasks

- [x] Reproduce the CI policy gaps with failing workflow tests.
- [x] Replace the Linux KVM red-success diagnostic step.
- [x] Make Rust integration coverage blocking.
- [x] Protect the coverage summary pipe with `set -o pipefail`.
- [x] Move Codecov test analytics to `codecov/codecov-action@v5`.
- [x] Opt ordinary workflows into the Node 24 action runtime.
- [x] Document the invariant in `release-process`.
- [x] Add changelog entry.
- [x] Run focused local gates.
- [x] Commit.
- [ ] Push branch and open PR.
- [ ] Watch GitHub CI.

## Notes

- Discovery: the prior main CI was green but carried a red annotation from the
  KVM setup step because `continue-on-error` only softened the conclusion.
- Discovery: `cargo llvm-cov report --no-cfg-coverage` emitted an unsupported
  flag error, and the pipe to `tee` hid the failing command status.
- Discovery: the Rust integration coverage lane was currently masked by
  `|| true`, while the latest GitHub run showed those tests passing.
- Discovery: Codecov has deprecated `codecov/test-results-action@v1`; test
  analytics now go through `codecov/codecov-action@v5` with
  `report_type: test_results`.

## Coverage Ledger

- Unit/contract:
  - `uv run --offline pytest tests/test_ci_codesign_runner.py -q` passed
    with 12 tests.
  - `uv run --offline pytest tests/test_release_workflow_policy.py -q` passed
    with 17 tests.
- Functional:
  - GitHub Actions PR run after push.
- Adversarial:
  - Workflow tests fail if `continue-on-error`, `|| true`, hidden coverage pipe
    behavior, deprecated Codecov test-results action, or missing Node 24 runtime
    opt-in returns.
- E2E/VM:
  - Not applicable; no VM product path changed.
- Telemetry:
  - Not applicable; no telemetry path changed.
- Performance:
  - Not applicable; no runtime path changed.
- Missing/deferred:
  - Full `just test` is outside this YAML policy hardening slice. The CI run is
    the functional proof for this change.
