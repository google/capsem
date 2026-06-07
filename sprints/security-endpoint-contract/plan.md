# Security Endpoint Contract Sprint

## Goal

Make the security API explicit and auditable. The gateway must not use a
catch-all service proxy, and security endpoint JSON must be backed by the same
runtime/ledger structs used by the security engine and session database.

## Tasks

- Replace the gateway catch-all proxy with an explicit service route table.
- Add tests proving unknown gateway paths are not forwarded.
- Keep `/security/{id}/latest` and `/security/{id}/info` forwarded through
  explicit routes only.
- Follow with a serializable `SecurityEvent` wire contract and evaluate/rule
  mutation endpoints.
- Add first-party decision state to `SecurityEvent`; preprocess plugins,
  CEL rules, postprocess plugins, and ask resolution all consume and return
  the same event object.
- Add real debug plugins that exercise both sides of the pipeline:
  `dummy_pre_*` plugins for preprocess mutation/decision and `dummy_post_*`
  plugins for postprocess mutation/decision. Use the EICAR harmless antivirus
  test string as the malware seed for block-path tests.
- Add profile/corp plugin policy:
  `[plugins.dummy_pre_eicar] mode = "rewrite" detection_level = "critical"`
  with allowed modes `block | ask | allow | rewrite | disable`. The engine
  must consult the merged plugin policy before running any plugin.
- Add `rewrite` as the canonical mutation action. `redact`, `mutate`, and
  `neutralize` are accepted authoring aliases; logs, DB rows, and APIs expose
  canonical `rewrite`.
- Add one durable plugin man page per plugin. Each page documents purpose,
  stage, inputs, mutations, decision requests, config keys, logging fields,
  failure behavior, and test coverage.
- Enforce an absolute block invariant: once any stage requests/effective
  decision is `block`, no later stage can downgrade it.
- Record stage transitions as ledger data so final decision, previous decision,
  requested decision, effective decision, actor, rule/plugin id, and reason are
  reconstructable from `session.db`.
- Carry detection reporting on the `SecurityEvent` itself. Rules with
  `detection_level` and enabled plugin executions append
  `SecurityDetectionEvent` records into `SecurityEvent.detections`; reporting
  and logging consume that vector instead of a second detection path.

## Done

- No `.fallback(proxy::handle_proxy)` remains in gateway runtime or tests.
- Unknown paths return gateway 404 without touching the service UDS.
- Security routes are visibly declared in the gateway route table.
- Follow-up endpoint contract work has focused tests, including the typed
  `SerializableSecurityEvent` wire DTO, evaluate/latest/info route proof, and
  add/delete/reload rule-management endpoints.
- Plugin/rule decision behavior is one pipeline: no hidden plugin verdict path,
  no side-channel decision return, and no block downgrade path.
- Plugin execution is configurable by profile/corp policy and fully testable:
  enabled plugins can force `allow`, `ask`, `block`, or `rewrite`; disabled
  plugins do not execute and produce a clear decision/log story.
- The decision ledger uses explicit decision fields, not `outcome`: every row
  records what a stage wanted and what actually became effective.
- Runtime enforcement/detection/plugin endpoints are explicit:
  `/enforcements/evaluate`, `POST|DELETE /enforcements/rules/{rule_id}`,
  `/enforcements/reload`, table-backed latest/info aliases, and plugin
  global/per-VM controls.
