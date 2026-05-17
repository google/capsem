# S03 - Service Settings Implementation

## Goal

Implement approved typed service settings.

## Tasks

- [x] Add Serde/TOML parsing.
- [x] Add Rust semantic validation.
- [x] Add defaulting without `config/defaults.json`.
- [x] Add Rust-owned UI descriptors.
- [x] Add credential TOML support for cutover.
- [x] Add typed asset/manifest/image location settings.
- [x] Add tests for valid, missing, unknown, and invalid service configs.
- [x] Add service settings file discovery/loading from configured paths.
- [x] Wire service settings into service startup asset resolution.
- [x] Wire service settings into status/debug payloads.
- [x] Add process-level E2E/VM coverage for service.toml-owned assets.
- [x] Capture old-caller removal as S01 carry-over work.

## Implemented Slice

`crates/capsem-core/src/settings_profiles/mod.rs` now includes typed
`ServiceSettings` with `deny_unknown_fields`, endpoint validation for
service-scoped telemetry and remote policy plugins, TOML credential storage,
profile roots, asset/manifest/image locations, default values, file load/save
helpers, and UI descriptor metadata.

The service startup path now loads `<CAPSEM_HOME>/service.toml`, resolves the
asset directory with explicit origin (`cli`, `service_settings`, or `default`),
and stores the resolved asset/manifest/image source on `ServiceState`.
`installed` manifests continue to use the cached assets manifest, `local-file`
manifests are read from the configured path and must pass minisign verification,
and `remote-url` manifests do not block service startup on the network; startup
uses the cached local manifest while the asset service owns remote
reconciliation. `/setup/assets` and the pasteable debug report both expose the
resolved asset locations and provenance.

Focused test command:

```sh
cargo test -p capsem-core settings_profiles
```

Current result: 41 focused `settings_profiles` tests passed.

Additional focused verification:

```sh
cargo test -p capsem-service --lib debug_report::tests
cargo test -p capsem-service startup_
cargo test -p capsem-service handle_asset_status_reports_resolved_asset_location_sources
cargo test -p capsem-service
CAPSEM_ASSETS_DIR=/Users/elie/git/capsem/assets uv run python -m pytest tests/capsem-service/test_svc_service_settings_runtime.py -v --tb=short
CAPSEM_ASSETS_DIR=/Users/elie/git/capsem/assets uv run python -m pytest tests/capsem-service/test_svc_setup.py::TestSetupAssets tests/capsem-service/test_svc_service_settings_runtime.py -v --tb=short
```

Current result: debug report tests passed, startup manifest tests passed,
`/setup/assets` provenance test passed, and the full `capsem-service` Rust suite
passed outside the sandbox (95 lib tests, 113 service-bin tests). The S03
process-level E2E file passed with 3 tests:
real `capsem-service` reading `service.toml` without `--assets-dir`, real
gateway proxying `/setup/assets`, malformed legacy service TOML failing startup
before accepting a socket, and VM boot/exec using service.toml-owned assets.

## Coverage Ledger

- Unit/contract: parse/default/validate/file-load/file-save/descriptor tests are
  present for the implemented service-settings slice, including asset
  resolution provenance.
- Functional: service settings load/save works at the model layer; service
  startup consumes typed service settings for asset directory/manifest source;
  `/setup/assets` and debug report expose resolved asset provenance.
- Adversarial: unknown fields and invalid enabled endpoint state are covered;
  malformed TOML, invalid endpoint scheme, empty credential values, and invalid
  write rejection are covered. Remote manifest missing URL, local manifest with
  remote URL, invalid asset download endpoint, unsigned explicit local
  manifests, invalid installed manifest signatures, and remote manifest startup
  non-fetch behavior are covered.
- E2E/VM: covered for the implemented S03 runtime slice by
  `tests/capsem-service/test_svc_service_settings_runtime.py`. The test boots a
  VM without a CLI `--assets-dir`, so asset lookup must come from
  `service.toml`; it also proves UDS + gateway `/setup/assets` provenance and
  malformed `service.toml` startup failure. The local verification used
  `CAPSEM_ASSETS_DIR=/Users/elie/git/capsem/assets`; the test skips when VM
  assets are unavailable.
- Telemetry: observability and remote policy fields parse and validate at the
  model layer.
- Performance: settings load is cheap and deterministic.
- Missing/deferred: deleting old service/process/frontend v1 settings callers is
  tracked in S01.
