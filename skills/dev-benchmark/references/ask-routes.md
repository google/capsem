# Original HTTP Ask controls

`[benchmark.routes.ask_controls]` declares finite approve and deny cases for
`POST /vms/{id}/security/asks/{ask_id}/resolve`, over service UDS and the
authenticated gateway. Each uses a separate disposable normal pinned reference
image, shipping service/process/guest artifacts and the existing hermetic mock.
The private corporate rule holds a genuine mapped-workload HTTP request; no
pending rows, approval decisions or upstream effects are seeded.

The existing approval fixture waits for the single original request to appear in
the public queue. The resolver must reject a foreign boot generation without
releasing it, then accept its first decision once. The receipt must identify the
same Ask and event; a duplicate decision must conflict. Approval returns exact
original bytes and delivers one exact request to the independent mock transcript.
Denial returns HTTP403 without private body disclosure or upstream delivery.
The final public queue must have no live pending operation and retain its terminal
decision. All original callers and owned services have finite cleanup budgets.

Strict header/first-body/complete resolver HTTP receipts exclude original launch,
pending discovery and human dwell. `verified_completion_ms` validates the resolver
receipt only. `original_to_pending_ms` and
`original_completion_after_decision_ms` are distinct client observations; the
latter includes resolver HTTP, original execution completion and independent
upstream observation. No artificial human sleep is added. Config owns finite
cycles, timeouts and transcript/response bounds. The existing sampler phase spans
the complete owned lifecycle, not just resolver CPU; Rust remains the statistics,
raw-sample, fitness and store owner. These cases never enter 512-RPS read loops.

The canonical `test_security_approval_lifecycle` and HTTP edge acceptance owners
retain the physical `security_ask_events`/`net_events` join after stop/drain.
The collector opens no ledger. Pending-list latency distributions, expiry,
concurrent-waiter separation, retirement and other operation types are separate
state/acceptance debt. Source tests with labelled effect doubles are not native
execution or performance evidence. Actual matching-cohort collector runs and fit
repeats remain required.
