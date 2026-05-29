# Sprint: Mac Benchmark Results

## Tasks

- [x] Create branch from current `origin/main`.
- [x] Run `just bench` on macOS.
- [x] Fix blockers if the benchmark harness fails.
- [ ] Commit benchmark results.

## Notes

- Branch: `codex/mac-benchmark-results-20260529`.
- First `just bench` stopped in `_ensure-setup`: Docker/Colima was not running,
  VM assets were missing, and guest binaries were not packed yet.
- `just doctor fix` completed after restarting Colima; doctor recheck reported
  41 passed, 0 skipped, 0 warnings.
- Default `just bench` then built/signed binaries but stopped because the
  installed user service owned `~/.capsem/run/service.sock`; rerunning with an
  isolated `CAPSEM_HOME` for benchmark capture.
- First isolated run completed setup but failed provisioning because the service
  started before setup wrote the profile-backed asset pin; rerunning in the same
  isolated home restarts the service with the completed setup state.
- Permanent fix: `_ensure-service` now refreshes local setup/profile pins
  against repo assets after `_pack-initrd`, even when the caller did not export
  `CAPSEM_ASSETS_DIR`.
- `CAPSEM_HOME=$PWD/target/bench-home CAPSEM_RUN_DIR=$PWD/target/bench-home/run just bench`
  passed on macOS arm64.
- Captured guest `capsem-bench` JSON to
  `benchmarks/capsem-bench/data_1.2.1779673506_arm64.json`.
- No Linux benchmark JSON is present in this checkout or visible remote branch
  names; comparison needs the Linux team's JSON artifact or branch.

## Coverage Ledger

- Unit/contract: `uv run python -m pytest tests/test_build_assets_script.py::test_ensure_service_refreshes_local_profile_after_asset_repack -q`
- Functional: `CAPSEM_HOME=$PWD/target/bench-home CAPSEM_RUN_DIR=$PWD/target/bench-home/run just bench`
- Adversarial: not needed unless code changes.
- E2E/VM: in-VM `capsem-bench` and host lifecycle/fork benchmarks passed via `just bench`.
- Telemetry: not claimed.
- Performance: new JSON under `benchmarks/capsem-bench/`, `benchmarks/lifecycle/`, `benchmarks/fork/`, and `benchmarks/security-engine/`.
- Missing/deferred: Linux comparison artifact is not present locally.
