# T0: Contract and Trace

## Purpose

Before implementation, prove we understand the current install system and
freeze the replacement contract. This prevents another round of patching a
race with another race.

## Current Flow to Trace

### macOS `just install`

```text
_stamp-version
_pack-initrd
cargo build --release
pnpm build
cargo tauri build --bundles app
scripts/build-pkg.sh
open -W packages/Capsem-<version>.pkg
scripts/sync-dev-assets.sh ~/.capsem/assets
capsem start
capsem status
open /Applications/Capsem.app
```

Problem: the sync happens after the package installer and after postinstall can
start the service/open the app.

### macOS `.pkg` Payload

Current payload contains:

- `Capsem.app`
- companion binaries under `/usr/local/share/capsem/bin`
- `manifest.json`
- entitlements

Current payload does not contain hash-prefixed VM assets.

### macOS Postinstall

Current script:

- finds installing user
- creates `~/.capsem`
- copies binaries
- codesigns binaries
- copies packaged assets
- runs `capsem install`
- runs `capsem setup`
- waits for service/gateway
- opens app

Problem: `capsem setup` is stateful onboarding, not an idempotent install
finalizer.

### Linux `.deb` Postinst

Current script:

- creates `~/.capsem`
- symlinks binaries
- runs `capsem install`
- runs `capsem setup`

Problem: it shares the same setup dependency and lacks the new readiness
contract.

## Replacement Contract

### Package Install

Package install must be deterministic:

```text
install package payload
register service
start service
wait for service/gateway
exit success
```

If service/gateway does not become reachable, package install fails with an
actionable error.

### Asset State

Asset state is recoverable and reported through daemon/API/CLI. Missing assets
must not be expressed as "setup incomplete".

### UI Launch

UI launch requires service/gateway. UI launch does not require assets ready.

### Dev Install

Local dev install must use the normal package path. The package must either:

- include current host arch hash-prefixed assets, or
- include a local asset source that the daemon can fetch before sessions are
  created.

No `just install` step may mutate `~/.capsem` after package install completes.

## Open Decision

Prefer bundling current host arch assets into local dev `.pkg` for this sprint:

- It proves the package path itself.
- It avoids a local HTTP server or release URL special case.
- It matches the user need: manually test UI/terminal from the just-built tree.

Release `.pkg` can stay manifest-only if T2 makes asset download/status first
class.

## T0 Exit Criteria

- The chosen dev asset policy is marked in `tracker.md`.
- The flow above is updated if code inspection finds drift.
- T1 implementation starts only after the replacement contract is agreed.
