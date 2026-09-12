//! Private flow admission: a VM owner asks, the service decides, the
//! destination owner takes the stream (TCP) or the frames (UDP, ICMP echo).
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

/// What both routes admit: who asks, for which flow, to where.
struct Ask {
    source_vm: String,
    owner_secret: String,
    source_generation: u64,
    protocol: &'static str,
    destination: std::net::Ipv4Addr,
    port: u16,
    source_port: u16,
    process_name: String,
}

/// The grant: the destination owner's socket for this protocol, the token.
struct Admitted {
    network: uuid::Uuid,
    destination_vm: String,
    socket: String,
    token: String,
}

fn audit_event(
    peer: &PrivatePeer,
    ask: &Ask,
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
                "protocol": ask.protocol,
                "source": { "vm": { "id": ask.source_vm }, "port": ask.source_port, "process": ask.process_name },
                "destination": { "vm": { "id": peer.vm_id }, "address": peer.address.to_string(), "port": ask.port },
            },
            "decision": { "effective": decision, "reason": reason },
        }),
    )
    .map_err(|error| AppError(StatusCode::INTERNAL_SERVER_ERROR, format!("audit event: {error}")))
}

/// Admit one flow: the owner's secret, the registry's answer, the destination
/// owner's acceptance, both members' audit row, and the membership re-check
/// after the owner answered.
async fn admit(state: &ServiceState, ask: Ask) -> Result<Admitted, AppError> {
    let expected = state
        .instances
        .lock()
        .unwrap()
        .get(&ask.source_vm)
        .map(|instance| instance.owner_secret.clone());
    match expected {
        Some(secret) if secrets_match(&ask.owner_secret, &secret) => {}
        _ => {
            return Err(AppError(
                StatusCode::FORBIDDEN,
                format!("VM {} has no running owner presenting this secret", ask.source_vm),
            ))
        }
    }
    let peer = {
        let registry = state.networks.lock().await;
        registry.resolve_private(&ask.source_vm, ask.destination)
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
        let event = audit_event(&peer, &ask, connection, "block", "destination_stopped")?;
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
    // Sixteen hex digits of a random u64: what a handoff or relay frame carries.
    let token = format!("{:016x}", uuid::Uuid::new_v4().as_u128() as u64);
    let (source_name, network_name) = {
        let name = state
            .instances
            .lock()
            .unwrap()
            .get(&ask.source_vm)
            .map(|instance| instance.name.clone())
            .unwrap_or_else(|| ask.source_vm.clone());
        let network = state
            .networks
            .lock()
            .await
            .summary(peer.network)
            .map(|summary| summary.name)
            .unwrap_or_default();
        (name, network)
    };
    let accept = ServiceToProcess::PrivateAccept {
        id: connection.as_u128() as u64,
        token: token.clone(),
        network: peer.network.to_string(),
        network_name,
        source_vm: ask.source_vm.clone(),
        source_name,
        source_generation: ask.source_generation,
        source_address: network_routes::vm_private_address(state, &ask.source_vm)?,
        source_port: ask.source_port,
        port: ask.port,
        protocol: ask.protocol.to_string(),
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
    // The owner answered after the setup deadline's worth of time may have
    // passed; a membership that changed meanwhile (a detach, a retirement)
    // wins over the grant, so a flow is never handed to a VM that is no
    // longer a peer. Checked and recorded under the same lock.
    let registry = state.networks.lock().await;
    let (decision, reason, outcome) = match registry.resolve_private(&ask.source_vm, ask.destination) {
        Ok(current) if current == peer => (decision, reason, outcome),
        _ => (
            "block",
            "membership_changed",
            Err(AppError(StatusCode::CONFLICT, "membership changed during setup".into())),
        ),
    };
    let event = audit_event(&peer, &ask, connection, decision, reason)?;
    registry
        .record(peer.network, event)
        .await
        .map_err(network_routes::network_error)?;
    drop(registry);
    Ok(Admitted {
        network: peer.network,
        destination_vm: peer.vm_id,
        socket: outcome?,
        token,
    })
}

pub(super) async fn handle_private_connect(
    State(state): State<Arc<ServiceState>>,
    Json(request): Json<PrivateConnectRequest>,
) -> Result<Json<PrivateConnectResponse>, AppError> {
    if request.source_port == 0 || request.port == 0 {
        return Err(AppError(
            StatusCode::BAD_REQUEST,
            "a private connection names both ports".into(),
        ));
    }
    let admitted = admit(
        &state,
        Ask {
            source_vm: request.source_vm,
            owner_secret: request.owner_secret,
            source_generation: request.source_generation,
            protocol: "tcp",
            destination: request.destination,
            port: request.port,
            source_port: request.source_port,
            process_name: request.process_name,
        },
    )
    .await?;
    Ok(Json(PrivateConnectResponse {
        network: admitted.network.to_string(),
        destination_vm: admitted.destination_vm,
        handoff_socket: admitted.socket,
        token: admitted.token,
    }))
}

pub(super) async fn handle_private_datagram(
    State(state): State<Arc<ServiceState>>,
    Json(request): Json<PrivateDatagramRequest>,
) -> Result<Json<PrivateDatagramResponse>, AppError> {
    let protocol = match request.protocol.as_str() {
        "udp" if request.port != 0 && request.source_port != 0 => "udp",
        "udp" => return Err(AppError(StatusCode::BAD_REQUEST, "a udp flow names both ports".into())),
        "icmp" if request.port == 0 => "icmp",
        "icmp" => return Err(AppError(StatusCode::BAD_REQUEST, "icmp echo has no port".into())),
        other => {
            return Err(AppError(
                StatusCode::BAD_REQUEST,
                format!("unknown datagram protocol {other:?}"),
            ))
        }
    };
    let admitted = admit(
        &state,
        Ask {
            source_vm: request.source_vm,
            owner_secret: request.owner_secret,
            source_generation: request.source_generation,
            protocol,
            destination: request.destination,
            port: request.port,
            source_port: request.source_port,
            process_name: String::new(),
        },
    )
    .await?;
    Ok(Json(PrivateDatagramResponse {
        network: admitted.network.to_string(),
        destination_vm: admitted.destination_vm,
        relay_socket: admitted.socket,
        token: admitted.token,
    }))
}
