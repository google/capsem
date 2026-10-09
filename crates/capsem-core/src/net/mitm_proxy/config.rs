use std::sync::Arc;

use capsem_logger::DbWriter;

use super::mcp_endpoint::McpEndpointState;
use super::pipeline;
use super::telemetry_hook;
use super::upstream::TcpUpstreamGrants;
use crate::net::cert_authority::CertAuthority;
use crate::net::proxy_engine::ProxyPolicyHandle;

/// Configuration for the MITM proxy.
pub struct MitmProxyConfig {
    pub ca: Arc<CertAuthority>,
    /// Guest-facing TLS config, built once (`make_server_tls_config`): one session cache for all.
    pub server_tls: Arc<rustls::ServerConfig>,
    /// One digest-keyed active-policy revision, snapshotted per request.
    pub policy: ProxyPolicyHandle,
    pub db: Arc<DbWriter>,
    /// Cached upstream TLS config (shared across all connections).
    pub upstream_tls: Arc<rustls::ClientConfig>,
    /// Telemetry deps shared with the `TelemetryHook` registered in
    /// `pipeline`. Held here as the same `Arc` so the hook and any
    /// remaining direct callers (rare; should fold into the hook) read
    /// the same `pricing` table + `trace_state` mutex. The Arc breaks
    /// the would-be cycle (config → pipeline → hook → config); the
    /// hook only points at this `TelemetryDeps`, not the surrounding
    /// `MitmProxyConfig`.
    pub telemetry: Arc<telemetry_hook::TelemetryDeps>,
    /// Hook pipeline. `make_production_pipeline` registers the sync
    /// ChunkHook chain (decompression → SSE parse →
    /// provider interpreters → telemetry). `handle_request` dispatches
    /// L1 events through this pipeline and seeds per-request context
    /// into the `ChunkDispatchBody`'s `HookState` before serving.
    pub pipeline: Arc<pipeline::Pipeline>,
    /// T3 framed MCP endpoint on the MITM listener. Dispatch state lives
    /// here so the low-privilege aggregator remains DB-free while MITM
    /// owns policy, timeouts, protocol telemetry, and MCP-origin `tool_calls`.
    pub mcp_endpoint: Option<Arc<McpEndpointState>>,
    /// Resolves guest-named upstreams before policy; the dial goes only to what it judged.
    pub upstream_resolver: crate::net::upstream_address::UpstreamResolver,
    /// Trusted coordinator connection grants for confined workers.
    pub upstream_grants: Option<Arc<dyn TcpUpstreamGrants>>,
}
