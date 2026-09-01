---
name: dev-testing-python
description: Python test infrastructure for capsem-builder. Use when running Python tests, checking coverage, or working with golden fixtures and generated schemas.
---

# Python Testing (capsem-builder)

## Quick reference

```bash
PYTHONPATH=src:tests uv run --project build_system pytest \
  build_system/tests/image/                                      # Image-builder tests
PYTHONPATH=src:tests uv run --project build_system pytest build_system/tests/image/ \
  --cov=capsem_builder.image --cov-fail-under=85                 # With coverage
PYTHONPATH=src:tests uv run --project build_system pytest \
  build_system/tests/image/test_validate.py -k "test_E001"      # Single test
just test                                                  # Full suite
just _generate-settings                                          # Regenerate settings outputs
```

## Package config

`build_system/pyproject.toml`:
- Distribution: `capsem-builder`
- Import: direct `build_system/builder/` mapping to `capsem_builder`
- Entry point: `capsem-builder = capsem_builder.image.cli:main`
- Build: setuptools, with no nested `src/` or compatibility package tree
- Tests: `build_system/tests/`, using the locked `build_system/uv.lock`

During the staged migration, `PYTHONPATH=src:tests` supplies the not-yet-moved
gate package and retained cross-system test helpers. Remove that temporary test
environment edge when `builder/gate/` lands; do not add a compatibility package.

## Test directory: `build_system/tests/image/`

| File | What it covers |
|------|----------------|
| `test_validate.py` | TOML config linting, error codes E001-E305, warnings W001-W012 |
| `test_models.py` | Pydantic image and profile-workspace models |
| `test_cli.py` | Backend-only Click CLI surface |
| `test_docker.py` | Jinja rendering and image-build execution primitives |
| `test_manifest.py` | BOM collection, manifest rendering, package parsers |
| `test_config.py` | TOML loading, defaults generation, roundtrip |
| `test_doctor.py` | Build prerequisite and source-completeness checks |
| `test_audit.py` | Trivy/grype parsing and severity summaries |
| `test_image_build_backend.py` | Private capsem-admin backend command |
| `test_image_module_boundary.py` | Exact source, package, and entrypoint ownership |

## Coverage

- Floor: 85% enforced by `--cov-fail-under=85` in `just test`
- Report: `cache/target/coverage/python/codecov.xml` (XML for CI upload)
- `codecov.yml`: builder component includes `build_system/builder/**`
- Current image package: 91.12% (551 tests, measured during the repository move)

## Golden fixtures and cross-language conformance

Golden fixture at `tests/settings_spec/golden.json` with expected output at `tests/settings_spec/expected.json`. Three language parsers must produce identical results:

| Language | Test file | Tests |
|----------|-----------|-------|
| Python | `tests/test_settings_spec.py` | 73 |
| Rust | `crates/capsem-core/tests/settings_spec.rs` | 12 |
| TypeScript | `web/app/src/lib/__tests__/settings_spec.test.ts` | 14 |

If you change the settings schema (node types, metadata fields), all three must be updated together.

## Schema generation pipeline

```
config/settings/settings.toml -> Pydantic models -> config/settings/schema.generated.json (JSON Schema)
                                                   -> config/settings/ui-metadata.generated.json (UI metadata)
```

- `just schema` runs `generate_schema.py` which calls `export_json_schema()` and `generate_defaults_json()`
- Rust reads `config/settings/ui-metadata.generated.json` via `include_str!()` in `registry.rs`
- TypeScript validates against `config/settings/schema.generated.json` in conformance tests

## In-VM tests (NOT pytest on host)

`guest/artifacts/diagnostics/` contains 207 pytest tests that run INSIDE the VM via `just exec "capsem-doctor"`. These are NOT part of the host `uv run --project build_system --frozen pytest` suite. They test the guest environment (mounts, networking, sandbox, MCP, runtimes). See `/dev-testing-vm` for details.

## Source layout

```text
build_system/
  pyproject.toml
  uv.lock
  builder/                 # installs as capsem_builder
    image/
        __init__.py
        cli.py           Backend-only Click CLI entry point
        config.py         TOML config loading, defaults generation
        models.py         Pydantic models (GuestImageConfig, ArchConfig, etc.)
        schema.py         Settings schema (SettingsRoot, GroupNode, SettingNode)
        docker.py         Jinja Dockerfile rendering, Docker build execution
        manifest.py       BOM collection, manifest rendering
        validate.py       Compiler-style linting with error codes
        audit.py          Trivy/grype output parsing
        image_build_backend.py Private capsem-admin image build backend
        doctor.py         Build environment doctor checks
  tests/
    image/
```

Product/profile image templates remain under `config/docker/`; they are not
Python package data and do not move into `build_system/`.
