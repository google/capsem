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

**Telemetry pipeline issues**: The six tables (net_events, model_calls, tool_calls, tool_responses, mcp_calls, fs_events) each have their own pipeline. If a table is empty or has wrong data:
- Check if the guest daemon started (boot logs)
- Check if the vsock connection was accepted (host logs)
- Check timing -- did the VM shut down before the debouncer flushed? (add `sleep 1`)

Write down what you find. The diagnosis should explain *why* the bug exists, not just *where* the symptom appears.

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
