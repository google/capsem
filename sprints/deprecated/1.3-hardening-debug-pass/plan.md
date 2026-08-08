# Sprint: 1.3 Hardening Debug Pass

## Goal

Prepare one high-signal AGY/manual validation loop by hardening the remaining
trust boundaries and improving debug evidence before asking for another
install/OAuth run.

This sprint covers four coupled bugs:

- MCP aggregator must fail loud when the subprocess binary is missing.
- Raw guest VSOCK access must not bypass audited service entry points.
- Unknown-domain AI/model traffic must be detected from bounded protocol shape,
  not only canonical hostnames.
- Credential broker reuse must be visible, profile/VM scoped, and logged well
  enough to debug capture and replay without exposing secrets.

## Contracts

- No fallback success for missing security/runtime components.
- No compatibility rail for retired Policy V2, MCP decision providers, or
  shortcut rules.
- No local one-off rate limiting; rate limiting belongs to the security rail.
- Debug evidence must be structured and route-backed.
- Do not kill, purge, reinstall, or mutate the current evidence VM unless the
  user explicitly approves it.

## Slices

### T0 Debug Evidence

Add or extend debug/status surfaces so a single report can show:

- service version and route surface,
- active profiles and asset status,
- VM state and resume blockers,
- plugin/broker inventory,
- recent model/MCP/security events,
- relevant structured log paths and snippets,
- explicit degraded components such as missing aggregator.

### T1 Aggregator Fail-Loud

Replace the empty MCP aggregator stub with an explicit degraded/error state.
Missing `capsem-mcp-aggregator` must be visible in process/service debug output
and MCP routes must not look like "no tools".

### T2 Unknown-Domain AI Sniffing

Add bounded request/response protocol-shape detection for OpenAI, Anthropic,
Google/Gemini, AGY, and custom compatible gateways. Same event must carry both
`http.host` and `model.provider`.

### T3 Broker Reuse/Replay Evidence

Expose broker inventory/grant/reuse state per profile/VM and log capture/replay
decisions. This sprint may add scaffolding and evidence routes; replay must not
ship as an invisible shortcut.

### T4 Raw VSOCK Boundary

Inventory host VSOCK listeners, document the allowed guest/host VSOCK contract,
and add tests proving raw guest access cannot bypass audited service routes.

## Verification Matrix

- Unit/contract: missing aggregator fails loud; AI protocol sniffing uses
  bounded previews; broker inventory/reuse state serializes; VSOCK allowlist
  rejects unknown listeners.
- Functional: route/debug outputs expose degraded components, broker state, and
  recent security/model/MCP events.
- Adversarial: missing aggregator binary, malformed model bodies, oversized
  request/response bodies, unknown VSOCK service, denied broker grant.
- E2E/VM: one final AGY loop after implementation, preserving the existing
  evidence VM until approval.
- Telemetry: structured log fields for aggregator, broker capture/replay,
  model sniffing, VSOCK rejection.
- Performance: sniffing remains bounded; broker/debug surfaces avoid hot-path DB
  reads outside explicit debug/status calls.

## Done

- Tests fail before code for each changed behavior.
- Tests pass after implementation.
- Tracker and changelog updated.
- Branch committed at logical milestones.
- User is pinged when ready for the integrated AGY/manual loop.
