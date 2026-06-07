//! Network Engine transport and network-policy primitives.
//!
//! This crate is the first Bedrock Network Engine boundary. It starts with the
//! pure domain/HTTP policy primitives used by runtime MCP and network tooling;
//! heavier MITM/DNS transport modules can move behind this boundary in later
//! structural slices without changing callers' vocabulary.

pub mod ai_provider;
pub mod dns_parser;
pub mod dns_security;
pub mod dns_transport;
pub mod domain_policy;
pub mod http_policy;
pub mod http_security;
pub mod mcp_security;
pub mod model_evidence;
pub mod model_request;
pub mod model_security;
pub mod model_stream;
pub mod sse_parser;
