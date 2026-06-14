# Sprint: Security Endpoint Contract

## Tasks

- [x] T1 remove catch-all gateway proxy -- `capsem-gateway` now merges an explicit service route table; no runtime/test `.fallback(proxy::handle_proxy)` remains.
- [x] T2 prove unknown paths do not forward -- `gateway_unknown_paths_are_not_forwarded_to_service` returns gateway 404 without touching UDS; `gateway_security_routes_are_explicitly_forwarded` proves declared security routes forward.
- [x] T3 expose serializable security event wire DTO -- `capsem-core::security_engine::SerializableSecurityEvent` is the public wire shape for evaluated events; it exposes all first-party roots with null absent roots and excludes raw credential observations.
- [x] T4 add first-party decision state to `SecurityEvent` -- `SecurityEvent.decision` is first-party CEL data (`security.decision`), uses an absolute `allow < ask < block` lattice, and is serialized into forensic event JSON.
- [x] T5 add merged `[plugins.<id>]` policy for profile/corp with `block | ask | allow | rewrite | disable` plus `detection_level` -- profile/corp config parses as typed `SecurityPluginConfig`; corp overrides user; enabled plugin executions append detections to the event.
- [x] T6 add real `dummy_pre_*` and `dummy_post_*` plugins, including EICAR seed path -- `dummy_pre_eicar` and `dummy_post_allow` are built-in plugins and prove EICAR block plus postprocess downgrade resistance.
- [x] T7 add canonical `rewrite` mutation action with aliases `redact | mutate | neutralize` -- typed action parses aliases, logs/stores canonical `rewrite`, and participates in rule matching without dispatching plugins from rules.
- [x] T8 add plugin man pages for every built-in/debug plugin -- added pages for `credential_broker`, `dummy_pre_eicar`, and `dummy_post_allow`.
- [x] T9 enforce absolute block decision lattice across plugins/rules/ask resolution -- plugin policy tests prove later allow cannot downgrade block; existing ask-resolution tests prove denied ask blocks like block.
- [x] T10 log decision transition ledger rows from the same DB writer -- `SecurityDecisionEvent` is a `WriteOp`; matched rules write explicit previous/requested/effective rows and preserve block over later allow.
- [x] T11 add evaluate/latest/info/add/delete/reload rule endpoint contract tests -- evaluate/latest/info have plugin/enforcement/detection route proof; `POST|DELETE /enforcements/rules/{rule_id}` validate user profile rules through the native compiler before touching `user.toml`; `POST /enforcements/reload` aliases config reload and gateway tests prove all routes forward explicitly.
- [x] T12 add plugin/detection/enforcement route taxonomy -- `/plugins` controls plugin config globally, `/plugins/{id}` reports per-VM effective plugin config, `/enforcements/evaluate` sends a test event through the real engine, and `/detections/{id}/latest|info` plus `/enforcements/{id}/latest|info` are table-backed aliases over the ledger rows.
- [x] Changelog/docs -- `CHANGELOG.md` records the endpoint taxonomy and rule-management routes; `docs/src/content/docs/security/policy.md` documents runtime enforcement/detection/plugin endpoints.

## Coverage Ledger

- Unit/contract: `cargo test -p capsem-gateway gateway_ --no-default-features`; `cargo test -p capsem-core rewrite_is_canonical_mutation_action_with_aliases_and_requires_plugin --no-default-features`; `cargo test -p capsem-logger security_rule_events_accept_rewrite_rule_action --no-default-features`; `cargo test -p capsem-logger security_decision_events_record_explicit_decisions_and_reject_magic_outcome --no-default-features`; `cargo test -p capsem-logger security_decision_event_roundtrip_preserves_explicit_transition --no-default-features`; `cargo test -p capsem-core emit_matching_security_rules_with_decision_uses_same_evaluation_as_ledger --no-default-features`; `cargo test -p capsem-core net::policy_config::security_rule_profile::tests --no-default-features`; `cargo test -p capsem-core merged_settings_expose_typed_plugin_policy_with_corp_override --no-default-features`; `cargo test -p capsem-core security_engine::tests --no-default-features` (45 passed); `cargo test -p capsem-core --no-default-features` (1937 passed, 1 ignored before DTO; focused DTO test passed after); `cargo test -p capsem-service plugin_endpoint_matrix_dynamically_controls_enforcement_evaluation --no-default-features`; `cargo test -p capsem-service enforcement_rule_endpoints_add_delete_reload_and_reject_invalid_rules_atomically --no-default-features`; `cargo test -p capsem-service --no-default-features` (90 lib + 110 main passed); `cargo test -p capsem-gateway gateway_ --no-default-features` (2 passed); `cargo test -p capsem-gateway gateway_security_routes_are_explicitly_forwarded --no-default-features`; `git diff --check`.
- Functional: `cargo test -p capsem-gateway proxy --no-default-features`
- Adversarial: unknown gateway path returns 404; oversized forwarded body still returns 413; missing UDS still returns 502.
- Missing/deferred: no endpoint-contract release hold remains in this sprint
  tracker. Plugin decision/detection events now carry on
  `SecurityEvent`, matched rule ledger JSON is enriched with rule detections,
  and plugin control has dynamic global/per-VM endpoint proof.
- E2E/VM or integration:
- Telemetry/observability:
- Performance:
- Missing/deferred:
