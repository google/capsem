# Profile V2 test-fix sprint

## Goal
Make branch tests pass under Profile V2 semantics where settings keys are strictly `policy.<http|dns|mcp|model>.<rule_name>` and `/settings` responses expose `effective_rules` rather than legacy `policy`.

## Scope
- Update failing E2E tests that still post `security.web.*` keys.
- Update assertions expecting `saved["policy"]` to use `effective_rules`.
- Fix any V2 rule generation/runtime conversion bug causing rules to be dropped (currently suspected DNS condition field mismatch).
- Re-run the previously failing tests and summarize outcomes.

## Done Criteria
- Targeted failing tests pass locally.
- No legacy `security.web.*` usage remains in the touched failing tests.
- TL;DR delivered with what changed and why.
