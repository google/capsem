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
- S08a decides the policy-rule versus detection-rule abstraction, the real CEL
  enforcement runtime, the real Sigma-compatible detection path, and
  profile-owned rule-pack semantics before telemetry, plugins, rule UI, Confirm
  UX, or docs harden around it.
- S08b separates Network Engine, File Engine, Process Engine, Security Engine,
  and Resolved Event Emitter contracts before CLI/UI/status/telemetry/plugin
  surfaces consume normalized activity events.
- `session.db` must have one canonical resolved-event journal. Existing
  domain-specific tables can remain as emitter-written projections/read models,
  but they are not the source of security truth after S08b.
- Everyday agent work needs a first-class Conversation Engine feeding the
  unified timeline. Codex/Claude SDK-backed sessions and terminal fallback
  sessions must produce reviewable/searchable structured timeline elements
  linked to resolved security events.
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
7. Assemble profiles into VM-effective settings/resolver cutover (S06), then
   ablate the remaining legacy `NetworkPolicy` runtime (S06c) so runtime
   enforcement is Profile V2-only. Run S06d to split oversized modules/tests
   inside `capsem-core` before final cleanup/renaming. Do not create new engine
   crates until S08b defines their contracts.
8. Add UDS API.
9. Add signed profile-catalog manifest, package/tool contracts,
   profile-owned assets, first-use download, cleanup retention, and VM
   profile/revision/asset pinning.
10. Add service-settings schema/admin contract, then `capsem-admin` tooling for
    profile creation, profile-derived image builds, image verification,
    manifest generate/check/sign, bootstrap install, and release packaging.
11. Add HTTP API.
12. Decide policy/detection abstraction, real CEL, real Sigma-compatible
    detection, and profile-owned rule-pack semantics in S08a.
13. Split Network Engine, File Engine, Process Engine, Security Engine, and
    Resolved Event Emitter contracts/crates in S08b, including file writes/
    deletes/snapshots/restores plus process/audit attribution as normalized
    security events. Add the canonical `session.db` resolved-event journal and
    migrate existing event-family writes behind the emitter. Add Conversation
    Engine capture and the structured `/timeline/{id}` read API as part of the
    same session DB story.
14. Add shared rule/evidence corpus parity in S08c, then S08d security-engine
    performance benchmarks that measure real VM-originated allow/block/ask/
    detect latency, CEL/Sigma matching speed, rule-count scaling, and
    backtest/hunt scan rates before public speed claims.
15. Add CLI.
16. Add credential brokerage, status/debug, observability, remote enforcement.
17. Build reusable rule/settings UI components.
18. Build settings/profile/security UI.
19. Build unified timeline and agent workbench UI for SDK-backed and
    terminal-fallback sessions.
20. Update public docs/site architecture, security, and configuration pages.
21. Refresh the marketing landing page around the four-pillar product story,
    using S08d benchmark artifacts for any security-engine speed claims.
22. Run full verification and install/release gate.

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
  generate/check/sign. After S08a, it also validates/schema-exports policy and
  detection packs through typed Pydantic models. Release image builds fail if
  package/tool/image settings are read from hand-edited image config instead of
  profiles.
- Security-engine performance claims are backed by S08d benchmark artifacts that
  include real VM-originated allow/block/detect paths, correctness assertions,
  percentile latency, and rule/detection-pack scale context.
- Status/debug/CLI/UI expose installed profile revision, package/tool contract,
  asset readiness, and VM profile/revision/asset pins.
- Network, DNS, MCP/model, file, process, snapshot, VM lifecycle, and profile
  activity flow through normalized security events with a complete resolved
  journal before telemetry/audit/logging/detection export.
- `session.db` stores canonical resolved security events, ordered event steps,
  detection findings, and event correlation links. Existing net/DNS/MCP/model/
  fs/snapshot/exec/audit/policy-hook tables are projections/read models unless
  explicitly retired.
- Timeline threads, structured elements, artifacts, and search indexes are
  first-class session DB read models linked to canonical security events. Raw
  PTY logs are forensic artifacts/fallback inputs, not the user-facing
  timeline.
- Enforcement policy rules use a real CEL implementation; the current
  Capsem-only CEL-like evaluator is not a release contract.
- Detection rules are profile-owned and use the S08a-approved real
  Sigma-compatible path; detection is not just ad hoc telemetry querying.
- VM status health is live and typed: running VMs report model call count,
  provider/model usage, token counts, estimated cost, and detection finding
  health from the S12 accumulator. Persistent VMs recompute/seed from
  `session.db` once at process load; status/list/metrics fan-out never reads
  SQLite.
- File writes, deletes, snapshots, restores, quarantine, and observe-only file
  activity are owned by the File Engine and represented in the same Security
  Engine pipeline as network activity.
- Exec chains, audit-derived process events, process lineage, and
  process-to-file/network attribution are owned by the Process Engine and can
  enrich File Engine and Network Engine events without merging those engines.
- Resolver uses explicit parent inheritance with deterministic layer application
  and corp lock/forbid enforcement.
- Resolver emits auditable per-path override traces alongside effective settings.
- Model request rewrite rules can rewrite `request.body` (not fail as
  unsupported).
- MCP and skills list/add/delete/show are available through the new model.
- Status and debug report explain active settings/profile/rule provenance.
- UI uses reusable typed controls and rule builder components, plus a unified
  timeline workbench for reviewing everyday agent work.
- Docs explain the settings/profile engine, corporate profile governance,
  custom profiles, enforcement, detection format, VM health/OTel metrics,
  telemetry, remote policy, custom images/rootfs dependencies, and debug-report
  provenance.
- Marketing site explains the product in four concrete sections: Ship Fast With
  AI, Ship Safely, Scale Your Productivity Without Drag, and Enterprise Ready,
  with feature claims tied to shipped scope or clearly tracked roadmap work.
- E2E proves a session launched with a profile enforces VM-effective settings.
- Fresh install still works after v1 removal.

## Coverage Matrix

- Unit/contract: typed parsing, validation, profile discovery, precedence,
  descriptors, derived rules.
- Functional: UDS, HTTP, CLI service settings/profile/MCP/skills flows;
  manifest-driven profile install/update/remove/revoke; profile-backed VM
  create with first-use asset download; `capsem-admin` profile/image/manifest
  workflows from bootstrap-installed and packaged layouts; paginated
  `/timeline/{id}` review/search workflows over typed timeline blocks.
- Adversarial: malformed TOML, unknown fields, duplicate ids, locked mutations,
  forbidden user profiles, invalid rules, bad connector references, revoked
  profile revisions, bad profile/asset signatures, asset hash mismatch, bad
  admin-tool URL schemes, HTTP HEAD mismatch, and attempted hand-edited image
  settings usage.
- E2E/VM: create/fork/delete/select/launch profile and verify enforcement;
  create a VM from a profile revision with missing assets and prove verified
  first-use download before boot; verify a profile-derived image boots with the
  declared package/tool versions; run an SDK-backed or terminal-fallback agent
  workflow and verify timeline/event linkage.
- Telemetry: observability plugin, audit events, credential brokerage,
  debug/status provenance, profile catalog status, package contract, asset
  readiness, VM pin drift/revocation warnings, resolved-event emitter delivery,
  detection findings attached before sink fan-out, and live VM health counters
  for model provider/model/cost usage.
- Performance: profile discovery/assembly cost, remote policy timeout behavior,
  observability batching overhead, security-event engine overhead, network
  streaming chunk overhead, file-engine snapshot/hash cost, and emitter
  backpressure.
- Documentation/site: docs build, snippets match shipped TOML/API/CLI, and old
  v1 terminology is removed.
- Marketing/site: landing page build passes; responsive screenshots show the
  four pillars clearly; copy audit removes v1/defaults-json/hand-edited-image
  language and unsupported claims.
- Missing/deferred: none accepted at final release gate; each sprint may carry
  explicit temporary debt in `tracker.md`.
