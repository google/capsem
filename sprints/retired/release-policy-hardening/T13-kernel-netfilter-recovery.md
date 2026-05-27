# T13: Kernel Netfilter Recovery and Full Test Gate

## Objective

Recover the guest network-policy execution path by fixing the kernel/netfilter
regression that leaves `iptables` tables unavailable in booted VMs. T13 owns
the root fix, guardrails, and regression verification until `just test` is
fully green again.

This track is mandatory before any "next sprint" expansion. No new feature
work starts while this gate is open.

## Owned Files

- `guest/config/build.toml`
- `src/capsem/builder/templates/Dockerfile.kernel.j2`
- `guest/artifacts/capsem-init`
- `tests/capsem-guest/test_guest_network.py`
- `guest/artifacts/diagnostics/test_network.py`
- `guest/artifacts/diagnostics/test_sandbox.py`
- `sprints/release-policy-hardening/MASTER.md`
- `sprints/release-policy-hardening/plan.md`
- `sprints/release-policy-hardening/tracker.md`

## Findings

- [P0] Guest boot logs show every redirect insert failing:
  `iptables ... Table does not exist`.
- [P0] In affected VMs, `iptables-legacy -L` fails for `filter`/`nat`, and
  `/proc/net/ip_tables_names` is absent.
- [P0] DNS/MITM redirects are never installed; DNS resolution and `net_events`
  telemetry fail, cascading into 13 failing suites.
- [P1] Current kernel selection uses `kernel_branch = "auto"` and can drift
  onto a new LTS line without netfilter parity verification.
- [P1] `capsem-init` currently logs rule failures but still reports
  "network ready", masking a broken policy path.

## Task List

### T13.1 Failure Baseline

- [x] Capture failing evidence from focused tests and preserved artifacts:
  serial logs, process logs, and session DB event counts.
- [x] Record exact failing test set and categorize by shared root cause.

### T13.2 Deterministic Kernel Line

- [x] Pin guest kernel branch away from `auto` to a deterministic `X.Y` line
  for both arches (current target: `6.6`).
- [x] Document the rationale in sprint notes and leave upgrade path explicit.

### T13.3 Build-Time Netfilter Contract

- [x] Add a hard assertion in kernel build to fail if required symbols are not
  present in the post-`olddefconfig` `.config`.
- [x] Required symbols must cover iptables tables and redirect targets used by
  guest boot (`IP_NF_IPTABLES`, `IP_NF_NAT`, `NF_NAT`, `XTABLES`,
  `XT_TARGET_REDIRECT`).

### T13.4 Boot-Time Fail Closed

- [x] Update `capsem-init` to fail boot if redirect rules cannot be installed.
- [x] Ensure failure messaging is explicit and points to kernel/netfilter
  mismatch instead of allowing a silent degraded runtime.

### T13.5 Test Hardening

- [x] Tighten guest diagnostics/tests to require successful table access and
  redirect rule presence (not just non-empty command output).
- [x] Keep assertions aligned with actual runtime contract (`iptables-legacy`
  primary path with clear failure text).

### T13.6 Rebuild + Focused Verification

- [x] Rebuild affected VM assets and repack initrd through standard recipes.
- [x] Verify in-VM `iptables` tables, redirect rules, DNS resolution, and
  `net_events` emission.
- [x] Rerun previously failing focused tests until green.

### T13.7 Full Gate

- [x] Run full `just test`.
- [x] Keep this track open until full suite passes.
- [x] Only after full pass, mark T13 complete and clear "no next sprint"
  hold.
- [x] Completion evidence: local `just test` exited `0` on 2026-05-14.

## Proof Matrix

| Category | Required proof |
|---|---|
| Unit/contract | Kernel build fails fast when required netfilter symbols are absent. |
| Functional | Guest boot installs redirects successfully and policy path is active. |
| Adversarial | Misconfigured/missing netfilter fails loud at boot, not silently. |
| E2E/VM | Focused failing suites are green after asset rebuild. |
| Telemetry | `net_events`/policy telemetry returns for guest curl/model policy traffic. |
| Performance | n/a (no performance claim in this track). |

## Verification

- [ ] `just build-assets arm64`
- [ ] `just build-assets x86_64`
- [ ] `just _pack-initrd`
- [ ] `uv run pytest -q tests/capsem-session-lifecycle/test_exec_events.py::test_exec_curl_creates_net_event`
- [ ] `uv run pytest -q tests/capsem-guest/test_guest_network.py::TestGuestNetwork::test_iptables_redirect`
- [ ] `uv run pytest -q tests/capsem-e2e/test_model_policy_mitm.py::test_guest_model_request_policy_block_records_session_db_no_leak`
- [ ] `uv run pytest -q tests/capsem-e2e/test_policy_v2_http_dns_mitm.py::test_guest_http_policy_v2_block_and_header_strip_records_session_db`
- [ ] `uv run pytest -q tests/capsem-gateway/test_mitm_policy.py::test_mitm_policy_telemetry`
- [x] `just test`

## Exit Criteria

- [x] Redirect rules are reliably installed on guest boot.
- [x] DNS + MITM policy traffic uses the intended redirect path.
- [x] All previously failing network-policy/session telemetry tests pass.
- [x] Full `just test` is green.
