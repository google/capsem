# H07 - Docs Changelog And Release Gate

## Goal

Close the sprint with durable docs, changelog entries, and release-quality
validation.

## Scope

- Update development docs, benchmark docs, doctor docs, and relevant skills.
- Update bootstrap/doctor expectations when host prerequisites or diagnostics
  change.
- Keep `CHANGELOG.md` updated per functional milestone.
- Run final validation gates and record residual risk.

## Done

- The next engineer can understand the hypervisor choices from repo artifacts,
  not chat history.
- The user can see VM resource behavior through status and telemetry.
- Linux performance and correctness changes are backed by committed evidence.

## Proof

- `git diff --check`
- focused tests for touched areas
- `just run "capsem-doctor"`
- `just benchmark`
- final tracker coverage ledger

