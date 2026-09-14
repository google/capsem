//! One confined switch per network, and each running member's cable in it.
//!
//! The switch never sees the service's API: it gets one cable per plugged
//! member and reports when a port closes. Plugging a VM is a handshake with
//! its owner (`LinkAttach` over IPC, then the token on the handoff socket,
//! then the guest stream back on that connection) and a grant to the
//! switch; the service keeps the handoff connection as the port's
//! keepalive and drops it to unplug. Membership states follow: `declared`
//! until the VM runs, `attaching` during the handshake, `ready` once the
//! switch has the cable, `failed` when the owner or the switch refused, and
//! back to `declared` when the cable ends (the VM stopped or died). Every
//! transition writes a row in the network's ledger; a cable's close row
//! carries the switch's counters for its port, an unplugged one included.
//!
//! Membership is the authority, and every plug and unplug bumps the
//! attachment's generation. A plug finishes only if, under the registry
//! lock, the VM is still a member and its generation is still the one the
//! plug started with; otherwise it unplugs the port it just made. Leaving
//! revokes the membership first and unplugs second, so a disconnect that
//! returned can never leave a port behind (review finding 1 of PR #200).
use super::*;
use anyhow::ensure;
use capsem_core::net::network_registry::NetworkRegistry;
use capsem_core::net::switch_host::{PortReports, SwitchHost};
use capsem_foundation::unix::router_channel::{Receiver, Sender};
use capsem_logger::{MembershipState, TransportEvent, TransportEventKind};
use capsem_proto::privatelink::{seat_frame, SEAT_LINK};
use capsem_router::DropReason;
use capsem_router::PortReport;
use std::net::Ipv4Addr;
use std::os::fd::AsFd;
use std::time::Duration;
use tokio::sync::mpsc;
use uuid::Uuid;

/// How long the owner has to answer `LinkAttach`.
const ATTACH_TIMEOUT_SECS: u64 = 8;
/// How long the owner may take to hand the stream over: a VM plugged at
/// creation is still booting its guest.
const STREAM_TIMEOUT: Duration = Duration::from_secs(70);
/// How long a VM plugged as it starts may take to boot before its owner can
/// be asked for the stream.
const OWNER_READY_TIMEOUT_SECS: u64 = 70;

type Starting = std::pin::Pin<Box<dyn std::future::Future<Output = Result<Arc<SwitchHost>>> + Send>>;
type Starter = Arc<dyn Fn(Uuid, PortReports) -> Starting + Send + Sync>;

struct Plugged {
    vm_id: String,
    address: Ipv4Addr,
    generation: u32,
    connection: Uuid,
    /// The owner's handoff connection: dropped, the owner ends the guest stream.
    keepalive: std::os::unix::net::UnixStream,
}

/// A port the service unplugged whose close report has not arrived yet.
struct Unplugging {
    vm_id: String,
    address: Ipv4Addr,
    connection: Uuid,
}

struct NetworkSwitch {
    host: Arc<SwitchHost>,
    ports: HashMap<u64, Plugged>,
    unplugging: HashMap<u64, Unplugging>,
}

pub(crate) struct Switches {
    starter: Starter,
    networks: tokio::sync::Mutex<HashMap<Uuid, NetworkSwitch>>,
    /// The current generation of every attachment that was ever plugged or
    /// unplugged; never held across an await.
    generations: std::sync::Mutex<HashMap<(Uuid, String), u32>>,
    /// Holds a disconnect between revoking the membership and unplugging,
    /// so a test can finish a plug inside that window.
    #[cfg(test)]
    pub(crate) detach_window: std::sync::Mutex<Option<Arc<(tokio::sync::Notify, tokio::sync::Notify)>>>,
}

impl Switches {
    /// Switches as confined `capsem-router --network` children.
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
                    capsem_router::CONNECTION_LIMIT,
                ));
                let host = SwitchHost::attach(
                    Sender::new(parent.try_clone()?)?,
                    tokio::net::UnixStream::from_std(parent)?,
                    reports,
                    None,
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
            generations: std::sync::Mutex::new(HashMap::new()),
            #[cfg(test)]
            detach_window: std::sync::Mutex::new(None),
        }
    }

    /// Start a new generation of `vm_id`'s attachment to `network`: any plug
    /// still under way for an older one will unplug itself.
    fn next_generation(&self, network: Uuid, vm_id: &str) -> u32 {
        let mut generations = self.generations.lock().unwrap();
        let generation = generations.entry((network, vm_id.to_string())).or_insert(0);
        *generation = generation.wrapping_add(1).max(1);
        let current = *generation;
        drop(generations);
        current
    }

    fn is_current(&self, network: Uuid, vm_id: &str, generation: u32) -> bool {
        self.generations.lock().unwrap().get(&(network, vm_id.to_string())) == Some(&generation)
    }

    /// The ports plugged into `network`'s switch, for tests.
    #[cfg(test)]
    pub(crate) async fn plugged(&self, network: Uuid) -> Vec<String> {
        self.networks
            .lock()
            .await
            .get(&network)
            .map(|switch| switch.ports.values().map(|port| port.vm_id.clone()).collect())
            .unwrap_or_default()
    }
}

fn now_hex_id() -> String {
    format!(
        "{:012x}",
        u64::try_from(vm_lifecycle::unix_time_ms()).unwrap_or(0) & 0xffff_ffff_ffff
    )
}

/// What a ledger row about a cable says happened.
enum Outcome<'a> {
    Linked,
    Refused(&'a str),
    Ended(&'a str, Option<&'a PortReport>),
}

/// One ledger row about a VM's cable: the VM on both ends and no port.
fn link_event(
    network: Uuid,
    connection: Uuid,
    vm_id: &str,
    address: Ipv4Addr,
    outcome: Outcome<'_>,
) -> Result<TransportEvent, String> {
    let endpoint = json!({ "vm": { "id": vm_id }, "address": address.to_string(), "port": 0 });
    let (kind, decision, reason, report) = match outcome {
        Outcome::Linked => (TransportEventKind::Connect, "allow", "linked", None),
        Outcome::Refused(reason) => (TransportEventKind::Connect, "block", reason, None),
        Outcome::Ended(reason, report) => (TransportEventKind::Close, "allow", reason, report),
    };
    let mut facts = json!({
        "network": { "context": "private", "protocol": "link", "source": endpoint, "destination": endpoint },
        "decision": { "effective": decision, "reason": reason },
    });
    if let Some(report) = report {
        let dropped: serde_json::Map<String, serde_json::Value> = DropReason::ALL
            .iter()
            .map(|reason| (reason.name().to_string(), json!(report.dropped[*reason as usize])))
            .collect();
        facts["frames"] = json!({
            "reason": format!("{:?}", report.reason),
            "frames_in": report.frames_in,
            "bytes_in": report.bytes_in,
            "frames_out": report.frames_out,
            "bytes_out": report.bytes_out,
            "dropped": dropped,
        });
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

fn is_member(registry: &NetworkRegistry, network: Uuid, vm_id: &str) -> bool {
    registry
        .members(network)
        .is_some_and(|members| members.iter().any(|member| member.vm_id == vm_id))
}

/// Write a membership state and a ledger row; the caller holds the registry.
async fn record_locked(
    registry: &mut NetworkRegistry,
    network: Uuid,
    vm_id: &str,
    membership: MembershipState,
    event: Result<TransportEvent, String>,
) {
    if let Err(error) = registry
        .set_state(network, vm_id, membership, vm_lifecycle::unix_time_ms())
        .await
    {
        warn!(%network, vm_id, %error, "membership state was not recorded");
    }
    match event {
        Ok(event) => {
            if let Err(error) = registry.record(network, event).await {
                warn!(%network, vm_id, %error, "cable audit row was not recorded");
            }
        }
        Err(error) => warn!(%network, vm_id, %error, "cable audit row was not built"),
    }
}

/// The network's switch, started on first use. Its port reports are
/// consumed for as long as it lives; when it dies, every member it had is
/// plugged again into a fresh one.
async fn host(state: &Arc<ServiceState>, network: Uuid) -> Result<Arc<SwitchHost>> {
    let mut networks = state.switches.networks.lock().await;
    if let Some(switch) = networks.get(&network) {
        if !switch.host.is_closed() {
            return Ok(Arc::clone(&switch.host));
        }
    }
    let (reports, mut closed) = mpsc::channel(64);
    let host = (state.switches.starter)(network, reports).await?;
    let replaced = networks.insert(
        network,
        NetworkSwitch {
            host: Arc::clone(&host),
            ports: HashMap::new(),
            unplugging: HashMap::new(),
        },
    );
    drop(networks);
    if let Some(dead) = replaced {
        // A plug got here before the dead switch's watcher: its members are
        // replugged now, alongside this one.
        tokio::spawn(replug_orphans(Arc::clone(state), network, dead));
    }
    let watched = Arc::clone(state);
    let reporting = Arc::clone(&host);
    tokio::spawn(async move {
        while let Some((port, report)) = closed.recv().await {
            port_closed(&watched, network, port, &report).await;
        }
        let orphans = {
            let mut networks = watched.switches.networks.lock().await;
            match networks.get(&network) {
                Some(switch) if Arc::ptr_eq(&switch.host, &reporting) => networks.remove(&network),
                _ => None,
            }
        };
        match orphans {
            Some(orphans) => replug_orphans(watched, network, orphans).await,
            None => reporting.retire().await,
        }
    });
    Ok(host)
}

/// A switch is gone: it is reaped, and whoever was still plugged into it is
/// plugged again into a fresh one. Boxed: plugging starts a switch, whose
/// watcher replugs.
fn replug_orphans(
    state: Arc<ServiceState>,
    network: Uuid,
    dead: NetworkSwitch,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>> {
    Box::pin(async move {
        dead.host.retire().await;
        warn!(%network, switch_pid = ?dead.host.pid(), members = dead.ports.len(), "network switch ended; replugging its members");
        for (_, unplugged) in dead.unplugging {
            record_unplugged(&state, network, &unplugged, None).await;
        }
        for plugged in dead.ports.into_values() {
            drop(plugged.keepalive);
            if let Err(error) = plug(&state, network, &plugged.vm_id).await {
                warn!(%network, vm_id = plugged.vm_id, %error, "member was not replugged");
            }
        }
    })
}

/// The switch reported a port's end: the VM stopped, died, or its pump did.
/// A member whose owner still runs is plugged again once the pump has
/// reconnected; one that is gone stays `declared` until it resumes.
async fn port_closed(state: &Arc<ServiceState>, network: Uuid, port: u64, report: &PortReport) {
    let (plugged, unplugged) = match state.switches.networks.lock().await.get_mut(&network) {
        Some(switch) => (switch.ports.remove(&port), switch.unplugging.remove(&port)),
        None => (None, None),
    };
    if let Some(unplugged) = unplugged {
        record_unplugged(state, network, &unplugged, Some(report)).await;
        return;
    }
    let Some(plugged) = plugged else { return };
    info!(%network, vm_id = plugged.vm_id, ?report, "network cable closed");
    drop(plugged.keepalive);
    {
        let mut registry = state.networks.lock().await;
        if !is_member(&registry, network, &plugged.vm_id)
            || !state.switches.is_current(network, &plugged.vm_id, plugged.generation)
        {
            return;
        }
        let event = link_event(
            network,
            plugged.connection,
            &plugged.vm_id,
            plugged.address,
            Outcome::Ended("closed", Some(report)),
        );
        record_locked(&mut registry, network, &plugged.vm_id, MembershipState::Declared, event).await;
    }
    tokio::spawn(replug_later(Arc::clone(state), network, plugged.vm_id));
}

/// A member whose owner still runs is plugged again after its port closed:
/// its pump restarts after its first backoff and the owner waits for the
/// fresh stream. Boxed: plugging starts a switch whose watcher reports
/// closes here.
fn replug_later(
    state: Arc<ServiceState>,
    network: Uuid,
    vm_id: String,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>> {
    Box::pin(async move {
        tokio::time::sleep(Duration::from_secs(1)).await;
        if let Err(error) = plug(&state, network, &vm_id).await {
            info!(%network, vm_id, %error, "member not replugged after its port closed");
        }
    })
}

/// Ask the owner for the guest stream and plug it into the switch as
/// `generation`. Returns the port and the owner's handoff connection.
async fn handshake(
    state: &Arc<ServiceState>,
    network: Uuid,
    uds_path: &StdPath,
    address: Ipv4Addr,
    generation: u32,
) -> Result<(Arc<SwitchHost>, u64, std::os::unix::net::UnixStream)> {
    let summary = state
        .networks
        .lock()
        .await
        .summary(network)
        .context("the network retired during the plug")?;
    let token = format!("{:016x}", Uuid::new_v4().as_u128() as u64);
    let ask = ServiceToProcess::LinkAttach {
        id: Uuid::new_v4().as_u128() as u64,
        token: token.clone(),
        network: network.to_string(),
        network_name: summary.name,
        address,
        prefix: summary.subnet.prefix_len(),
        generation,
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
    let port = host.plug(generation, address, stream.as_fd()).await?;
    Ok((host, port, socket))
}

/// Plug a running member's cable into its network's switch; a VM that is
/// not running stays `declared` until it is. Membership and generation are
/// checked again, under the registry lock, once the switch has the cable: a
/// member that left meanwhile, or a newer plug or unplug, unplugs it.
pub(crate) async fn plug(state: &Arc<ServiceState>, network: Uuid, vm_id: &str) -> Result<()> {
    let running = state
        .instances
        .lock()
        .unwrap()
        .get(vm_id)
        .map(|instance| instance.uds_path.clone());
    let Some(uds_path) = running else {
        return Ok(());
    };
    let (generation, address) = {
        let mut registry = state.networks.lock().await;
        let Some(address) = registry.address_of(network, vm_id) else {
            return Ok(());
        };
        registry
            .set_state(network, vm_id, MembershipState::Attaching, vm_lifecycle::unix_time_ms())
            .await
            .map_err(|error| anyhow!("{error}"))?;
        let generation = state.switches.next_generation(network, vm_id);
        drop(registry);
        (generation, address)
    };
    let connection = Uuid::new_v4();
    match handshake(state, network, &uds_path, address, generation).await {
        Ok((host, port, keepalive)) => {
            let mut registry = state.networks.lock().await;
            if !is_member(&registry, network, vm_id) || !state.switches.is_current(network, vm_id, generation) {
                drop(registry);
                info!(%network, vm_id, generation, "plug outlived its attachment; unplugging it");
                if let Err(error) = host.unplug(port).await {
                    warn!(%network, vm_id, %error, "switch did not take the stale unplug");
                }
                drop(keepalive);
                return Ok(());
            }
            state
                .switches
                .networks
                .lock()
                .await
                .get_mut(&network)
                .filter(|switch| Arc::ptr_eq(&switch.host, &host))
                .context("the network's switch was replaced during the plug")?
                .ports
                .insert(
                    port,
                    Plugged {
                        vm_id: vm_id.to_string(),
                        address,
                        generation,
                        connection,
                        keepalive,
                    },
                );
            info!(%network, vm_id, %address, port, "network cable plugged");
            let event = link_event(network, connection, vm_id, address, Outcome::Linked);
            record_locked(&mut registry, network, vm_id, MembershipState::Ready, event).await;
            drop(registry);
            Ok(())
        }
        Err(error) => {
            warn!(%network, vm_id, %address, %error, "network cable was not plugged");
            let mut registry = state.networks.lock().await;
            if is_member(&registry, network, vm_id) && state.switches.is_current(network, vm_id, generation) {
                let event = link_event(
                    network,
                    connection,
                    vm_id,
                    address,
                    Outcome::Refused(&format!("{error:#}")),
                );
                record_locked(&mut registry, network, vm_id, MembershipState::Failed, event).await;
            }
            drop(registry);
            Err(error)
        }
    }
}

/// Unplug a member's cable, if it has one, and end any plug under way.
/// Returns the attachment generation the unplug started, which every plug
/// before it is older than.
pub(crate) async fn unplug(state: &Arc<ServiceState>, network: Uuid, vm_id: &str) -> u32 {
    let generation = state.switches.next_generation(network, vm_id);
    let mut networks = state.switches.networks.lock().await;
    let Some(switch) = networks.get_mut(&network) else {
        return generation;
    };
    let port = switch
        .ports
        .iter()
        .find(|(_, plugged)| plugged.vm_id == vm_id)
        .map(|(port, _)| *port);
    let Some(port) = port else { return generation };
    let plugged = switch.ports.remove(&port).unwrap();
    let unplugged = Unplugging {
        vm_id: plugged.vm_id,
        address: plugged.address,
        connection: plugged.connection,
    };
    switch.unplugging.insert(port, unplugged);
    let host = Arc::clone(&switch.host);
    drop(networks);
    drop(plugged.keepalive);
    // The close row waits for the switch's report on the port, which carries
    // its counters; a switch that cannot take the unplug will send none.
    if let Err(error) = host.unplug(port).await {
        warn!(%network, vm_id, %error, "switch did not take the unplug");
        let unplugged = match state.switches.networks.lock().await.get_mut(&network) {
            Some(switch) => switch.unplugging.remove(&port),
            None => None,
        };
        if let Some(unplugged) = unplugged {
            record_unplugged(state, network, &unplugged, None).await;
        }
    }
    generation
}

/// The close row of a cable the service unplugged, with the switch's report
/// when there is one.
async fn record_unplugged(
    state: &Arc<ServiceState>,
    network: Uuid,
    unplugged: &Unplugging,
    report: Option<&PortReport>,
) {
    let vm_id = &unplugged.vm_id;
    let event = link_event(
        network,
        unplugged.connection,
        vm_id,
        unplugged.address,
        Outcome::Ended("unlinked", report),
    );
    match event {
        Ok(event) => {
            if let Err(error) = state.networks.lock().await.record(network, event).await {
                warn!(%network, vm_id, %error, "unplug row was not recorded");
            }
        }
        Err(error) => warn!(%network, vm_id, %error, "unplug row was not built"),
    }
}

/// Revoke a membership, then unplug its cable: in that order, a plug that
/// finishes in between finds the membership gone and unplugs itself.
pub(crate) async fn detach(
    state: &Arc<ServiceState>,
    network: Uuid,
    vm_id: &str,
) -> Result<(), capsem_core::net::network_registry::NetworkError> {
    state
        .networks
        .lock()
        .await
        .detach(network, vm_id, vm_lifecycle::unix_time_ms())
        .await?;
    #[cfg(test)]
    {
        let window = state.switches.detach_window.lock().unwrap().clone();
        if let Some(window) = window {
            window.0.notify_one();
            window.1.notified().await;
        }
    }
    let generation = unplug(state, network, vm_id).await;
    take_cable_down(state, network, vm_id, generation);
    Ok(())
}

/// A running VM that left a network has its owner take that cable down in
/// the guest. Nothing waits on it: the VM already left, and an owner that
/// cannot answer is a VM that is stopping. The request names the leave's
/// generation, so if it reaches the owner after a rejoin's plug, the owner
/// keeps the rejoined cable.
fn take_cable_down(state: &Arc<ServiceState>, network: Uuid, vm_id: &str, generation: u32) {
    let running = state
        .instances
        .lock()
        .unwrap()
        .get(vm_id)
        .map(|instance| instance.uds_path.clone());
    let Some(uds_path) = running else { return };
    let vm_id = vm_id.to_string();
    tokio::spawn(async move {
        let ask = ServiceToProcess::LinkDetach {
            id: Uuid::new_v4().as_u128() as u64,
            network: network.to_string(),
            generation,
        };
        match send_ipc_command(&uds_path, ask, Some(ATTACH_TIMEOUT_SECS)).await {
            Ok(ProcessToService::LinkDetachResult { error: None, .. }) => {}
            Ok(ProcessToService::LinkDetachResult { error: Some(error), .. }) => {
                warn!(%network, vm_id, %error, "owner did not take the cable down")
            }
            Ok(other) => warn!(%network, vm_id, ?other, "unexpected owner reply to a detach"),
            Err(error) => info!(%network, vm_id, %error, "owner unreachable to take the cable down"),
        }
    });
}

/// Every network a VM belongs to gets its cable plugged, once the VM's owner
/// runs: after a provision, a resume, or a switch that came back.
pub(crate) fn plug_memberships(state: Arc<ServiceState>, vm_id: String) {
    tokio::spawn(async move {
        let networks = state.networks.lock().await.memberships_of(&vm_id);
        if networks.is_empty() {
            return;
        }
        // This runs as the VM starts, before its owner binds the socket the
        // plug asks through; asking then failed every plug for good. The
        // ready sentinel is the owner's word that it answers, and the wait
        // ends early if the VM goes away.
        let uds_path = state
            .instances
            .lock()
            .unwrap()
            .get(&vm_id)
            .map(|instance| instance.uds_path.clone());
        let Some(uds_path) = uds_path else { return };
        if let Err(error) =
            crate::vm_files::wait_for_vm_ready(&uds_path, OWNER_READY_TIMEOUT_SECS, Some(&state), Some(&vm_id)).await
        {
            warn!(vm_id, %error, "members were not plugged: the VM's owner never became ready");
            return;
        }
        for network in networks {
            if let Err(error) = plug(&state, network, &vm_id).await {
                warn!(%network, vm_id, %error, "member was not plugged at start");
            }
        }
    });
}

/// A VM is gone: its cables are unplugged everywhere.
pub(crate) async fn unplug_everywhere(state: &Arc<ServiceState>, vm_id: &str) {
    let networks: Vec<Uuid> = state.switches.networks.lock().await.keys().copied().collect();
    for network in networks {
        unplug(state, network, vm_id).await;
    }
}

/// A retired network's switch stops for good: its process is reaped before
/// this returns.
pub(crate) async fn retire(state: &Arc<ServiceState>, network: Uuid) {
    let switch = state.switches.networks.lock().await.remove(&network);
    state
        .switches
        .generations
        .lock()
        .unwrap()
        .retain(|(attached, _), _| *attached != network);
    if let Some(switch) = switch {
        switch.host.retire().await;
        for (_, unplugged) in switch.unplugging {
            record_unplugged(state, network, &unplugged, None).await;
        }
    }
}
