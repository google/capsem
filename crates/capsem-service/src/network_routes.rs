//! Named networks: a subnet each, and VMs that lease an address in it.
//!
//! The registry behind `ServiceState::networks` is the authority and makes
//! every change durable before answering; these handlers only translate
//! between it and the wire. Attaching records a `declared` membership; the
//! cable plugged into the network's switch that makes it `ready` is `switches`' job.
use super::*;
use capsem_core::net::network_registry::{LogQuery, NetworkError, NetworkRegistry, NETWORK_AUDIT_RETENTION};
use uuid::Uuid;

/// Retire every network a deleted VM leaves empty and drop retired databases
/// past retention. Nothing here can fail the deletion: the VM is gone either
/// way, and what could not be recorded is logged with the reason.
pub(super) async fn vm_deleted(state: &Arc<ServiceState>, vm_id: &str) {
    // Memberships go first, cables second: a plug finishing in between finds
    // no membership and unplugs itself.
    let now_unix_ms = vm_lifecycle::unix_time_ms();
    let mut registry = state.networks.lock().await;
    let retired = match registry.vm_deleted(vm_id, now_unix_ms).await {
        Ok(departures) => departures
            .iter()
            .filter(|departure| departure.retired)
            .map(|departure| departure.network)
            .collect(),
        Err(error) => {
            tracing::warn!(vm_id, %error, "deleted VM left a network membership behind");
            Vec::new()
        }
    };
    sweep_retired(&mut registry, now_unix_ms);
    drop(registry);
    switches::unplug_everywhere(state, vm_id).await;
    for network in retired {
        tracing::info!(vm_id, %network, "network retired with its last member");
        switches::retire(state, network).await;
    }
}

/// The retention sweep, run wherever a network retires and at startup.
pub(super) fn sweep_retired(registry: &mut NetworkRegistry, now_unix_ms: i64) {
    for network in registry.sweep_retired(now_unix_ms, NETWORK_AUDIT_RETENTION) {
        tracing::info!(%network, "retired network database removed after retention");
    }
}

pub(super) fn network_error(error: NetworkError) -> AppError {
    let status = match &error {
        NetworkError::InvalidName(_) | NetworkError::Cursor(_) => StatusCode::BAD_REQUEST,
        NetworkError::NameTaken { .. }
        | NetworkError::HasMembers { .. }
        | NetworkError::SubnetsExhausted { .. }
        | NetworkError::AddressesExhausted { .. } => StatusCode::CONFLICT,
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
        .filter_map(|member| {
            let state = match member.state {
                capsem_logger::MembershipState::Declared => NetworkMemberState::Declared,
                capsem_logger::MembershipState::Attaching => NetworkMemberState::Attaching,
                capsem_logger::MembershipState::Ready => NetworkMemberState::Ready,
                capsem_logger::MembershipState::Failed => NetworkMemberState::Failed,
                capsem_logger::MembershipState::Detached => return None,
            };
            Some(NetworkMemberInfo {
                vm_id: member.vm_id,
                address: member.address,
                state,
                updated_unix_ms: member.updated_unix_ms,
            })
        })
        .collect();
    Some(NetworkInfo {
        id: id.to_string(),
        name: summary.name,
        subnet: summary.subnet.to_string(),
        created_unix_ms: summary.created_unix_ms,
        members,
    })
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
    state: &Arc<ServiceState>,
    vm_id: &str,
    networks: &[Uuid],
) -> Result<(), AppError> {
    if networks.is_empty() {
        return Ok(());
    }
    let mut registry = state.networks.lock().await;
    for network in networks {
        registry
            .attach(*network, vm_id, vm_lifecycle::unix_time_ms())
            .await
            .map_err(network_error)?;
    }
    drop(registry);
    // The owner is registered by now; its guest may still be booting, which
    // the plug waits for.
    switches::plug_memberships(Arc::clone(state), vm_id.to_string());
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
    let now_unix_ms = vm_lifecycle::unix_time_ms();
    let mut registry = state.networks.lock().await;
    registry.retire(network, now_unix_ms).await.map_err(network_error)?;
    sweep_retired(&mut registry, now_unix_ms);
    drop(registry);
    switches::retire(&state, network).await;
    Ok(Json(json!({ "success": true })))
}

pub(super) async fn handle_network_attach(
    State(state): State<Arc<ServiceState>>,
    Path((id, vm_id)): Path<(String, String)>,
) -> Result<Json<NetworkInfo>, AppError> {
    let network = parse_network_id(&id)?;
    let known = state.instances.lock().unwrap().contains_key(&vm_id)
        || vm_lifecycle::find_persistent_entry_by_route_id(&state, &vm_id).is_some();
    if !known {
        return Err(AppError(StatusCode::NOT_FOUND, format!("sandbox not found: {vm_id}")));
    }
    state
        .networks
        .lock()
        .await
        .attach(network, &vm_id, vm_lifecycle::unix_time_ms())
        .await
        .map_err(network_error)?;
    // A running member is plugged before the answer, so the caller sees
    // `ready`, or `failed` with the reason in the network's history. The
    // membership stands either way: the VM joined, its cable is plugged again
    // when it next starts.
    if let Err(error) = switches::plug(&state, network, &vm_id).await {
        tracing::warn!(%network, vm_id, %error, "member joined but its cable was not plugged");
    }
    let info = network_info(&*state.networks.lock().await, network).expect("attached to an existing network");
    Ok(Json(info))
}

pub(super) async fn handle_network_detach(
    State(state): State<Arc<ServiceState>>,
    Path((id, vm_id)): Path<(String, String)>,
) -> Result<Json<NetworkInfo>, AppError> {
    let network = parse_network_id(&id)?;
    switches::detach(&state, network, &vm_id).await.map_err(network_error)?;
    let info = network_info(&*state.networks.lock().await, network).expect("detached from an existing network");
    Ok(Json(info))
}

/// A page of a network's audit history; retired networks keep theirs.
pub(super) async fn handle_network_logs(
    State(state): State<Arc<ServiceState>>,
    Path(id): Path<String>,
    Query(query): Query<NetworkLogsQuery>,
) -> Result<Json<NetworkLogsResponse>, AppError> {
    let network = parse_network_id(&id)?;
    let query = LogQuery {
        cursor: query.cursor,
        limit: query.limit,
        vm: query.vm,
        connection: query.connection,
        event_type: query.event_type,
        decision: query.decision,
        since_unix_ms: query.since,
        until_unix_ms: query.until,
    };
    let page = state.networks.lock().await.logs(network, &query).await;
    let page = page.map_err(network_error)?;
    let mut events = Vec::with_capacity(page.events.len());
    for event in page.events {
        // The ledger wrote this JSON; a row that no longer parses is a broken
        // database, reported as such rather than shown as an empty event.
        let parsed = serde_json::from_str(&event.event_json).map_err(|error| {
            AppError(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("network {network} event {} is not JSON: {error}", event.event_id),
            )
        })?;
        events.push(NetworkLogEvent {
            sequence: event.sequence,
            event_id: event.event_id,
            timestamp_unix_ms: event.timestamp_unix_ms,
            event_type: event.event_type,
            connection_id: event.connection_id,
            event: parsed,
        });
    }
    Ok(Json(NetworkLogsResponse {
        events,
        cursor: page.cursor,
        next_cursor: page.next_cursor,
    }))
}
