# Profile Foundation Plan

## What We Are Building

A clean Foundation meta sprint that replaces the mixed post-ship S-numbered
Profile V2 queue with ordered F-numbered sub-sprints. The sprint exits only
when the core Profile V2 product, Security Event system, plugins, product
graph, dashboard, metrics, workbench, credentials, quotas, and docs have stable
contracts and verification.

## Key Decisions

- Keep `policy-settings-profiles/` as historical evidence and detailed source
  docs.
- Make `sprints/profile-foundation/MASTER.md` the active execution entry point.
- Use F-numbering for the new order so old "bedrock/post-bedrock/later"
  language does not imply priority.
- Treat every open Profile V2 item as in scope, sequenced into child sprints.
- Begin with code and installed-product proof before adding new product layers.

## Files Modified

- `sprints/profile-foundation/MASTER.md`
- `sprints/profile-foundation/tracker.md`
- `sprints/profile-foundation/F*.md`
- `sprints/README.md`
- `sprints/policy-settings-profiles/NOW.md`
- `CHANGELOG.md`

## Dependencies And Ordering

1. F00 establishes code reality and trust.
2. F01 proves the installed product.
3. F02/F03 close the event contract and runtime journal.
4. F04/F05 close rule/detection/confirm UX.
5. F06/F07 add credentials, graph/dashboard, and operational truth.
6. F08-F11 add workbench, plugins, integrations, and quotas.
7. F12 closes docs/site/release story and final gates.

## Done

- The active sprint inventory points to `profile-foundation`.
- Every open Profile V2 item maps to an F-numbered sub-sprint.
- The first code reality check is recorded with command and result.
- No open child lane is described as outside Foundation scope.
