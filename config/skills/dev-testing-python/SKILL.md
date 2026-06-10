---
name: dev-testing-python
description: Python test infrastructure for the capsem-builder package. Use when running Python tests, checking coverage, debugging test failures, working with golden fixtures, or generating schemas. Covers pytest config, coverage floors, cross-language conformance tests, and the schema generation pipeline.
---

# Python Testing (capsem-builder)

## Quick reference

```bash
uv run python -m pytest tests/                                    # All tests
uv run python -m pytest tests/ --cov=src/capsem --cov-fail-under=90  # With coverage
uv run python -m pytest tests/test_validate.py -k "test_E001"     # Single test
just test                                                          # Full suite (Rust + Python + frontend)
just schema                                                        # Regenerate JSON schema + defaults
```

## Package config

`pyproject.toml`:
- Package: `capsem`, entry point `capsem-builder = capsem.builder.cli:main`
- Build: hatchling, wheel packages `src/capsem`
- Test deps: `pytest>=8.0`, `pytest-cov>=6.0` (in `[dependency-groups] dev`)
- `testpaths = ["tests"]`

## Test directory: `tests/`

| File | Tests | What it covers |
|------|-------|----------------|
| `test_validate.py` | 96 | TOML config linting, error codes E001-E305, warnings W001-W012 |
| `test_models.py` | 80 | Pydantic models (GuestImageConfig, ArchConfig, all sub-models) |
| `test_cli.py` | 79 | Click CLI commands (build, validate, inspect, init, add, audit, mcp, doctor) |
| `test_docker.py` | 75 | Jinja Dockerfile rendering, conformance with legacy Dockerfiles |
| `test_settings_spec.py` | 73 | Settings schema conformance (golden fixture round-trip) |
| `test_manifest.py` | 48 | BOM collection, manifest rendering, dpkg/pip/npm parsers |
| `test_config.py` | 41 | TOML config loading, defaults generation, roundtrip |
| `test_doctor.py` | 27 | Build doctor checks (Docker, tools, disk, permissions) |
| `test_scaffold.py` | 23 | init/add scaffold commands |
| `test_mcp.py` | 20 | JSON-RPC 2.0 MCP stdio server |
| `test_audit.py` | 20 | Trivy/grype JSON parsing, severity summary |

## Coverage

- Floor: 90% enforced by `--cov-fail-under=90` in `just test`
- Report: `codecov-python.xml` (XML for CI upload)
- codecov.yml: builder component at `src/capsem/**`, included in `unit` flag
- Current: ~97% (as of Phase 7 completion)

## Golden fixtures and cross-language conformance

Golden fixture at `tests/settings_spec/golden.json` with expected output at `tests/settings_spec/expected.json`. Three language parsers must produce identical results:

| Language | Test file | Tests |
|----------|-----------|-------|
| Python | `tests/test_settings_spec.py` | 73 |
| Rust | `crates/capsem-core/tests/settings_spec.rs` | 12 |
| TypeScript | `frontend/src/lib/__tests__/settings_spec.test.ts` | 14 |

If you change the settings schema (node types, metadata fields), all three must be updated together.

## Schema generation pipeline

```
guest/config/*.toml -> Pydantic models -> config/settings-schema.json (JSON Schema)
                                       -> config/defaults.json (settings interchange)
```

- `just schema` runs `generate_schema.py` which calls `export_json_schema()` and `generate_defaults_json()`
- Rust reads `config/defaults.json` via `include_str!()` in `registry.rs`
- TypeScript validates against `config/settings-schema.json` in conformance tests

## In-VM tests (NOT pytest on host)

`guest/artifacts/diagnostics/` contains 207 pytest tests that run INSIDE the VM via `just run "capsem-doctor"`. These are NOT part of the host `uv run pytest` suite. They test the guest environment (mounts, networking, sandbox, MCP, runtimes). See `/dev-testing-vm` for details.

## Source layout

```
src/capsem/
    __init__.py
    builder/
        __init__.py
        cli.py           Click CLI entry point
        config.py         TOML config loading, defaults generation
        models.py         Pydantic models (GuestImageConfig, ArchConfig, etc.)
        schema.py         Settings schema (SettingsRoot, GroupNode, SettingNode)
        docker.py         Jinja Dockerfile rendering, Docker build execution
        manifest.py       BOM collection, manifest rendering
        validate.py       Compiler-style linting with error codes
        scaffold.py       init/add scaffolding
        audit.py          Trivy/grype output parsing
        mcp_server.py     JSON-RPC 2.0 MCP stdio server
        doctor.py         Build environment doctor checks
        templates/
            Dockerfile.rootfs.j2
            Dockerfile.kernel.j2
```
