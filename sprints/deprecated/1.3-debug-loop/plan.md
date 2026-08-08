# Sprint: 1.3 Debug Loop

## Purpose

Capture and execute the late-release bug loop without losing the current
runtime evidence. These bugs are discovered from a live installed Capsem build
and must be handled TDD-style when implementation resumes.

## Ground Rules

- Do not kill, purge, reinstall, or restart the current working VM unless the
  user explicitly clears that action.
- Treat the current VM as evidence. Prefer source inspection, logs, status
  endpoints, and non-destructive commands.
- Add failing tests before changing implementation.
- Keep each bug independently reproducible and independently commit-worthy.

## Bugs Captured

1. VM lifecycle/status actions: `capsem` and the TUI must reflect each VM state
   correctly, never offer resume/start for non-resumable VMs, and purge must
   delete defunct VMs.
2. AGY guest experience and observability:
   - AGY works after OAuth, but the profile should provide an alias/wrapper
     that launches AGY with the required dangerous-permission allowance.
   - AGY activity is not visible in stats: no model activity, tool calls, or
     related security-event evidence appears while AGY is used.

## Done Means

- Each captured bug has a root-cause note, failing test, implementation patch,
  and verification result.
- Live-VM evidence is preserved until the user approves destructive actions.
- Stats/security-event fixes prove AGY activity through the same ledger-backed
  path used by other agents.
