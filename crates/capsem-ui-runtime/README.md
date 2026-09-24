# capsem-ui-runtime

Reusable Rust runtime boundary for Capsem workspace records, projections,
checkpoints, topology, and editable UI state.

This crate is the production dependency seam for mainline integration. In this
prototype sprint it intentionally re-exports the already-tested workspace and
editable-state implementation from `capsem-ui-catalog`; the next extraction
slice should move ownership here once artifact types are split cleanly from the
catalogue/tooling crate.

The rule for route, MCP, and service code is: depend on `capsem-ui-runtime` for
records, projection, replay, checkpoints, and Loro editable state. Depend on
`capsem-ui-catalog` only for UI catalogue/tool helpers and artifact construction
helpers that still live there during the prototype.
