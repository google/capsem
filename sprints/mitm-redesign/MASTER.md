# Meta-sprint: mitm-redesign

Decompose the 2847-line `mitm_proxy.rs` monolith into a hookable pipeline with first-class plain HTTP, a real DNS proxy, MCP protocol awareness, performance gates, and a hook surface shaped to host a future security engine (credential rewrite is the proof case but is out of scope here).

Full plan: `/Users/elie/.claude/plans/revisit-the-mitm-sprint-tender-rainbow.md` (load-bearing source of truth; this file is the index + status board).

## Status

| Phase | Name | Status | What it delivers | Depends on |
|---|---|---|---|---|
| **T0** | [extract-and-re-baseline](T0-extract-baseline.md) | Not Started | Split `mitm_proxy.rs` into `mitm/{listener,connection,tls,http,events,body,telemetry,metrics}.rs`. Move AI parsers to `parsers/` + `interpreters/` with sibling `tests.rs`. Wire `init_telemetry`. Commit pre-rewrite criterion + mitm-load baselines. **Zero behavior change.** | observability sprint W2 |
| **T1** | [pipeline-and-hook-traits](T1-pipeline-hooks.md) | Not Started | Single `Hook` trait + `Event<'_>` enum + `EventMask` + `HookCtx::emit()`. Rewire policy/decompression/telemetry/AI-interpretation as hooks. Logging contract + `metrics` counters/histograms wired at every seam. | T0 |
| **T2** | [protocol-demux-plain-http](T2-plain-http.md) | Not Started | Peek-based TLS vs. HTTP demux. iptables redirects port 80 + configurable list. Host-header policy on plain HTTP. Brotli + zstd decompression. End-to-end Ollama smoke. | T1 |
| **T3** | [dns-proxy](T3-dns-proxy.md) | Not Started | `hickory-server` DNS resolver in capsem-process. Port 53 iptables redirect. Policy-aware resolution. New `dns_events` table with `trace_id` from day one. vsock DNS envelope uses `rmp-serde`. Drop dnsmasq. | T1, observability W6 |
| **T4** | [mcp-protocol-aware-mitm](T4-mcp-aware.md) | Not Started | `JsonRpcParserHook` (L1→L2) + `McpInterpreterHook` (L2→L3). Emit `mcp_calls` from MITM for HTTP-transport MCP. Populate `tool_calls.mcp_call_id` FK. | T1 |
| **T5** | [hardening-perf-coverage](T5-hardening.md) | Not Started | Adversarial suite + `cargo fuzz` for each parser. Coverage gate (≥80% mitm, ≥90% parsers/interpreters). Performance regression CI: `critcmp` + `mitm-load` p99. No-blocking-on-async, telemetry-backpressure, memory-bound tests. Hot-path fixes proven by bench numbers. | T2, T3, T4 |

The credential-rewrite phase originally numbered T5 is **out of scope** for this meta-sprint. It lands in its own sprint that owns the security engine. This sprint guarantees the hook surface is shaped to host it without trait changes.

## Phase grouping

**Phase A — foundation (T0 + T1)**: extract, reorganize, introduce hook traits, wire observability + metrics.
**Phase B — protocol surface (T2 + T3 + T4)**: plain HTTP, DNS, MCP — three parallelizable phases each adding a protocol axis.
**Phase C — hardening (T5)**: adversarial coverage, fuzz, perf regression gates. Lands once the surface area is stable.

## Decisions (confirmed during planning)

1. **Hand-rolled spine** on `rustls + hyper + tokio`. `hickory-server` is the only new heavyweight dep (DNS only, T3). No external MITM framework on the security boundary.
2. **Full hickory DNS proxy in T3.** Drops dnsmasq.
3. **All of T0–T5 in one meta-sprint.** Hardening ships with the new surface area.
4. **One `Hook` trait + layered `Event` enum.** Parsers are hooks; higher-level consumers are hooks; same dispatch path. L1 (raw) / L2 (protocol) / L3 (semantic) ladder, statically prevents re-emit cycles.
5. **L1 mutation allowed; L2/L3 mutation allowed but no wire writeback yet.** Trait shape supports the future security engine's regex-based body content rewrite without breaking changes.
6. **MessagePack** only for the new vsock DNS envelope and parser test fixtures. Not on HTTP/TLS/JSON-RPC/SSE wires.
7. **Each parser in its own file** with sibling `tests.rs`, ≥40 unit tests each, replay corpora in `*.rmp`.

## Just recipes relevant

```bash
just test                           # all gates incl. new mitm tests
just smoke                          # in-VM smoke after each phase
just bench                          # capsem-bench incl. new mitm-load mode (after T0)
just inspect-session                # joins net_events/model_calls/mcp_calls/dns_events on trace_id
cargo bench -p capsem-core          # criterion microbenches (after T0)
```

## Conventions

- Each phase = its own sub-sprint stub today, filled out at kickoff.
- Tracker in `tracker.md` shows the active phase.
- Commits update tracker + CHANGELOG in the same commit as the code change.
- Stage files explicitly (no `git add -A`).
- Author: Elie Bursztein <github@elie.net>. No `Co-Authored-By` trailers.
- Conventional commit prefixes: `sprint(mitm-redesign):`, `feat(mitm):`, `fix(mitm):`, `test(mitm):`, `bench(mitm):`, `chore(mitm):`.

## Sequencing note

Observability sprint W1–W6 + T1–T5 **landed** (commits dd8ba29..18ca6c3). `crates/capsem-core/src/telemetry.rs` ships `init`, `child_trace_env`, `current_parent_traceparent`, `ambient_capsem_trace_id`. T0 wires those directly — no stub needed.

W6 already added `trace_id` columns to `mcp_calls`, `net_events`, `fs_events`, `snapshot_events`, `tool_calls`, `tool_responses`, `audit_events`. T3's new `dns_events` table just follows the same pattern.
