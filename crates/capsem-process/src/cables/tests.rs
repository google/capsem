use super::*;
use capsem_core::net::policy_config::{SecurityRuleProfile, SecurityRuleSet, SecurityRuleSource};
use capsem_core::security_engine::network::ledger::NetworkSecurity;
use capsem_foundation::unix::router_channel::{Receiver, Sender};
use std::os::fd::AsRawFd;
use std::sync::RwLock;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const TEAM: &str = "6ba7b810-9dad-11d1-80b4-00c04fd430c8";
const OTHER: &str = "6ba7b811-9dad-11d1-80b4-00c04fd430c8";
const IN_TEAM: Ipv4Addr = Ipv4Addr::new(10, 128, 0, 3);
const IN_OTHER: Ipv4Addr = Ipv4Addr::new(10, 128, 1, 3);

/// A guest stream over a socket pair: the owner's end as a `VsockConnection`,
/// the guest's end for the test to watch.
fn guest_stream() -> (VsockConnection, tokio::net::UnixStream) {
    let (owner, guest) = std::os::unix::net::UnixStream::pair().unwrap();
    guest.set_nonblocking(true).unwrap();
    let raw = owner.as_raw_fd();
    let anchor: Box<dyn Send> = Box::new(owner);
    (
        VsockConnection::new(raw, capsem_proto::VSOCK_PORT_NETWORK, anchor),
        tokio::net::UnixStream::from_std(guest).unwrap(),
    )
}

fn authorized_publisher(action: &str) -> Arc<Publisher> {
    let text = format!(
        "[profiles.rules.private]\nname = \"private\"\naction = \"{action}\"\nmatch = 'network.mode == \"private\" && network.protocol == \"link\"'"
    );
    let rules = SecurityRuleSet::compile_profile(
        &SecurityRuleProfile::parse_toml(&text).unwrap(),
        SecurityRuleSource::User,
    )
    .unwrap();
    let engine = Arc::new(NetworkSecurity {
        db: Arc::new(capsem_logger::DbWriter::open_in_memory(128).unwrap()),
        rules: Arc::new(RwLock::new(Arc::new(rules))),
        plugins: Arc::new(RwLock::new(Arc::new(std::collections::BTreeMap::new()))),
    });
    Arc::new(Publisher::default().with_security("vm-b".into(), "beta".into(), engine))
}

fn network(id: &str) -> NetworkIdentity {
    NetworkIdentity::parse(id, "team".into()).unwrap()
}

/// Cables whose guest instructions land in a channel the test reads.
fn cables(action: &str) -> (Arc<Cables>, mpsc::Receiver<ServiceToProcess>) {
    let (control, instructions) = mpsc::channel(16);
    (
        Arc::new(Cables::new(authorized_publisher(action), control)),
        instructions,
    )
}

async fn instruction(instructions: &mut mpsc::Receiver<ServiceToProcess>) -> ServiceToProcess {
    tokio::time::timeout(Duration::from_secs(2), instructions.recv())
        .await
        .expect("the guest is told")
        .expect("the control channel is open")
}

/// Expect a plug for `network` and return the cable the guest was told to
/// bring up for it.
async fn plugged(
    cables: &Cables,
    instructions: &mut mpsc::Receiver<ServiceToProcess>,
    token: u64,
    id: &str,
    address: Ipv4Addr,
) -> u32 {
    cables
        .expect(&format!("{token:016x}"), network(id), address, 24)
        .await
        .unwrap();
    match instruction(instructions).await {
        ServiceToProcess::PlugCable {
            cable,
            address: told,
            prefix,
        } => {
            assert_eq!((told, prefix), (address, 24));
            cable
        }
        other => panic!("expected PlugCable, got {other:?}"),
    }
}

/// The service's side of one plug: the token frame goes out, the answer
/// frame and its descriptor come back.
struct Service {
    sender: Sender,
    receiver: Receiver,
}

impl Service {
    fn asks(cables: &Arc<Cables>) -> Self {
        let (owner_side, service_side) = std::os::unix::net::UnixStream::pair().unwrap();
        owner_side.set_nonblocking(true).unwrap();
        let taking = Arc::clone(cables);
        tokio::spawn(async move {
            let std = owner_side;
            let receiver = Receiver::new(std.try_clone().unwrap()).unwrap();
            let frame = receiver.recv().await.unwrap();
            let token = u64::from_be_bytes(frame.bytes[2..].try_into().unwrap());
            if let Err(error) = taking.take(token, std).await {
                tracing::debug!(%error, "plug refused");
            }
        });
        Self {
            sender: Sender::new(service_side.try_clone().unwrap()).unwrap(),
            receiver: Receiver::new(service_side).unwrap(),
        }
    }

    async fn present(&self, token: u64) {
        self.sender.send(&seat_frame(SEAT_LINK, token), &[]).await.unwrap();
    }

    async fn stream(&self) -> Option<tokio::net::UnixStream> {
        let frame = tokio::time::timeout(Duration::from_secs(3), self.receiver.recv())
            .await
            .expect("the owner answers")
            .ok()?;
        assert_eq!(frame.bytes[1], SEAT_LINK);
        let fd = frame.fds.into_iter().next().expect("the answer carries the stream");
        let std = std::os::unix::net::UnixStream::from(fd);
        std.set_nonblocking(true).unwrap();
        Some(tokio::net::UnixStream::from_std(std).unwrap())
    }
}

async fn ended(guest: &mut tokio::net::UnixStream) -> bool {
    matches!(
        tokio::time::timeout(Duration::from_secs(2), guest.read(&mut [0u8; 1])).await,
        Ok(Ok(0)) | Ok(Err(_))
    )
}

#[tokio::test]
async fn plugging_a_network_brings_its_cable_up_in_the_guest_once() {
    let (cables, mut instructions) = cables("allow");
    let team = plugged(&cables, &mut instructions, 0xa1, TEAM, IN_TEAM).await;
    let again = plugged(&cables, &mut instructions, 0xa2, TEAM, IN_TEAM).await;
    assert_eq!(again, team, "a network keeps its cable across plugs");
    let other = plugged(&cables, &mut instructions, 0xa3, OTHER, IN_OTHER).await;
    assert_ne!(other, team, "every network has its own cable");
    assert!(team > 0 && other > 0, "cable ids start at one");
}

#[tokio::test]
async fn a_blocked_profile_refuses_the_plug_and_a_token_is_expected_once() {
    let (blocked, mut told) = cables("block");
    let refused = blocked
        .expect("00000000000000aa", network(TEAM), IN_TEAM, 24)
        .await
        .unwrap_err();
    assert!(format!("{refused:#}").contains("block"), "{refused:#}");
    assert!(told.try_recv().is_err(), "a refused plug brings no cable up");
    let (control, _) = mpsc::channel(1);
    let unconfigured = Cables::new(Arc::new(Publisher::default()), control);
    assert!(unconfigured
        .expect("00000000000000aa", network(TEAM), IN_TEAM, 24)
        .await
        .is_err());
    let (allowed, _told) = cables("allow");
    allowed
        .expect("00000000000000aa", network(TEAM), IN_TEAM, 24)
        .await
        .unwrap();
    assert!(
        allowed
            .expect("00000000000000aa", network(TEAM), IN_TEAM, 24)
            .await
            .is_err(),
        "the same token cannot be expected twice"
    );
    assert!(allowed.redeem(0xaa).is_some());
    assert!(allowed.redeem(0xaa).is_none(), "redeemed once");
}

#[tokio::test]
async fn a_reconnecting_pump_replaces_its_cables_stream_and_no_other() {
    let (cables, mut instructions) = cables("allow");
    let team = plugged(&cables, &mut instructions, 0xb1, TEAM, IN_TEAM).await;
    let other = plugged(&cables, &mut instructions, 0xb2, OTHER, IN_OTHER).await;
    let (first, mut first_guest) = guest_stream();
    cables.attach_guest(team, first);
    let (beside, mut beside_guest) = guest_stream();
    cables.attach_guest(other, beside);
    let (second, _second_guest) = guest_stream();
    cables.attach_guest(team, second);
    assert!(ended(&mut first_guest).await, "the replaced stream ends");
    assert!(
        tokio::time::timeout(Duration::from_millis(200), beside_guest.read(&mut [0u8; 1]))
            .await
            .is_err(),
        "the other cable's stream is untouched"
    );
}

#[tokio::test]
async fn a_stream_for_a_cable_nobody_plugged_is_closed() {
    let (cables, mut instructions) = cables("allow");
    let team = plugged(&cables, &mut instructions, 0xb3, TEAM, IN_TEAM).await;
    let (conn, mut guest) = guest_stream();
    cables.attach_guest(team + 7, conn);
    assert!(ended(&mut guest).await);
}

#[tokio::test]
async fn two_networks_plugged_at_once_each_get_their_own_cable() {
    let (cables, mut instructions) = cables("allow");
    let team = plugged(&cables, &mut instructions, 0xc1, TEAM, IN_TEAM).await;
    let other = plugged(&cables, &mut instructions, 0xc2, OTHER, IN_OTHER).await;
    let (conn, mut team_guest) = guest_stream();
    cables.attach_guest(team, conn);
    let (conn, mut other_guest) = guest_stream();
    cables.attach_guest(other, conn);
    let team_service = Service::asks(&cables);
    team_service.present(0xc1).await;
    let other_service = Service::asks(&cables);
    other_service.present(0xc2).await;
    let mut team_stream = team_service.stream().await.expect("team's cable");
    let mut other_stream = other_service
        .stream()
        .await
        .expect("other's cable, while team holds its own");
    team_stream.write_all(b"team").await.unwrap();
    other_stream.write_all(b"other").await.unwrap();
    let mut heard = [0u8; 4];
    team_guest.read_exact(&mut heard).await.unwrap();
    assert_eq!(&heard, b"team");
    let mut heard = [0u8; 5];
    other_guest.read_exact(&mut heard).await.unwrap();
    assert_eq!(&heard, b"other");
}

#[tokio::test]
async fn the_service_gets_the_cable_and_the_owner_lets_go_when_the_service_does() {
    let (cables, mut instructions) = cables("allow");
    let team = plugged(&cables, &mut instructions, 0xbb, TEAM, IN_TEAM).await;
    let (conn, mut guest) = guest_stream();
    cables.attach_guest(team, conn);
    let service = Service::asks(&cables);
    service.present(0xbb).await;
    let mut stream = service.stream().await.expect("the plug is granted");
    stream.write_all(b"to the guest").await.unwrap();
    let mut heard = [0u8; 12];
    guest.read_exact(&mut heard).await.unwrap();
    assert_eq!(&heard, b"to the guest");
    guest.write_all(b"from the guest").await.unwrap();
    let mut heard = [0u8; 14];
    stream.read_exact(&mut heard).await.unwrap();
    assert_eq!(&heard, b"from the guest");
    // The service is done: the owner ends the cable's stream so the pump
    // reconnects for the next plug.
    drop(service);
    drop(stream);
    assert!(ended(&mut guest).await, "the guest stream ends");
}

#[tokio::test]
async fn a_plug_asked_before_the_pump_connected_waits_for_it() {
    let (cables, mut instructions) = cables("allow");
    let team = plugged(&cables, &mut instructions, 0xcc, TEAM, IN_TEAM).await;
    let service = Service::asks(&cables);
    service.present(0xcc).await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    let (conn, mut guest) = guest_stream();
    cables.attach_guest(team, conn);
    let mut stream = service.stream().await.expect("granted once the pump is there");
    stream.write_all(b"late").await.unwrap();
    let mut heard = [0u8; 4];
    guest.read_exact(&mut heard).await.unwrap();
    assert_eq!(&heard, b"late");
}

#[tokio::test]
async fn an_unknown_token_gets_no_stream() {
    let (cables, mut instructions) = cables("allow");
    let team = plugged(&cables, &mut instructions, 0xd1, TEAM, IN_TEAM).await;
    let (conn, _guest) = guest_stream();
    cables.attach_guest(team, conn);
    let service = Service::asks(&cables);
    service.present(0xdd).await;
    assert!(service.stream().await.is_none(), "the connection ends without a frame");
}

/// A plug that fails after taking the stream lets go of it exactly as a
/// released one does: the cable can be plugged again once the pump
/// reconnects.
async fn assert_let_go_and_pluggable_again(
    cables: &Arc<Cables>,
    instructions: &mut mpsc::Receiver<ServiceToProcess>,
    guest: &mut tokio::net::UnixStream,
    token: u64,
) {
    assert!(
        ended(guest).await,
        "the stream the failed plug took ends, so the pump reconnects"
    );
    let team = plugged(cables, instructions, token, TEAM, IN_TEAM).await;
    let again = Service::asks(cables);
    again.present(token).await;
    let (conn, _fresh_guest) = guest_stream();
    cables.attach_guest(team, conn);
    assert!(again.stream().await.is_some(), "the cable plugs again");
}

#[tokio::test]
async fn a_plug_whose_answer_cannot_be_sent_does_not_keep_the_cable_held() {
    let (cables, mut instructions) = cables("allow");
    let team = plugged(&cables, &mut instructions, 0xf1, TEAM, IN_TEAM).await;
    let (conn, mut guest) = guest_stream();
    cables.attach_guest(team, conn);
    let (owner_side, service_side) = std::os::unix::net::UnixStream::pair().unwrap();
    drop(service_side);
    let failed = cables
        .take_within(0xf1, owner_side, Duration::from_secs(1))
        .await
        .unwrap_err();
    assert!(format!("{failed:#}").contains("answer the service"), "{failed:#}");
    assert_let_go_and_pluggable_again(&cables, &mut instructions, &mut guest, 0xf2).await;
}

#[tokio::test]
async fn a_plug_abandoned_while_held_does_not_keep_the_cable_held() {
    let (cables, mut instructions) = cables("allow");
    let team = plugged(&cables, &mut instructions, 0xf3, TEAM, IN_TEAM).await;
    let (conn, mut guest) = guest_stream();
    cables.attach_guest(team, conn);
    let (owner_side, service_side) = std::os::unix::net::UnixStream::pair().unwrap();
    owner_side.set_nonblocking(true).unwrap();
    let abandoned = tokio::time::timeout(
        Duration::from_millis(300),
        cables.take_within(0xf3, owner_side, Duration::from_secs(1)),
    )
    .await;
    assert!(abandoned.is_err(), "the plug was still held when it was dropped");
    drop(service_side);
    assert_let_go_and_pluggable_again(&cables, &mut instructions, &mut guest, 0xf4).await;
}

#[tokio::test]
async fn after_a_release_the_next_plug_waits_for_a_fresh_stream() {
    let (cables, mut instructions) = cables("allow");
    let team = plugged(&cables, &mut instructions, 0xe3, TEAM, IN_TEAM).await;
    let (conn, mut guest) = guest_stream();
    cables.attach_guest(team, conn);
    let service = Service::asks(&cables);
    service.present(0xe3).await;
    let stream = service.stream().await.expect("plugged");
    drop(service);
    drop(stream);
    assert!(ended(&mut guest).await, "released");
    // The pump has not reconnected yet: the stream the old port may still
    // be draining is never handed out again.
    plugged(&cables, &mut instructions, 0xe4, TEAM, IN_TEAM).await;
    let (owner_side, service_side) = std::os::unix::net::UnixStream::pair().unwrap();
    let stale = tokio::time::timeout(
        Duration::from_secs(2),
        cables.take_within(0xe4, owner_side, Duration::from_millis(300)),
    )
    .await
    .expect("bounded")
    .unwrap_err();
    assert!(format!("{stale:#}").contains("fresh"), "{stale:#}");
    drop(service_side);
    plugged(&cables, &mut instructions, 0xe5, TEAM, IN_TEAM).await;
    let again = Service::asks(&cables);
    again.present(0xe5).await;
    let (conn, mut fresh_guest) = guest_stream();
    cables.attach_guest(team, conn);
    let mut stream = again.stream().await.expect("the fresh stream is plugged");
    stream.write_all(b"fresh").await.unwrap();
    let mut heard = [0u8; 5];
    fresh_guest.read_exact(&mut heard).await.unwrap();
    assert_eq!(&heard, b"fresh");
}

#[tokio::test]
async fn detaching_takes_the_cable_down_and_leaves_the_other_one() {
    let (cables, mut instructions) = cables("allow");
    let team = plugged(&cables, &mut instructions, 0x91, TEAM, IN_TEAM).await;
    let other = plugged(&cables, &mut instructions, 0x92, OTHER, IN_OTHER).await;
    let (conn, mut team_guest) = guest_stream();
    cables.attach_guest(team, conn);
    let (conn, mut other_guest) = guest_stream();
    cables.attach_guest(other, conn);
    cables.detach(TEAM).await.unwrap();
    assert!(matches!(
        instruction(&mut instructions).await,
        ServiceToProcess::UnplugCable { cable } if cable == team
    ));
    assert!(ended(&mut team_guest).await, "the detached cable's stream ends");
    assert!(
        tokio::time::timeout(Duration::from_millis(200), other_guest.read(&mut [0u8; 1]))
            .await
            .is_err(),
        "the other network's cable stays"
    );
    let (late, mut late_guest) = guest_stream();
    cables.attach_guest(team, late);
    assert!(
        ended(&mut late_guest).await,
        "a pump still dialing a detached cable is closed"
    );
    assert!(cables.detach(TEAM).await.is_ok(), "detaching twice is harmless");
    let again = plugged(&cables, &mut instructions, 0x93, TEAM, IN_TEAM).await;
    assert_ne!(again, team, "a network plugged after detaching gets a new cable");
}
