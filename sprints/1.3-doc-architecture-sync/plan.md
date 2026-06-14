# 1.3 Documentation Architecture Sync

## Why

The 1.3 rescue changed the product contract: profiles own VM runtime behavior and assets, settings are UI/app preferences, corp owns constraints/reporting, security runs through one CEL `SecurityEvent` rail, plugins own side-effectful filtering/mutation, and the gateway uses explicit route allowlists. The docs and project skills must stop teaching old setup, provider, temporary/persistent split, Policy V2, or global-route mental models.

## Scope

Update public docs and internal skills that define the architecture:

- Service/API docs: profile-scoped routes, VM lifecycle, explicit gateway allowlist, status/info semantics.
- Security policy docs: current route set, plugin object responsibilities, no fake CEL roots, no plugin-invoking rules.
- Session telemetry docs: current Stats tab / Inspector behavior and ledger-backed route truth.
- Asset/profile docs: `config/` source vs `target/config/`, profiles own assets/rules/MCP/plugins, EROFS/LZ4HC rootfs contract.
- CLI/MCP/doctor docs where they still describe temporary/persistent or setup-era flows.
- Skills that future agents consult (`site-architecture`, `dev-capsem`, `dev-mcp`, `dev-testing`, `dev-session-debug`, etc.) so context does not regress.

Historical release pages are allowed to describe historical behavior. Changelog history is not rewritten except for a new Unreleased docs bullet.

## Done

- No current architecture page points users to Policy V2, old callback decision paths, setup wizard authority, settings-owned provider credentials, or global provider routes.
- Endpoint docs match `crates/capsem-gateway/src/main.rs` and `crates/capsem-service/src/main.rs` route allowlists.
- Profile/corp/settings ownership is documented consistently.
- Stats/Inspector docs reflect current `session.db` tables and VM-scoped security ledger routes.
- Internal skills use the same model as public docs.
- Docs build or, if build is blocked by existing site issues, the failure is captured in the tracker.
