//! One confined switch per network, and each running member's link to it.
//!
//! The switch never sees the service's API: it gets one stream per linked
//! member and reports when a link ends. Linking a VM is a handshake with
//! its owner (`LinkAttach` over IPC, then the token on the handoff socket,
//! then the guest stream back on that connection) and a grant to the
//! switch; the service keeps the handoff connection as the link's keepalive
//! and drops it to end the link. Membership states follow: `declared` until
//! the VM runs, `attaching` during the handshake, `ready` once the switch
//! has the stream, `failed` when the owner or the switch refused, and back
//! to `declared` when the stream ends (the VM stopped or died). Every
//! transition writes a row in the network's ledger.
use super::*;
use anyhow::ensure;
use capsem_core::net::switch_host::SwitchHost;
use capsem_foundation::unix::router_channel::{Receiver, Sender};
use capsem_logger::{MembershipState, TransportEvent, TransportEventKind};
use capsem_proto::privatelink::{seat_frame, SEAT_LINK};
use capsem_router::CloseReport;
use std::net::Ipv4Addr;
use std::os::fd::AsFd;
use std::time::Duration;
use tokio::sync::mpsc;
use uuid::Uuid;

/// How long the owner has to answer `LinkAttach`.
const ATTACH_TIMEOUT_SECS: u64 = 8;
/// How long the owner may take to hand the stream over: a VM linked at
/// creation is still booting its guest.
const STREAM_TIMEOUT: Duration = Duration::from_secs(70);

type Reports = mpsc::Sender<(u64, CloseReport)>;
type Starting = std::pin::Pin<Box<dyn std::future::Future<Output = Result<Arc<SwitchHost>>> + Send>>;
type Starter = Arc<dyn Fn(Uuid, Reports) -> Starting + Send + Sync>;

struct Linked {
    vm_id: String,
    address: Ipv4Addr,
    connection: Uuid,
    /// The owner's handoff connection: dropped, the owner ends the guest stream.
    keepalive: std::os::unix::net::UnixStream,
}

struct NetworkSwitch {
    host: Arc<SwitchHost>,
    links: HashMap<u64, Linked>,
}

pub(crate) struct Switches {
    starter: Starter,
    networks: tokio::sync::Mutex<HashMap<Uuid, NetworkSwitch>>,
}

impl Switches {
    /// Switches as confined `capsem-router --switch` children.
    pub(crate) fn confined() -> Self {
        Self::with_starter(Arc::new(|network, reports| {
            Box::pin(SwitchHost::start(network, reports))
        }))
    }

    /// Switches running in this process, for tests that cannot spawn the
    /// confined binary; the protocol and the host are the real ones.
    #[cfg(test)]
    pub(crate) fn in_process() -> Self {
        Self::with_starter(Arc::new(|_, reports| {
            Box::pin(async move {
                let (parent, child) = std::os::unix::net::UnixStream::pair()?;
                parent.set_nonblocking(true)?;
                child.set_nonblocking(true)?;
                let grants = Receiver::new(child.try_clone()?)?;
                tokio::spawn(capsem_router::switch::run(
                    grants,
                    tokio::net::UnixStream::from_std(child)?,
                    capsem_router::CONNECTIONS_PER_CLASS,
                ));
                let host = SwitchHost::attach(
                    0,
                    Sender::new(parent.try_clone()?)?,
                    tokio::net::UnixStream::from_std(parent)?,
                    reports,
                );
                host.ready().await?;
                Ok(host)
            })
        }))
    }

    fn with_starter(starter: Starter) -> Self {
        Self {
            starter,
            networks: tokio::sync::Mutex::new(HashMap::new()),
        }
    }
}

fn now_hex_id() -> String {
    format!(
        "{:012x}",
        u64::try_from(vm_lifecycle::unix_time_ms()).unwrap_or(0) & 0xffff_ffff_ffff
    )
}

/// What a link row says happened: linked, refused with the reason, or ended.
enum Outcome<'a> {
    Linked,
    Refused(&'a str),
    Ended(&'a str, Option<&'a CloseReport>),
}

/// One ledger row about a VM's link: the same shape as a private
/// connection's, with the VM on both ends and no port.
fn link_event(
    network: Uuid,
    connection: Uuid,
    vm_id: &str,
    address: Ipv4Addr,
    outcome: Outcome<'_>,
) -> Result<TransportEvent, String> {
    let endpoint = json!({ "vm": { "id": vm_id }, "address": address.to_string(), "port": 0 });
    let (kind, decision, reason, frames) = match outcome {
        Outcome::Linked => (TransportEventKind::Connect, "allow", "linked", None),
        Outcome::Refused(reason) => (TransportEventKind::Connect, "block", reason, None),
        Outcome::Ended(reason, report) => (TransportEventKind::Close, "allow", reason, report),
    };
    let mut facts = json!({
        "network": { "context": "private", "protocol": "link", "source": endpoint, "destination": endpoint },
        "decision": { "effective": decision, "reason": reason },
    });
    if let Some(report) = frames {
        facts["frames"] = json!({ "forwarded": report.from_source, "delivered": report.to_source, "reason": format!("{:?}", report.reason) });
    }
    TransportEvent::new(
        now_hex_id(),
        vm_lifecycle::unix_time_ms(),
        kind,
        Some(network),
        Some(connection),
        &facts,
    )
}

/// Write a membership state and a ledger row under the registry lock; a VM
/// that left the network meanwhile gets neither.
async fn record(
    state: &ServiceState,
    network: Uuid,
    vm_id: &str,
    address: Ipv4Addr,
    membership: MembershipState,
    event: TransportEvent,
) {
    let mut registry = state.networks.lock().await;
    let still_member = registry
        .members(network)
        .is_some_and(|members| members.iter().any(|member| member.vm_id == vm_id));
    if !still_member {
        return;
    }
    if let Err(error) = registry
        .attach(network, vm_id, address, membership, vm_lifecycle::unix_time_ms())
        .await
    {
        warn!(%network, vm_id, %error, "membership state was not recorded");
    }
    if let Err(error) = registry.record(network, event).await {
        warn!(%network, vm_id, %error, "link audit row was not recorded");
    }
    drop(registry);
}

/// The network's switch, started on first use. Its close reports are
/// consumed for as long as it lives; when it dies, every member it linked
/// is linked again on a fresh one.
async fn host(state: &Arc<ServiceState>, network: Uuid) -> Result<Arc<SwitchHost>> {
    let mut networks = state.switches.networks.lock().await;
    if let Some(switch) = networks.get(&network) {
        if !switch.host.closed.is_cancelled() {
            return Ok(Arc::clone(&switch.host));
        }
    }
    let (reports, mut closed) = mpsc::channel(64);
    let host = (state.switches.starter)(network, reports).await?;
    networks.insert(
        network,
        NetworkSwitch {
            host: Arc::clone(&host),
            links: HashMap::new(),
        },
    );
    drop(networks);
    let watched = Arc::clone(state);
    tokio::spawn(async move {
        while let Some((id, report)) = closed.recv().await {
            link_closed(&watched, network, id, &report).await;
        }
        relink_orphans(watched, network).await;
    });
    Ok(host)
}

/// The switch is gone: whoever was still linked is linked again on a fresh
/// one. Boxed: linking starts a switch, whose watcher relinks.
fn relink_orphans(
    state: Arc<ServiceState>,
    network: Uuid,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>> {
    Box::pin(async move {
        let orphans = state.switches.networks.lock().await.remove(&network);
        let Some(orphans) = orphans else { return };
        warn!(%network, switch_pid = orphans.host.pid, members = orphans.links.len(), "network switch ended; relinking its members");
        for linked in orphans.links.into_values() {
            drop(linked.keepalive);
            if let Err(error) = link(&state, network, &linked.vm_id).await {
                warn!(%network, vm_id = linked.vm_id, %error, "member was not relinked");
            }
        }
    })
}

/// The switch reported a link's end: the VM stopped, died, or its pump did.
/// A member whose owner still runs is linked again once the pump has
/// reconnected; one that is gone stays `declared` until it resumes.
async fn link_closed(state: &Arc<ServiceState>, network: Uuid, id: u64, report: &CloseReport) {
    let linked = state
        .switches
        .networks
        .lock()
        .await
        .get_mut(&network)
        .and_then(|switch| switch.links.remove(&id));
    let Some(linked) = linked else { return };
    info!(%network, vm_id = linked.vm_id, ?report, "private link closed");
    drop(linked.keepalive);
    match link_event(
        network,
        linked.connection,
        &linked.vm_id,
        linked.address,
        Outcome::Ended("closed", Some(report)),
    ) {
        Ok(event) => {
            record(
                state,
                network,
                &linked.vm_id,
                linked.address,
                MembershipState::Declared,
                event,
            )
            .await
        }
        Err(error) => warn!(%network, %error, "link close row was not built"),
    }
    tokio::spawn(relink_later(Arc::clone(state), network, linked.vm_id));
}

/// A member whose owner still runs is linked again after its link closed:
/// its pump restarts after its first backoff and the owner waits for the
/// fresh stream. Boxed: linking starts a switch whose watcher reports
/// closes here.
fn relink_later(
    state: Arc<ServiceState>,
    network: Uuid,
    vm_id: String,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>> {
    Box::pin(async move {
        tokio::time::sleep(Duration::from_secs(1)).await;
        if let Err(error) = link(&state, network, &vm_id).await {
            info!(%network, vm_id, %error, "member not relinked after its link closed");
        }
    })
}

/// Ask the owner for the guest stream and grant it to the switch.
async fn handshake(
    state: &Arc<ServiceState>,
    network: Uuid,
    uds_path: &StdPath,
    address: Ipv4Addr,
) -> Result<(u64, std::os::unix::net::UnixStream)> {
    let network_name = state
        .networks
        .lock()
        .await
        .summary(network)
        .map(|summary| summary.name)
        .unwrap_or_default();
    let token = format!("{:016x}", Uuid::new_v4().as_u128() as u64);
    let ask = ServiceToProcess::LinkAttach {
        id: Uuid::new_v4().as_u128() as u64,
        token: token.clone(),
        network: network.to_string(),
        network_name,
    };
    let handoff_socket = match send_ipc_command(uds_path, ask, Some(ATTACH_TIMEOUT_SECS)).await {
        Ok(ProcessToService::LinkAttachResult {
            error: None,
            handoff_socket,
            ..
        }) => handoff_socket,
        Ok(ProcessToService::LinkAttachResult { error: Some(error), .. }) => anyhow::bail!("owner refused: {error}"),
        Ok(other) => anyhow::bail!("unexpected owner reply: {other:?}"),
        Err(error) => anyhow::bail!("owner unreachable: {error}"),
    };
    let socket = std::os::unix::net::UnixStream::connect(&handoff_socket)
        .with_context(|| format!("connect the owner's seat at {handoff_socket}"))?;
    let sender = Sender::new(socket.try_clone()?)?;
    let receiver = Receiver::new(socket.try_clone()?)?;
    let token = u64::from_str_radix(&token, 16)?;
    sender
        .send(&seat_frame(SEAT_LINK, token), &[])
        .await
        .context("present the link token")?;
    let answer = tokio::time::timeout(STREAM_TIMEOUT, receiver.recv())
        .await
        .context("the owner did not hand the guest stream over in time")?
        .context("the owner closed the seat without the guest stream")?;
    ensure!(
        answer.fds.len() == 1,
        "the owner answered with {} descriptors, not one",
        answer.fds.len()
    );
    let stream = answer.fds.into_iter().next().unwrap();
    let host = host(state, network).await?;
    let id = host.link(address, stream.as_fd()).await?;
    Ok((id, socket))
}

/// Link a running member to its network's switch; a VM that is not running
/// stays `declared` until it is.
pub(crate) async fn link(state: &Arc<ServiceState>, network: Uuid, vm_id: &str) -> Result<()> {
    let running = state
        .instances
        .lock()
        .unwrap()
        .get(vm_id)
        .map(|instance| (instance.uds_path.clone(), instance.private_address));
    let Some((uds_path, address)) = running else {
        return Ok(());
    };
    {
        let mut registry = state.networks.lock().await;
        registry
            .attach(
                network,
                vm_id,
                address,
                MembershipState::Attaching,
                vm_lifecycle::unix_time_ms(),
            )
            .await
            .map_err(|error| anyhow!("{error}"))?;
        drop(registry);
    }
    let connection = Uuid::new_v4();
    match handshake(state, network, &uds_path, address).await {
        Ok((id, keepalive)) => {
            let mut networks = state.switches.networks.lock().await;
            let switch = networks.get_mut(&network).context("the network's switch vanished")?;
            switch.links.insert(
                id,
                Linked {
                    vm_id: vm_id.to_string(),
                    address,
                    connection,
                    keepalive,
                },
            );
            drop(networks);
            info!(%network, vm_id, %address, link_id = id, "private link ready");
            let event =
                link_event(network, connection, vm_id, address, Outcome::Linked).map_err(|error| anyhow!("{error}"))?;
            record(state, network, vm_id, address, MembershipState::Ready, event).await;
            Ok(())
        }
        Err(error) => {
            warn!(%network, vm_id, %address, %error, "private link failed");
            let event = link_event(
                network,
                connection,
                vm_id,
                address,
                Outcome::Refused(&format!("{error:#}")),
            )
            .map_err(|error| anyhow!("{error}"))?;
            record(state, network, vm_id, address, MembershipState::Failed, event).await;
            Err(error)
        }
    }
}

/// End a member's link, if it has one.
pub(crate) async fn unlink(state: &Arc<ServiceState>, network: Uuid, vm_id: &str) {
    let mut networks = state.switches.networks.lock().await;
    let Some(switch) = networks.get_mut(&network) else {
        return;
    };
    let id = switch
        .links
        .iter()
        .find(|(_, linked)| linked.vm_id == vm_id)
        .map(|(id, _)| *id);
    let Some(id) = id else { return };
    let linked = switch.links.remove(&id).unwrap();
    let host = Arc::clone(&switch.host);
    drop(networks);
    if let Err(error) = host.unlink(id).await {
        warn!(%network, vm_id, %error, "switch did not take the unlink");
    }
    drop(linked.keepalive);
    if let Ok(event) = link_event(
        network,
        linked.connection,
        vm_id,
        linked.address,
        Outcome::Ended("unlinked", None),
    ) {
        if let Err(error) = state.networks.lock().await.record(network, event).await {
            warn!(%network, vm_id, %error, "unlink row was not recorded");
        }
    }
}

/// Every network a VM belongs to gets its link, once the VM's owner runs:
/// after a provision, a resume, or a switch that came back.
pub(crate) fn link_memberships(state: Arc<ServiceState>, vm_id: String) {
    tokio::spawn(async move {
        let networks = state.networks.lock().await.memberships_of(&vm_id);
        for network in networks {
            if let Err(error) = link(&state, network, &vm_id).await {
                warn!(%network, vm_id, %error, "member was not linked at start");
            }
        }
    });
}

/// A VM is gone: its links end everywhere.
pub(crate) async fn unlink_everywhere(state: &Arc<ServiceState>, vm_id: &str) {
    let networks: Vec<Uuid> = state.switches.networks.lock().await.keys().copied().collect();
    for network in networks {
        unlink(state, network, vm_id).await;
    }
}

/// A retired network's switch stops.
pub(crate) async fn retire(state: &Arc<ServiceState>, network: Uuid) {
    if let Some(switch) = state.switches.networks.lock().await.remove(&network) {
        switch.host.closed.cancel();
    }
}
