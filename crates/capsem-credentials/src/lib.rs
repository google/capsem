//! Broker-owned credential storage without telemetry or policy dependencies.

mod durable;
mod provider;
mod store;

pub use durable::STORE_PATH_ENV;
pub use provider::CredentialProvider;
pub use store::{
    broker_reference_replay_available, credential_store_account, credential_store_status,
    hydrate_credential_runtime_cache_from_durable_store, resolve_broker_reference_for_provider,
    CredentialStore, CredentialStoreStatus,
};

/// Return whether `value` is an opaque broker-owned credential reference.
pub fn is_broker_reference(value: &str) -> bool {
    capsem_proto::credential_reference::is_credential_reference(value)
}

#[cfg(test)]
mod tests;
