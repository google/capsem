---
name: dev-sprint
description: How to run a development sprint in Capsem. Use when starting a new feature, multi-step task, or any work that spans multiple changes. Covers sprint planning, progress tracking, changelog discipline, commit strategy, testing gates, and release. Enforces the workflow -- plan first, track progress, commit at functional milestones, always finish with testing.
---

# Development Sprint

Every non-trivial task follows this workflow. No shortcuts.

## 1. Plan

Create a sprint directory and write the plan before touching code:

```bash
mkdir -p tmp/<sprint-name>
```

Write `tmp/<sprint-name>/plan.md`:
- What we're building and why
- Key decisions and trade-offs
- Files to create/modify
- Dependencies and ordering
- What "done" looks like

The plan is a living document. Update it as the sprint evolves -- crossed-out items, new discoveries, changed approach. The plan is evidence of thinking, not a contract.

## 2. Track

Create `tmp/<sprint-name>/tracker.md` as a checklist:

```markdown
# Sprint: <name>

## Tasks
- [x] Task 1 -- description
- [x] Task 2 -- description
- [ ] Task 3 -- description
- [ ] Testing gate
- [ ] Changelog
- [ ] Commit

## Notes
- Discovery: found that X needs Y
- Changed approach: Z instead of W because...
```

Update the tracker as you go. Check items off. Add notes about surprises, blockers, and changed approaches. This is your scratchpad -- future you (or the next conversation) reads this to understand what happened.

## 3. Build

Write code. Follow the project skills:
- `/dev-debugging` for bug investigation (reproduce first, diagnose, then fix)
- `/dev-testing` for TDD (write test, see it fail, implement, refactor)
- `/dev-rust-patterns` for async/cross-compile patterns
- `/dev-mitm-proxy`, `/dev-mcp` for subsystem-specific guidance

## 4. Commit at functional milestones

Do NOT commit after every file edit. Do NOT batch everything into one giant commit at the end. Commit when:

- A logical unit of work is complete and functional
- Tests pass for that unit
- The codebase is in a good state (not half-refactored)

Each commit should:
- Be self-contained (revertable without breaking things)
- Include its CHANGELOG.md entry
- Stage files explicitly (no `git add -A`)
- Use conventional messages: `feat:`, `fix:`, `chore:`, `docs:`

Bad: 20 tiny commits for each file touched. Also bad: 1 commit with 40 files after hours of work.
Good: 3-5 commits per sprint, each representing a meaningful milestone.

## 5. Changelog

Update `CHANGELOG.md` under `## [Unreleased]` as part of each commit. Write from the user's perspective:
- Added: new capability
- Changed: modified behavior
- Fixed: bug fix
- Security: security improvement

Do not batch changelog entries at the end. Each commit carries its own entry.

## 6. Testing gate

Every sprint ends with testing. No exceptions.

```bash
just test                           # Unit + cross-compile + frontend (fast)
just run "capsem-doctor"            # VM smoke test
```

If the sprint touched telemetry, network, or MCP:
```bash
just full-test                      # Full validation (3x VM boot)
just inspect-session                # Verify telemetry after a real session
```

If tests fail, fix them before considering the sprint done. See `/dev-debugging` for the methodology.

## 7. Clean up

- Remove or archive `tmp/<sprint-name>/` (it's gitignored)
- Verify no debug prints, TODO comments, or temporary hacks remain
- Run `/simplify` if significant code was written

## Sprint artifacts

```
tmp/<sprint-name>/
  plan.md           What we're building, key decisions
  tracker.md        Checklist + notes
  changelog.md      Draft changelog entries (optional, can go straight to CHANGELOG.md)
```

The `tmp/` directory is gitignored. Sprint artifacts are ephemeral -- they inform the work but don't ship.

## Anti-patterns

- **No plan**: jumping straight to code leads to rework and wrong abstractions
- **Commit per file**: noise in git history, impossible to revert cleanly
- **One mega commit**: can't bisect, can't review, can't cherry-pick
- **Skip testing**: "I'll test later" means "I'll ship bugs now"
- **Stale tracker**: if the tracker doesn't match reality, it's useless
