use std::sync::Arc;

use capsem_core::container::publish::Publisher;
use capsem_core::net::mitm_proxy::{McpEndpointState, ScopedMcpTools};
use capsem_core::net::policy_config::{ModelEndpointRegistry, SecurityRuleSet, SharedPluginPolicy};
use capsem_logger::DbWriter;
use capsem_proto::mcp_aggregator::AggregatorClient;
use capsem_proto::mcp_contracts::{McpToolDef, ToolAnnotations};
use capsem_proto::{ipc::ServiceToProcess, PublicationTarget};
use serde::Deserialize;
use tokio::sync::mpsc;

const GUEST_EXPOSE_TOOL: &str = "capsem__expose_port";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GuestExposureRequest {
    guest_port: u16,
    #[serde(default)]
    host_port: u16,
    target: PublicationTarget,
}

pub(crate) struct GuestExposureTools {
    publisher: Arc<Publisher>,
    control: mpsc::Sender<ServiceToProcess>,
}

impl GuestExposureTools {
    pub(crate) fn new(publisher: Arc<Publisher>, control: mpsc::Sender<ServiceToProcess>) -> Self {
        Self { publisher, control }
    }
}

impl ScopedMcpTools for GuestExposureTools {
    fn definitions(&self) -> Vec<McpToolDef> {
        vec![McpToolDef {
            namespaced_name: GUEST_EXPOSE_TOOL.to_string(),
            original_name: "expose_port".to_string(),
            description: Some(
                "Expose a declared TCP port from this VM or its container on host loopback. The current VM identity is supplied by the trusted relay."
                    .to_string(),
            ),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "guest_port": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": 65535,
                        "description": "Port inside the selected guest namespace."
                    },
                    "host_port": {
                        "type": "integer",
                        "minimum": 0,
                        "maximum": 65535,
                        "default": 0,
                        "description": "Host loopback port, or zero to allocate one."
                    },
                    "target": {
                        "type": "string",
                        "enum": ["container", "vm"],
                        "description": "Explicit guest namespace; Capsem never falls back between targets."
                    }
                },
                "required": ["guest_port", "target"],
                "additionalProperties": false
            }),
            server_name: "capsem".to_string(),
            annotations: Some(ToolAnnotations {
                title: Some("Expose guest port".to_string()),
                read_only_hint: false,
                destructive_hint: false,
                idempotent_hint: false,
                open_world_hint: true,
            }),
            timeout_secs: None,
        }]
    }

    fn call_tool<'a>(
        &'a self,
        name: &'a str,
        arguments: serde_json::Value,
    ) -> futures::future::BoxFuture<'a, Result<serde_json::Value, String>> {
        Box::pin(async move {
            if name != GUEST_EXPOSE_TOOL {
                return Err(format!("unknown scoped tool {name}"));
            }
            let request: GuestExposureRequest =
                serde_json::from_value(arguments).map_err(|error| format!("invalid exposure request: {error}"))?;
            let publication = self
                .publisher
                .publish_saved(
                    request.host_port,
                    request.guest_port,
                    request.target,
                    self.control.clone(),
                )
                .await
                .map_err(|error| format!("{error:#}"))?;
            let structured = serde_json::json!({
                "host_port": publication.host_port,
                "guest_port": publication.guest_port,
                "target": publication.target,
            });
            Ok(serde_json::json!({
                "content": [{
                    "type": "text",
                    "text": format!(
                        "Published {} port {} on host loopback port {}",
                        match publication.target {
                            PublicationTarget::Container => "container",
                            PublicationTarget::Vm => "VM",
                        },
                        publication.guest_port,
                        publication.host_port.expect("guest exposure creates a loopback listener"),
                    )
                }],
                "structuredContent": structured,
                "isError": false
            }))
        })
    }
}

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

#[cfg(test)]
mod tests;
