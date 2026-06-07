---
title: Corporate Security
description: Enterprise entry point for profile governance, enforcement, detection, telemetry, and audit.
sidebar:
  order: 5
---

Corporate security teams govern Capsem through signed profiles, enforcement
packs, detection packs, telemetry configuration, and runtime evidence.

## What To Configure

| Area | Where |
|---|---|
| Profile governance | [Corporate Deployment](/configuration/corporate-deployment/) |
| Profile format and pins | [Profile Format](/configuration/profiles/) |
| Signed catalog rollout | [Profile Catalogs](/configuration/profile-catalogs/) |
| Realtime blocking | [Enforcement](/security/enforcement/) |
| Detection and forensic search | [Detection Format](/security/detection/) |
| VM health and metrics | [VM Health](/observability/vm-health/) |
| Telemetry extension rules | [Extending Telemetry](/observability/extending-telemetry/) |
| Admin CLI workflows | [capsem-admin](/configuration/capsem-admin/) |

## Enforcement Versus Detection

Enforcement is synchronous and can allow, block, ask, or rewrite. Detection is
finding generation and forensic analysis. Detection findings are attached to
the resolved event before telemetry/logging/export sinks, but they do not
silently become blocking decisions.

Runtime operators can validate, compile, backtest, install, list, delete, and
inspect stats through `/enforcement/*` and `/detection/*`. Corp admins can
validate and backtest packs offline with `capsem-admin` before publishing them
through signed profiles.

## Evidence

Backtest and hunt return aggregate counts plus up to 100 matched event rows by
default. Rows are deduplicated by evidence signature to show diversity. Local
evidence is full-fidelity for users who can access Capsem. Export/support
bundle redaction is an explicit separate flow.

