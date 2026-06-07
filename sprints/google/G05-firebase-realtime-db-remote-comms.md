# G05 - Firebase Realtime DB Remote Comms

## Goal

Use Firebase Realtime Database as the Google remote communications path with
explicit security and observability semantics.

## Scope

- Realtime DB channel model for remote comms.
- Auth and project selection.
- Message schema, ordering, replay, dedupe, backoff, and offline behavior.
- Remote decision and remote alert integration where applicable.
- Redaction, audit, graph, metrics, and support-bundle output.

## Acceptance Criteria

- Remote comms has a typed message/channel contract.
- Missing auth, revoked auth, wrong project, permission denied, stale channel,
  replay, duplicate, and network partition cases are tested.
- Security-sensitive remote decisions fail closed.
- Alerts and messages link to canonical event ids when they are event-driven.
