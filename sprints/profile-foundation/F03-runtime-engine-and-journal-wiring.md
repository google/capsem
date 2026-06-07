# F03 - Runtime Engine And Journal Wiring

## Goal

Make the canonical resolved security-event journal the runtime source of truth.

## Scope

- Network, file, process, model, MCP, credential, VM, profile, conversation,
  and snapshot events.
- Service/process/gateway dispatch into the Security Engine.
- Session DB `security_events`, `security_event_steps`, detection findings,
  tags, and links as canonical journal tables.
- Domain tables remain projections, not policy authority.

## Acceptance Criteria

- Each shipped event family has a runtime path into the canonical journal.
- Real VM proof shows journal rows for representative event families.
- Status/debug/logs can point to canonical event ids.
