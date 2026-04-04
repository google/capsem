# Sprint T22: Recipe Tests
## Goal
Verify just recipes (run-service, run, run-doctor) actually work

## Files
- tests/capsem-recipes/conftest.py
- tests/capsem-recipes/test_run_service.py
- tests/capsem-recipes/test_run_doctor.py
- tests/capsem-recipes/test_build.py

## Tasks
- [x] test_run_service: subprocess just run-service, verify socket created, capsem list works, kill service
- [x] test_run_doctor: subprocess just run-doctor, verify exit code 0 or known output
- [x] test_cargo_build_workspace: cargo build --workspace succeeds, all expected binaries exist
- [x] Marked recipe, SLOW

## Verification
pytest tests/capsem-recipes/ -m recipe

## Depends On
T14
