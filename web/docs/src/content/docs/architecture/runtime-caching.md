---
title: Runtime Caching
description: Shared cache mechanics, domain-owned freshness, metrics, and performance proof.
sidebar:
  order: 7
---

Capsem uses `capsem-cache` as the control plane for in-memory runtime caches.
It provides bounded concurrent storage, single-flight loading, identity and
generation snapshots, invalidation, and one set of metrics. The implementation
uses the maintained Rust `quick_cache` crate rather than another local map and
eviction policy.

Cache mechanics are shared. Freshness remains with the subsystem that knows
what a change means. A DNS answer expires by TTL and policy identity, a ledger
query expires when the database read epoch moves, and a VM list expires when
its complete source identity changes. Treating those as one universal TTL
would make the code uniform by making its answers wrong.

## Shared primitives

| Primitive | Use |
|---|---|
| `BoundedCache` | Concurrent keyed values with a fixed entry bound, optional freshness predicate, single-flight miss loading, and removal or retention. |
| `IdentityCache` | One snapshot that is reusable only while its caller-supplied identity is unchanged. |
| `KeyedIdentityCache` | A bounded set of keyed snapshots, each reusable only for its identity. |
| `GenerationCache` | One snapshot published only if the generation observed before loading is still current. |
| `CacheMetrics` | The same counters and metric labels for a domain cache whose async or database locking must remain specialized. |

Values are shared through `Arc`, capacities are mandatory, and a concurrent
miss for the same bounded-cache key performs one load. Refreshing an existing
key is distinct from a capacity eviction, so operational counters explain why
an entry changed.

## Freshness owners

| Cache | Freshness rule | Shared mechanism |
|---|---|---|
| Service plugin policy | Explicit policy generation | `GenerationCache` |
| Asset status and VM list | Complete input identity | `IdentityCache` |
| Persistent resume state | VM ID plus persisted-state fingerprint | `KeyedIdentityCache` |
| Storage diagnostics | Session path and explicit invalidation | `BoundedCache` |
| DNS answers | Lookup key, policy identity, TTL, and negative-answer TTL | `BoundedCache` with a freshness predicate |
| TLS leaf certificates | Normalized server name | `BoundedCache` with single-flight minting |
| Ledger query batches | Canonical query batch plus logger-owned read epoch | `KeyedIdentityCache` inside `capsem-logger` |
| Image catalog | Source, 30-minute refresh, one-minute failure retry, and last-good fallback | Domain async lock with `CacheMetrics` |

The logger remains the storage and execution owner for session data. Service,
gateway, UI, and MCP routes do not open SQLite or keep serialized ledger
responses. `capsem-logger` obtains freshness from its writer generation or
SQLite `data_version`, then keys its bounded query cache with that read epoch.
A WAL-only commit therefore invalidates results even when the database file's
size and modification time do not change.

Several structures deliberately sit outside this cache control plane:

- The OCI image store is durable content-addressed storage with receipts,
  reconciliation, and disk retention policy.
- The credential store's memory map is authoritative runtime state, including
  after a durable write fails; evicting it could change authentication.
- Gateway `/status` keeps previous VM states for transition events and uses a
  refresh gate to coalesce polls. Every response still comes from a service
  read that began after that request arrived.
- Registries of live processes, grants, pending requests, and locks own active
  resources. They are lifecycle state rather than reusable computed values.

## Metrics

Every shared cache name uses the same bounded label vocabulary:

| Metric | Labels | Meaning |
|---|---|---|
| `cache.lookups_total` | `cache`, `result=hit|miss` | Reuse effectiveness. |
| `cache.mutations_total` | `cache`, `operation=insert|refresh|invalidate|evict|load_error` | Why resident state changed. |
| `cache.load_duration_ms` | `cache`, `status=success|error` | Work performed after a miss. |
| `cache.entries` | `cache` | Current resident entry count. |

The primitives also expose a `CacheStats` snapshot with hits, misses,
insertions, refreshes, invalidations, evictions, load errors, and entries for
focused tests. Cache names identify the owner, such as `core.dns_answer`,
`core.tls_leaf`, `logger.query_many`, and `service.image_catalog`.

## Performance proof

Polled API reads must remain well below one millisecond at the service boundary
where possible. Cache work is measured through the repository benchmark
system: run the unchanged tree twice, run the candidate twice, and compare the
latest two authoritative runs from each SQLite store. Quick runs remain useful
for development but are excluded from evidence.

See [Benchmarking](/development/benchmarking/) for the commands, store format,
noise rules, and route coverage contract.
