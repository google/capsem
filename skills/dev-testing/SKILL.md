---
name: dev-testing
description: Capsem testing policy and workflow. Use whenever running tests, writing new tests, or verifying changes work. Covers the three test tiers (unit, smoke, full), TDD red-green-refactor, adversarial security testing, coverage policy, and the mandatory end-to-end VM validation. For VM-specific tests see dev-testing-vm, for hypervisor tests see dev-testing-hypervisor, for frontend tests see dev-testing-frontend.
---

# Testing

## Test tiers

Three tiers, fast to thorough. Every change must pass all three before it ships.

| Tier | Command | What | VM? |
|------|---------|------|-----|
| Fast | `just test` | Unit tests (llvm-cov) + cross-compile agent + frontend check + build | No |
| Smoke | `just smoke` | test + repack + sign + boot + session DB validation (~30s) | Yes |
| Full | `just full-test` | smoke + build-assets + cross-compile + integration + bench | Yes |

Why all three matter: `just test` catches logic bugs and type errors without a VM. `just smoke` (runs `scripts/doctor_session_test.py`) catches sandbox, network, and telemetry regressions that only manifest inside the guest, without the 10-minute overhead of a full image build. `just full-test` catches fresh image build, packaging, and performance regressions.

Skipping the smoke tier is how bugs ship -- unit tests pass but the VM sandbox behaves differently or telemetry is broken.

## TDD workflow

Write tests first:
1. Write failing tests that capture expected behavior
2. Verify they fail for the right reason
3. Write minimal implementation to pass them
4. Refactor

Without a failing test first, it's easy to write tests that pass by accident or don't actually verify the behavior you intended.

## Adversarial testing

Capsem is a security product. Every security-relevant feature needs tests that actively try to break invariants. Think like an attacker:
- Can a corp-blocked domain be snuck through another provider's list?
- Does an overlapping wildcard in allow+block always deny?
- Does malformed input (empty strings, unicode, huge payloads, invalid JSON) get rejected?
- Can path traversal escape the VirtioFS sandbox?
- Can a guest process modify its own binaries?

Stress-test boundary conditions. Write tests for the attacks you'd attempt yourself.

## Where tests live

- Rust unit: `#[cfg(test)] mod tests` in each module
- Rust integration: `crates/capsem-core/tests/`
- In-VM diagnostics: `guest/artifacts/diagnostics/test_*.py` (see dev-testing-vm)
- Hypervisor: KVM + Apple VZ tests (see dev-testing-hypervisor)
- Frontend: `frontend/src/lib/__tests__/` (see dev-testing-frontend)
- Python (builder): `tests/`

## Coverage

- Rust: `cargo llvm-cov` via `just test`
- Python: `--cov-fail-under=90`
- `codecov.yml` maps components to code paths. Update it when files or directories are added, moved, or renamed.

## End-to-end validation is not optional

After any change touching guest binaries, network policy, telemetry, MCP, or VM lifecycle:

1. `just run "capsem-doctor"` -- verifies sandbox integrity inside the VM
2. After telemetry/logging changes: run a real session and verify with `just inspect-session` that all 6 tables (net_events, model_calls, tool_calls, tool_responses, mcp_calls, fs_events) are populated correctly

## When tests fail

Never dismiss a test failure as "pre-existing" or "unrelated." Every failure must be investigated. Follow the dev-debugging workflow:

1. **Do not change the test to make it pass.** The test is evidence. Changing the assertion to match broken behavior destroys that evidence.
2. **Reproduce and diagnose first.** Understand *why* it fails before writing any fix. See the dev-debugging skill for the full methodology: reproduce with a test, diagnose root cause, then fix comprehensively.
3. **Fix the code, not the test.** If the test is genuinely wrong (not the code), explain in detail why the test's expectation is incorrect before changing it.

## Platform gating tests

`cargo test --test platform_gating` scans all `.rs` files under `crates/` for macOS-only and Linux-only symbols (`libc::clonefile`, `AppleVzHypervisor`, `KvmHypervisor`, `FICLONE`, etc.) and verifies they appear inside `#[cfg(target_os = "...")]` blocks. This catches ungated platform APIs before they reach CI. Run this test when adding any platform-specific code.

## Testable design

Extract logic into `capsem-core` -- never embed business logic in the app layer where it's coupled to Tauri. If you can't test something without booting a VM or launching the GUI, it belongs in core.
