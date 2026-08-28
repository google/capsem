---
name: dev-sprint
description: Run multi-step Capsem development in Sprinty. Use for features, refactors, migrations, release preparation, or any work spanning several commits and verification gates.
---

# Development Sprint

Sprinty is the sole active sprint ledger. Git owns source and permanent
history; Sprinty owns the goal, dependency graph, work items, notes, evidence,
coverage, gates, and closeout. Do not create repository planning directories
or parallel Markdown trackers.

## Start or resume

Call `mcp__sprinty.info()` first on every new session or continuation.

- If a matching active sprint exists, bind it with
  `mcp__sprinty.sprint_resume()` using the explicit repository `git_dir` and
  its worktree-scoped, ignored `data_dir`.
- If no matching sprint exists, call `mcp__sprinty.sprint_new()` with the
  concrete goal, explicit `git_dir`, ignored `data_dir`, and durable context
  notes.
- Show the dashboard URL to the user. Use `overview()` for sprint shape and
  `next()` for the current work window; do not reconstruct state from memory.

Never start a replacement sprint because the work is difficult or context was
compacted. Resume the existing ledger and inspect its artifacts.

## Plan the dependency graph

Use `mcp__sprinty.subsprint_new()` for independently reviewable features or
phases. Each subsprint names:

- the outcome and why it matters;
- ordered dependencies;
- explicit goals;
- executable or manual gates that prove the complete behavior.

Use `mcp__sprinty.item_add()` before editing files or mutating another system.
An item owns one revertable outcome and records:

- a specific title and complete description;
- exact code locations;
- dependency edges;
- focused tests, type checks, builds, or manual evidence gates.

Adjacent notes are not a substitute for a dedicated item when work has its
own files, risk, or outcome. Split broad items instead of hiding multiple
commits behind one checkbox.

For Capsem release, VM, network, model, MCP, credential broker,
package-manager, doctor, benchmark, or security acceptance work, load
`/ironbank` and include its black-box proof in the owning item before coding.

## Keep durable evidence in Sprinty

- Use notes for short discoveries, decisions, blockers, and user checkpoints.
- Use artifacts for audits, manifests, proof matrices, command output
  summaries, or handoffs that must survive compaction.
- Attach each artifact to the item or items it proves.
- Keep coverage honest: name unit/contract, functional, adversarial, E2E/VM,
  telemetry, and performance evidence, and explicitly record categories that
  do not apply.

Do not treat a benchmark as functional proof or a Rust unit test as proof of a
user-visible VM path. Ironbank gates cannot be closed with status-only replay,
row-exists checks, internal-only expectations, or skipped tests.

## Build with project rules

Load the relevant project skills before coding:

- `/dev-debugging` for reproduce-first bug investigation;
- `/dev-testing` for RED, GREEN, and refactor discipline;
- `/dev-rust-patterns` for Rust, async, and cross-platform work;
- `/dev-mitm-proxy`, `/dev-mcp`, `/build-images`, and other owning skills for
  their subsystems;
- `/ironbank` for release-critical black-box acceptance.

Keep profile and config ownership crisp:

- Read `config/README.md` and `tests/README.md` before changing profile source,
  generated config, or config fixtures.
- Checked-in profile files are source contracts. Generated hashes and
  materialized config belong under `target/` and are produced by the same
  `capsem-admin` rail CI and release use.
- Developer skills live in repository-level `skills/`; product configuration
  must not mirror them.
- Prefer functional names that state ownership over temporary or origin-story
  names.

## Commit at functional milestones

Commit when one logical item or milestone is complete, its focused evidence is
green, and the tree is not half-migrated. Each commit must be self-contained
and revertable.

- Stage files explicitly; never use broad staging.
- Follow the repository conventional subject and author rules.
- Update `CHANGELOG.md` in the same commit only for user-visible behavior.
- Preserve a clean status before moving to the next item.

After the commit, call `mcp__sprinty.item_done()` with the exact commit id, a
SemVer changelog verb and line, and evidence for every manual gate. Sprinty
runs executable gates itself where configured. Do not close an item against
uncommitted work or a commit that does not contain its files.

## Verify proportionally, then verify completely

Every item gets the smallest focused proof that fully covers its outcome.
Every sprint ends with the repository-authoritative final gates required by
its scope.

The proof matrix should consider:

- unit and contract tests;
- production-facing functional behavior;
- malformed input, denial, timeout, race, and leak adversaries;
- real CLI, service, MCP, network, package, or VM paths when crossed;
- session database or external-state evidence when auditability is claimed;
- performance measurements only when performance is part of the claim.

Run direct commands through the repository's bounded-command wrapper. Read the
current gate digest before claiming a gate passes. Keep release holds active
until focused checks and the final required gates are green.

## Close

Before closeout:

1. Use `overview()` and `next()` to prove no item or subsprint remains open.
2. Reconcile notes, artifacts, coverage, changelog entries, and commit ids.
3. Run the final required gates on the exact final source state.
4. Verify the worktree is clean and no temporary compatibility path remains.
5. Call `mcp__sprinty.sprint_close()` only after Sprinty accepts the complete
   evidence ledger.

If work is blocked, keep the owning item open and record the exact external
condition. Do not shrink the goal or declare completion around the blocker.
