//! Sibling case modules of `mitm_integration.rs`. Each reaches the shared
//! fixtures (`make_proxy_config*`, `spawn_proxy`, `spawn_fake_upstream`,
//! `read_http11_request`) through `super::*`.
use super::*;

mod content_filters;
mod deadlines;
mod host_normalization;
mod mcp_gateway_route;
mod mcp_over_http;
mod mcp_response_approval;
mod resolved_upstream;
mod semantic_body;
mod websocket_policy;
