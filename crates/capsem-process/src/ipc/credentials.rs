//! Injection ends at host broker memory; it has no guest control sender.
use capsem_core::credential_broker::CredentialStore;
use capsem_proto::credential_injection::CredentialMaterial;
use capsem_proto::ipc::ProcessToService;

pub(super) fn apply(store: &CredentialStore, id: u64, credentials: Vec<CredentialMaterial>) -> ProcessToService {
    ProcessToService::CredentialsInjected {
        id,
        error: store.import_memory_credentials(credentials).err(),
    }
}

#[cfg(test)]
mod tests;
