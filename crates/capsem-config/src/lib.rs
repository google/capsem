//! Pure configuration contracts for Capsem.
//!
//! This crate owns stable, serializable configuration identity without taking
//! dependencies on VM runtime, policy execution, telemetry storage, or MCP
//! transports.

mod condition;
mod lint;
pub mod mcp;
pub mod model;
mod provider_profile;
mod resolver;
mod security_rule_profile;
mod settings_metadata;
mod tree;
mod types;
mod validation;

pub use lint::*;
pub use provider_profile::*;
pub use resolver::*;
pub use security_rule_profile::*;
pub use settings_metadata::{default_settings_file, setting_definitions};
pub use tree::*;
pub use types::*;
#[doc(hidden)]
pub use validation::{
    validate_identifier, validate_identifier_shape, validate_non_empty, validate_profile_target,
    IdentifierError,
};

/// True when a value has the broker-owned credential reference shape.
pub use capsem_proto::credential_reference::is_credential_reference;

pub fn security_event_type_is_known(value: &str) -> bool {
    matches!(
        value,
        "http.request"
            | "model.call"
            | "mcp.tool_call"
            | "mcp.tool_list"
            | "mcp.event"
            | "dns.query"
            | "file.event"
            | "file.import"
            | "file.export"
            | "process.exec"
            | "process.exec_complete"
            | "process.audit"
            | "credential.substitution"
            | "security.rule"
            | "security.ask"
    )
}

/// Sorted rule-authoring contract for fields exposed by security events.
pub const SECURITY_EVENT_CEL_FIELDS: &[&str] = &[
    "dns.qname",
    "dns.qtype",
    "dns.valid",
    "file.content",
    "file.create.content",
    "file.create.ext",
    "file.create.mime_type",
    "file.create.name",
    "file.create.path",
    "file.create.valid",
    "file.delete.content",
    "file.delete.ext",
    "file.delete.mime_type",
    "file.delete.name",
    "file.delete.path",
    "file.delete.valid",
    "file.export.content",
    "file.export.ext",
    "file.export.mime_type",
    "file.export.name",
    "file.export.path",
    "file.export.valid",
    "file.import.content",
    "file.import.ext",
    "file.import.mime_type",
    "file.import.name",
    "file.import.path",
    "file.import.valid",
    "file.read.content",
    "file.read.ext",
    "file.read.mime_type",
    "file.read.name",
    "file.read.path",
    "file.read.valid",
    "file.valid",
    "file.write.content",
    "file.write.ext",
    "file.write.mime_type",
    "file.write.name",
    "file.write.path",
    "file.write.valid",
    "http.body",
    "http.host",
    "http.method",
    "http.path",
    "http.query",
    "http.status",
    "http.valid",
    "ip.valid",
    "ip.value",
    "ip.version",
    "mcp.event.valid",
    "mcp.method",
    "mcp.request.arguments",
    "mcp.request.id",
    "mcp.request.method",
    "mcp.request.valid",
    "mcp.response.content",
    "mcp.response.valid",
    "mcp.server.name",
    "mcp.server.valid",
    "mcp.tool_call.name",
    "mcp.tool_call.valid",
    "mcp.tool_list",
    "mcp.tool_list.valid",
    "mcp.valid",
    "model.name",
    "model.provider",
    "model.request.body",
    "model.request.tool_calls",
    "model.request.valid",
    "model.response.body",
    "model.response.valid",
    "model.tool_call.valid",
    "model.valid",
    "process.audit.valid",
    "process.command",
    "process.exec.exit_code",
    "process.exec.id",
    "process.exec.path",
    "process.exec.stderr",
    "process.exec.stdout",
    "process.exec.valid",
    "process.name",
    "process.valid",
    "tcp.port",
    "tcp.valid",
    "udp.port",
    "udp.valid",
];
