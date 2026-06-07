---
title: Dummy Post Allow Plugin
description: Debug security plugin for proving postprocess stages cannot downgrade a block.
---

Plugin id: `dummy_post_allow`

Stage: intended for `postprocess` rules.

Config:

```toml
[plugins.dummy_post_allow]
mode = "allow"
detection_level = "informational"
```

Inputs: any `SecurityEvent`; tests usually match on `security.decision == "block"`.

Mutation: requests `allow` and records a trace marker.

Decision: cannot downgrade an effective `block`. The decision lattice keeps the highest-severity request.

Detection contract: enabled executions append one plugin detection record to `SecurityEvent.detections`; disabled executions append none.

Failure: no external I/O; failures should only come from rule/plugin registration errors.

Tests: `security_rule_plugin_policy_block_is_absolute_after_later_allow` and `builtin_dummy_plugins_block_eicar_and_cannot_be_downgraded_by_postprocess`.
