# Finite lifecycle route cases

`[benchmark.routes.lifecycle]` binds exactly six method/path operations in the
existing routes collector. One cycle uses one isolated service home, one exact
pinned local reference image and its existing producer/root receipt. Existing
local TLS registry admission and the service-owned gateway remain authoritative;
missing artifacts fail before launch, without a fetch/build or metadata bypass.

Each target (UDS and gateway) has three independent full cycles, quick one. This
is not a concurrent mutation workload, a p99 or capacity claim. Configure counts,
CPU/RAM, response budget and deadlines in the existing gate config. Every
original request is single-attempt through `Transport.measure_request`; failed
or ambiguous admissions fail the collector rather than retry. Setup readiness
and independent verification execs are not counted as timed exec samples.

The measured sequence creates a named persistent image owner, waits for its real
container and namespace probe, writes known workspace bytes, independently reads
them, forks into an initially stopped owner, starts that fork and verifies its
image/namespace/preserved bytes, stops it and verifies process exit/retained
session directory, then deletes it and verifies list/path/process absence.
Source identity and process remain unchanged by fork. Only IDs returned in this
isolated cycle are eligible for lifecycle control. Normal fixture teardown owns
emergency cleanup after an ambiguous acknowledgement, never an automatic replay
of the original mutation. The source owner is deleted outside operation clocks.

API headers, first observed body byte and complete body are separate original
raw samples; `verified_completion_ms` includes independent completion checks
under one per-operation deadline. First byte is client-observed, not a wire
hardware timestamp. Process-tree CPU/RSS/PSS/raw wall receipts are for the whole
lifecycle cycle, including verification/setup work, not isolated per-request CPU.
Existing Rust stats/store/fitness and exact-source cohort owners are unchanged.

Stop's public contract acknowledges owner process exit. Physical producer-ledger
drain is explicitly left to canonical Ironbank after drain, never inferred from
status or file presence. Known exec bytes do not certify per-request CPU limits.
Save/warm restore, resume, purge, one-shot run, live VM policy acknowledgements
and other lifecycle states remain explicit coverage debt. These finite cases do
not discharge them. Three samples cannot establish a tail latency distribution.

Source unit tests use explicit synthetic services, processes and HTTP responses.
They certify collector assertions, not product behavior or benchmark results.
No real collector/VM/image preparation runs are part of this source handoff;
matching native-cohort execution, pre/post fitness and fit repeats are still due.
