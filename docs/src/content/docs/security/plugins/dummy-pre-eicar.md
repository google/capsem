---
title: Dummy Pre EICAR Plugin
description: Debug security plugin for exercising preprocess detection and absolute block behavior.
---

Plugin id: `dummy_pre_eicar`

Stage: preprocess. Plugin mode may request `rewrite`, `ask`, `allow`, `block`,
or disabled behavior according to the profile/corp plugin config.

Config:

```toml
[plugins.dummy_pre_eicar]
mode = "rewrite"
detection_level = "critical"
```

Inputs: `SecurityEvent` file, HTTP, or model text fields.

Mutation: scans event text for the harmless EICAR test string and requests `block` when found.

Decision: an EICAR match requests `block`; plugin policy can also request `allow`, `ask`, `block`, or `rewrite`. The effective decision uses the absolute lattice `allow < ask < block`.

Detection contract: enabled executions append one plugin detection record to `SecurityEvent.detections`. Matching rules with `detection_level` append their own rule detection records before plugin execution.

Failure: no external I/O; failures should only come from plugin descriptor or
profile/corp plugin config errors.

Tests: `builtin_dummy_plugins_block_eicar_and_cannot_be_downgraded_by_postprocess`.
