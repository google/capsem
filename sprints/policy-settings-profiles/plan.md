# Policy, Settings, Profiles Meta Plan

## What We Are Building

A replacement configuration system where service settings and VM/session profiles
are separate typed TOML-backed objects. Profiles become first-class objects used
to launch sessions and materialize VM-effective settings. The old v1 settings
and policy stack is removed rather than migrated.

## Key Decisions

- No v1 compatibility, no v1 migration, no v1 special diagnostics.
- Service settings are service/app-scoped.
- Profiles are VM/session-scoped.
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
9. Add HTTP API.
10. Add CLI.
11. Add credential brokerage, status/debug, observability, remote policy.
12. Build reusable rule/settings UI components.
13. Build settings/profile/security UI.
14. Update public docs/site architecture, security, and configuration pages.
15. Run full verification and install/release gate.

## Done Definition

- `config/defaults.json` is not interpreted as runtime or UI authority.
- Typed TOML-backed service settings and profiles are validated by Rust code.
- Profile CRUD, resolution, and VM-effective settings work over UDS, HTTP, and
  CLI.
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
- Functional: UDS, HTTP, CLI service settings/profile/MCP/skills flows.
- Adversarial: malformed TOML, unknown fields, duplicate ids, locked mutations,
  forbidden user profiles, invalid rules, bad connector references.
- E2E/VM: create/fork/delete/select/launch profile and verify enforcement.
- Telemetry: observability plugin, audit events, credential brokerage,
  debug/status provenance.
- Performance: profile discovery/assembly cost, remote policy timeout behavior,
  observability batching overhead.
- Documentation/site: docs build, snippets match shipped TOML/API/CLI, and old
  v1 terminology is removed.
- Missing/deferred: none accepted at final release gate; each sprint may carry
  explicit temporary debt in `tracker.md`.
