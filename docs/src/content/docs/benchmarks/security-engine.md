---
title: Security Engine Methodology
description: How Capsem measures CEL, Sigma, enforcement, detection, and VM-originated policy latency.
sidebar:
  order: 2
---

Security Engine performance claims must cite recorded benchmark artifacts. Do
not use host microbenchmarks as end-to-end latency claims, and do not use
VM-originated latency numbers as proof of CEL expression speed.

## Benchmark Lanes

| Lane | Command | Proves |
|---|---|---|
| CEL microbench | `cargo bench -p capsem-security-engine --bench security_engine_cel` | compile/evaluate cost, rule-count scaling, policy context projection, dedupe cost. |
| Detection pack microbench | `cargo bench -p capsem-core --bench security_packs` | Detection IR parse/lowering and pack compile cost. |
| VM-originated serial path | `uv run pytest tests/capsem-serial/test_security_engine_benchmark.py -xvs` | real service + VM + transport + telemetry/log/status path. |
| Full bench gate | `just bench` | in-VM bench suite, lifecycle/fork, and Security Engine benchmark lanes. |

## What To Record

Every artifact must name:

- Capsem version;
- host OS and architecture;
- profile id/revision;
- VM id/session id when VM-originated;
- rule pack size;
- event family and event type;
- decision type: allow, ask, block, rewrite, detect;
- latency percentiles or Criterion slope;
- artifact path under `benchmarks/security-engine/`.

## VM-Originated Path

The VM-originated benchmarks send real events through the same path operators
use:

```text
guest workload
  -> Network/File/Process/MCP transport
  -> SecurityEvent
  -> Security Engine
  -> resolved event emitter
  -> session.db projections
  -> logs/status/debug counters
```

The benchmark must assert correctness before recording speed:

- the workload was blocked/allowed/detected as expected;
- runtime match counters changed;
- `security_events` rows exist with VM/profile/user/rule attribution;
- domain projection rows such as `net_events`, `dns_events`, or `mcp_calls`
  carry matching decision fields when applicable;
- `capsem logs` exposes enough context to debug the event.

## Current Artifact Families

The S08d artifact set currently covers:

- CEL compile/evaluate microbenchmarks;
- Detection IR parse/lowering microbenchmarks;
- process exec enforcement from a live VM;
- HTTP request enforcement from a live VM;
- DNS request enforcement from a live VM;
- framed MCP request enforcement from a live VM.

Model/file VM-originated benchmarks, concurrency cases, and backtest/hunt
scan-rate artifacts remain open until their S08d slices land.

## Marketing Rule

Marketing and landing-page copy can only use numbers that link to benchmark
artifacts or the [Performance Results](/benchmarks/results/) page. Acceptable
claims name the lane:

- "CEL condition evaluation measured in the host microbench harness";
- "blocked process exec measured through a live VM";
- "Detection IR lowering measured by the security-packs benchmark".

Do not write "Security Engine blocks in X ms" unless X comes from a
VM-originated artifact for that event family and includes the host/arch/profile
context.

## Interpreting Slow Paths

For HTTP, split guest wall-clock latency from `curl` phase timing when possible:

- name lookup and connect;
- TLS/MITM appconnect;
- time to first byte;
- total transfer time.

For MCP and file/process paths, separate Security Engine evaluation time from
transport, subprocess, filesystem, and logging overhead. If a regression is in
transport, fix transport; do not tune CEL to hide it.
