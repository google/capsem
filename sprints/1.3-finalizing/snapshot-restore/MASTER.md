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
