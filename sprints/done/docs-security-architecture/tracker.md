# Sprint: Security & Architecture Documentation

## S1: Security Pages

### Kernel Hardening
- [x] Write threat model intro (what an unhardened guest kernel enables)
- [x] Defconfig table: disabled subsystems (modules, debugfs, devmem, BPF, io_uring, etc.)
- [x] Memory mitigations section (init_on_alloc, slab hardening, page shuffling, usercopy)
- [x] Architecture-specific hardening (x86: KPTI, retpoline, CET; arm64: BTI, PAC, KASLR)
- [x] Boot cmdline params with rationale
- [x] Validation section mapping capsem-doctor tests to each property
- [x] Review pass: verify every claim against defconfig.arm64 and defconfig.x86_64

### Network Isolation
- [x] Air-gapped architecture diagram (dummy0, fake DNS, iptables, vsock pipeline)
- [x] Guest network setup walkthrough (capsem-init lines 241-295)
- [x] MITM proxy overview (TLS termination, cert minting, HTTP inspection)
- [x] Domain policy section (exact/wildcard match, provider groups, custom_allow/block)
- [x] HTTP policy section (method-level control, read vs write)
- [x] Telemetry section (per-request logging to session.db)
- [x] "What gets blocked" section with concrete examples
- [x] Capsem-doctor L1-L7 validation layer reference
- [x] Cross-link to MITM proxy architecture page for implementation details

### Build Verification
- [x] Code signing section (Developer ID cert, entitlements, ad-hoc dev signing)
- [x] Notarization section (xcrun notarytool, Apple API key flow)
- [x] SBOM section (cargo-sbom, SPDX 2.3 format)
- [x] SLSA attestation section (build provenance for DMG/deb/rootfs)
- [x] Asset integrity section (BLAKE3 hashes, manifest.json, compile-time embedding, runtime verification)
- [x] Manifest signing section (minisign)
- [x] Supply chain section (pinned toolchains, cargo-audit, Docker base images)

## S2: Architecture Deep Dives

### MITM Proxy Architecture
- [x] Pipeline diagram (mermaid: ClientHello -> SNI -> TLS -> HTTP -> policy -> upstream -> telemetry)
- [x] Cert authority section (CA generation, leaf minting, ECDSA P-256, 24h validity, cache)
- [x] Domain policy engine section (pattern matching, wildcard, provider groups)
- [x] HTTP policy section (method decisions, path filtering)
- [x] AI traffic handling section (SSE parser, provider-specific parsers, token counting, cost)
- [x] Trace state correlation section (tool_call_id tracking across streaming)
- [x] Performance notes (connection pooling, TLS reuse)

### Session Telemetry
- [x] Schema diagram (7 tables with key columns and relationships)
- [x] Data flow section (MITM proxy -> async channel -> DbWriter -> SQLite)
- [x] AI traffic enrichment (SSE events -> model_call rows with tokens/cost)
- [x] Aggregation queries (GlobalStats, ProviderSummary, ToolSummary)
- [x] Access patterns (/inspect, /stats, frontend sql.js)
- [x] Per-VM isolation (one session.db per sandbox, lifetime)

### MCP Gateway
- [x] Two-server diagram (host capsem-mcp vs guest capsem-mcp-server)
- [x] Host MCP server section (stdio, rmcp, 21 tools, service HTTP bridge)
- [x] Guest gateway section (NDJSON over vsock:5003, tool routing)
- [x] Tool origin tracking (native, local, mcp_proxy)
- [x] MCP call logging (session.db mcp_calls table)
- [x] Configuration (mcp/*.toml, settings integration)

## S3: Cross-Linking & Polish

- [x] Update security/overview.md links to newly-filled pages
- [x] Add sidebar ordering for new architecture pages
- [x] Update getting-started.md "What's next" section
- [x] Cross-link security pages <-> architecture deep dives
- [x] `cd docs && pnpm build` -- clean build, no broken links
- [x] Final read-through for consistency and accuracy

## Notes

- All 6 pages written and verified
- Build produces 38 pages with no errors
- Security overview already had correct links to sub-pages
- Sidebar auto-generates from directory structure (no config changes needed)
- New architecture pages use sidebar order 15/20/25 to slot after existing pages
