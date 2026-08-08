# Capsem v0.7 sprint

This is the only active checked-in Capsem sprint. The architecture backbone is
[`design.md`](design.md); its companion JSON Schema and OpenAPI snapshots live
beside it and are reviewed as one package.

The package is a T0 design baseline, not implemented API authority. As each
tranche lands, typed Rust registries generate the checked-in contract snapshots
and the running binaries serve the same generated OpenAPI documents.

Use Sprinty for live item state. Use [`tracker.md`](tracker.md) as the durable
cross-machine handoff when Sprinty process state is unavailable.
