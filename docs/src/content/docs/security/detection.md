---
title: Detection Format
description: Profile-owned detection packs, Sigma validation, Detection IR, and fixture checks.
sidebar:
  order: 27
---

Detection packs describe findings. They do not block traffic or mutate
runtime behavior. Enforcement belongs to policy packs; detection results are
attached to resolved security events and exported through telemetry, audit
logging, and future detection sinks.

## Trust Chain

```mermaid
graph LR
  PROFILE["Signed profile"] --> PACK["Detection pack"]
  PACK --> PYSIGMA["pySigma parse and validate"]
  PYSIGMA --> IR["capsem.detection.ir.v1"]
  IR --> RUST["Rust Security Engine"]
  RUST --> FINDINGS["Detection findings"]
  FINDINGS --> SINKS["Telemetry / audit / detection export"]
```

`capsem-admin` validates the detection-pack envelope with Pydantic, validates
Sigma YAML with pySigma, and compiles the supported subset to
`capsem.detection.ir.v1`. `capsem-core` validates, parses, and evaluates that
same Detection IR artifact in Rust.

## Detection Pack

```yaml
schema: capsem.detection-pack.v1
id: corp-default-detections
version: 2026.0521.1
status: active
owner: corp
description: Default corp detections.
field_mapping:
  http:
    Host: http.request.host
sources:
  - id: metadata-access
    type: sigma
    format: yaml
    content: |
      title: Metadata endpoint access
      id: 11111111-1111-4111-8111-111111111111
      status: test
      logsource:
        product: capsem
        category: http
      detection:
        selection:
          Host: 169.254.169.254
        condition: selection
      level: high
findings:
  default_severity: high
  default_confidence: medium
  tags:
    - attack.discovery
```

| Field | Meaning |
|---|---|
| `schema` | Must be `capsem.detection-pack.v1`. |
| `id` / `version` | Pack identity pinned by the profile. |
| `status` | `active`, `deprecated`, or `revoked`. Revoked packs must not install or launch. |
| `owner` | `corp`, `vendor`, or `user`. |
| `sources` | Embedded Sigma YAML, local IR/reference payloads, or signed references. |
| `field_mapping` | Explicit Sigma-field to normalized-event-field mapping. No implicit Windows/Linux/cloud mapping is used. |
| `findings` | Default severity, confidence, tags, and export routes. |

## Compile And Backtest

```bash
capsem-admin detection validate corp-detections.yml --json
capsem-admin detection compile corp-detections.yml --out detection.ir.json --json
capsem-admin detection backtest corp-detections.yml --events policy-contexts.jsonl --json
```

`validate` proves the envelope shape. `compile` proves pySigma accepts the
Sigma YAML and the supported subset maps into Detection IR. `backtest` compiles
the pack and evaluates typed policy-context JSONL fixtures.

Example fixture line:

```json
{"schema":"capsem.policy-context-fixture.v1","event_ref":{"corpus":"corp-smoke","session_id":"session-1","event_id":"evt-1","sequence":1,"timestamp_unix_ms":1789002001},"expected_labels":["metadata-egress"],"context":{"schema_version":1,"common":{"event_type":"http.request"},"http":{"request":{"host":"169.254.169.254","body":{"state":"missing"}}}}}
```

## Supported Sigma Subset

The first supported subset is intentionally narrow:

| Supported | Rejected |
|---|---|
| `logsource.product: capsem` | Implicit mappings for external products. |
| One named selection | Compound conditions such as `selection and not filter`. |
| AND-linked fields | OR-linked selections or aggregations. |
| OR-linked exact values per field | Wildcards, placeholders, and modifiers. |
| Explicit `field_mapping` | Unmapped Sigma fields. |

Rejected constructs fail closed at compile time. This keeps detection content
portable for enterprise teams while avoiding a second, ad hoc Sigma
implementation inside Capsem.

## Detection IR

Detection IR is the runtime contract:

```json
{
  "schema": "capsem.detection.ir.v1",
  "pack_id": "corp-default-detections",
  "pack_version": "2026.0521.1",
  "pack_status": "active",
  "owner": "corp",
  "rules": [
    {
      "id": "metadata-access",
      "source_id": "metadata-access",
      "sigma_id": "11111111-1111-4111-8111-111111111111",
      "title": "Metadata endpoint access",
      "event_family": "http",
      "condition": "selection",
      "matchers": [
        {
          "field_path": "http.request.host",
          "operator": "equals_any",
          "values": ["169.254.169.254"],
          "sigma_field": "Host"
        }
      ],
      "severity": "high",
      "confidence": "medium",
      "tags": ["attack.discovery"]
    }
  ]
}
```

Schema artifact:

```text
schemas/capsem.detection.ir.v1.schema.json
```

Golden fixtures:

```text
schemas/fixtures/detection-ir-v1-valid.json
schemas/fixtures/detection-ir-v1-invalid-extra-field.json
```

The Python compiler output is compared against the golden fixture, and Rust
tests validate, parse, and evaluate that same fixture.

See [Rule Corpus Workflow](/security/rule-corpus/) for the fixture and
cross-language parity process.
