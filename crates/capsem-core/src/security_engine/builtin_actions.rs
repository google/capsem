//! The built-in security plugin set, built once.
//!
//! `SecurityActionRegistry::with_builtin_actions()` used to construct the
//! four built-in plugins and a fresh map on every evaluated event: two or
//! three times per proxied request, once per DNS query, three times per MCP
//! call. The plugins are stateless, so one registry is built on first use
//! and cloned (four `Arc`s and a small map) for each evaluation; the
//! per-evaluation plugin policy is still applied on the clone.

use std::sync::OnceLock;

use super::{
    CredentialBrokerPlugin, DummyPostAllowPlugin, DummyPreEicarPlugin, LogSanitizerPlugin, SecurityActionRegistry,
};

pub(super) fn registry() -> &'static SecurityActionRegistry {
    static BUILTIN: OnceLock<SecurityActionRegistry> = OnceLock::new();
    BUILTIN.get_or_init(|| {
        SecurityActionRegistry::new()
            .register_plugin(CredentialBrokerPlugin)
            .and_then(|registry| registry.register_plugin(DummyPreEicarPlugin))
            .and_then(|registry| registry.register_plugin(DummyPostAllowPlugin))
            .and_then(|registry| registry.register_plugin(LogSanitizerPlugin))
            .expect("built-in security plugin ids are unique")
    })
}

#[cfg(test)]
mod tests;
