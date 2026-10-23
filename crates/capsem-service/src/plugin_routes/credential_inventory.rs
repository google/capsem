//! Observed associations come from session-owned counters, never global access grants.
use super::*;

#[derive(Debug, Clone, Serialize)]
pub(super) struct SessionInventory {
    vm_id: String,
    inventory: Vec<BrokeredCredentialStatus>,
}

pub(super) async fn read(
    state: &ServiceState,
    config: SecurityPluginConfig,
) -> Result<(Vec<BrokeredCredentialStatus>, Vec<SessionInventory>), AppError> {
    let named = activity::named_session_counters(state).await?;
    let mut sessions = Vec::with_capacity(named.len());
    let mut snapshots = Vec::with_capacity(named.len());
    for (vm_id, counters) in named {
        let mut runtime = plugin_runtime_config_status(config);
        hydrate_credential_broker_runtime(std::slice::from_ref(&counters), &mut runtime);
        sessions.push(SessionInventory {
            vm_id,
            inventory: runtime.brokered_credentials,
        });
        snapshots.push(counters);
    }
    let mut runtime = plugin_runtime_config_status(config);
    hydrate_credential_broker_runtime(&snapshots, &mut runtime);
    Ok((runtime.brokered_credentials, sessions))
}
