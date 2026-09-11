//! Named networks: groups of VMs, each bringing its lifetime address.
//!
//! The registry behind `ServiceState::networks` is the authority and makes
//! every change durable before answering; these handlers only translate
//! between it and the wire. Attaching records a `declared` membership; the
//! data plane that makes it `ready` is the network process's concern.
use super::*;
use capsem_core::net::network_registry::{NetworkError, NetworkRegistry};
use uuid::Uuid;

pub(super) fn network_error(error: NetworkError) -> AppError {
    let status = match &error {
        NetworkError::InvalidName(_) => StatusCode::BAD_REQUEST,
        NetworkError::NameTaken { .. } | NetworkError::HasMembers { .. } => StatusCode::CONFLICT,
        NetworkError::NotFound(_) | NetworkError::NotAMember { .. } => StatusCode::NOT_FOUND,
        NetworkError::Database { .. } => StatusCode::INTERNAL_SERVER_ERROR,
    };
    AppError(status, error.to_string())
}

fn parse_network_id(id: &str) -> Result<Uuid, AppError> {
    Uuid::parse_str(id).map_err(|_| AppError(StatusCode::NOT_FOUND, format!("network not found: {id}")))
}

fn network_info(registry: &NetworkRegistry, id: Uuid) -> Option<NetworkInfo> {
    let summary = registry.summary(id)?;
    let members = registry
        .members(id)?
        .into_iter()
        .map(|member| NetworkMemberInfo {
            vm_id: member.vm_id,
            address: member.address,
            state: member.state.as_str().to_string(),
            updated_unix_ms: member.updated_unix_ms,
        })
        .collect();
    Some(NetworkInfo {
        id: id.to_string(),
        name: summary.name,
        created_unix_ms: summary.created_unix_ms,
        members,
    })
}

/// The VM's lifetime address, running or stopped. An entry written before
/// addresses existed cannot join a network until it resumes and gets one.
pub(super) fn vm_private_address(state: &ServiceState, vm_id: &str) -> Result<std::net::Ipv4Addr, AppError> {
    if let Some(instance) = state.instances.lock().unwrap().get(vm_id) {
        return Ok(instance.private_address);
    }
    match vm_lifecycle::find_persistent_entry_by_route_id(state, vm_id) {
        Some(entry) => entry.private_address.ok_or_else(|| {
            AppError(
                StatusCode::CONFLICT,
                format!("VM {vm_id} has no private address until it resumes"),
            )
        }),
        None => Err(AppError(StatusCode::NOT_FOUND, format!("sandbox not found: {vm_id}"))),
    }
}

/// Resolve network names to ids, refusing the whole request on any unknown
/// name so a VM is never created half-connected.
pub(super) fn resolve_network_names(registry: &NetworkRegistry, names: &[String]) -> Result<Vec<Uuid>, AppError> {
    names
        .iter()
        .map(|name| {
            registry
                .find(name)
                .ok_or_else(|| AppError(StatusCode::BAD_REQUEST, format!("unknown network: {name}")))
        })
        .collect()
}

/// Record a freshly provisioned VM in every network it asked for.
pub(super) async fn attach_provisioned(
    state: &ServiceState,
    vm_id: &str,
    address: std::net::Ipv4Addr,
    networks: &[Uuid],
) -> Result<(), AppError> {
    if networks.is_empty() {
        return Ok(());
    }
    let mut registry = state.networks.lock().await;
    for network in networks {
        registry
            .attach(
                *network,
                vm_id,
                address,
                capsem_logger::MembershipState::Declared,
                vm_lifecycle::unix_time_ms(),
            )
            .await
            .map_err(network_error)?;
    }
    drop(registry);
    Ok(())
}

pub(super) async fn handle_networks_list(State(state): State<Arc<ServiceState>>) -> Json<NetworkListResponse> {
    let networks = {
        let registry = state.networks.lock().await;
        registry
            .list()
            .into_iter()
            .filter_map(|summary| network_info(&registry, summary.id))
            .collect()
    };
    Json(NetworkListResponse { networks })
}

pub(super) async fn handle_network_create(
    State(state): State<Arc<ServiceState>>,
    Json(payload): Json<CreateNetworkRequest>,
) -> Result<(StatusCode, Json<NetworkInfo>), AppError> {
    let info = {
        let mut registry = state.networks.lock().await;
        let summary = registry
            .create(&payload.name, vm_lifecycle::unix_time_ms())
            .await
            .map_err(network_error)?;
        let info = network_info(&registry, summary.id).expect("just created");
        drop(registry);
        info
    };
    Ok((StatusCode::CREATED, Json(info)))
}

pub(super) async fn handle_network_inspect(
    State(state): State<Arc<ServiceState>>,
    Path(id): Path<String>,
) -> Result<Json<NetworkInfo>, AppError> {
    let network = parse_network_id(&id)?;
    let info = network_info(&*state.networks.lock().await, network);
    info.map(Json)
        .ok_or_else(|| network_error(NetworkError::NotFound(network)))
}

/// Retire an empty network. Members must be disconnected first; the
/// network's database stays for its audit history.
pub(super) async fn handle_network_delete(
    State(state): State<Arc<ServiceState>>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    let network = parse_network_id(&id)?;
    let retired = state
        .networks
        .lock()
        .await
        .retire(network, vm_lifecycle::unix_time_ms())
        .await;
    retired.map_err(network_error)?;
    Ok(Json(json!({ "success": true })))
}

pub(super) async fn handle_network_attach(
    State(state): State<Arc<ServiceState>>,
    Path((id, vm_id)): Path<(String, String)>,
) -> Result<Json<NetworkInfo>, AppError> {
    let network = parse_network_id(&id)?;
    let address = vm_private_address(&state, &vm_id)?;
    let info = {
        let mut registry = state.networks.lock().await;
        registry
            .attach(
                network,
                &vm_id,
                address,
                capsem_logger::MembershipState::Declared,
                vm_lifecycle::unix_time_ms(),
            )
            .await
            .map_err(network_error)?;
        let info = network_info(&registry, network).expect("attached to an existing network");
        drop(registry);
        info
    };
    Ok(Json(info))
}

pub(super) async fn handle_network_detach(
    State(state): State<Arc<ServiceState>>,
    Path((id, vm_id)): Path<(String, String)>,
) -> Result<Json<NetworkInfo>, AppError> {
    let network = parse_network_id(&id)?;
    let info = {
        let mut registry = state.networks.lock().await;
        registry
            .detach(network, &vm_id, vm_lifecycle::unix_time_ms())
            .await
            .map_err(network_error)?;
        let info = network_info(&registry, network).expect("detached from an existing network");
        drop(registry);
        info
    };
    Ok(Json(info))
}
