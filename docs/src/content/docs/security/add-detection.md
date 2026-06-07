---
title: Add Detection
description: Author Sigma-compatible detections, validate with capsem-admin, and hunt sessions.
sidebar:
  order: 29
---

Detection produces findings. It does not block or rewrite. Findings attach to
the resolved Security Event before telemetry, logging, and export sinks.

## Workflow

1. Choose target families and fields from the canonical policy context.
2. Author Sigma-compatible detections inside a `capsem.detection-pack.v1`
   envelope.
3. Validate with pySigma-backed `capsem-admin detection validate`.
4. Compile to `capsem.detection.ir.v1`.
5. Backtest against shared fixtures or a selected session.
6. Publish through a signed profile.
7. Verify findings in timeline/session evidence, VM health, OTel summaries,
   detection stats, and logs.

```bash
capsem-admin detection validate corp-detections.yml --json
capsem-admin detection compile corp-detections.yml --out detection.ir.json --json
capsem-admin detection backtest corp-detections.yml --events policy-contexts.jsonl --json
```

For forensic work, use Sigma against a specific timeline/session journal
without installing the detection pack live. The service route is
`POST /sessions/{id}/detection/hunt`.

