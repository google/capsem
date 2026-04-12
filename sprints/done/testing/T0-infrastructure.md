# Sprint T0: Test Infrastructure

## Goal

Stand up the full test infrastructure: directory layout, pytest markers, just recipes, and codecov configuration. Every subsequent sprint depends on this foundation being solid.

## Files

**Create:**
- `tests/capsem-service/conftest.py`
- `tests/capsem-cli/conftest.py`
- `tests/capsem-session/conftest.py`
- `tests/capsem-snapshots/conftest.py`
- `tests/capsem-isolation/conftest.py`
- `tests/capsem-security/conftest.py`
- `tests/capsem-config/conftest.py`
- `tests/capsem-bootstrap/conftest.py`
- `tests/capsem-stress/conftest.py`

**Modify:**
- `pyproject.toml` -- add pytest markers
- `justfile` -- update test recipe, add new recipes
- `codecov.yml` -- add component definitions
- `.cargo/config.toml` or workspace `Cargo.toml` -- ensure llvm-cov covers all 9 crates
- CI config (`.github/workflows/`) -- verify all new crates compile

## Tasks

- [ ] Create `tests/capsem-service/` directory with empty `conftest.py`
- [ ] Create `tests/capsem-cli/` directory with empty `conftest.py`
- [ ] Create `tests/capsem-session/` directory with empty `conftest.py`
- [ ] Create `tests/capsem-snapshots/` directory with empty `conftest.py`
- [ ] Create `tests/capsem-isolation/` directory with empty `conftest.py`
- [ ] Create `tests/capsem-security/` directory with empty `conftest.py`
- [ ] Create `tests/capsem-config/` directory with empty `conftest.py`
- [ ] Create `tests/capsem-bootstrap/` directory with empty `conftest.py`
- [ ] Create `tests/capsem-stress/` directory with empty `conftest.py`
- [ ] Add all pytest markers to `pyproject.toml`: `mcp`, `integration`, `session`, `snapshot`, `isolation`, `security`, `config`, `bootstrap`, `stress`
- [ ] Update justfile default `test` recipe to exclude all slow markers: `-m "not mcp and not integration and not session and not snapshot and not isolation and not security and not config and not stress"`
- [ ] Add `just test-service` recipe (runs `tests/capsem-service/` with `-m integration`)
- [ ] Add `just test-cli` recipe (runs `tests/capsem-cli/`)
- [ ] Add `just test-session` recipe (runs `tests/capsem-session/` with `-m session`)
- [ ] Add `just test-snapshots` recipe (runs `tests/capsem-snapshots/` with `-m snapshot`)
- [ ] Add `just test-isolation` recipe (runs `tests/capsem-isolation/` with `-m isolation`)
- [ ] Add `just test-security` recipe (runs `tests/capsem-security/` with `-m security`)
- [ ] Add `just test-config` recipe (runs `tests/capsem-config/` with `-m config`)
- [ ] Add `just test-bootstrap` recipe (runs `tests/capsem-bootstrap/` with `-m bootstrap`)
- [ ] Add `just test-stress` recipe (runs `tests/capsem-stress/` with `-m stress`)
- [ ] Add `just test-all` recipe (runs all markers, no exclusions)
- [ ] Add `just coverage` recipe (runs `cargo llvm-cov` across all 9 crates, produces lcov + HTML reports)
- [ ] Update `codecov.yml`: add `service` component (covers `crates/capsem-service/`)
- [ ] Update `codecov.yml`: add `cli` component (covers `crates/capsem/`)
- [ ] Update `codecov.yml`: add `mcp-server` component (covers `crates/capsem-mcp/`)
- [ ] Ensure `cargo llvm-cov` instrument flags cover all 9 crates: capsem-core, capsem-service, capsem-process, capsem, capsem-mcp, capsem-app, capsem-agent, capsem-proto, capsem-logger
- [ ] Verify CI compiles all new crates (run `cargo check --workspace` in CI)

## Verification

- `just test` runs and skips all slow markers
- Each `just test-*` recipe runs without error (even if 0 tests collected)
- `just test-all` collects from every test directory
- `just coverage` produces an lcov report
- `cargo check --workspace` passes

## Depends On

Nothing -- this is the foundation sprint.
