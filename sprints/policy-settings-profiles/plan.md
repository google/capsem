# Policy, Settings, Profiles Meta Plan

## What We Are Building

A replacement configuration system where service settings and VM/session profiles
are separate typed TOML-backed objects. Profiles become first-class objects used
to launch sessions, materialize VM-effective settings, declare guest
package/tool assumptions, and drive the VM assets needed to satisfy those
assumptions. The old v1 settings and policy stack is removed rather than
migrated.

## Key Decisions

- No v1 compatibility, no v1 migration, no v1 special diagnostics.
- Service settings are service/app-scoped.
- Profiles are VM/session-scoped.
- The signed manifest is the profile catalog: it lists profile ids, immutable
  revisions, status, payload location, payload signature/hash, and binary
  compatibility.
- Profiles declare package/tool versions and VM asset locations/signatures/hashes
  required by that profile revision.
- VM creation pins the selected profile id/revision plus exact asset hashes;
  profile updates do not silently mutate existing VMs.
- Corp/admin tooling is `capsem-admin`, a uv-managed Python CLI package shipped
  by bootstrap and release packages. It derives image build plans and manifests
  from profiles; hand-edited image settings are not a compatibility surface.
- UDS API lands before HTTP gateway API.
- CLI lands after the UDS and HTTP contracts are tested.
- UI lands after backend contracts and reusable components exist.
- Telemetry and remote policy plugins are service-scoped.
- Credentials may live in TOML initially; Keychain is stretch work in credential
  brokerage.
- Canonical profile rule format is `security.rules.<type>.<rule_name>` with
  profile-rule default priority `1`.
- `ask` decisions route through `confirm()` with telemetry; placeholder
  behavior may return accept until interactive confirm sprint lands.
- `model.request` rewrite support is required (dedicated sprint S06a).
- Public docs are release-blocking because the redesign changes the operating
  model for corporate deployment, security posture, settings, profiles, and
  remote policy.

## Dependencies And Ordering

1. Meta sprint setup.
2. Design service settings, then implement them.
3. Design profile contract (S04), including canonical rules + inheritance.
4. Implement canonical profile parser/model (S05).
5. Remove remaining v1 runtime/UI authority (S01) after S04+S05 checkpoints.
6. Land network/confirm/model/migration prereqs (S06-pre, S06a, S06b).
7. Assemble profiles into VM-effective settings/resolver cutover (S06).
8. Add UDS API.
9. Add signed profile-catalog manifest, package/tool contracts,
   profile-owned assets, first-use download, cleanup retention, and VM
   profile/revision/asset pinning.
10. Add `capsem-admin` tooling for profile creation, profile-derived image
    builds, image verification, manifest generate/check/sign, bootstrap install,
    and release packaging.
11. Add HTTP API.
12. Add CLI.
13. Add credential brokerage, status/debug, observability, remote policy.
14. Build reusable rule/settings UI components.
15. Build settings/profile/security UI.
16. Update public docs/site architecture, security, and configuration pages.
17. Run full verification and install/release gate.

## Done Definition

- `config/defaults.json` is not interpreted as runtime or UI authority.
- Typed TOML-backed service settings and profiles are validated by Rust code.
- Profile CRUD, resolution, and VM-effective settings work over UDS, HTTP, and
  CLI.
- Signed manifest profile catalog install/update/remove/revoke works, profiles
  expose package/tool contracts, and profile-backed VM creation downloads and
  verifies only the assets required by the selected profile revision.
- `capsem-admin` is installed by bootstrap/release packages and supports profile
  validation/creation, profile-derived image plan/build/verify, and manifest
  generate/check/sign. Release image builds fail if package/tool/image settings
  are read from hand-edited image config instead of profiles.
- Status/debug/CLI/UI expose installed profile revision, package/tool contract,
  asset readiness, and VM profile/revision/asset pins.
- Resolver uses explicit parent inheritance with deterministic layer application
  and corp lock/forbid enforcement.
- Resolver emits auditable per-path override traces alongside effective settings.
- Model request rewrite rules can rewrite `request.body` (not fail as
  unsupported).
- MCP and skills list/add/delete/show are available through the new model.
- Status and debug report explain active settings/profile/rule provenance.
- UI uses reusable typed controls and rule builder components.
- Docs explain the settings/profile engine, corporate profile governance,
  custom profiles, telemetry, remote policy, custom images/rootfs dependencies,
  and debug-report provenance.
- E2E proves a session launched with a profile enforces VM-effective settings.
- Fresh install still works after v1 removal.

## Coverage Matrix

- Unit/contract: typed parsing, validation, profile discovery, precedence,
  descriptors, derived rules.
- Functional: UDS, HTTP, CLI service settings/profile/MCP/skills flows;
  manifest-driven profile install/update/remove/revoke; profile-backed VM
  create with first-use asset download; `capsem-admin` profile/image/manifest
  workflows from bootstrap-installed and packaged layouts.
- Adversarial: malformed TOML, unknown fields, duplicate ids, locked mutations,
  forbidden user profiles, invalid rules, bad connector references, revoked
  profile revisions, bad profile/asset signatures, asset hash mismatch, bad
  admin-tool URL schemes, HTTP HEAD mismatch, and attempted hand-edited image
  settings usage.
- E2E/VM: create/fork/delete/select/launch profile and verify enforcement;
  create a VM from a profile revision with missing assets and prove verified
  first-use download before boot; verify a profile-derived image boots with the
  declared package/tool versions.
- Telemetry: observability plugin, audit events, credential brokerage,
  debug/status provenance, profile catalog status, package contract, asset
  readiness, and VM pin drift/revocation warnings.
- Performance: profile discovery/assembly cost, remote policy timeout behavior,
  observability batching overhead.
- Documentation/site: docs build, snippets match shipped TOML/API/CLI, and old
  v1 terminology is removed.
- Missing/deferred: none accepted at final release gate; each sprint may carry
  explicit temporary debt in `tracker.md`.
