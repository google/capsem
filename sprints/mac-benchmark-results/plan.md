# Mac Benchmark Results

## Goal

Run the current `origin/main` benchmark suite on macOS so the Linux team can
compare architecture-specific performance results.

## Scope

- Start from current `origin/main` in this worktree.
- Run `just bench`, which covers in-VM `capsem-bench` plus host lifecycle and
  fork benchmarks.
- Fix benchmark harness issues only if they block the run.
- Commit the generated benchmark data and sprint notes.

## Done

- Benchmark command completes or any remaining blocker is captured clearly.
- Generated benchmark JSON is committed.
- Coverage debt is explicit.

## Testing Proof Matrix

- Unit/contract: not applicable unless benchmark code changes.
- Functional: `just bench`.
- Adversarial: not applicable unless benchmark code changes.
- E2E/VM: covered by `just bench` provisioning and in-VM benchmark execution.
- Telemetry: not claimed.
- Performance: generated benchmark JSON under `benchmarks/`.
