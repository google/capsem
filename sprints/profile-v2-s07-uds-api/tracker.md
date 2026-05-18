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

## Notes

- Started after S06 cleanup/hardening commit `8f19deda`.
- Scope is intentionally the proto foundation called out by S07 so S12 can
  build on stable types.

## Coverage Ledger

- Unit/contract:
  RED proof `cargo test -p capsem-proto metrics_snapshot_ipc_roundtrip_bincode -- --nocapture`
  failed on missing `capsem_proto::metrics` and IPC variants.
  RED proof `cargo test -p capsem-service --bin capsem-service handle_list_profiles_returns_catalog_with_default_profile -- --nocapture`
  failed on missing read-only profile handlers.
- Functional:
  `cargo test -p capsem-process ipc -- --nocapture` passed 18 focused process
  IPC tests, including the process-owned default metrics snapshot.
  `cargo test -p capsem-service --bin capsem-service profile -- --nocapture`
  passed 5 focused service profile tests, including list/get/resolve handlers.
- Adversarial:
  `cargo test -p capsem-proto ipc -- --nocapture` passed 36 focused proto IPC
  tests, including the real bincode wire-format metrics snapshot roundtrip.
  `handle_get_profile_returns_not_found_for_unknown_profile` covers typed 404
  behavior for unknown profile ids.
- E2E/VM: not required for this proto foundation slice.
- Telemetry: no live accumulator yet; S12 owns runtime counters.
- Performance:
  RED proof `cargo test -p capsem-process classify_get_metrics_snapshot -- --nocapture`
  failed on missing read-only metrics IPC classification.
  GREEN proof is included in the process IPC suite; the request is classified as
  `HealthCheck`, not job/lifecycle mutation.
- Missing/deferred: profile create/fork/update/delete routes, Rules API, confirm
  pending listing, skills routes, gateway mirror, Python/VM route proof, and live
  metrics accumulator remain open S07/S08/S12 work.
