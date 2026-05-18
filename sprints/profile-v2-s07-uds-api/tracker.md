# Sprint: profile-v2-s07-uds-api

## Tasks

- [x] Add red proto/IPC contract tests.
- [x] Implement metrics snapshot structs.
- [x] Add service/process IPC variants.
- [x] Handle metrics snapshot request in capsem-process.
- [x] Run focused verification.
- [x] Update S07/Profile V2 trackers and changelog.
- [x] Commit first S07 slice.
- [x] Add read-only profile route red tests.
- [x] Implement profile list/get/effective handlers and routes.
- [x] Run focused service verification.
- [x] Update S07 trackers and changelog for read-only profile routes.
- [x] Commit read-only profile route slice.
- [x] Add profile mutation route red tests.
- [x] Implement create/fork/update/delete profile handlers and routes.
- [x] Run focused mutation verification.
- [x] Update S07 trackers and changelog for profile mutations.
- [x] Commit profile mutation route slice.
- [x] Add rules API read/evaluate red tests.
- [x] Implement rules list/get/evaluate handlers and routes.
- [x] Run focused rules API verification.
- [x] Update S07 trackers and changelog for rules read/evaluate.
- [x] Commit rules read/evaluate slice.

## Notes

- Started after S06 cleanup/hardening commit `8f19deda`.
- Scope is intentionally the proto foundation called out by S07 so S12 can
  build on stable types.
- Current slice narrows the Rules API to list/get/evaluate first. Rule
  add/delete stay open for the next S07 rules slice so evaluator semantics and
  response provenance are locked before mutations.

## Coverage Ledger

- Unit/contract:
  RED proof `cargo test -p capsem-proto metrics_snapshot_ipc_roundtrip_bincode -- --nocapture`
  failed on missing `capsem_proto::metrics` and IPC variants.
  RED proof `cargo test -p capsem-service --bin capsem-service handle_list_profiles_returns_catalog_with_default_profile -- --nocapture`
  failed on missing read-only profile handlers.
  RED proof `cargo test -p capsem-service --bin capsem-service handle_create_profile_persists_user_profile -- --nocapture`
  failed on missing profile mutation handlers and fork request type.
  RED proof `cargo test -p capsem-service --bin capsem-service handle_list_rules_returns_effective_rules_with_canonical_ids -- --nocapture`
  failed on missing Rules API query/request structs and handlers.
- Functional:
  `cargo test -p capsem-process ipc -- --nocapture` passed 18 focused process
  IPC tests, including the process-owned default metrics snapshot.
  `cargo test -p capsem-service --bin capsem-service profile -- --nocapture`
  passed 11 focused service profile tests, including list/get/resolve and
  create/fork/update/delete handlers.
  `cargo test -p capsem-service --bin capsem-service rule -- --nocapture`
  passed 8 focused service rules/settings tests, including canonical rule list,
  single-rule provenance lookup, and dry-run V2 evaluation returning the matched
  canonical id.
- Adversarial:
  `cargo test -p capsem-proto ipc -- --nocapture` passed 36 focused proto IPC
  tests, including the real bincode wire-format metrics snapshot roundtrip.
  `handle_get_profile_returns_not_found_for_unknown_profile` covers typed 404
  behavior for unknown profile ids.
  `handle_update_profile_rejects_path_body_id_mismatch` and
  `handle_delete_profile_rejects_locked_builtin_profile` cover route/body
  mismatch and locked-profile mutation failures.
  `handle_evaluate_rule_rejects_unknown_callback` covers unsupported dry-run
  callback rejection so `POST /rules/evaluate` fails closed instead of silently
  using a non-runtime callback alias.
- E2E/VM: not required for this proto foundation slice.
- Telemetry: no live accumulator yet; S12 owns runtime counters.
- Performance:
  RED proof `cargo test -p capsem-process classify_get_metrics_snapshot -- --nocapture`
  failed on missing read-only metrics IPC classification.
  GREEN proof is included in the process IPC suite; the request is classified as
  `HealthCheck`, not job/lifecycle mutation.
- Missing/deferred: Rules API create/delete, confirm pending listing, skills
  routes, gateway mirror, Python/VM route proof, and live metrics accumulator
  remain open S07/S08/S12 work.
