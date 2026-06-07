# S0 Loss Inventory

Status: initial evidence from the cleanup snapshot diff.

Source command:

```sh
git diff --name-status 82e7a58c^1 82e7a58c
```

Parent `82e7a58c^1` is the restored-main tree that still had the work. Commit
`82e7a58c` is the cleanup snapshot tree. This inventory is not permission to
cherry-pick the old tree. It is a map for restoring capabilities into the
current profile-first, single security-rule/CEL architecture.

## Diff Shape

Path count: 1057

| Status | Count |
|---|---:|
| Added | 111 |
| Deleted | 476 |
| Modified | 383 |
| Renamed | 87 |

Top-level clusters:

| Cluster | Count | Initial Decision |
|---|---:|---|
| `sprints/` | 292 | evidence only; restore useful release/benchmark notes, not stale plans |
| `crates/` | 288 | inspect by capability |
| `tests/` | 145 | restore/port tests that prove current contracts |
| `frontend/` | 60 | conceptual port into profile/plugin/settings contract |
| `docs/` | 60 | restore current-truth docs, burn old setup/provider docs |
| `benchmarks/` | 49 | restore current benchmark evidence/harness, burn policy-v2 framing |
| `scripts/` | 34 | restore typed admin/asset/release helpers where still valid |
| `src/` | 23 | inspect CLI/app surfaces |
| `schemas/` | 23 | restore profile/service schema contracts after reconciliation |
| `guest/` | 23 | inspect packages/config; no fake credentials |
| `data/` | 14 | port security corpus to current rule/CEL contract |
| `skills/` | 12 | restore useful dev skills/docs if current |
| `config/` | 9 | conceptual port only; current config contract is authoritative |

## Mandatory Restore / Conceptual Port

These losses map to current 1.3 contract work and must come back in the new
shape.

| Capability | Representative Lost Paths | Decision |
|---|---|---|
| Profile-owned assets/catalogs | `config/profiles/base/*.profile.toml`, `crates/capsem-core/src/profile_manifest.rs`, `crates/capsem-core/src/profile_payload_schema.rs`, `schemas/capsem.profile.v2.schema.json`, `docs/src/content/docs/configuration/profile-*` | conceptual port into `profile.toml` + signed manifest/profile asset chain |
| Asset supervisor and saved VM pins | `crates/capsem-service/src/asset_supervisor.rs`, `crates/capsem-service/src/saved_vm_assets.rs` | exact restore where compatible, then adapt to profile-first contract |
| `capsem-admin` / admin pipeline | `docs/src/content/docs/configuration/capsem-admin.md`, `docs/src/content/docs/development/capsem-admin.md`, `scripts/prepare-admin-cli.sh`, `scripts/build-assets.sh`, `scripts/prepare-install-assets.sh`, `scripts/materialize-install-profiles.py` | restore typed admin command surface; avoid shell-only release logic |
| TUI-backed shell | `crates/capsem-tui/src/*`, `crates/capsem/src/status.rs`, `crates/capsem/src/status/tests.rs` | restore functionally, preserving memory-only status hot paths |
| Linux/KVM/filesystem work | `crates/capsem-core/src/hypervisor/kvm/*`, `scripts/fix-linux-kvm-devices.sh`, KVM benchmark artifacts | Linux-team scoped work is authoritative unless it violates security/profile contract |
| EROFS/LZ4HC benchmarks | `benchmarks/*data_1.2*`, `benchmarks/security-engine/*`, `scripts/archive_*benchmark*`, `scripts/compare_benchmark_artifacts.py` | restore benchmark harness/evidence; update numbers after current run |
| Security corpus/backtests | `data/detection/*`, `data/enforcement/*`, `schemas/capsem.detection-*`, `schemas/capsem.enforcement-*`, `crates/capsem-core/tests/security_packs.rs` | port to current rule format, Sigma facade, and `SecurityRuleSet` |
| Network parser improvements | `crates/capsem-network-engine/src/*` renamed into `crates/capsem-core/src/net/parsers/*` and `ai_traffic/*` | preserve parser improvements; keep decisions out of network engine |
| Gateway diagnostics and explicit routes | `crates/capsem-gateway/src/main.rs` tests, `frontend/src/lib/__tests__/gateway-store.test.ts` | preserve explicit allowlist; extend for profile/plugin/VM routes |

## Intentional Burn

These were removed for good unless a future sprint deliberately designs a new
contract.

| Capability | Representative Lost Paths | Burn Reason |
|---|---|---|
| Policy-v2 framing | `benchmarks/policy-v2/README.md` | old policy architecture |
| Separate network decision providers | `crates/capsem-network-engine/src/domain_policy.rs`, `http_policy.rs`, `dns_security.rs`, `mcp_security.rs`, `model_security.rs` | security decisions belong to one `SecurityRuleSet`/CEL rail |
| Old standalone engine crates as topology | `crates/capsem-security-engine/*`, `crates/capsem-file-engine/*`, `crates/capsem-process-engine/*` | port concepts/tests, not separate engines |
| Setup/provider onboarding | `crates/capsem/src/setup.rs`, onboarding/provider UI tests/components | old setup wheel; provider state comes from profile/rules/runtime/plugin status |
| Settings-owned profile/security behavior | `config/user.toml.default`, old `settings.ai.*` defaults, service-settings profile roots | settings must stay UI/app preferences only |
| Credential profile API | service/gateway `/profiles/{profile_id}/credentials/*` paths found in current code | replace with plugin runtime status/stats; no AI broker |
| Fake `credential` and `snapshot` CEL roots | current `SecurityEvent`/CEL root drift found in code | burn from first-party rule roots for 1.3 |

## Needs Focused Review

These areas may contain both good work and old assumptions.

- `config/defaults.toml`: contains old `settings.ai.*` and credential injection
  blocks. Burn or reshape into profile-owned rules plus plugin runtime status.
- `crates/capsem-core/src/net/policy_config/*`: contains current rule/CEL work
  but still has stale plugin-action/provider/credential assumptions.
- `crates/capsem-core/src/security_engine/*`: contains the unified rail but
  still exposes fake `credential`/`snapshot` roots and old plugin coupling.
- `crates/capsem-service/src/main.rs`: contains useful profile/plugin route
  scaffolding and stale credential/profile fallback endpoints.
- `frontend/src/lib/components/settings/*`: likely useful UI surface, but must
  be rebuilt around profile/settings/plugin contracts and backend-owned labels.

## S0 Current Conclusions

- Restore capabilities, not ancestry.
- Profile/admin, TUI, Linux/KVM/EROFS, security corpus, and benchmark proof are
  real losses and must be restored.
- Old decision systems, setup/onboarding, settings-owned behavior, fake
  credential/snapshot roots, and fallback routes stay burned.
- Gateway explicit allowlist and memory-only VM status are release invariants.
