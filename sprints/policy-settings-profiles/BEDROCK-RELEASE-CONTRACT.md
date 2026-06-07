# Profile V2 Bedrock Release Contract

## Purpose

This document is the sharp split between the rescue sprint and the improvement
era.

The Profile V2 bedrock release must ship a working, usable, documented engine
and profile system. Later work may extend it, but the core terms must stand.

## Must Stand In This Release

- **Profiles:** signed catalog, profile revisions/status, profile-owned package
  and asset contracts, profile-owned enforcement/detection packs, VM profile
  pins, and forward-only VM identity.
- **Network Engine:** HTTP, DNS, MCP, and model transport mechanics parse,
  transmit, and apply typed Security Engine responses. Transport code does not
  decide policy.
- **File Engine:** file IPC, MCP file-tool operations, filesystem observation,
  snapshots, restore/revert, quarantine, and observe-only file behavior emit
  normalized file/snapshot security events.
- **Process Engine:** exec, audit, parent/child process identity, command
  attribution, and process-to-file/network links emit normalized process
  security events.
- **Security Engine:** preprocessors, CEL enforcement, ask/confirm lifecycle,
  detection before sinks, postprocessors, runtime registries, backtest/hunt,
  match counters, decisions, declarative mutations, and final action projection.
- **Resolved Event Emitter:** canonical journal first, then logs, telemetry,
  detection export, timeline/domain projections, and debug/status read models.
- **Rule Authoring:** canonical typed roots from `capsem-proto` such as `http`,
  `dns`, `mcp`, `model`, `file`, `process`, `profile`, and `common`.
  Authored `event.*` remains rejected.
- **Runtime Routes:** UDS/HTTP `/enforcement/*` and `/detection/*` validate,
  compile, backtest, live list/add/update/delete, stats, and detection hunt.
- **CLI:** operators can use the shipped contract without raw HTTP/UDS/SQL.
- **UI:** operators can select profiles, create profile-backed VMs, inspect VM
  profile state, and operate runtime enforcement/detection overlays.
- **Docs:** operators and corp admins can understand and use the bedrock without
  reading Rust code.
- **Release Gate:** S18 proves install, VM boot, profile pins, CLI, UI, logs,
  status/debug, enforcement/detection, docs, and benchmark claims together.

## Explicitly Split Out

- Credential brokerage is S10.
- Remote enforcement/observer plugins are S13.
- Rich workbench and security UI polish are S16a/S17.
- Marketing refresh is S19a.
- Reporting setup is S19b.
- OpenAPI-to-MCP is S20.
- Local LLM support is S21.
- Rate limits, budgets, and quotas are S22.
- Other extension work lands in S23.

Those sprints consume the bedrock contracts. They do not rename event identity,
policy roots, engine boundaries, profile pinning, route families, or CLI/UI
semantics.

## Release Blockers

- Any shipped event family bypasses the Security Engine and Resolved Event
  Emitter.
- Any public rule surface accepts `event.*`.
- Any profile-backed VM can launch without profile/revision/package/asset pins.
- CLI or UI requires raw HTTP/UDS/SQL to operate the shipped contract.
- Docs claim credential brokerage, quotas, remote plugins, OTel polish, or
  marketing performance numbers that are not proven by landed tests/artifacts.
- `ask` is exposed as a user-facing decision without a real confirm path, or it
  silently behaves as allow.
- S18 cannot reproduce install, VM boot, logs/status/debug, runtime policy,
  profile pin, CLI, UI, and benchmark evidence.
