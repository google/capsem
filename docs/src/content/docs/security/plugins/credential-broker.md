---
title: Credential Broker Plugin
description: Built-in Capsem security plugin for brokered credential capture.
---

Plugin id: `credential_broker`

Stage: `preprocess`, `rewrite`, or `postprocess` when referenced by a matching security rule.

Config:

```toml
[plugins.credential_broker]
mode = "rewrite"
detection_level = "informational"
```

Inputs: credential observations already attached to the `SecurityEvent`.

Mutation: stores observed credentials through the broker and writes the brokered `credential:blake3:*` reference back onto the event.

Decision: plugin policy can request `allow`, `ask`, `block`, or `rewrite`; `rewrite` keeps the effective decision at `allow` while recording mutation intent.

Detection contract: enabled executions append one `SecurityDetectionEvent` to `SecurityEvent.detections` with `source = "plugin"`, the configured `detection_level`, plugin id, matched rule id, rule action, plugin mode, and reason.

Failure: broker storage errors abort plugin execution and the event is not emitted by the security engine.

Tests: `credential_broker_capture_action_brokers_observation_into_event_ref`, `credential_broker_plugin_uses_matched_security_rule_metadata`, and `security_engine::tests`.
