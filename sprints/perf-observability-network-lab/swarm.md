# Swarm: Credential Broker Prior Art

## Purpose

Review `Infisical/agent-vault` for credential-broker and agent-secret patterns
that should inform Capsem's provider rules, credential broker, and
security-event pipeline. This is an investigation swarm only; do not copy code
or merge external architecture wholesale.

## Rules

- Current Capsem security event/CEL path remains authoritative.
- Findings must be conceptual ports, not bulk imports.
- Prefer patterns that preserve reference-only credential logging, BLAKE3
  substitution references, Keychain-backed storage, and first-party security
  event emission.
- Call out anything that would create a second policy engine, bypass
  security-event emission, or hide audit state.

## Status Legend

- Not launched
- In progress
- Completed
- Captured

## Finding Docs Index

| Domain | Status | Agent | Finding Doc | Sprint Targets |
| --- | --- | --- | --- | --- |
| Infisical agent-vault patterns | Captured | Erdos (`019e999f-7099-7fc2-824d-6595ee10373f`), Firecrawl (`019e99a0-30ac-7212-a4d3-81d1b362c11d`, not relied on), local repo read | `swarm-findings/agent-vault.md` | T6 provider/broker foundations, install/setup credential broker, future plugin foundations |

## Resume Protocol

1. Read this file.
2. Poll or resume any in-progress agent.
3. Capture the final answer into the finding doc.
4. Mark the index row completed/captured.
5. Update `MASTER.md` and `tracker.md` only after findings are captured.

## Required Finding Shape

- Severity-ranked findings or explicit "no blocker".
- Exact upstream repo paths/functions when known.
- Conceptual Capsem port.
- What to avoid.
- Risks and missing proof.
- Recommended implementation order.

## Completed Agents

- Erdos (`019e999f-7099-7fc2-824d-6595ee10373f`) -- superseded/captured by local clone review in `swarm-findings/agent-vault.md`.
- Firecrawl (`019e99a0-30ac-7212-a4d3-81d1b362c11d`) -- repeatedly polled and still `processing`; marked not relied on so the sprint does not depend on a stale external result.
- Local clone review -- completed against upstream revision `234dbf0d27d4749b35690c91713fd2789c810cd7`.

## Active Agents

- None.

## Launch Queue

- None.

## Intake Checklist

- [x] Agent result captured in `swarm-findings/agent-vault.md`.
- [x] Firecrawl result captured or explicitly marked redundant.
- [x] P0/P1 findings deduplicated.
- [x] Tracker updated with accepted tasks or explicit deferrals.

## Completeness Gate

This swarm is not complete until no active agents remain, the finding doc has no
`Awaiting agent output` placeholder, and any accepted pattern has an owner in the
sprint tracker.
