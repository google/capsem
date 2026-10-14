## Parallel tests as dogfooding (n=4 is non-negotiable)

`just test` runs the python suite under `pytest -n 4 --dist=loadfile`. Four real VMs boot simultaneously. **This is the canary, not just a speed-up.** We ship Capsem as a multi-VM sandbox for AI agents -- if our own test suite cannot safely boot 4 concurrent VMs, real users running an agent farm will hit the exact same bug. Treat any concurrency flake as a Capsem-side bug, not a test-tuning problem:

- "Suspend timed out" under load -> service IPC handling is racy, not "bump the timeout"
- "Session did not become ready" -> Apple VZ resource serialization, VirtioFS lock contention, or service handling concurrent provisions; investigate, don't suppress
- Two tests both want the same VM name -> name-collision bug in `validate_vm_name` / registry, not "isolate test names better"
- Stale socket between tests -> service didn't reap a child cleanly, real production bug

Anti-patterns when a test flakes under `-n 4`:
- Adding `time.sleep()` to "let things settle" -- masking a race
- Bumping the per-test timeout -- buying time for a real bug to manifest in prod instead of CI
- Marking the test `serial` so it runs alone -- defeating the dogfooding signal

The exception is a true timing or benchmark probe whose assertion is the
measured number. Those tests must already be marked `serial` and `just test`
runs them immediately after the `-n 4` canary. That is not a flake escape
hatch: it prevents another benchmark file from stealing the same Apple VZ
launch budget and corrupting the number we are trying to publish.

The host has plenty of headroom (48 GB RAM, 14 cores; 4 VMs at 2 GB / 2 CPU each = 8 GB / 8 cores). If concurrency surfaces a flake, fix the product, then re-run. Bumping `-n` higher (8, 12) is the natural follow-on once n=4 is stable -- real users will run more.

### Orphan processes across runs are a product bug (not a test bug)

After an interrupted test run, readiness failures, UDS connection refusals, or HTTP 500s can come from companions that survived their parent. Confirm the failing run's owned processes before diagnosing a leak. Use the repository's PID-file and owned process-tree control modules to stop only that run's exact PIDs. AGENTS.md's "Never kill by pattern" rule applies to binary-path patterns too: they can match another session's processes.

Fix the companion lifecycle when a leak is confirmed. Every spawned companion must use `capsem-guard::install(parent_pid, lock_path)` to refuse standalone launches, enforce its singleton, and exit when its parent dies. See `/dev-rust-patterns` lesson 18. Preserve `tests/capsem-service/test_companion_lifecycle.py` and extend its regressions when adding a new companion.

### Apple VZ lifecycle serialization is part of the product

Apple's Virtualization.framework does not tolerate overlapping checkpoint
lifecycle operations (`saveMachineStateToURL` and `restoreMachineStateFromURL`)
on sibling VMs, and teardown must not cross those checkpoint edges. Capsem uses
`ServiceState::save_restore_lock` plus the host-wide `VzHostLock` flock:
cold starts and teardown take shared/read guards, save and restore take
exclusive/write guards. The rail holds even when pytest-xdist spawns one
`capsem-service` per worker, while independent cold starts can still run
together for the boot-latency gate.

Do not demote suspend/resume, lifecycle, provisioning, or teardown tests to
`-n 1` to sidestep VZ races. `just test` at `-n 4` is the contract; if a
concurrent run sees restore permission errors, loop-device corruption,
connection-refused startup races, or readiness misses, fix the lifecycle rail.
Full context and failure signatures live in
`web/docs/src/content/docs/gotchas/concurrent-suspend-resume.md`.
