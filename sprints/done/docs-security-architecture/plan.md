# Sprint: Security & Architecture Documentation

## What

Fill the three security stub pages (kernel hardening, network isolation, build verification) and add missing architecture deep-dive pages (MITM proxy, session telemetry, MCP gateway). All content already exists in code -- this sprint is extraction and documentation, not implementation.

## Why

- Three security pages linked from the getting-started guide and security overview say "Content coming soon" -- bad first impression for new users and enterprise evaluators
- The MITM proxy, session telemetry, and MCP gateway are core differentiators but have zero dedicated documentation
- Security documentation is table-stakes for enterprise adoption; evaluators need to see the hardening story without reading source code
- Architecture docs let contributors understand the system without loading 170KB of mitm_proxy.rs into context

## Source Material

Every page has a rich code source to extract from. No invention needed.

| Page | Primary sources |
|------|----------------|
| Kernel hardening | `guest/config/kernel/defconfig.{arm64,x86_64}`, `capsem-core/src/vm/config.rs:12-18` (cmdline), `test_sandbox.py` (validation tests) |
| Network isolation | `guest/artifacts/capsem-init:241-295` (setup), `capsem-core/src/net/mitm_proxy.rs` (proxy), `capsem-core/src/net/domain_policy.rs` + `http_policy.rs` (policy), `test_network.py` (L1-L7 tests) |
| Build verification | `.github/workflows/release.yaml` (signing, notarization, SBOM, attestation), `justfile` (codesign recipes), `assets/manifest.json` (B3 hashes) |
| MITM proxy arch | `capsem-core/src/net/mitm_proxy.rs`, `cert_authority.rs`, `domain_policy.rs`, `http_policy.rs`, `ai_traffic/` (SSE parsing, provider parsers, pricing) |
| Session telemetry | `capsem-logger/src/{schema.rs,events.rs,writer.rs,reader.rs,db.rs}`, `capsem-core/src/net/ai_traffic/mod.rs` (trace state) |
| MCP gateway | `capsem-mcp/src/main.rs` (host MCP server), `capsem-agent/src/mcp_server.rs` (guest gateway), vsock:5003 protocol |

## Phasing

### S1: Security Pages (fill the three stubs)

These are the highest-priority pages -- they're linked from existing docs and currently embarrassing.

1. **Kernel Hardening** (`docs/src/content/docs/security/kernel-hardening.md`)
   - Threat: what a guest kernel can do without hardening
   - Defconfig table: modules, debugfs, devmem, IPv6, BPF, io_uring, userfaultfd, kexec, hibernation, sysrq
   - Memory mitigations: init_on_alloc, slab hardening, page shuffling, hardened usercopy, stack protector, FORTIFY_SOURCE
   - Architecture-specific: x86 (KPTI, retpoline, CET) vs arm64 (BTI, PAC, KASLR, UNMAP_KERNEL_AT_EL0)
   - Boot cmdline params: ro, init_on_alloc=1, slab_nomerge, page_alloc.shuffle=1
   - Validation: which capsem-doctor tests enforce each property
   - Design philosophy: why each option was chosen (minimize attack surface, not just "enable everything")

2. **Network Isolation** (`docs/src/content/docs/security/network-isolation.md`)
   - Air-gapped architecture: no real NIC, dummy0 10.0.0.1/24, fake DNS
   - iptables pipeline: port 443 -> 10443 redirect, net-proxy -> vsock:5002 -> host MITM
   - MITM proxy overview: TLS termination, cert minting (ECDSA P-256, 24h validity), HTTP inspection
   - Domain policy: exact match, wildcard, per-provider allow lists, custom_allow/custom_block
   - HTTP policy: method-level control (allow_get, allow_post), path filtering
   - Telemetry: every request logged to session.db (domain, method, path, status, bytes, latency)
   - What gets blocked: direct IP access, HTTP port 80, non-standard ports, unlisted domains
   - Capsem-doctor L1-L7 validation layers

3. **Build Verification** (`docs/src/content/docs/security/build-verification.md`)
   - Code signing: Developer ID cert, entitlements plist, ad-hoc for dev
   - Notarization: xcrun notarytool, Apple API key
   - SBOM: cargo-sbom, SPDX 2.3 JSON
   - SLSA attestation: actions/attest-build-provenance@v4 for all release artifacts
   - Asset integrity: BLAKE3 hashes in manifest.json, compile-time hash embedding, runtime verification
   - Manifest signing: minisign for release manifests
   - Supply chain: Rust stable toolchain, pinned Docker base images, cargo-audit

### S2: Architecture Deep Dives (new pages)

These provide the detailed "how it works" for contributors and advanced users.

4. **MITM Proxy Architecture** (`docs/src/content/docs/architecture/mitm-proxy.md`)
   - Pipeline diagram: ClientHello -> SNI extraction -> TLS handshake -> HTTP parse -> policy check -> upstream forward -> telemetry
   - Cert authority: CA key generation, per-domain leaf minting, cache
   - Domain policy engine: pattern matching, wildcard rules, provider groups
   - HTTP policy: method-level decisions, read vs write classification
   - AI traffic handling: SSE parsing, provider-specific parsers (Anthropic/OpenAI/Google), token counting, cost estimation
   - Trace state: tool_call_id correlation across streaming responses
   - Performance: connection pooling, TLS session reuse

5. **Session Telemetry** (`docs/src/content/docs/architecture/session-telemetry.md`)
   - Schema: 7 tables (net_events, model_calls, tool_calls, tool_responses, mcp_calls, file_events, snapshots)
   - Data flow: MITM proxy -> async channel -> DbWriter -> SQLite
   - AI traffic enrichment: how SSE events become model_call rows with token counts and cost
   - Aggregation: GlobalStats, ProviderSummary, ToolSummary queries
   - Access patterns: /inspect SQL queries, /stats endpoint, frontend sql.js
   - Per-VM isolation: one session.db per sandbox, lifetime tied to session directory

6. **MCP Gateway** (`docs/src/content/docs/architecture/mcp-gateway.md`)
   - Two MCP servers: host capsem-mcp (stdio, 18 tools) vs guest capsem-mcp-server (vsock:5003)
   - Host server: tool registry, service HTTP bridge, rmcp crate
   - Guest gateway: NDJSON over vsock, tool routing to external MCP servers
   - Tool origin tracking: native, local, mcp_proxy
   - MCP call logging: session.db mcp_calls table (server, method, tool, latency, decision)
   - Configuration: guest/config/mcp/*.toml, settings system integration

### S3: Cross-Linking & Sidebar Polish

7. Update security overview to link to the now-complete sub-pages
8. Add architecture pages to sidebar with correct ordering
9. Update getting-started.md "What's next" links (kernel-hardening and network-isolation now have content)
10. Cross-link between security pages and architecture deep dives (e.g., network-isolation -> mitm-proxy for implementation details)

## Verification

- `cd docs && pnpm build` -- all pages render without errors
- Every internal link resolves (no broken `[text](/path/)` references)
- Each security page covers at least: threat, mechanism, configuration, and validation
- Each architecture page includes at least one diagram (mermaid)
- No "coming soon" stubs remain in the security section

## Not In Scope

- Writing new capsem-doctor tests (existing tests already validate all documented properties)
- Updating benchmark results to v0.16.1 (needs a fresh benchmark run, separate task)
- CHANGELOG entry (docs-only changes don't need changelog)
- New security features or hardening work
