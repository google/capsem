# `capsem doctor` VM dies mid-diagnostic

## Summary

`capsem doctor` provisions a temp VM and runs the in-VM diagnostic suite
(`/usr/local/lib/capsem-tests/`, 305 tests). The host CLI fails with
`IPC error: could not read` somewhere between 8% and 65% through the suite.
Manifests as `tests/capsem-e2e/test_e2e_lifecycle.py::TestDoctor::test_doctor_passes`
failing intermittently: **about 1 in 5 runs**.

Host-side summary line at the crash: `assert 1 == 0` where the capsem binary
exited with stderr `IPC error: could not read`. All in-VM tests that ran
before the crash pass. When the VM doesn't crash, the full 305-test suite
is green.

## Scope

- Only reproduces inside `capsem doctor`; interactive `capsem shell` + manual
  pytest runs have not been seen hitting it. The diagnostic VM runs at the
  default 2 GB RAM / 2 CPU.
- Not tied to a specific in-VM test. Crash points observed so far:
  - `test_npm_install_global_works` (`npm install -g cowsay`)
  - `test_tcp_443_reaches_proxy` (TCP connect to 10.0.0.1:443)
  - `test_scenario_triple_snap_no_change` (3× `snapshots_create`)
  - `test_snapshots_changes`, `test_uv_add_package_works` — others from
    neighbouring runs.

All three look like short, well-understood operations; the common thread is
**lots of short-lived vsock connections** through the session — in the process
log we see hundreds of `vsock: accepted connection, port:5003` / `MCP session
started` entries before the crash, and one `vsock: accepted connection,
port:5002` (MITM) near the end each time.

## What the logs show

In every crashing session the last entries in
`sessions/<vm>/process.log` are:

```
{"level":"INFO","fields":{"message":"vsock: accepted connection","fd":27,"port":5003}}
{"level":"INFO","fields":{"message":"MCP session started","process":"python3"}}
{"level":"INFO","fields":{"message":"capsem-mcp-builtin shutting down"}}
{"level":"INFO","fields":{"message":"capsem_mcp_aggregator: stdin closed, shutting down"}}
```

i.e. the aggregator subprocess has its stdin closed by its caller, then the
builtin follows. That caller is the MCP gateway task inside capsem-process
(`crates/capsem-core/src/mcp/gateway.rs::serve_mcp_session`), which manages
the aggregator connection for the lifetime of the per-VM process. The fact
that the aggregator is shutting down *before* the VM is killed suggests
either (a) capsem-process is panicking/exiting on its own, or (b) the
aggregator subprocess has been killed externally.

The guest serial log shows the in-VM pytest printing test results normally
right up to the boundary, then nothing — the VM loses its console channel,
not a kernel panic.

## Hypotheses worth checking

1. **Aggregator stdio lifecycle**: `serve_mcp_session` holds the aggregator
   connection. If any single vsock client connection drops the gateway out
   of its accept loop (panic, unreachable branch, fd exhaustion under N
   concurrent clients), the aggregator's stdin closes and the builtin goes
   down. Review the match arms in `serve_mcp_session` and
   `aggregator::call_tool` for error paths that could propagate all the way
   out to the task's top level.
2. **Per-connection fd leak or ordering**: repeated
   `"vsock: accepted connection","fd":27` at the same fd number is suspect
   -- the kernel recycling fd 27 is fine, but if a previous session hasn't
   closed the fd, we'd be serving on a half-closed socket. Check whether
   `drop(conn)` in `capsem-process/src/vsock.rs:258-263` actually runs on
   every path, including when the spawned task is canceled.
3. **Apple VZ / VirtioFS saturation**: the crash point correlates with the
   in-VM tests that do the most host-round-trip I/O (`npm install -g`
   writes thousands of files through VirtioFS; `snapshots_create` walks the
   workspace and hard-links into slot dirs). Suspect a virtiofs FUSE
   request that blocks indefinitely, tripping capsem-process's own
   watchdog, or an Apple VZ handle that drops under load.
4. **Process RSS**: capsem-process holds session state for the duration of
   the VM. If the aggregator spawns a fresh capsem-mcp-builtin subprocess
   per session and doesn't reap cleanly, file descriptors / memory creep up
   over the 305-test run. `ulimit -n` / `vmmap` on a running per-VM process
   near the crash point would tell.
5. **Host tokio runtime wedge**: `capsem-process` uses a single-threaded /
   multi-threaded runtime depending on build. A `tokio::spawn` task
   holding the aggregator's pipe could be starved by a long
   `tokio::task::spawn_blocking` unit (recently we added
   `spawn_blocking` around fork + provision in commit `34d0e3f`). Verify
   the MCP gateway task isn't being preempted for 20s+ during heavy
   provision/fork paths inside the diagnostic VM.

## Reproduction

```
cargo build -p capsem-service -p capsem-process -p capsem-mcp-builtin
just _pack-initrd
for i in $(seq 1 10); do
  uv run pytest tests/capsem-e2e/test_e2e_lifecycle.py::TestDoctor::test_doctor_passes --tb=no -q
done
```

Expect ~1-2 failures out of 10 runs. Each failing run leaves a tmp dir at
`/var/folders/.../T/capsem-e2e-*` with `service.log`, `service.stderr.log`,
`sessions/<vm>/process.log`, and `sessions/<vm>/serial.log` intact for
forensics (the e2e fixture deliberately doesn't clean on failure).

To force-capture logs on success as well, set `RUST_LOG=capsem=trace`
before the run — but be warned that this makes the runs 2-3× slower and
may change the timing enough to hide the bug.

## Starting points in the code

- `crates/capsem-core/src/mcp/gateway.rs::serve_mcp_session` — the
  aggregator lifecycle is owned here.
- `crates/capsem-core/src/mcp/aggregator.rs::call_tool` — unwrap paths.
- `crates/capsem-process/src/vsock.rs` — the `match conn.port` blocks for
  5002/5003 at lines 250-263 and 328-340.
- `crates/capsem-process/src/main.rs` around the session DB / scheduler
  initialisation — anything that panics on the main task takes the VM
  with it.
- `crates/capsem-core/src/auto_snapshot.rs` — slot creation; fill level
  was ~13/22 at one crash point, well under the limit.

## Out of scope for this ticket

- The host-side pytest flakes (`test_two_vms_isolated`,
  `test_fork_benchmark`, `test_lifecycle_benchmark`,
  `test_create_five_vms`) that appear once or twice across 5 parallel
  full-suite runs. They're load-sensitive tests that pass in isolation;
  worth their own investigation but independent of the doctor VM crash.

## What this ticket blocks

- 100% green `just test`. With doctor passing 4/5 runs, CI will flake
  without a retry loop. Releasing `capsem doctor` as the user-facing
  diagnostic command is blocked on this.
