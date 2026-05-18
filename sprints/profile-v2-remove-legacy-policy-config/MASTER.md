# Profile V2 Remove Legacy Policy Config

## Status

| Area | Status | Proof |
| --- | --- | --- |
| Legacy policy module | Done | `net::policy_config` removed from `net/mod.rs`; source guard tests assert the module and directory stay absent. |
| Runtime settings fallback | Done | VM boot/process/service setup paths now use Profile V2 effective settings or defaults, not v1 settings files. |
| Setup/install/support tooling | Done | Corp provisioning installs Profile V2 profiles; support bundle/uninstall/reinstall use `service.toml` and profile roots. |
| Tests and fixtures | Done | v1 integration fixtures/examples removed or rewritten as Profile V2 profile fixtures. |
| Full VM gate | Deferred | Focused Rust/Python proof passed; `just smoke`/`just test` still remains as the later VM matrix. |

## Verification

- `cargo check -p capsem-core -p capsem-service -p capsem-process -p capsem`
- `cargo test -p capsem-core policy_v2_ --lib -- --nocapture`
- `cargo test -p capsem-process mcp_runtime -- --nocapture`
- `cargo test -p capsem-core host_config --lib -- --nocapture`
- `cargo test -p capsem-service settings -- --nocapture`
- `cargo test -p capsem setup -- --nocapture`
- `cargo test -p capsem support_bundle -- --nocapture`
- `cargo test -p capsem uninstall -- --nocapture`
- `python3 -m py_compile scripts/integration_test.py scripts/injection_test.py`
- `cargo fmt --all --check`
- `git diff --check`

## Release Hold

Release hold remains active until the full VM/install gate runs after this
focused removal milestone.
