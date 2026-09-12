//! Private TCP admission: a VM owner asks, the service decides, the
//! destination owner takes the stream.
//!
//! The owner proves itself with the secret minted for its VM at spawn. The
//! registry says whether the destination is a member of an active network
//! the source is also in -- decided here, before any owner is contacted.
//! Both VMs get an audit row in that network's ledger before the answer, so
//! `capsem network logs` shows the connection from either side even when
//! the destination owner then refuses. Rules run where they run today: the
//! destination owner evaluates its side when it connects its guest, as a
//! published port does.
use super::*;
use capsem_core::net::network_registry::{NetworkError, PrivatePeer};
use capsem_logger::{TransportEvent, TransportEventKind};

/// How long the destination owner has to answer a `PrivateAccept`: the guest
/// setup deadline the publication path already uses.
const ACCEPT_TIMEOUT_SECS: u64 = 8;
const OWNER_SECRET_FILE: &str = "owner-secret";

/// Mint the secret an owner will show, and leave it in its session directory
/// for that owner alone.
pub(super) fn mint_owner_secret(session_dir: &std::path::Path) -> Result<String> {
    let secret = format!("{}{}", uuid::Uuid::new_v4().simple(), uuid::Uuid::new_v4().simple());
    let path = session_dir.join(OWNER_SECRET_FILE);
    let mut file = std::fs::OpenOptions::new();
    file.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        file.mode(0o600);
    }
    std::io::Write::write_all(
        &mut file.open(&path).with_context(|| format!("create {}", path.display()))?,
        secret.as_bytes(),
    )
    .with_context(|| format!("write {}", path.display()))?;
    Ok(secret)
}

fn secrets_match(presented: &str, expected: &str) -> bool {
    // Same length, then every byte compared: a mismatch costs the same as a
    // match, so timing says nothing about how much of the secret was right.
    presented.len() == expected.len()
        && presented
            .bytes()
            .zip(expected.bytes())
            .fold(0u8, |acc, (a, b)| acc | (a ^ b))
            == 0
}

fn now_hex_id() -> String {
    format!(
        "{:012x}",
        u64::try_from(vm_lifecycle::unix_time_ms()).unwrap_or(0) & 0xffff_ffff_ffff
    )
}

fn audit_event(
    peer: &PrivatePeer,
    request: &PrivateConnectRequest,
    connection: uuid::Uuid,
    decision: &str,
    reason: &str,
) -> Result<TransportEvent, AppError> {
    TransportEvent::new(
        now_hex_id(),
        vm_lifecycle::unix_time_ms(),
        TransportEventKind::Connect,
        Some(peer.network),
        Some(connection),
        &json!({
            "network": {
                "context": "private",
                "source": { "vm": { "id": request.source_vm }, "process": request.process_name },
                "destination": { "vm": { "id": peer.vm_id }, "address": peer.address.to_string(), "port": request.port },
            },
            "decision": { "effective": decision, "reason": reason },
        }),
    )
    .map_err(|error| AppError(StatusCode::INTERNAL_SERVER_ERROR, format!("audit event: {error}")))
}

pub(super) async fn handle_private_connect(
    State(state): State<Arc<ServiceState>>,
    Json(request): Json<PrivateConnectRequest>,
) -> Result<Json<PrivateConnectResponse>, AppError> {
    let expected = state
        .instances
        .lock()
        .unwrap()
        .get(&request.source_vm)
        .map(|instance| instance.owner_secret.clone());
    match expected {
        Some(secret) if secrets_match(&request.owner_secret, &secret) => {}
        _ => {
            return Err(AppError(
                StatusCode::FORBIDDEN,
                format!("VM {} has no running owner presenting this secret", request.source_vm),
            ))
        }
    }
    let peer = {
        let registry = state.networks.lock().await;
        registry.resolve_private(&request.source_vm, request.destination)
    }
    .map_err(|error| match error {
        NetworkError::NoPrivatePath { .. } => AppError(StatusCode::NOT_FOUND, error.to_string()),
        other => network_routes::network_error(other),
    })?;
    let destination_uds = state
        .instances
        .lock()
        .unwrap()
        .get(&peer.vm_id)
        .map(|instance| instance.uds_path.clone());
    let connection = uuid::Uuid::new_v4();
    let Some(destination_uds) = destination_uds else {
        let event = audit_event(&peer, &request, connection, "block", "destination_stopped")?;
        state
            .networks
            .lock()
            .await
            .record(peer.network, event)
            .await
            .map_err(network_routes::network_error)?;
        return Err(AppError(
            StatusCode::CONFLICT,
            format!("destination VM {} is not running", peer.vm_id),
        ));
    };
    let token = uuid::Uuid::new_v4().simple().to_string();
    let accept = ServiceToProcess::PrivateAccept {
        id: connection.as_u128() as u64,
        token: token.clone(),
        source_vm: request.source_vm.clone(),
        source_address: network_routes::vm_private_address(&state, &request.source_vm)?,
        port: request.port,
    };
    let reply = vm_files::send_ipc_command(&destination_uds, accept, Some(ACCEPT_TIMEOUT_SECS)).await;
    let (decision, reason, outcome) = match &reply {
        Ok(ProcessToService::PrivateAcceptResult {
            error: None,
            handoff_socket,
            ..
        }) => ("allow", "admitted", Ok(handoff_socket.clone())),
        Ok(ProcessToService::PrivateAcceptResult { error: Some(error), .. }) => (
            "block",
            "destination_refused",
            Err(AppError(
                StatusCode::CONFLICT,
                format!("destination owner refused: {error}"),
            )),
        ),
        Ok(other) => (
            "block",
            "destination_unreachable",
            Err(AppError(
                StatusCode::BAD_GATEWAY,
                format!("unexpected owner reply: {other:?}"),
            )),
        ),
        Err(error) => (
            "block",
            "destination_unreachable",
            Err(AppError(StatusCode::BAD_GATEWAY, format!("destination owner: {error}"))),
        ),
    };
    let event = audit_event(&peer, &request, connection, decision, reason)?;
    state
        .networks
        .lock()
        .await
        .record(peer.network, event)
        .await
        .map_err(network_routes::network_error)?;
    let handoff_socket = outcome?;
    Ok(Json(PrivateConnectResponse {
        network: peer.network.to_string(),
        destination_vm: peer.vm_id,
        handoff_socket,
        token,
    }))
}
