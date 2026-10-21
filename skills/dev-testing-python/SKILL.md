---
name: dev-testing-python
description: Python test infrastructure for capsem-builder. Use when running Python tests, checking coverage, or working with golden fixtures and generated schemas.
---

# Python Testing (capsem-builder)

## Quick reference

```bash
python3 build_system/scripts/ci/run-bounded-command.py --timeout-seconds 120 -- \
  uv run --project build_system --frozen python -m pytest \
  -c build_system/pyproject.toml --rootdir . build_system/tests/image/ --no-cov

python3 build_system/scripts/ci/run-bounded-command.py --timeout-seconds 60 -- \
  uv run --project build_system --frozen python -m pytest \
  -c build_system/pyproject.toml --rootdir . \
  build_system/tests/image/test_validate.py -k E001 --no-cov

just test <source-commit>  # Complete qualification, governed by AGENTS.md
```

## Package config

`build_system/pyproject.toml`:
- Distribution: `capsem-builder`
- Import: direct `build_system/builder/` mapping to `capsem_builder`
- Entry point: `capsem-builder = capsem_builder.gatelaunch:builder_main`, which
  selects the project interpreter before dispatching to the image CLI
- Build: setuptools, with no nested `src/` or compatibility package tree
- Tests: `build_system/tests/`, using the locked `build_system/uv.lock`

The gate already lives in `build_system/builder/gate/`. The installed package
mapping and pytest's configured search paths own imports; do not add an obsolete
source root to `PYTHONPATH`. Explicit `-c` and `--rootdir` keep mixed image,
gate and cross-system tests under the same configuration and import namespace.

## Test directory: `build_system/tests/image/`

| File | What it covers |
|------|----------------|
| `test_validate.py` | TOML config linting, error codes E001-E305, warnings W001-W012 |
| `test_models.py` | Pydantic image and image-workspace models |
| `test_cli.py` | Backend-only Click CLI surface |
| `test_docker.py` | Jinja rendering and image-build execution primitives |
| `test_manifest.py` | BOM collection, manifest rendering, package parsers |
| `test_config.py` | TOML loading, defaults generation, roundtrip |
| `test_doctor.py` | Build prerequisite and source-completeness checks |
| `test_audit.py` | Trivy/grype parsing and severity summaries |
| `test_image_build_backend.py` | Private capsem-admin backend command |
| `test_image_module_boundary.py` | Exact source, package, and entrypoint ownership |

## Coverage

- Floors, combined coverage and report locations belong to `config/gate.toml`
  and the owning gate steps. A focused `--no-cov` run is diagnostic feedback.
- `codecov.yml`: builder component includes `build_system/builder/**`

## Golden fixtures and cross-language conformance

Golden fixture at `tests/settings_spec/golden.json` with expected output at `tests/settings_spec/expected.json`. Three language parsers must produce identical results:

| Language | Test file |
|----------|-----------|
| Python | `tests/test_settings_spec.py` |
| Rust | `crates/capsem-core/tests/settings_spec.rs` |
| TypeScript | `web/app/src/lib/__tests__/settings_spec.test.ts` |

If you change the settings schema (node types, metadata fields), all three must be updated together.

## Schema generation pipeline

```
config/settings/settings.toml -> Pydantic models -> config/settings/schema.generated.json (JSON Schema)
                                                   -> config/settings/ui-metadata.generated.json (UI metadata)
```

- `just _generate-settings` runs the settings-generation owner. Wrap this direct
  development command with the bounded runner specified in AGENTS.md.
- Rust reads `config/settings/ui-metadata.generated.json` via `include_str!()` in `registry.rs`
- TypeScript validates against `config/settings/schema.generated.json` in conformance tests

## In-VM tests (NOT pytest on host)

`guest/artifacts/diagnostics/` contains the pytest tests that run INSIDE the VM via `just exec "capsem-doctor"`. These are NOT part of the host `uv run --project build_system --frozen pytest` suite. They test the guest environment (mounts, networking, sandbox, MCP, runtimes). See `/dev-testing-vm` for details.

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

Runtime image templates remain under `config/docker/`; they are not
Python package data and do not move into `build_system/`.
