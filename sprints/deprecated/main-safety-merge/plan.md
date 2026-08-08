# Main Safety Merge

## Goal

Preserve the broad 1.3 worktree and port it onto `main` without ancestry-merging
the old `abbd9330` policy branch.

## Approach

- Commit the dirty `codex/kernel-7-erofs-zstd` worktree as a preservation
  commit.
- Create a backup pointer for local `main`.
- Cherry-pick the preservation commit onto `main` so Git ports only the patch,
  not the old branch ancestry.
- Resolve conflicts explicitly if they appear.
- Run focused gates first, then leave final release gates for the release
  sprint.

## Done

- The preservation commit exists.
- `main` contains the work as a new commit or an explicitly resolved
  integration commit.
- The working tree is clean or any remaining release-gate fallout is listed in
  the tracker.
