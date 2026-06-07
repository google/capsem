# Snapshot Restore Master

This sub-sprint repairs the accidental blast radius from:

```text
82e7a58c chore: apply 1.3 cleanup snapshot
```

The cleanup snapshot intentionally burned old setup/policy compatibility, but
it also omitted real 1.2/1.3 foundations. This sub-sprint separates mandatory
restores from intentional burns so the 1.3 release can close on the right
architecture.

## Source Diff

Use this as the canonical loss inventory:

```text
git diff --name-status 82e7a58c^1 82e7a58c
```

Parent `82e7a58c^1` is restored main with the lost work. The merge result is
the cleanup snapshot tree.

## What Happened

During the 1.3 cleanup, we deliberately burned old decision systems: policy-v2
hooks, domain/MCP decision providers, provider onboarding/setup flows, fallback
compatibility routes, and settings-owned VM/security behavior. That part was
intentional. The desired architecture is profile-first configuration plus a
single typed security-event/CEL rule rail.

The mistake was accepting the cleanup snapshot as the final tree. That snapshot
did not only remove bad compatibility paths; it also omitted real 1.2/1.3
product foundations. The loss was not a line-by-line conflict review. It was a
tree-level omission.

The biggest accidental losses are:

- profile-owned assets and profile catalog/revision trust,
- persistent VM profile/base-asset pins,
- `capsem-admin` and the typed profile-derived asset/manifest build pipeline,
- TUI-backed `capsem shell`,
- Linux-team KVM/filesystem/EROFS/LZ4HC and benchmark proof,
- security corpus/backtest/benchmark gates that need to be ported to the new
  rule engine.

## Product Contract To Preserve

Capsem operates on independent profiles. A VM executes exactly one immutable
profile id. Settings are UI/application preferences only. Corp config owns
constraints, locks, and reporting integrations over profiles. Profile owns the
runtime behavior: assets, VM defaults, rules, detections, MCP, skills,
credential/plugin config, availability, name, description, and icon.

The runtime asset chain must be:

```text
vm.profile_id
-> load profile manifest/config
-> profile.assets selects asset release/logical assets
-> asset manifest/cache resolves hashes
-> boot uses those resolved paths
```

The profile is the root of personalization and boot truth. It is how corp/user
configuration selects different VM assets, UI behavior, MCP servers/tools,
skills, credentials/plugins, and security posture. If assets are resolved from a
service-global manifest without profile identity, the contract is broken.

## Burned On Purpose

Do not restore these as code paths:

- policy-v2 hooks,
- old domain policy/network security decision providers,
- old MCP policy/decision providers,
- old provider setup/onboarding wizard,
- `capsem setup`,
- compatibility aliases and fallback routes,
- settings-owned VM/security/provider behavior,
- multiple enforcement engines.

Why: these were the wheels we intentionally burned. Security decisions must run
through one typed security-event path and one `SecurityRuleSet`/CEL rail. The
network engine owns mechanics such as parsing, capture, DNS/proxy mechanics,
ports, caching, decompression, routing mechanics, and provider metadata. It
does not own security decisions. MCP owns server/tool/resource/prompt mechanics.
It does not own security decisions.

## Must Come Back

These are not optional:

- `capsem-admin` as the typed admin command surface.
- Profile and service-settings schemas/fixtures.
- Profile-derived image plan/verify/workspace/build commands.
- Manifest check/download-check/generate/sign/verify commands.
- `just`/CI/release using the typed admin rail instead of shell-only ad hoc
  asset builds.
- Profile catalog/loader/revision trust.
- Profile-aware asset supervisor/reconcile/status/ensure.
- Persistent VM profile/base-asset pins and fail-closed resume/fork/save.
- TUI-backed `capsem shell`.
- Linux-team scoped KVM/filesystem/EROFS/LZ4HC work and benchmark evidence.
- Detection/enforcement corpus, Sigma facade, backtests, and benchmarks ported
  to the new security rule rail.

## Gotchas

- Do not blindly cherry-pick large ranges. Port by capability into the current
  architecture.
- Do not reintroduce old policy-v2/domain/MCP decision paths while restoring
  admin security pack compile/backtest behavior.
- Do not let `settings.toml` regain ownership of profiles, assets, rules, MCP,
  skills, credentials, or VM defaults.
- Do not keep a `default`-only profile validator. Real profile ids must load
  real profile contracts.
- Do not use service-global asset status as profile asset truth. Service-global
  status may report runtime/cache health only.
- Do not invent UI copy for profile/rule/plugin names and descriptions. UI
  reflects backend/profile contracts.
- Linux-team scoped commits are authoritative. If they conflict with cleanup,
  adapt cleanup around them unless they violate the security/profile contract.
- Debug/status diagnostics are useful but lower priority than restoring the
  product contract.

## Restore Policy

- Do not restore old policy-v2/domain/MCP decision engines.
- Do not restore `capsem setup` or provider onboarding wizard behavior.
- Do not restore old standalone engine topology solely because files existed.
- Port capabilities into the current profile-first, single security-rule/CEL
  architecture.
- Linux-team scoped KVM/filesystem/EROFS/benchmark commits are authoritative in
  their files unless they directly violate the current security/profile
  contract.
- Debug/status diagnostics are useful but lower priority than the product
  contract. Restore only what is needed for install/support proof.

## Workstreams

| Stream | Status | Required Outcome |
| --- | --- | --- |
| S0 Inventory | Not Started | Every deleted cluster is classified as exact restore, conceptual port, intentional burn, or Linux handoff. |
| S1 Profile/Admin | Not Started | Profiles, schemas, `capsem-admin`, profile-derived image/manifest commands, and package proof are back. |
| S2 Runtime Assets/Pins | Not Started | `vm.profile_id -> profile assets -> asset cache/manifest -> resolved boot paths`; persistent VMs store profile/base-asset pins and fail closed. |
| S3 TUI/Shell | Not Started | `capsem shell` works through the TUI again; profile/session readiness is visible in terminal. |
| S4 Linux/KVM/Bench | Not Started | Linux-team KVM/filesystem/EROFS/LZ4HC work and benchmark harness/proof are restored or handed off explicitly. |
| S5 Security Corpus | Not Started | Detection/enforcement corpus, Sigma/pack/backtest, and benchmark gates exist on the new `SecurityRuleSet`/CEL rail. |
| S6 Docs/Verification | Not Started | Current-truth docs, changelog, tests, smoke/install, and benchmark records are updated. |

## Release Hold

1.3 is blocked until S1-S5 are complete or each remaining item is documented as
an explicit owner-accepted release blocker.
