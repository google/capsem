# Original run and purge route evidence

`[benchmark.routes.run_purge]` owns finite2/quick1 cycles,240s control deadlines,
60s execution timeout,1MiB response cap,2vCPU/1GiB disposable VM resources,
two retained stopped sessions and the exact bounded synthetic marker.
These cases run in quick mode too; they are not host-only read polls.

`POST /run` is explicitly **VM-only**: the public request has no container or
target field. The same original one-shot request provisions its ephemeral VM,
executes and removes it. It is never retried. A single worker issues that
request through the existing per-thread Transport while the owning thread
observes the one new ephemeral identity through public reads. The original
VM command writes its actual UID/GID, VM user-namespace map and requested
environment marker into its workspace, then waits under its execution deadline
for a release file. Public file export independently verifies exact original
bytes. Public file import releases that same invocation; no host write into
guest directories, additional execution authority or second transport is added.

The response must be untruncated exact output with zero exit and empty stderr.
After it completes, original process identity must be gone, public listing empty,
exact session path absent and session-directory membership restored; failed or
hidden leftovers fail. Timing includes provisioning, this explicit observer
fence, execution and teardown. It is an end-to-end **fenced fixture case**, not
ordinary command latency, readiness latency or body first-byte evidence for
exec-ready. The original strict header/first-body/completion receipt is retained
separately from verified completion by the existing observations owner.

`POST /purge` first creates two genuine owned persistent image workloads, verifies
namespace-scoped execution, exact original marker bytes through a separate public
download, and normally stops both. The original processes must exit and exact
paths and stopped/resumable identities must remain. Default purge is an **untimed
negative control**: healthy stopped sessions must remain with matching workspace
entries. Its zero count is not successful mutation timing. A single original timed
`all=true` request must independently remove both exact owners and paths; returned
persistent/ephemeral/total counts must match those actual effects. No user home,
ambient VM or real credential is touched. Failure cleanup belongs to the existing
isolated service fixture; original mutations are not replayed on ambiguous results.

The canonical lifecycle/local-image fixture and strict HTTP/Cycle/resource/raw/
Rust statistics/store/fitness owners are reused. File-policy exceptions name only
the three synthetic marker/release files. No SQLite/DbReader or projection is
opened. Source tests use explicit effect doubles and do not launch a VM.
Actual performance needs parent-owned coherent native execution and fit repeats;
source bindings and functional gates do not constitute measured endpoint passes.
