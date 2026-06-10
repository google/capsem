# 1.3 Finalizing Master

This sprint is closed on branch `release/1.3-cleanup-pr-v2`.

## Final Posture

The 1.3 finalizing work ended as a rescue and reconciliation sprint. The broad
parent checklist was intentionally superseded by `snapshot-restore/` after the
cleanup snapshot was found to have dropped real 1.2/1.3 foundations alongside
the intentionally burned old decision/setup systems.

The authoritative execution record is:

- `tracker.md` for the parent closeout ledger.
- `snapshot-restore/MASTER.md` for the restore sprint summary.
- `snapshot-restore/tracker.md` for commit-by-commit decisions, proof, and S1-S6
  implementation gates.
- `snapshot-restore/S0-loss-inventory.md` for the loss inventory.

## Workstreams

| Stream | Status | Outcome |
| --- | --- | --- |
| T0 Schema and ownership | Done | Profile/settings/corp ownership is codified and tested. Settings are UI/app preferences only; profile owns VM behavior; corp owns constraints/reporting. |
| T1 Service/gateway API | Done | Authoring routes are profile-addressed, VM routes live under `/vms`, service/global routes are runtime/ledger only, and retired/fallback routes fail closed. |
| T2 Security rail burn-down | Done | Policy-v2/domain/MCP decision rails remain burned; decisions flow through typed `SecurityEvent` + `SecurityRuleSet`/CEL; defaults are visible rules. |
| T3 Profile/settings/corp UI/API split | Done for 1.3 | Frontend/API contract work reflects settings/profile/corp separation; remaining richer UI polish is outside the 1.3 release hold. |
| T4 MCP/plugins/skills UI | Done for 1.3 | MCP mechanics are profile/server scoped; plugin config/runtime status is plugin-owned; credential broker state is opaque plugin evidence. |
| T5 VM lifecycle/assets/install | Done | Snapshot restore S1-S4 restored profile assets/pins, `capsem-admin`, profile-derived EROFS/LZ4HC builds, TUI, Linux scoped work, and install/package proof. |
| T6 Docs/changelog/skills | Done | Docs, skills, benchmark notes, and changelog were updated to current-truth 1.3 behavior. |
| T6.5 Invariant review | Done | Snapshot restore S6 reconciled the invariant sweep and fixed the real loader/gateway/test drift found during final smoke. |
| T7 Release verification | Done locally | Full local smoke, VM doctor, snapshot paths, focused tests, package build handoff, and benchmark gates are recorded. Linux runtime KVM/DAX execution remains a Linux-team/CI handoff. |

## Ground Rules Preserved

- No resurrection of policy-v2, domain policy, or MCP decision providers.
- No fallback/compatibility authoring routes.
- No settings-owned VM/security/provider/credential behavior.
- No fake credential or snapshot CEL roots.
- No manifest signing/minisign authority rail.
- No generic `rule-files` API.
- No `NetworkRouting` abstraction.
- The network engine owns mechanics; the security engine owns decisions.
- The runtime ledger remains forensic truth.

## Verification Summary

- `just smoke` passed in `214s`.
- `cargo fmt --check` passed.
- `git diff --check` passed.
- `cargo check -p capsem-admin -p capsem-core -p capsem-service
  -p capsem-gateway -p capsem-tui` passed.
- `just install` built/stamped `1.0.1780977620` and produced
  `packages/Capsem-1.0.1780977620.pkg`; macOS GUI installer click-through is a
  human handoff.
- Benchmark evidence is recorded in S4/S5 and the benchmark docs.

## Release Hold

The local 1.3 finalizing release hold was cleared before the later repository
ontology review found remaining guest/config and profile-ledger drift. Current
release work must complete `sprints/repo-ontology-cleanup/` before guest tool
config, image input, or package manifest changes are treated as release-ready.

Accepted handoff: Linux runtime KVM/DAX execution must be completed by the
Linux team or CI on Linux hardware. The Linux-team code and EROFS/LZ4HC proof
are restored; local macOS cannot execute that runtime lane.
