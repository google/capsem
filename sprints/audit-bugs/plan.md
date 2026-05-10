# Audit Bugs Plan

## Purpose

Capture the code-review findings from the broad Capsem audit as an investigation
queue. This sprint is for validating, reproducing, fixing, and testing each
issue one at a time.

## Scope

The queue covers:

- Gateway and desktop-app local trust boundaries.
- MCP server configuration precedence and aggregator isolation claims.
- Builtin MCP pooling behavior.
- Snapshot/file tool robustness.
- Failed-session cleanup observability.
- Frontend design-token drift found during the UI pass.

Out of scope for this queue:

- Full `just test` flake triage unless a listed bug requires it.
- Broad UI redesign.
- Dependency modernization from `cargo audit` warnings unless it becomes a
  blocker for a listed item.

## Approach

1. Start with P1 security and policy items.
2. For each bug, add or update a focused regression test before fixing.
3. Keep fixes separate enough that each can be reviewed and reverted alone.
4. Update `tracker.md` as items move from unconfirmed to confirmed, fixed, or
   intentionally descoped.
5. Run the smallest relevant test first, then the broader gate for the touched
   area.

## Done

- Every active queue item is either fixed with a regression test, moved to a
  follow-up with rationale, or explicitly closed as not reproducible.
- Gateway/auth fixes have negative tests for malicious localhost-like origins
  and token exposure paths.
- MCP policy fixes have corp/user collision tests.
- Frontend token drift has shared semantic status styling or a documented
  design-system exception.
- Final validation status is recorded in this directory.
