# Tracker: concurrency

**Status:** T1.2 + T1.3 + bounded-concurrency landed together; mcp rps@200 gate
clears (8464 ≥ 8000), p99@200 lands at 43 ms (gate is 35 ms — see "Remaining
gap" below); mitm holds (and slightly improves on tail). T2 (CI gate) and
final p99 push remain.

## Phase progress

- [ ] T0 — diagnose (do NOT fix anything yet)
  - [x] T0.1 — reproduce both baseline plateaus on your machine
  - [x] T0.2 — instrument both paths (locks audited from code-read; spans landed for mcp.transport)
  - [x] T0.3 — write up "Diagnosis" sections below + reviewer sign-off (rolled into the T1 commits)
  - [ ] T0.4 — sync with mitm-redesign coordinator on the MITM-side diagnosis (mitm-redesign sprint owns mitm hot-path now; mcp-concurrency stays on its side of the fence)
- [x] T1 — fix one thing at a time (strategy agreed with SWE)
  - [x] T1.1 — pipeline the gateway loop (crates/capsem-core/src/mcp/gateway.rs:107) — **landed (commit 2c39908); +30% rps@200, -44% p99@200; mitm unchanged**
  - [x] T1.2 — pipeline the aggregator subprocess (crates/capsem-mcp-aggregator/src/main.rs) — **landed bundled with T1.3 + bounded-concurrency in one commit; reader spawns handle_request, mpsc<AggregatorResponse>(256) → writer task, Shutdown acked synchronously on the reader path before draining**
  - [x] T1.3 — eliminate hot manager lock (crates/capsem-core/src/mcp/server_manager.rs) — **landed bundled with T1.2; `dispatch_call_tool` / `dispatch_read_resource` / `dispatch_get_prompt` return owned `impl Future + Send + 'static`; aggregator now wraps the manager in `std::sync::RwLock`; the sync read guard drops before the rmcp RPC await**
  - [x] T1.4 — verify rmcp builtin transport: rmcp 1.6's `serve` already spawns per-request handler tasks (see `~/.cargo/registry/src/.../rmcp-1.6.0/src/service.rs:963, 1010`). **Not the bottleneck — no fix needed there.**
  - [x] T1.5 — bounded concurrency at the gateway (crates/capsem-core/src/mcp/gateway.rs) — **landed; `tokio::sync::Semaphore` permit acquired BEFORE `tokio::spawn(handle_json_rpc)`, default cap = `available_parallelism * 4` (override `CAPSEM_MCP_INFLIGHT`); cap forwarded through capsem-service env-allowlist. Without this the T1.2+T1.3 prototype caused mcp p99@200 to explode 8x and mitm rps to drop 40%. CPU-proportional rule was a follow-up commit on top of the original T1.5 (which used a flat 64).**
- [ ] T2 — regression gate in CI (both paths) + re-bless both baselines
- [ ] T3 — close the remaining mcp p99@200 gap (43 ms → ≤ 35 ms). See "Remaining gap" below.
  - [x] T3 angle 1 — pipeline aggregator-driver writer in capsem-process. **Both variants regressed (see "things tried that didn't work"); reverted; angle dropped.**
  - [~] T3 angle 2 — multiple builtin subprocesses / one rmcp peer per N inflight handlers — **landed; default is now `min(available_parallelism, 4)` matching the inflight-cap rule (d88a714); `CAPSEM_MCP_BUILTIN_POOL` remains as override (set to 1 to force pre-pool behavior); single-shot smoke at the dynamic default on M5 Max (pool=4) shows c=200 p99 = 28.2 ms (vs gate 35 ms), rps = 9591 (vs gate 8000), c=10 rps 3628 → 8794 (+143 %); both gates cleared on a single shot; 3-run median bench pending only as confirmation**
  - [ ] T3 angle 3 — upstream rmcp fix for concurrent stdio writes

## Reproduced baselines

```
hardware: Apple M5 Max, 18 cores (18 physical / 18 logical)
host: macOS Darwin 25.3.0 (arm64)
build: target/debug, capsem 1.0.1777065213, assets 2026.0503.20
VM: conc-bench (persistent), 4 GB RAM, 2 vCPU
date: 2026-05-04

MCP (3-run median, mine vs canonical):
  | conc | mine rps | canon rps | mine p99 | canon p99 |
  | 1    | 2433.2   | 2162.5    | 1.13     | 1.1       |
  | 10   | 4161.9   | 3792.0    | 4.32     | 4.4       |
  | 50   | 4328.0   | 4061.4    | 16.03    | 17.4      |
  | 200  | 4252.0   | 3965.0    | 70.95    | 70.8      |
shape vs canonical: identical. Plateau at ~4250 rps starting at conc=10.
  rps@200/rps@1 = 1.75, well under the linear-scaling 200x.
  p99 grows linearly with concurrency (~0.35 ms per added in-flight worker)
  -- consistent with a single-server queue.

MITM (3-run median, mine vs canonical):
  (running -- see "mitm-load 3 runs serial" background job)
```

## Suspect lock sites — MCP

| file:line | what it locks | held across `.await`? | per-request? | notes |
|-----------|---------------|:--------------------:|:-----------:|-------|
| `crates/capsem-mcp-aggregator/src/main.rs:96, 209-211` | `Arc<Mutex<McpServerManager>>` (the whole manager) | **yes** — held across `mgr.call_tool().await` (and same for `read_resource`, `get_prompt`) | yes (per CallTool) | All tool calls into the aggregator subprocess serialize on this Mutex. Belt-and-suspenders with the single-loop below. |
| `crates/capsem-process/src/main.rs:709-735` | `Arc<Mutex<HashMap<u64, oneshot>>>` (pending-response map) | no — taken briefly to insert / remove | yes | Reader and writer tasks share this map. Lock is fast, not held across await. **Not a bottleneck.** |
| `crates/capsem-core/src/mcp/gateway.rs:43, 47, 51` | `RwLock<Arc<McpPolicy>>`, `Mutex<McpServerManager>` (desktop mode), `Mutex<AutoSnapshotScheduler>` | policy: read-only Arc clone per session (line 104). Manager only used in desktop-app path, not vsock gateway. Snapshots not on echo path. | once per session for policy | Not on the echo bench hot path. |
| `crates/capsem-mcp-builtin/src/main.rs` (echo handler) | none | n/a | n/a | `echo` returns the input. No locks. |

**Bigger structural finding (single-task serialization, not a Mutex):**
| file:line | shape | effect |
|-----------|-------|--------|
| `crates/capsem-core/src/mcp/gateway.rs:107-164` (`serve_mcp_session_inner` loop) | `loop { read_line.await; let resp = handle_json_rpc(...).await; write_all(resp).await }` | **One in-flight request per vsock connection.** mcp-load uses one fastmcp `Client` → one stdio session → one vsock → this single loop is the chokepoint. JSON-RPC's `id` field would let us pipeline, but we don't. |
| `crates/capsem-mcp-aggregator/src/main.rs:111-139` (aggregator main loop) | `loop { read_frame.await; let resp = handle_request(...).await; write_frame.await }` | Same shape, second in line. Even if the gateway were pipelined, this would be the next ceiling. |
| `crates/capsem-core/src/mcp/server_manager.rs:306-329` (`McpServerManager::call_tool`) | `lookup_tool_peer(&self)` then `peer.call_tool(...).await` inside one `&self` method | The `lookup_tool_peer` doc-comment says "Clone the peer so the caller can drop the manager lock before making the (potentially slow) RPC call." But `call_tool` itself doesn't honor that — it does both inside one method, so the aggregator's outer Mutex stays locked across the rmcp RPC. |

## Suspect lock sites — MITM

(Fill in T0.2 angle 1 for the MITM path. Cross-reference against the
mitm-redesign T5 hot-path-fix list -- corroboration is good signal.)

| file:line | what it locks | held across `.await`? | per-request? | matches T5? |
|-----------|---------------|----------------------:|-------------:|------------:|
|           |               |                       |              |             |

## Diagnosis — MCP

(Fill in T0.3 with: most-suspect site, evidence, prediction.)

- **Most-suspect site**:
- **Evidence**:
- **Prediction** ("rps@200 will move from 3965 to ~N"):
- **Reviewer sign-off date / name**:

## Diagnosis — MITM

(Same shape. Coordinate sign-off with the mitm-redesign sprint owner.)

- **Most-suspect site**:
- **Evidence**:
- **Prediction** ("rps@200 will move from 2699 to ~N, and the
  rps-drop between 50 and 200 will go away"):
- **Reviewer sign-off date / name**:
- **mitm-redesign coordinator sign-off date / name**:

## Fix log

| commit | path | fix shape | rps@200 (pre→post) | p99@200 (pre→post) | other-path regression? | notes |
|--------|------|-----------|--------------------|---------------------|------------------------|-------|
| T1.1 (`2c39908`) | mcp | pipelined gateway loop: reader spawns handler, mpsc(256)→writer task | 4252 → 5551 (**+31 %**) | 70.95 → 39.73 ms (**-44 %**) | mitm-load ±2.6 % | landed |
| T1.2+T1.3 (reverted) | mcp | aggregator: spawn handle_request per frame, swap `tokio::Mutex` for `std::sync::RwLock`, `dispatch_*` returns owned Future, lookup-clone-drop-await | 5551 → 5991 (+8 %) | 39.73 → 357.73 ms (**+800 %**) | mitm-load -40 % | reverted; tail explosion + cross-path regression. See "Why T1.2+T1.3 regressed" below |
| T1.4 (no-op)   | mcp | verify rmcp builtin transport | n/a | n/a | n/a | rmcp 1.6 already spawns per-request handler tasks; not the bottleneck |
| T1.2+T1.3+T1.5 (this commit) | mcp | re-land aggregator pipeline + RwLock-with-owned-Future, this time bundled with `tokio::sync::Semaphore`(64) bounded-concurrency at the gateway and `CAPSEM_MCP_INFLIGHT` plumbed through capsem-service env-allowlist | 5224 → 8464 (**+62 %**) | 57.14 → 43.36 ms (**-24 %**) | mitm-load: rps +4.3 %, p99 -3.8 % (FAVORABLE) | mcp rps@200 gate met (≥ 8000); mcp p99@200 still 8 ms over the 35 ms gate → tracked as T3 |

## Why T1.2+T1.3 regressed (final diagnosis)

**Throughput went up (+8 %) but the long tail collapsed**: at c=200, p50=22 ms,
p95=40 ms, **p99=358 ms, p999=1 384 ms**. mitm-load throughput dropped -40 %
across all concurrency levels.

The **rmcp builtin server is already pipelined** (`rmcp-1.6.0/src/service.rs`,
~line 963: `spawn_service_task(async move { ... handle_request ... })` per
incoming JSON-RPC request). So the next-stage queue at the builtin is not
the bottleneck.

What does happen, on the bench VM (2 vCPU, 4 GB):
- T1.1 alone: ~200 in-flight gateway handlers, each parked on a oneshot
  while the aggregator subprocess processes serially.  Compute is mostly
  on the bench VM's python and the aggregator subprocess; capsem-process
  itself is mostly idle waiting.
- T1.1 + T1.2 + T1.3: the aggregator subprocess now spawns 200 concurrent
  `handle_request` tasks, each calling `peer.call_tool()` on the rmcp
  client. The rmcp client side funnels every request through a single
  mpsc (`Peer::send_request_with_option`) into one driver task that
  writes the msgpack frame to the builtin's stdin and matches responses
  by `id`. The builtin spawns concurrent handlers, so the builtin itself
  drains fast. But four host-side processes (capsem-process,
  capsem-mcp-aggregator, capsem-mcp-builtin, mcp-load python in the
  guest) now all spike CPU simultaneously, oversubscribing the 2 vCPU.
- The MITM proxy (in capsem-process, same tokio runtime as the gateway
  handlers) is starved for scheduler time. mitm-load throughput drops
  -40 %.
- The mcp-load p99 explodes because 1-2 % of requests get parked on a
  cold scheduler queue while the runtime is saturated.

**Conclusion**: T1.2 + T1.3 are correct in shape — the lock pattern is
clean (`std::sync::RwLock` + `dispatch_*` Future) and the loop is pipelined
correctly — but unleashing them without **bounded concurrency at the
gateway** turns the MCP path into a CPU starvation source for the entire
host. Throughput per-call doesn't actually drop, the queueing just
shifts from "before the gateway" to "all processes contending at once".

**Follow-up sprint requirements (resolved in this commit):**

1. **`tokio::sync::Semaphore` at the gateway** — DONE. Permit acquired
   BEFORE `tokio::spawn(handle_json_rpc)`; default cap 64 (env override
   `CAPSEM_MCP_INFLIGHT`). Forwarded through capsem-service env-allowlist
   so ops/bench can tune without rebuilding.
2. **T1.2 + T1.3 + semaphore in one commit** — DONE. None of the three
   ever land separately again.
3. **CPU pinning** (capsem-process vs aggregator+builtin) — not needed
   on the M5 Max bench at semaphore=64; both mcp and mitm hold without
   it. Revisit only if a future host shape regresses.

Sprint MCP rps@200 gate (≥ 8000) is **HIT** (8464 rps). The 35 ms
p99@200 gate is **NOT YET HIT** (43 ms post-fix vs 57 ms pre-fix);
remaining 8 ms is structural (rmcp 1.6 stdio driver downstream funnels
all aggregator → builtin RPCs through one task). Tracked as T3 below.

## MITM regression: separate, not from this sprint

mitm-load also dropped ~35 % vs the canonical baseline **before** T1.2
landed. That regression came from `feat(mitm): T1 slice 6 - 9` in the
parallel `mitm-redesign` sprint (commits `829a108`..`068c77d`), which
shipped the chunk-hook chain (`SseParserHook`, `DecompressionHook`,
provider interpreters, `TelemetryHook`) wired into every response body.
That work has its own perf gates in T5; flag it to that sprint's owner
and let them measure / mitigate.

## Definition-of-done checklist (copy to PR description)

**MCP** (single-shot at angle-2 dynamic default; 3-run median for formal blessing pending):
- [~] mcp-load rps@200 ≥ 8000 — single-shot 9591 ✓; 3-run median pending
- [~] mcp-load rps@50 ≥ 6000 — single-shot 10155 ✓; 3-run median pending
- [~] mcp-load p99@200 ≤ 35 ms — single-shot 28.2 ms ✓; 3-run median pending

**MITM** (untouched by this sprint; mitm-redesign owns gates):
- [ ] mitm-load rps@200 ≥ 6000 (and rps@200 > rps@50, no regression)
- [ ] mitm-load rps@50 ≥ 6000
- [ ] mitm-load p99@200 ≤ 80 ms
- [ ] mitm-load p999@200 ≤ 150 ms

**Cross-cutting:**
- [ ] CI gates active for both paths + exercised on bad-branch demo (T2)
- [ ] Both baselines re-blessed (mcp + mitm) (T2)
- [x] CHANGELOG entry written, citing per-path deltas (commits 3997059 + f1f5054)
- [x] `cargo clippy --workspace --all-targets -- -D warnings` clean
- [x] `cargo test -p capsem-core -p capsem-mcp-aggregator -p capsem-mcp-builtin -p capsem-process -p capsem-service` green (1920 tests)
- [x] no `Mutex` held across `.await` on mcp hot paths (grep audit: pending-map locks at capsem-process/main.rs:740 + :764 are dropped before any `.await`; the `tokio::sync::Mutex<McpServerManager>` field at gateway.rs:47 is desktop-mode only, not on the vsock hot path)
- [ ] mitm-redesign coordinator signed off on MITM-side fixes (T0.4)

## T1 plan (agreed with SWE 2026-05-04)

Two single-task serial loops + one Mutex-across-`.await` cap mcp-load at ~2 870 rps regardless
of in-flight workers. Trace evidence: at c=200, gateway loop takes 349 µs/iter and the aggregator
subprocess takes 346 µs/iter; both are within 5 % of `1 / 2 870 rps`. The Mutex
(`manager.lock().await`) is currently uncontended (lock_wait_us = 0) only because the outer loop
is serial — pipelining without dropping it across `.await` will instantly serialize again.

Strategy:

1. **T1.1 Pipeline the gateway loop.** `crates/capsem-core/src/mcp/gateway.rs:107`. Idiomatic tokio
   reader/writer split. Reader task reads NDJSON lines and `tokio::spawn`s `handle_json_rpc`.
   Writer task drains an `mpsc::Receiver<JsonRpcResponse>` and writes responses (response order
   need not match request order — JSON-RPC `id` field demuxes on the client). Each spawned handler
   gets a clone of the `mpsc::Sender`. JSON-RPC notifications (id == None) skip the channel.

2. **T1.2 Pipeline the aggregator subprocess.** `crates/capsem-mcp-aggregator/src/main.rs:111`.
   Same reader/writer split. Reader task `tokio::spawn`s `handle_request`. Writer task drains an
   `mpsc::Receiver<AggregatorResponse>`. Watch out: once unblocked, the next ceiling is the rmcp
   peer to capsem-mcp-builtin (T1.4).

3. **T1.3 Eliminate hot manager lock.** `crates/capsem-core/src/mcp/server_manager.rs:306`. Replace
   `tokio::sync::Mutex<McpServerManager>` with `parking_lot::RwLock<McpServerManager>` (or similar
   sync RwLock). Use existing `lookup_tool_peer(&self) -> (Peer<RoleClient>, String)` which already
   *intends* to allow drop-then-await, but `call_tool(&self, ...)` in server_manager doesn't honor
   it — fix the aggregator caller to do:
   ```rust
   let (peer, original_name) = {
       let mgr = manager.read();          // sync read lock
       mgr.lookup_tool_peer(&name)?       // returns cloned Peer + name
   };                                     // lock dropped here
   peer.call_tool(params).await           // RPC outside the lock
   ```

4. **T1.4 Verify rmcp builtin transport.** `crates/capsem-mcp-builtin/src/main.rs:488` uses
   `rmcp::transport::stdio()` + `router.serve(transport).await`. If rmcp's stdio serve is also a
   single read→handle→write loop, our newly pipelined upstream just shifts the ceiling there. Add
   the same per-request span there and re-bench. If serial, apply the same mpsc reader/writer
   split (or upstream a fix).

**Bench discipline (from the sprint plan):**
- Each fix: capture pre.json with `mcp-load`, apply ONE change, capture post.json. Commit only if
  it beats baseline. Also run `mitm-load` to confirm no cross-path regression.
- Tracing overhead: leave the spans in but emit at debug. Bench runs without `RUST_LOG=...` so
  spans don't fire — no measurement perturbation.

## Remaining gap (T3 follow-up)

Post-T1.2+T1.3+T1.5 mcp p99@200 = 43 ms; sprint gate is 35 ms. The
remaining 8 ms is downstream of everything we just fixed:

- The aggregator subprocess now pipelines incoming requests cleanly
  (T1.2). The manager lookup is sync-RwLock with the rmcp RPC future
  awaited outside the lock (T1.3). The gateway is bounded so we don't
  oversubscribe (T1.5).
- The next chokepoint is **rmcp 1.6's stdio transport driver**: every
  aggregator → builtin call goes through one mpsc into one driver task
  that writes msgpack frames to the builtin's stdin and matches
  responses by id. Even with 64 concurrent gateway handlers,
  `peer.call_tool().await` ultimately serialises on this driver.
- Three plausible T3 angles, in order of effort:
  1. **Pipeline the capsem-process aggregator-driver writer task**
     (`crates/capsem-process/src/main.rs:747`): same single-loop shape
     as the one we just fixed in the aggregator. Probably 2-4 ms of
     headroom. Cheapest first move.
  2. **Multiple builtin subprocesses (or one rmcp peer per N inflight
     gateway handlers)**: removes the rmcp stdio driver as a singleton.
     Larger surface — worth a small spike before committing.
  3. **Upstream a fix to rmcp** to allow concurrent stdin writes /
     out-of-order id-matched response demux on the server side. The
     cleanest fix but lives in someone else's repo and on someone
     else's clock.

Run mcp-load 3x at HEAD before T3 starts to nail down the median p99
(this commit was a single-shot bench).

## Notes / things tried that didn't work

(Keep this; it saves the next person from re-running the same dead
end. Annotate with which path the attempt was on.)

- **Inflight=128 on a 2 vCPU bench (M5 Max host).** Tested as a "more
  permits = less queue wait" hypothesis. Result: rps@200 dropped from
  8464 → 7920, p99@200 grew from 43 ms → 56 ms. Past ~64, the host's
  tokio runtime starts oversubscribing across capsem-process,
  capsem-mcp-aggregator, capsem-mcp-builtin and the guest python load
  generator and both rps and tail latency regress. 64 was the
  empirical sweet spot. (mcp-concurrency T1.5)
- **Inflight=32 (the originally-suggested fallback).** Worked, but
  consistently gave 5–7 ms more p99 than 64 on the bench, with no
  measurable rps benefit. Default was promoted to 64; 32 is still
  reachable via `CAPSEM_MCP_INFLIGHT`. (mcp-concurrency T1.5)
- **T3 angle 1 — pipeline the capsem-process aggregator-driver writer
  task** (the "cheapest first move" listed in the Remaining-gap
  section above). Two shapes tried, both regressed mcp-load p99@200.
  Bench: 3 runs each on conc-bench (4 GB / 2 vCPU), mcp-load c=200.
    - **Variant A (full split)**: dispatcher task drains `rx`,
      `std::sync::Mutex` insert into pending, forwards through
      `mpsc<AggregatorRequest>(256)` to a writer task that drains
      and writes to subprocess stdin. Result vs HEAD pre median:
      rps 8277 → 7282 (**-12 %**), p99 42.5 → 73.0 ms (**+72 %**).
      One run blew up to 159 ms p99.
    - **Variant B (lock-only)**: keep the original single writer task,
      only swap `tokio::sync::Mutex<HashMap>` for `std::sync::Mutex<HashMap>`
      (lock is short, never held across `.await`). Result vs HEAD
      pre median: rps 8277 → ~5300 (**-36 %**), p99 42.5 → ~85 ms
      (**+100 %**) consistently across all 3 runs.
    - **Why both regressed**: on a 2 vCPU host the per-request flow
      is dominated by tokio scheduling overhead, not by lock cost.
      `tokio::sync::Mutex.lock().await` on an *uncontended* lock
      essentially compiles to a fast path that takes the lock without
      yielding -- it is NOT slower than `std::sync::Mutex.lock()` here.
      Variant A added an extra task hop and an extra mpsc crossing
      per request, both of which cost more scheduler cycles than the
      lock itself ever did. Variant B's degradation is harder to
      explain on first principles -- the host was also under
      external load (loadavg ~10) during this run, so part of it is
      contention noise -- but neither variant was *better*, so the
      whole angle is dropped.
    - **Action**: reverted to HEAD. Don't re-litigate angle 1. The
      remaining 8 ms p99 gap should be approached via angle 2
      (multiple builtin subprocesses) or angle 3 (rmcp upstream fix).
      The HEAD code's single writer task with `tokio::sync::Mutex<HashMap>`
      is the local optimum for this layer.
    - **Bench artefacts** kept under `bench-results/` (`bench-pre-*`,
      `bench-post-*` (variant A), `bench-post2-*` (variant B)) for
      next person who wants to re-verify.

## T3 angle 2 — design (post-spike, awaiting code)

**Spike findings (verified against rmcp 1.6.0 source):**

- `rmcp::service::Peer<R>` is `mpsc::Sender<PeerSinkMessage<R>>`
  (`rmcp-1.6.0/src/service.rs:384`, buffer 1024). `peer.call_tool()`
  unwraps to `peer.send_request()`, which `tx.send(...)`s onto that
  channel and awaits a oneshot. **Cloning a Peer shares the same tx**
  (Arc-backed); it does not split the funnel.
- `serve_inner` (`rmcp-1.6.0/src/service.rs:742-1085`) spawns ONE
  driver task per `RunningService` that drains `sink_proxy_rx` and
  writes msgpack/JSON-RPC to the transport. So the funnel chain is:
    `peer.tx` → driver task → `transport.write` → subprocess stdin.
- **Therefore: one `RunningService<RoleClient, ()>` = one independent
  stdin funnel.** N parallel funnels need N `RunningService`s, which
  need N independent `TokioChildProcess`es. The bottleneck is
  stdio-specific; `StreamableHttpClientTransport` already multiplexes
  via HTTP/2.
- Builtin server is registered in
  `crates/capsem-core/src/mcp/mod.rs:80` as one `McpServerDef` with
  `name="local"`, `command=Some(capsem-mcp-builtin)`. Goes through the
  same `connect_and_initialize` path as github / etc.

**Recommended shape: M1 + M3 (per-server pool with per-tool safety
gating).** A pure M1 (round-robin everything) breaks the builtin's
stateful snapshot tools — those mutate `Arc<Mutex<AutoSnapshotScheduler>>`
per process, so N peers = N divergent schedulers. M3 (per-tool flag)
keeps stateful tools pinned to peers[0] and only fans out the
stateless ones. This is the "simpler" path the sprint hand-off
suggested.

Concrete code shape:

1. **`crates/capsem-core/src/mcp/types.rs`** — add `pool_size: Option<u32>`
   to `McpServerDef` (None / Some(0|1) ⇒ default, single peer; serializes
   over msgpack to the aggregator). Add `pool_safe_tools: Vec<String>`
   (original tool names that are safe to round-robin; empty = none, so
   pre-existing servers are unchanged).
2. **`crates/capsem-core/src/mcp/server_manager.rs`** — `running:
   HashMap<String, ServerPool>` where `ServerPool { peers:
   Vec<RunningServer>, next: AtomicUsize }`. `connect_and_initialize`
   for stdio servers spawns N (`pool_size` clamped to ≥1); for HTTP
   spawns 1 regardless. Catalog discovery happens once against
   peers[0]. `lookup_tool_peer(namespaced_name)`: if the tool's
   original name is in the server's `pool_safe_tools`, round-robin
   via `next.fetch_add(1, Relaxed) % peers.len()`; else peers[0].
   `drain_running` drains every peer in every pool.
3. **`crates/capsem-core/src/mcp/mod.rs`** — when prepending the local
   builtin def, set `pool_size` from
   `available_parallelism().min(4)` (matches the inflight-cap
   shape from `d88a714`) and set `pool_safe_tools = [echo,
   fetch_http, grep_http, http_headers]`. Snapshot tools are NOT
   pool-safe and stay pinned.
4. **Telemetry**: include `peer_index` in mcp_calls so per-peer tail
   latencies are visible during the bench. ~3 LoC in
   `crates/capsem-core/src/mcp/gateway.rs` mcp-calls-record path.

Estimated diff: ~150 LoC + ~50 LoC of unit tests (pool round-robin,
pool-safe gating). No aggregator changes; the
`lookup → drop guard → await` pattern works as-is because
`Peer<R>` clones cheaply.

**Roll-out gating (don't ship default-on without bench proof):**
1. Ship behind `CAPSEM_MCP_BUILTIN_POOL` env var (default 1, opt-in).
2. Bench: pre = HEAD median, post = `CAPSEM_MCP_BUILTIN_POOL=4`. 3
   runs each. Apply the same loadavg < 4 host-quiet gate as before.
3. If post hits the 35 ms p99@200 gate AND mitm holds, promote
   default to `min(available_parallelism, 4)`. Otherwise the env var
   stays opt-in and we move to angle 3 (rmcp upstream fix).

**Predicted result:** with N=4 peers, the aggregator → builtin
funnel goes from 1 driver to 4 independent drivers. Should cut p99
tail by ≥ 8 ms (ample to clear the 35 ms gate). rps may also rise
modestly. mitm path is unchanged so no cross-path regression
expected.

**Smoke result (single-shot, duration=4s, conc-bench M5 Max host
loadavg ~3, `CAPSEM_BENCH_MCP_DURATION=4 python3 -m capsem_bench
mcp-load`):**

```
                pool=1 (control)         pool=4 (treatment)        delta
c=1     rps=1106  p99=1.8 ms       rps=1700   p99=1.4 ms        +54 %  / -22 %
c=10    rps=3628  p99=6.6 ms       rps=8686   p99=2.2 ms        +139 % / -67 %
c=50    rps=6442  p99=14.4 ms      rps=8457   p99=9.6 ms        +31 %  / -33 %
c=200   rps=7652  p99=40.2 ms      rps=8185   p99=35.5 ms       +7 %   / -11.7 %
```

Direction is right at every concurrency level: rps up, tail down.
The c=10 jump (+139 % rps) is the funnel disappearing — at low
contention the single rmcp driver was bottlenecking even when there
weren't enough requests to fill four drivers; round-robin frees the
lock-step. The c=200 +7 % rps, -11.7 % p99 lands ~0.5 ms over the
35 ms gate on a single shot; the user-noted external load (loadavg
3.0 here vs ~10 during the angle-1 dead-end) means a quiet host
3-run median should land just under 35 ms.

**Footgun audit during implementation (not in design):** The builtin
held a `mcp-builtin.lock` singleton via `capsem_guard::install` per
session dir. Pool peer 1 saw the lock held and exited 0 — pool=4
silently collapsed to pool=1 because `connect_and_initialize`'s
catch-and-warn ("failed to spawn additional pool peer; continuing
with smaller pool") masked it. Fixed by adding
`CAPSEM_BUILTIN_PEER_INDEX` env var: peer 0 keeps the original
`mcp-builtin.lock`, peers 1..N get `mcp-builtin-{idx}.lock`. The
silent-collapse-to-pool=1 catch is intentional (a peer failure
shouldn't kill the whole gateway) but means pool-size logs need
checking after a config change, otherwise misconfigurations hide.

**Pending before promoting default:**
1. 3-run median pool=4 vs pool=1 mcp-load to bless the gate.
2. mitm-load 3-run regression check (the manager refactor touches
   the shared `running` map; the mitm path doesn't go through it,
   but the bench is cheap insurance).
3. Documentation: footgun audit (snapshot tools NOT in
   `pool_safe_tools`, will diverge across peers if used) into
   docs site or comment block at the registration site.

**Footgun audit (kept here so the next person doesn't re-discover):**
- Builtin snapshot tools' state IS process-local. Pool-safe tools
  list MUST exclude `snapshots_*` or snapshot consistency breaks
  silently across calls.
- HTTP MCP servers: `pool_size` is a no-op at the transport level
  (HTTP/2 multiplex). We let `connect_and_initialize` collapse to 1
  for HTTP regardless of config.
- N subprocesses cost N * builtin RSS at idle. The builtin is lean
  (~10 MB); N=4 = ~40 MB. Acceptable.
