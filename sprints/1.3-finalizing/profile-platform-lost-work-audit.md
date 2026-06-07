# Profile Platform Lost Work Audit

Status: release blocker. This is broader than the asset endpoint drift.

## Expected Runtime Chain

```text
vm.profile_id
-> load profile manifest/config
-> profile.assets selects asset release/logical assets
-> asset manifest/cache resolves hashes
-> boot uses those resolved paths
```

The current branch violates that chain: profile routes exist, but profile
catalog, signed profile revisions, profile asset declarations, VM pins, and
launchability are mostly gone.

## Current Code Signals

| Current file/function | Signal |
| --- | --- |
| `crates/capsem-service/src/main.rs::ServiceState` | Stores a service-global `ManifestV2` and `asset_reconcile`; no profile catalog, no asset supervisor. |
| `crates/capsem-service/src/main.rs::resolve_asset_paths` | Selects boot assets from `ManifestV2::resolve(current_version, arch, assets_dir)` or dev logical names. No `profile_id`. |
| `crates/capsem-service/src/main.rs::provision_sandbox` | Calls `self.resolve_asset_paths()` before spawn. No profile resolution, profile pin, profile-selected expected hashes, or profile asset reconcile. |
| `crates/capsem-service/src/main.rs::handle_profile_assets_status` | Validates route id, then returns service-global `asset_status_value(&state)`. |
| `crates/capsem-service/src/main.rs::validate_profile_route_id` | Accepts only `default`; independent profile catalog is not live. |
| `crates/capsem-core/src/net/policy_config/profile_contract.rs::ProfileAssetConfig` | Has only `channel/kernel/initrd/rootfs` strings. It cannot express per-arch URL/hash/signature/size/content-type assets. |
| `crates/capsem-service/src/registry.rs::PersistentVmEntry` | No `profile_id`, revision, payload hash, package hash, `SavedVmProfilePin`, or pinned base asset hashes. |
| `crates/capsem/src/client.rs::{ProvisionRequest, ProvisionResponse, SessionInfo}` | DTOs do not carry profile id/revision/status/pin/base assets. |
| current tree | `profile_manifest`, `settings_profiles`, `AssetSupervisor`, `SavedVmProfilePin`, `VmArchAssets`, `VmAssetDeclaration`, launchability, and `capsem-admin` symbols are absent or only exist in docs/history. |

## Exact Loss Mode

This was not removed by a clear, reviewed "delete capsem-admin" commit.

The current history restores old main, then applies a cleanup snapshot:

- `92fa3bd2 chore: establish true main snapshot`
- `82e7a58c chore: apply 1.3 cleanup snapshot`

`92fa3bd2` re-added a reduced `src/capsem/builder` tree from the trusted
cleanup work, but the tree omitted `src/capsem/admin/*`,
`src/capsem/builder/manifest_check.py`,
`src/capsem/builder/manifest_crypto.py`,
`src/capsem/builder/manifest_generate.py`,
`src/capsem/builder/profiles.py`,
`src/capsem/builder/service_settings.py`, and
`scripts/prepare-admin-cli.sh`.

So the loss happened as snapshot omission during history repair/cleanup, not as
an evaluated architectural decision. Treat it as release-blocking lost work.

## Lost Or Flattened Commit Clusters

Do not cherry-pick these wholesale. Use them to rebuild the current 1.3 design
without resurrecting old policy-v2 or settings-owned behavior.

### A. Signed Profile Catalog And Revision Trust

Evidence commits:

- `996de225 feat: add profile manifest catalog types`
- `d50d8a13 feat: add profile catalog lifecycle gates`
- `152c7780 feat: verify installable profile payloads`
- `237d2bbc feat: materialize verified profile payloads`
- `dd42a2d4 feat: verify profile payload signatures`
- `911d6a67 feat: fetch signed profile payloads`
- `6c398874 feat: record installed profile revisions`
- `2d2d5000 feat: pin installed profile payload identity`
- `12c7577f feat: reconcile profile catalog revisions`
- `05bac5fc feat: expose profile catalog reconciliation`
- `bceda448 feat: add profile catalog reconcile cli`
- `6250f423 feat: reconcile absent profile catalog entries`

Likely lost:

- Typed signed profile manifest with active/deprecated/revoked revisions.
- Profile payload signature verification.
- Installed profile revision records.
- Reconciliation lifecycle: install current, keep deprecated if installed,
  remove revoked/absent.
- CLI/service endpoints for catalog/revision reconciliation.
- Profile payload hash as part of runtime identity.

Current replacement is much weaker: a built-in `ProfileConfigFile::builtin_default()`
and `default`-only profile route validation.

### B. Profile-Owned Asset Resolution And Download

Evidence commits:

- `048d7cf5 feat: drive runtime assets from profiles`
- `d069710f feat: trigger profile asset reconcile from update`
- `deb1b083 refactor: remove legacy asset manifest runtime`
- `0a87e26a test: harden profile asset reconcile races`
- `7ba7161a fix: reconcile profile assets before vm create`
- `95155405 feat: expose profile asset provenance`
- `3c416735 test: chain profile asset operator flow`
- `3204f27a test: prove profile asset boot flow`

Likely lost:

- `AssetSupervisor`.
- `AssetRequirement::Profile`.
- `ProfileAssetRequirement`.
- Per-arch `VmArchAssets` and `VmAssetDeclaration`.
- Profile-selected hash-based filename resolution.
- Profile asset download with BLAKE3 verification.
- Expected kernel/initrd/rootfs hash propagation into boot.
- Per-profile asset status and provenance.
- Race tests around asset reconciliation.
- Proof that VM boot uses profile-selected assets.

Current branch has profile asset routes, but they use service-global state.

### C. Persistent VM Profile Pins And Resume/Fork Integrity

Evidence commits:

- `74c2fcfa feat: pin VM profile metadata`
- `2d7e1470 feat: derive profile asset retention roots`
- `f5a8125a feat: wire profile asset cleanup`
- `5f9ce6d7 fix: require profile pins on resume`
- `33e53d21 feat: report vm profile status`
- `1ff2fe15 fix: require profile revision pins for vm state`
- `82d45852 test: cover fork profile integrity`
- `37cb10ca fix: require profile payload hashes for vm pins`
- `2a1d079d test: prove vm fork lineage`

Likely lost:

- `SavedVmBaseAssets` and `SavedVmProfilePin`.
- VM profile pin stored in persistent registry.
- Resume/fork/save fail-closed when profile pin or asset pin is missing.
- Fork lineage checks preserving exact profile and asset identity.
- Asset cleanup retention roots from saved VM pins.
- VM profile status: current, needs update, deprecated, revoked, corrupted,
  unknown.

Current registry records only VM runtime basics and has no profile/asset truth.

### D. Profile-Aware VM Creation, Gateway, TUI, And UI

Evidence commits:

- `694aa75b feat: select profiles during vm create`
- `a4675df0 feat: start s08 gateway profile surface`
- `e3be977e feat: prove s08 profile-selected gateway create`
- `f719b3e7 fix: expose only launchable profiles`
- `584278d0 fix: port launchable profile filtering`
- `67344611 feat: create sessions with profile identity`
- `ae5e6ece feat: show vm profile state in sessions`
- `b236122c feat: show profile asset readiness in sessions`
- `d5b6e0bf feat: show profile catalog in settings`
- `7edc1f5 feat: select profiles from settings`
- `5020c1a5 feat: show profile provenance on vm provision`
- `38cc4295 feat: show profile pins in vm info`
- `9978e13b fix: wire onboarding wizard to profiles`
- `55a29727 fix: show profile asset readiness before launch`

Likely lost:

- Fresh VM create carries `profile_id`.
- Gateway forwards/returns profile identity and launchability.
- UI/TUI only offers launchable profiles.
- UI/TUI blocks corrupted profile-pin resume.
- Profile catalog/asset readiness shown before launch.
- Provision/list/info surfaces profile provenance and pinned asset hashes.

Current frontend/gateway expose profile-ish endpoints, but service returns a
single default summary and client DTOs lack profile pin/status fields.

### E. Admin Tooling, CI, And Release Asset/Profile Integration

Evidence commits:

- `d39756f3 feat: add service settings admin contract`
- `d0c1c988 feat: wire capsem-admin settings commands`
- `634b9730 feat: add capsem-admin profile validation`
- `be6909a0 feat: add profile section editability gates`
- `d2834490 feat: add capsem-admin profile init`
- `839c1114 feat: add capsem-admin settings init`
- `2fb45076 feat: add capsem-admin image plan`
- `2cc49f7a feat: add capsem-admin image verify`
- `e2946acd feat: add capsem-admin manifest fast check`
- `3e5bb3cb feat: add capsem-admin manifest download check`
- `6559bf3b feat: add capsem-admin manifest generate`
- `22016426 feat: add capsem-admin manifest crypto`
- `f856d8ac test: prove bootstrap installs capsem-admin`
- `879c9d59 test: prove packages include capsem-admin`
- `31425d04 feat: materialize profile image workspaces`
- `a02537ad feat: add profile-derived image build command`
- `5b4e4274 feat: generate profile ui base profiles`
- `fd86e8ed feat: derive built-in profiles from guest config`
- `c9fd7b4b feat: require profiles for asset builds`
- `0ffb816a feat: verify image package inventory`
- `33c83bd0 feat: verify per-arch image inventories`
- `2d02b6e0 fix: require image inventory proof`
- `7277c17b feat: generate guest image sboms`
- `f5aea0fc test: gate release image boot proof`
- `6daf264a fix: point package profiles at release assets`

Likely lost:

- `capsem-admin` CLI package:
  - `settings schema|init|validate|doctor`
  - `profile schema|init|validate|manifest`
  - `image plan|verify|workspace|build`
  - `manifest check|download-check|generate|sign|verify`
  - security pack validation/compile/backtest commands
- Profile/settings typed admin contracts:
  - `src/capsem/builder/profiles.py`
  - `src/capsem/builder/service_settings.py`
- Profile-derived image build helpers:
  - `src/capsem/builder/image_plan.py`
  - `src/capsem/builder/image_verify.py`
  - `src/capsem/builder/image_workspace.py`
- Manifest helpers:
  - `src/capsem/builder/manifest_check.py`
  - `src/capsem/builder/manifest_crypto.py`
  - `src/capsem/builder/manifest_generate.py`
  - `src/capsem/builder/manifest_version.py`
- Package/install wrapper:
  - `scripts/prepare-admin-cli.sh`
  - package tests proving `capsem-admin` is included.
- CI/release gates requiring profiles for asset builds.
- `scripts/build-assets.sh --profile <profile>` delegating kernel/rootfs build
  to `capsem-admin image build`.
- Per-arch image inventory proof.
- SBOM/image package inventory proof.
- Package profiles pointing at release assets.

Current release workflow still builds EROFS assets and `assets/manifest.json`,
but it appears disconnected from signed profile payloads and profile-owned
asset selection.

The old `scripts/build-assets.sh` contract was profile-first:

```text
scripts/build-assets.sh --profile <profile> [--assets-dir assets] [--arch ...]
-> uv run capsem-admin image build <profile> --arch <arch> --template kernel
-> uv run capsem-admin image build <profile> --arch <arch> --template rootfs
-> generate checksums/manifest for the profile-derived assets
```

The current `just build-assets` path has shell/Docker mechanics, but it is not
driven by a profile payload. That violates the release contract.

### F. Security Pack / Detection Corpus Tooling From Same Era

Evidence commits:

- `d773481f feat: validate security packs`
- `66141eee feat: compile detection packs`
- `0e1e6b1b feat: add detection ir parity`
- `80a416be feat: add admin policy compile`
- `099152a4 feat: add admin policy backtest corpus`
- `7b14ccb4 feat: add admin detection backtest corpus`
- `2bedce99 feat: seed policy context rule corpus`
- `9944c7ba feat: expand admin policy context parity`
- `391eaece fix: compile-check policy backtests before replay`
- `a12f9209 test: pin s08c detection ir drift`
- `365065c2 bench: add vm security engine benchmark`
- `9a628bf1 bench: add http security engine benchmark`
- `745938b7 bench: add dns security engine benchmark`
- `91898df5 bench: add mcp security engine benchmark`

Current status:

- Some security/CEL benchmarking and runtime rule work was rebuilt in the
  current branch, but the `capsem-admin` pack/corpus workflow appears gone.
- Need a separate check before release: make sure the new `SecurityRuleProfile`
  and Sigma facade have equivalent compile/backtest/corpus gates, without
  reintroducing old named policy runtime.

## Immediate Repair Order

1. Rebuild profile catalog/loader and route validation.
2. Rebuild profile asset declarations and profile-aware asset supervisor.
3. Rebuild VM profile/base-asset pins and fail-closed resume/fork/save.
4. Restore service/gateway/client DTOs for profile identity/status/pins.
5. Restore launchable profile filtering in UI/TUI/gateway.
6. Reconcile CI/package profile asset generation so release profiles point at
   release EROFS/lz4hc assets.
7. Restore `capsem-admin` as the typed asset/profile/security-pack command
   surface used by `just`, CI, packages, and release verification.
8. Audit admin/security-pack equivalents after the new profile rail is real.

## Do Not Restore

- old policy-v2 decision paths,
- MCP decision providers,
- network/domain security hooks,
- settings-owned VM behavior,
- global authoring routes,
- compatibility aliases,
- fallback profile behavior.

The correct fix is to rebuild these capabilities in the current profile-first,
single security-rule/CEL architecture.
