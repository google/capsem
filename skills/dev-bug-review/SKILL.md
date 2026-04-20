---
name: dev-bug-review
description: Triage and resolve incoming bug reports one by one. Use when the user brings in one or more bug reports (from a tracker, a dump, a "here are three bugs" paste, etc.) and expects each to be confirmed, the proposed fix evaluated or pushed back on, implemented only after agreement, then committed with a changelog entry. Enforces confirm-before-fixing, push-back-with-reasoning, and per-bug commit discipline. Do NOT use for ad-hoc single-bug debugging where the user hasn't framed it as a review queue -- use dev-debugging for that.
---

# Bug Review

A disciplined workflow for working a queue of bug reports. One bug at a time. No skipping steps. No batching.

## The contract

For every incoming bug report, execute these five phases in order. Do not proceed to the next phase without the previous one's output.

1. **Confirm the finding** -- reproduce or evidence the bug before believing it
2. **Validate the solution or push back** -- evaluate the proposed fix; disagree with reasoning if warranted
3. **Get agreement** -- wait for the user to agree before touching code
4. **Implement** -- apply the fix, add tests, verify
5. **Summarize, commit, changelog** -- one commit per bug, changelog entry included

If there are N bugs, you run this loop N times. Do not try to land all bugs in one commit unless the user explicitly says so.

## Phase 1: Confirm the finding

Before writing any code, prove the bug is real and that you understand it.

- Read the code path the report implicates. Cite file paths and line numbers.
- Reproduce it where feasible: a failing test, a `just run "<cmd>"` that demonstrates the issue, a session DB inspection, a screenshot, a log snippet.
- If the report is vague ("it's slow", "it crashes sometimes"), nail it down before moving on. Ask a targeted question rather than guess.
- If the bug is **not reproducible or not present in the code**, say so clearly and stop. Do not manufacture a fix for a bug that doesn't exist.

Output for this phase is a short statement: *what the bug is, where it lives, and the evidence*. Do not proceed silently.

## Phase 2: Validate the solution or push back

The report usually arrives with a proposed fix. Treat it as a hypothesis, not an order (see memory: "Push back on proposed fixes").

Evaluate the proposed fix against:
- **Does it address the root cause, or just the symptom?** Symptom patches leave the bug to resurface elsewhere.
- **Is the pattern systemic?** If the same mistake exists in 7 other places, fixing only the reported site is deferred breakage. See `/dev-debugging` "Fix the pattern, not the instance".
- **Does it break invariants?** Ephemeral VM model, guest binary read-only, codesigning entitlement, gateway auth (never weaken), Tauri embed-at-build -- all listed in CLAUDE.md.
- **Does it contradict a memory or skill?** Check relevant skills before accepting a fix that seems to fight them.
- **Is there a simpler or safer alternative?** Sometimes the right fix is deleting the feature, not patching it.

If the proposed fix is wrong or incomplete, **push back with reasoning**. State what you'd do instead and why. Do not silently "improve" the fix -- name the disagreement so the user can weigh in.

If the proposed fix is correct, say so plainly. Do not pad with fake alternatives.

## Phase 3: Get agreement

Stop and wait. Do not start editing code until the user confirms the plan for this specific bug. A single "sounds good" covers this one bug, not the whole queue.

Auto mode does not override this. Agreement gates on the fix plan are a feature, not an interruption -- the user explicitly asked for a review workflow.

## Phase 4: Implement (TDD)

Fixes land test-first. No exceptions.

1. **Write the test first, watch it fail.** Before editing implementation code, write a test that captures the bug and fails for the right reason. "Fails for the right reason" matters -- a test that fails because of a missing import tells you nothing. Run the test and see the red.
   - If the bug lives in a pure function, unit-test that function directly.
   - If the bug is only visible through I/O or timing, extract a pure helper (e.g. argument construction, state transition, decision logic) out of the buggy site and test the helper. Extraction is part of the fix, not scope creep.
   - If you literally cannot write a failing test (e.g. the bug is in a system call behavior you can't mock), state that out loud and describe the manual reproduction you ran instead. Do not skip this silently.
2. **Apply the fix. Watch the test go green.** Minimum code needed -- no opportunistic refactors beyond what the test extraction required (see CLAUDE.md "Minimize code").
3. **If the pattern is systemic, fix all instances in this pass.** Do not defer siblings to "a future cleanup". The audit from Phase 2 defines the scope.
4. **While fixing, surface any additional issues you uncover.** If the code you're touching has an adjacent bug (zombie children, duplicated branches, wrong error handling), flag it in your summary. Fold small ones into the same fix; call out larger ones for a separate bug review pass.
5. **Run the relevant gates:**
  - Rust change: `cargo check -p <crate>` + targeted `cargo test`
  - Cross-cutting Rust: `just test`
  - Frontend: `pnpm run check` (fail-on-warnings) + `pnpm test` where relevant
  - VM behavior: `just run "capsem-doctor -k <category>"` or the targeted diagnostic
  - Telemetry: `just inspect-session`
- Fix every warning surfaced. Warnings are errors (CLAUDE.md).

## Phase 5: Summarize, commit, changelog

Write a summary back to the user before committing:
- What the bug was (one line)
- Root cause (one or two lines)
- What you changed (files + intent, not line-by-line diff)
- How you verified (tests/commands run)

Then commit per project rules (CLAUDE.md "Commits"):
- Update `CHANGELOG.md` under `## [Unreleased]` in the **same commit** as the fix. Write from the user's perspective under `### Fixed`.
- Stage files explicitly. No `git add -A`.
- Conventional message: `fix: <one-line subject>`. Body can expand on root cause.
- Author: Elie Bursztein <github@elie.net>. No `Co-Authored-By` trailers.
- One bug per commit. If you fixed a systemic pattern across many files, that's still one commit -- but it's still one bug.

Then move to the next bug in the queue and repeat from Phase 1.

## Anti-patterns

- **Skipping the failing test**: going straight to the fix. The test-first gate catches wrong diagnoses and guards against regressions.
- **Skipping confirmation**: accepting the report at face value and jumping to a fix. You will fix the wrong thing.
- **Silent solution swap**: user proposed fix A, you silently shipped fix B. Surface the disagreement instead.
- **Agreement creep**: treating "sounds good" on bug #1 as authorization for bugs #2-#5. Re-agree per bug.
- **Batched commits**: "I fixed all five, here's the commit." Loses bisectability and blurs the changelog.
- **Skipped changelog**: "I'll add it at the end." Each commit carries its own entry.
- **Pre-existing dismissal**: "That failure is unrelated." Investigate every failure surfaced during the fix. Never deflect.
- **Symptom patching**: stripping a header to avoid a decoder bug instead of fixing the decoder. Address the system, not the surface.
- **Narrow fix for systemic bug**: fixing 1 of 8 identical sites. Audit first, then fix all in one pass.

## Relationship to other skills

- `/dev-debugging` -- the methodology for a *single* bug investigation (reproduce, diagnose, fix). Bug review composes debugging across a queue with extra gates (confirm, push back, per-bug commit).
- `/dev-sprint` -- for multi-change features. Bug review is lighter weight: no sprint dir, no tracker.md, one commit per bug.
- `/dev-testing` -- the testing gates invoked in Phase 4.
