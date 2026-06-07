# Sprint: concurrency

**Owner:** TBD (junior dev)
**Reviewer:** TBD (review the diagnosis BEFORE the fix)
**Coordinator:** the `mitm-redesign` sprint owner (for MITM-side
overlaps -- see "Coordination" below)
**Estimated effort:** 2–3 weeks
**Mission:** make the two host-side hot paths -- the **MCP transport**
and the **MITM proxy** -- scale closer to linear with concurrent
in-flight requests. The numbers say neither does today; we need to
know why, then fix it.

## TL;DR

Two pre-rewrite baselines, one per path. Both plateau. Both need
investigation. Both will give a junior dev a calibrated win if they
diagnose carefully.

### MCP path baseline -- `benchmarks/mcp-load/baseline.json`

| concurrency | rps    | p50_ms | p95_ms | p99_ms | p999_ms |
|------------:|-------:|-------:|-------:|-------:|--------:|
| 1           | 2162.5 | 0.4    | 0.6    | 1.1    | 2.3     |
| 10          | 3792.0 | 2.4    | 3.7    | 4.4    | 7.8     |
| 50          | 4061.4 | 12.0   | 13.9   | 17.4   | 31.9    |
| 200         | 3965.0 | 48.7   | 60.5   | 70.8   | 84.2    |

- 1 → 10: rps 2162 → 3792 = 1.75× (linear would be 10×).
- 10 → 50: rps 3792 → 4061 = 1.07× (linear would be 5×).
- 50 → 200: rps 4061 → 3965 = 0.98× (linear would be 4×).

Plateaus at ~4000 rps from concurrency 10 onwards.

### MITM path baseline -- `benchmarks/mitm-load/baseline.json`

| concurrency | rps    | p50_ms | p95_ms | p99_ms | p999_ms |
|------------:|-------:|-------:|-------:|-------:|--------:|
| 1           | 1037   | 0.9    | 1.4    | 2.3    | 3.3     |
| 10          | 3043   | 2.9    | 6.2    | 8.4    | 12.9    |
| 50          | 3029   | 13.5   | 34.9   | 53.4   | 157.6   |
| 200         | 2699   | 53.2   | 134.9  | 191.3  | 266.3   |

- 1 → 10: rps 1037 → 3043 = 2.93× (linear would be 10×).
- 10 → 50: rps 3043 → 3029 = -0.5% (linear would be 5×).
- 50 → 200: rps 3029 → 2699 = -10.9% (rps DROPS as concurrency grows).

Plateaus at ~3000 rps from concurrency 10 onwards, **and rps drops at
concurrency 200** -- the proxy actively gets worse under load. Tail
latency also grows much more aggressively than MCP (p999 of 266 ms at
concurrency 200 vs 84 ms for MCP). This is the worse of the two.

### Reproduce both

```
target/debug/capsem create --name conc-bench --ram 4 --cpu 2
# Update bench files in workspace so you don't need a rootfs rebuild:
for f in __init__.py __main__.py helpers.py mitm_load.py mcp_load.py; do
  target/debug/capsem cp guest/artifacts/capsem_bench/$f \
    conc-bench:capsem_bench/$f
done

# MCP run:
target/debug/capsem exec conc-bench \
  "cd /root && PYTHONPATH=/root python3 -m capsem_bench mcp-load \
   && cp /tmp/capsem-benchmark.json /root/mcp-yours.json"
target/debug/capsem cp conc-bench:mcp-yours.json /tmp/mcp-yours.json

# MITM run:
target/debug/capsem exec conc-bench \
  "cd /root && PYTHONPATH=/root python3 -m capsem_bench mitm-load \
   && cp /tmp/capsem-benchmark.json /root/mitm-yours.json"
target/debug/capsem cp conc-bench:mitm-yours.json /tmp/mitm-yours.json

target/debug/capsem delete conc-bench
```

Your numbers should look qualitatively the same shape on both runs.
If absolute rps differs by >2× from the canonical baseline you're on
different hardware -- fine, the scaling shape is what matters.

**Find each plateau. Fix each. Beat the baseline.**

## Scope

In scope, both paths:

**MCP path:**
```
fastmcp.Client (guest python)
  -> stdio -> /run/capsem-mcp-server (guest agent's MCP server;
     find it with `rg "MCP server\|mcp_server" crates/capsem-agent/`)
  -> vsock:5003 -> capsem-mcp-aggregator (host,
     `crates/capsem-mcp-aggregator/`)
  -> stdio -> capsem-mcp-builtin (host subprocess,
     `crates/capsem-mcp-builtin/`)
  -> echo handler returns text -> reverse chain
```

**MITM path:**
```
guest curl -> iptables REDIRECT -> capsem-net-proxy (guest, port 10443,
   `crates/capsem-agent/src/net_proxy.rs`)
  -> vsock:5002 -> Host MITM proxy
     (`crates/capsem-core/src/net/mitm_proxy/`)
  -> SNI parse -> domain policy check (now via Hook pipeline,
     `mitm_proxy/pipeline.rs`)
  -> TLS terminate (rustls + per-domain cert minted from CA in
     `mitm_proxy/cert_authority.rs`)
  -> upstream TCP+TLS dial -> hyper request forward -> response stream
```

For both:
- Any `Mutex` / `RwLock` / `tokio::sync::Mutex` on the hot path that
  could serialize concurrent calls.
- Any `single-stream` framing (one writer task, one reader task) that
  cannot pipeline N independent requests.
- Any `tokio::spawn`-vs-`block_on` mix that throws work onto a single
  task instead of fanning out.

### Coordination with `sprints/mitm-redesign/`

The MITM hot-path-fix list in `sprints/mitm-redesign/T5-hardening.md`
already names several MITM concurrency fixes (cert cache `OnceCell`
or `moka`, upstream connection pool, `AtomicU64` body counters,
off-task telemetry emission). **You do not have to invent these from
scratch -- they're written down.** But T5 is the LAST phase of that
sprint, so until it lands the proxy is plateau'd. Two rules to
prevent stepping on toes:

1. **Diagnose MITM independently first.** Don't read the T5 list
   before T0 here. Form your own diagnosis from `mitm-load` data,
   then check it against the T5 list. If they match, the diagnosis
   is corroborated; if they diverge, you've found something the
   redesign sprint missed -- bring that to the mitm-redesign
   coordinator.
2. **Coordinate the fix.** Before opening a PR for any MITM fix,
   ping the mitm-redesign sprint owner. Either:
   - they say "go ahead, this lands here, I'll merge T5's named-fix
     entry into your PR description", or
   - they say "wait, T1 needs to land first because X", and you
     pivot to MCP work in the meantime.
   Do NOT just merge MITM concurrency fixes without that coordination
   -- they affect the surface area `mitm-redesign` is rewriting.

**Out of scope** for this sprint:
- Adding new MCP or MITM features. This sprint changes how existing
  requests flow, not what they do.
- Replacing JSON-RPC or the vsock framing with a binary protocol
  (msgpack / capnp / protobuf). Would help, but it's a full protocol
  rewrite -- separate sprint, separate review.
- Cross-process tracing infrastructure. Use what
  `sprints/done/observability-stop-the-bleeding/` already shipped
  (`trace_id` propagates guest → aggregator → builtin via
  `CAPSEM_TRACE_ID` env + W5 in-band traceparent).

## Phase T0 — diagnose (do NOT fix anything yet)

Goal: identify the serialization point in EACH path with evidence,
not guesses. Reviewer signs off on the diagnosis before any code
change lands. **You diagnose both paths in T0**; you may decide in T1
to fix one before the other based on which has a clearer fix.

### T0.1 — confirm both baselines reproduce

Use the reproduction recipe from the TL;DR. Run each bench 3× and
take the median to filter noise. Record both numbers in `tracker.md`
under "Reproduced baselines" alongside the canonical numbers.

**Deliverable**: paragraph in `tracker.md` saying "I reproduced the
MCP plateau at ~Nrps and the MITM plateau at ~Mrps on <hardware>;
shape matches the canonical baseline."

### T0.2 — instrument both paths

Same three angles for each path, in order of effort. Do MCP first
(easier to read because it's small), then MITM.

**Angle 1 — count the locks (cheapest, do first)**

```
# MCP path
rg --type rust 'tokio::sync::Mutex|std::sync::Mutex|RwLock' \
   crates/capsem-agent/src/ \
   crates/capsem-mcp-aggregator/ \
   crates/capsem-mcp-builtin/ \
   crates/capsem-core/src/mcp/

# MITM path
rg --type rust 'tokio::sync::Mutex|std::sync::Mutex|RwLock' \
   crates/capsem-agent/src/net_proxy.rs \
   crates/capsem-core/src/net/mitm_proxy/ \
   crates/capsem-core/src/net/cert_authority.rs
```

For each hit, ask: is this lock held across an `.await` on the hot
path? Is it acquired per-request? Write each candidate site (with
file:line) to `tracker.md` under the per-path "Suspect lock sites"
table. Don't fix anything yet.

**Angle 2 — tracing spans on the hot path**

The observability sprint already gave us `#[instrument]`. Add spans
covering each path:

- **MCP**: the guest agent's MCP server (stdio→vsock forward); the
  aggregator's vsock-receive + dispatch; `BuiltinHandler::echo`.
  Span target: `mcp.transport`, fields `direction`, `request_id`,
  `bytes`, `duration_ms`.
- **MITM**: cert mint in `cert_authority.rs`; upstream dial in
  `mitm_proxy/mod.rs` near the `tokio::sync::Mutex<_>` upstream
  cache; telemetry emission in `mitm_proxy/telemetry.rs`. Span
  target: `mitm.transport`, fields `domain`, `kind`, `duration_ms`.

Run loads at `--concurrency 50` and `--concurrency 200` with
`RUST_LOG=mcp.transport=debug,mitm.transport=debug`, extract the
logs (`capsem cp <vm>:/var/log/capsem-agent.log /tmp/agent.log`,
`tail -f ~/.capsem/run/service.log` for the host).

Look for:
- Spans whose `duration_ms` **grows** with concurrency level (= lock
  contention or queue wait). This is the smoking gun.
- Span counts that stay constant when you raise concurrency (= the
  step is serialized; only one in-flight at a time).

**Angle 3 — tokio-console**

If angles 1+2 don't pinpoint the issue, install `tokio-console` and
profile the host-side aggregator + the MITM proxy task under load.
Look for tasks that are "busy" near 100% (CPU bound) vs "blocked"
(lock or channel bound). Tokio-console requires the
`tokio_unstable` cfg and has setup gotchas -- ask the reviewer for
help here.

**Deliverable**: `tracker.md` "Diagnosis" sections (one per path):
1. The single most-suspect serialization point + file:line.
2. Evidence: a paragraph describing what you saw in spans / locks /
   tokio-console output.
3. A *prediction*: "I think rewriting <X> as <Y> will move the
   plateau from ~Nrps to ~Mrps." Don't be afraid to be wrong; we'll
   measure.

**Get reviewer sign-off before T1.** Sit down with reviewer (and the
mitm-redesign coordinator if your MITM diagnosis touches their
roadmap) for ~30 min, walk through both diagnoses, get yes / no /
keep-digging on each.

## Phase T1 — fix one thing at a time

Goal: each commit changes one suspect site. Bench before, bench after,
commit only if better. Avoid bundle-fixes -- if you change three
things at once and the number stays flat, you don't know which change
helped.

### T1.X — fix shape (one per fix, per path)

Per fix:
1. Capture pre-fix bench (the relevant one for the path you're
   touching): `capsem-bench mcp-load` OR `capsem-bench mitm-load`
   → JSON, copy out via `capsem cp`. Save as `/tmp/pre.json`.
2. Make the change. Keep it small. If it grows past ~50 lines you
   probably need to split.
3. `cargo test -p <crate> --lib` green.
4. Capture post-fix bench, save as `/tmp/post.json`.
5. **Run the OTHER path's bench too** to confirm you didn't regress
   it. (E.g., a fix to MITM cert minting must not slow down the MCP
   path; if it does, the change has unintended cross-effects.)
6. Compare:
   ```
   python3 - <<EOF
   import json, sys
   key = sys.argv[1]   # 'mcp_load' or 'mitm_load'
   pre = {r['concurrency']: r for r in json.load(open('/tmp/pre.json'))[key]['concurrency_levels']}
   post = {r['concurrency']: r for r in json.load(open('/tmp/post.json'))[key]['concurrency_levels']}
   for c in [1, 10, 50, 200]:
       p, q = pre[c], post[c]
       print(f"  c={c:3d}  rps {p['rps']:7.1f} -> {q['rps']:7.1f}  "
             f"p99 {p['p99_ms']:6.1f} -> {q['p99_ms']:6.1f}  "
             f"({(q['rps']/p['rps']-1)*100:+.1f}% rps)")
   EOF
   ```
7. Commit: `perf(mcp): ...` or `perf(mitm): ...` with the rps/p99
   deltas (both paths, even if one is unchanged) in the commit body.

### Likely fix shapes (suggestive only -- diagnose first!)

Don't grab this list and start coding. Diagnose first.

**MCP path candidates**:

- **Reader/writer split on a single stdio stream**. If
  capsem-mcp-server's stdio transport reads + writes from the same
  task with a Mutex, every concurrent in-flight call serializes. Fix:
  one read task, one write task, channel between them, request_id
  multiplexing (which JSON-RPC supports natively).
- **Single vsock:5003 connection multiplexed via Mutex**. If the
  aggregator opens one vsock connection and locks it per request,
  N concurrent calls block on the lock. Fix: connection pool
  (`bb8`/`deadpool` style), or a single connection with
  request-id-keyed response routing (so reads/writes don't lock).
- **Subprocess stdio**. If aggregator spawns ONE
  capsem-mcp-builtin subprocess and serializes calls through its
  stdin/stdout, that's the bottleneck. Fix: subprocess pool, or
  async pipelined dispatch on the same subprocess.

**MITM path candidates** (mostly cribbed from `mitm-redesign T5`):

- **Cert cache `RwLock<HashMap>` mint-on-miss**. Concurrent
  first-time hits to the same domain serialize on the write lock.
  Fix: `tokio::sync::OnceCell` per domain (or
  `moka::future::Cache::get_with`) so concurrent waiters share one
  mint.
- **Per-connection upstream sender wrapped in `tokio::sync::Mutex`**.
  Pipelined requests on a keep-alive connection serialize on the
  lock. Fix: connection pool keyed by `(domain, port)` with
  `bb8`/`deadpool` checkout.
- **`Arc<Mutex<BodyStats>>` on every body poll**. Lock churn for
  what is effectively two atomics. Fix: `AtomicU64` counters; lock
  only at terminal emission.
- **Telemetry emission inline in `TelemetryBody::poll_frame`'s
  end-of-body**. Blocks response cleanup. Fix: hand off to a
  dedicated telemetry executor task via channel; parsing + DB write
  happen off the connection task.

**Shared / cross-cutting**:

- **`tokio::sync::Mutex` held across `.await` of an upstream call**.
  Cardinal sin -- the lock blocks the entire scheduler. Fix: drop
  the lock before awaiting, restructure to extract the work outside
  the lock.
- **`std::sync::Mutex` (blocking!) in async code**. If you find one
  on the hot path, that's a bug. Replace with `tokio::sync::Mutex`
  if the lock crosses await boundaries, or atomics if not.

For each shape: if you find it, document the chosen fix in tracker.md
BEFORE coding. The reviewer should be able to predict your diff from
your prose.

## Phase T2 — regression gate in CI

Goal: lock in the gain so we don't lose it next quarter.

1. Add CI jobs that run **both** `capsem-bench mcp-load` and
   `capsem-bench mitm-load` after the test suite passes. Today CI
   doesn't run benches; this sprint adds the recipes.
2. Compare against the corresponding baseline JSON
   (`benchmarks/mcp-load/baseline.json`,
   `benchmarks/mitm-load/baseline.json`) using a small python script
   (start from the diff snippet in T1).
3. **CI gate rule**: per path, if `rps` drops by >10% OR `p99_ms`
   rises by >25% at any concurrency level vs the baseline, fail the
   build. Coordinate with `mitm-redesign T5` -- their gate may be
   stricter; use the stricter of the two.
4. After the sprint passes, **re-bless both baselines**:
   ```
   # MCP
   target/debug/capsem cp <vm>:mcp.json benchmarks/mcp-load/baseline.json
   # MITM
   target/debug/capsem cp <vm>:mitm.json benchmarks/mitm-load/baseline.json
   git add benchmarks/mcp-load/baseline.json benchmarks/mitm-load/baseline.json
   git commit -m "bench: re-bless mcp-load + mitm-load baselines post-sprint"
   ```
   Include the before/after tables (both paths) in the commit body.

## Definition of done

The sprint closes when ALL of these are true:

**MCP gates:**
- [ ] `capsem-bench mcp-load` rps@200 ≥ **8000** (2× the 3965
      baseline; better is fine).
- [ ] mcp-load rps@50 ≥ **6000** (1.5× of 4061).
- [ ] mcp-load p99@200 ≤ **35 ms** (½ of 70.8).

**MITM gates:**
- [ ] `capsem-bench mitm-load` rps@200 ≥ **6000** (~2.2× the 2699
      baseline; the regression at concurrency 200 must be gone --
      rps must INCREASE, not decrease, between concurrency 50 and
      200).
- [ ] mitm-load rps@50 ≥ **6000** (~2× of 3029).
- [ ] mitm-load p99@200 ≤ **80 ms** (~½ of 191.3).
- [ ] mitm-load p999@200 ≤ **150 ms** (~½ of 266.3 -- worst tail
      improvement).

**Cross-cutting:**
- [ ] CI gates from T2 active and exercised on a deliberately-bad
      branch (verify both fail when they should).
- [ ] Both baselines re-blessed with post-sprint numbers + a
      CHANGELOG entry naming the fix(es) and citing the deltas for
      each path.
- [ ] All workspace tests still green (`just test`).
- [ ] No new `Mutex`-across-`.await` on the hot path. Run:
      ```
      rg 'lock\(\).*\.await|read\(\).*\.await|write\(\).*\.await' \
         crates/capsem-mcp-* crates/capsem-core/src/net/mitm_proxy/ \
         crates/capsem-agent/src/
      ```
      and inspect every hit on the request path. There should be none.

If you cannot hit the rps/p99 targets, do not silently lower them.
Open a follow-up sprint stub explaining what's left and why.

## Common traps

- **Measuring a cold cache.** Always run a warm-up call
  (`local__echo {"text":"warmup"}` for MCP; one curl for MITM)
  before timing. The first call pays the JIT import + subprocess
  spawn + vsock handshake / TLS handshake cost.
  `capsem-bench mcp-load` does this internally; `mitm-load` should
  too -- check.
- **Measuring noise.** Run each level for at least 10s. If your
  result varies >20% between runs, run it 3× and take the median.
- **Optimizing the hot tool, not the hot path.** `local__echo` is a
  no-op; the MITM bench target is non-routable. Anything you "speed
  up" in the echo handler or the upstream-fail path itself is
  measurement noise. The paths under test are the transports.
- **Premature commit.** Don't commit a fix that doesn't beat the
  baseline. If the diff makes things worse or has no effect, leave
  it on a branch and write up what you tried in tracker.md.
- **Touching one path while breaking the other.** A naive cert-mint
  fix that allocates per-call could regress MCP if it changes a
  shared allocator behavior. Always run BOTH benches after any fix.
- **Fighting `mitm-redesign` over the same MITM file.** If you find
  yourself rewriting `cert_authority.rs` and the mitm-redesign
  coordinator is mid-refactor, stop and resync. Their sprint owns
  the structural shape; you own the concurrency primitives within
  it.

## How to ask for help

Push your branch, link tracker.md, ping reviewer. Bring:
1. Your most recent before/after rps + p99 table.
2. The single suspect site (file:line) you're investigating.
3. The specific error / confusion you're stuck on.

Don't apologize for asking. The diagnosis phase especially benefits
from a second pair of eyes -- this is normal and expected.

## Files you will likely touch

(This is a hint, not a contract. Diagnose first.)

**MCP path:**
- `crates/capsem-agent/src/` — the in-guest MCP server that bridges
  python stdio to vsock:5003.
- `crates/capsem-mcp-aggregator/src/main.rs` — host-side dispatcher
  that accepts vsock:5003 connections and routes to subprocesses.
- `crates/capsem-mcp-builtin/src/main.rs` — host-side builtin tool
  subprocess (already async via rmcp).
- `crates/capsem-core/src/mcp/gateway.rs` — telemetry hook for
  `mcp_calls` rows; check whether mcp_calls writes block the hot
  path.

**MITM path** (coordinate with mitm-redesign):
- `crates/capsem-core/src/net/mitm_proxy/cert_authority.rs` — cert
  cache + mint-on-demand.
- `crates/capsem-core/src/net/mitm_proxy/mod.rs` — `handle_request`
  + the per-connection upstream sender mutex.
- `crates/capsem-core/src/net/mitm_proxy/body.rs` —
  `Arc<Mutex<BodyStats>>` on the body poll path.
- `crates/capsem-core/src/net/mitm_proxy/telemetry.rs` —
  `TelemetryBody` + the inline `TelemetryEmitter::emit` called from
  `poll_frame`.

## Out-of-scope follow-ups (do not do here)

- A `mitm-mcp-bridge` sprint to make the new T1 hook surface
  (`sprints/mitm-redesign/`) emit `mcp_calls` rows from the MITM as
  well as the gateway. That's T4 of the mitm-redesign meta-sprint.
- Migrating MCP transport to a length-framed binary protocol
  (replace JSON-RPC with msgpack or capnp). Would help, but it's a
  full protocol rewrite -- separate sprint, separate review.
- Adding a third concurrency surface (DNS proxy from
  mitm-redesign T3). DNS doesn't exist yet; revisit once T3 lands.
