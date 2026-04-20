---
name: dev-debugging
description: Debugging methodology for Capsem. Use when investigating bugs, test failures, unexpected behavior, or any issue that needs diagnosis. Enforces the correct workflow -- reproduce with a test first, diagnose the root cause, then offer a comprehensive fix. Never jump to fixing code without understanding why it broke.
---

# Debugging

## The rule

Never fix code before you understand why it broke. The temptation to "just make the test pass" or "just patch the symptom" leads to fragile fixes that hide deeper problems. Follow the three-step workflow below every time.

## Step 1: Reproduce with a test

Before touching any implementation code, write a test that captures the bug. This test must:
- Fail right now, demonstrating the broken behavior
- Be specific enough to distinguish the bug from correct behavior
- Live in the right test location (see dev-testing for where tests go)

If you can't reproduce it in a test, you don't understand it well enough to fix it. For VM-level issues, use capsem-doctor or write a targeted diagnostic command:
```bash
just run "<command that triggers the bug>"
```

For telemetry issues, use session inspection:
```bash
just inspect-session
```

## Step 2: Diagnose the root cause

With a failing test in hand, investigate. Do not skip this step. Common diagnostic approaches:

**Integration-test failures: read the preserved service log.** When any integration test fails, the test fixture (`tests/helpers/service.py::ServiceInstance`, the e2e `RealService`, and the MCP `_start_capsem_service`) archives its tmp_dir to `test-artifacts/<timestamp>-<worker>-<nodeid>/<tmp-basename>/` **before** the usual rmtree. The failing test's stderr has the exact path: look for a line `ARTIFACT: preserved /var/folders/... -> test-artifacts/...`. Inside that directory:

```
service.log                     host-side capsem-service debug log (RUST_LOG=debug)
logs/gateway.log                gateway stdout/stderr
logs/tray.log                   tray stdout/stderr (if spawned)
sessions/<vm-id>/process.log    per-VM capsem-process log (vsock bridge, IPC, spawn chain)
sessions/<vm-id>/serial.log     VM serial console (kernel boot, capsem-init, agent startup)
sessions/<vm-id>/session.db     SQLite telemetry DB (net_events, model_calls, ...)
persistent/<name>/...           persistent-VM state (checkpoint.vzsave, workspace)
```

`test-artifacts/` is gitignored. Multiple failures sharing a session-scoped service land in different subdirs but the latest run's name tags them by the most recent failing nodeid. First place to look for "VM didn't become exec-ready" style failures: `sessions/<id>/serial.log` (did the VM boot?) and `sessions/<id>/process.log` (did the agent come up + IPC handshake?). For "provision hung" or service-side contention: `service.log`, grep for the VM id.

**Rust code**: Read the code path the test exercises. Trace the data flow. Add `tracing` instrumentation if needed (`RUST_LOG=capsem=debug`). Check if the issue is in capsem-core, capsem-app, or capsem-agent.

**Guest VM issues**: Boot with targeted commands and inspect behavior:
```bash
just run "capsem-doctor -k <category>"   # Run specific diagnostic category
just run "<manual investigation command>"
```
Check boot logs for daemon startup failures, vsock connection issues, or timing problems.

**Network/policy issues**: Check the MITM proxy path -- SNI parsing, domain policy evaluation, HTTP rule matching, cert minting. Use session DB to see what actually happened:
```bash
just inspect-session   # Check net_events for domain, decision, status_code
```

**Frontend issues**: Run `just ui`, open Chrome DevTools, check console errors, use `take_screenshot` to capture state. See dev-testing-frontend for the full visual verification workflow.

**Build pipeline issues**: Check `target/build.log` -- all build infrastructure (runner, code signing, generation scripts) logs here. The runner (`scripts/run_signed.sh`) and `_generate-settings` recipe both append to this file. Never write diagnostics to stdout from build scripts (it contaminates binary output like `mcp-export`).

**Telemetry pipeline issues**: The six tables (net_events, model_calls, tool_calls, tool_responses, mcp_calls, fs_events) each have their own pipeline. If a table is empty or has wrong data:
- Check if the guest daemon started (boot logs)
- Check if the vsock connection was accepted (host logs)
- Check timing -- did the VM shut down before the debouncer flushed? (add `sleep 1`)

Write down what you find. The diagnosis should explain *why* the bug exists, not just *where* the symptom appears.

## Concurrency flakes are product bugs, not test-tuning problems

`just test` runs the python suite under `pytest -n 4 --dist=loadfile`. Four real VMs boot in parallel; this is dogfooding. Capsem ships as a multi-VM sandbox for AI agents -- if the test suite cannot safely run 4 concurrent VMs, real users running an agent farm will hit the same bug. When a test flakes only under concurrency, the diagnosis target is **Capsem's product code**, not the test:

- "Suspend timed out" appearing only at `-n 4` -> `handle_suspend` IPC race; investigate the `with_quiescence` path and the `Suspend` round-trip, not the test timeout
- "Session did not become ready" only with multiple parallel provisions -> Apple VZ resource contention, VirtioFS lock, or service handle_provision serialization gap
- Two tests collide on the same VM/session name -> `validate_vm_name` / persistent registry has a TOCTOU; UUID prefix in the test is not the bug
- "Connection refused" on a per-VM UDS only at `-n 4` -> service spawned the process but didn't wait for the socket to be bound; race in the spawn path
- A test passes serial but hangs at n=4 -> a global lock somewhere (state mutex held across an await, blocking Tokio worker; or a sync `std::Mutex` on a hot path)

Anti-patterns to avoid:
- Adding `time.sleep` in the test "to let things settle"
- Bumping a per-test timeout from 30s to 120s "because it's flaky"
- Marking the test `serial` -- defeats the dogfooding signal
- Adding retries with backoff in the client

Right pattern: capture a service log of the failing run (set `RUST_LOG=capsem=trace`), find the operation that took unexpectedly long or returned an error, fix the underlying race in capsem-service / capsem-process / capsem-core. Then re-run at `-n 4` to confirm.

## Step 2.5: Fix the pattern, not the instance

When diagnosis reveals a **systemic pattern** (the same mistake repeated across the codebase), the fix must cover every instance -- not just the one that was reported.

- **Audit the entire codebase for the same pattern.** If blocking I/O in async context caused one hang, grep for every other site that does the same thing. A bug is a symptom -- the pattern is the disease.
- **Never simplify a fix to the minimum diff.** A "quick fix" that patches one call site while 6 others have the identical problem is not a fix -- it's deferred breakage.
- **Document the pattern in the relevant skill** (e.g., dev-rust-patterns) so it's never reintroduced.
- **Add tests that would catch the pattern** if it recurs (e.g., a contract test between the frontend and backend response format).

Example: Snapshot MCP hang was caused by blocking I/O (clonefile, walkdir, blake3) on tokio worker threads. The same anti-pattern existed in 7 file tool handlers, the auto-snapshot timer, and asset hash verification. Fixing only the reported `snapshots_create` call would have left 9 other sites broken.

## Step 3: Fix with a comprehensive solution

Now that you understand the root cause, write the fix. The fix should:
- Make your reproducing test pass
- Not break any existing tests (`just test`)
- Address the root cause, not just the symptom
- Include the test from Step 1 in the same commit

After the fix, run the full validation:
1. `just test` -- unit + cross-compile + frontend
2. `just run "capsem-doctor"` -- VM smoke test
3. If the bug touched telemetry: `just inspect-session` after a real session

## What NOT to do

- **Do not "fix" a failing test by changing the test assertion.** The test is telling you something. Listen to it. If the test is genuinely wrong, explain why in detail before changing it.
- **Do not dismiss failures as "pre-existing" or "unrelated."** Investigate every failure. If it truly is pre-existing, file it and fix it -- don't leave broken windows.
- **Do not guess-and-check.** Random changes hoping something sticks waste time and often introduce new bugs. Understand first, then act.
- **Do not patch symptoms.** If requests fail because gzip content-encoding isn't handled, don't strip the Accept-Encoding header -- implement proper decompression. Fix the system, not the surface.
- **Do not apply narrow fixes to systemic problems.** If the same anti-pattern exists in 7 places and you fix 1, you haven't fixed the bug -- you've hidden 6 more. Audit first, then fix all instances in a single pass.
