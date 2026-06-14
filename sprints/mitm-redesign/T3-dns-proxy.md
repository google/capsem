# T3: dns-proxy

**Status:** Not Started
**Depends on:** T1, observability sprint W6 (`trace_id` on session.db tables — for `dns_events`)
**Blocks:** T5

## Goal

Replace the in-guest dnsmasq fake (`/#/10.0.0.1`) with a real `hickory-server`-based DNS proxy on the host. Port 53 redirects from the guest via iptables → vsock to host. Resolution is policy-aware: blocked domains return NXDOMAIN with a logged decision. Every query produces a `dns_events` row with `trace_id`. The vsock DNS envelope uses `rmp-serde` (length-framed, matches existing vsock control bridge convention).

## Deliverables

- `crates/capsem-core/src/net/dns/` — `mod.rs`, `server.rs` (hickory wiring), `resolver.rs` (upstream resolution), `server/tests.rs`.
- `crates/capsem-core/src/net/parsers/dns_parser.rs` — wraps `hickory-proto` parse; bytes ↔ `DnsQuery`/`DnsAnswer`.
- `crates/capsem-agent/src/dns_proxy.rs` — guest-side TCP-on-port-53 listener bridging to host via vsock with `rmp-serde` length-framed envelope.
- `crates/capsem-core/src/net/mitm/hooks/` (or in `dns/`) — `DnsRequestHook` (handles L1 DNS protocol), `DnsTelemetryHook` (L2 DnsAnswer → `dns_events` row).
- `crates/capsem-logger/src/schema.rs` — `dns_events` table migration (`rowid, ts, qname, qtype, qclass, rcode, decision, source_addr, upstream_resolver_ms, trace_id`).
- `crates/capsem-logger/src/events.rs` + `writer.rs` — `DnsEvent` struct + writer.
- `guest/artifacts/capsem-init` — iptables redirect for port 53; remove dnsmasq invocation.

## Acceptance

- A guest's `dig anthropic.com` returns a real answer through the proxy.
- A guest's `dig blocked-domain.com` returns NXDOMAIN; `dns_events.decision = "block"` row exists with non-NULL `trace_id`; `mitm.dns_queries_total{decision="block"}` counter incremented; `warn!` event on `target = "mitm.dns"` emitted.
- DNS round-trip parse fuzz (cargo fuzz target) survives 60s.
- `dns_parser/fixtures/*.rmp` corpora exist for: simple A query, AAAA query, EDNS, truncated, multi-question, NX response.
- `inspect-session` query joins `dns_events` to `net_events` on `trace_id` for a single curl invocation.
- dnsmasq is gone from the guest image; `ps aux | grep dnsmasq` in the VM returns nothing.
- `mitm-load` baseline regression check passes.

## Commit shape

Four expected commits:
1. `feat(dns): hickory-server resolver + policy hook in capsem-core` — DNS server + policy + parser.
2. `feat(dns): vsock DNS envelope (rmp-serde) + agent dns_proxy` — guest-side bridge.
3. `feat(dns): dns_events table + telemetry hook + trace_id` — logger schema + writer + hook.
4. `chore(guest): drop dnsmasq, redirect port 53 via iptables` — guest image change.
