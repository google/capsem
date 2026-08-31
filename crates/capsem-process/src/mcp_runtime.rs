use std::sync::Arc;

use capsem_core::net::mitm_proxy::McpEndpointState;
use capsem_core::net::policy_config::{ModelEndpointRegistry, SecurityRuleSet, SharedPluginPolicy};
use capsem_logger::DbWriter;
use capsem_proto::mcp_aggregator::AggregatorClient;

/// Shared MCP state for capsem-process after the guest transport cutover.
///
/// This is deliberately not a guest "gateway" config. Guest MCP traffic now
/// enters through the MITM framed endpoint on vsock:5002; this state is only
/// the in-process holder for aggregator access and live policy reload.
pub(crate) struct McpRuntime {
    pub(crate) aggregator: AggregatorClient,
    pub(crate) endpoint: Arc<McpEndpointState>,
    pub(crate) db: Arc<DbWriter>,
    pub(crate) security_rules: Arc<std::sync::RwLock<Arc<SecurityRuleSet>>>,
    pub(crate) plugin_policy: SharedPluginPolicy,
    pub(crate) model_endpoints: Arc<std::sync::RwLock<Arc<ModelEndpointRegistry>>>,
}
