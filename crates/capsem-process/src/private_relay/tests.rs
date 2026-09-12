use super::*;
use capsem_core::net::policy_config::{SecurityRuleProfile, SecurityRuleSet, SecurityRuleSource};
use capsem_core::security_engine::network::ledger::NetworkSecurity;
use std::num::NonZeroU64;
use std::os::fd::AsRawFd;
use std::sync::RwLock;
use tokio::io::AsyncReadExt;

const A: Ipv4Addr = Ipv4Addr::new(10, 128, 0, 2);
const B: Ipv4Addr = Ipv4Addr::new(10, 128, 0, 3);
const STRANGER: Ipv4Addr = Ipv4Addr::new(10, 128, 0, 9);

/// IPv4 + UDP headers, as the guest kernel would emit them (checksums are
/// not read by the relay).
fn udp(source: Ipv4Addr, sport: u16, destination: Ipv4Addr, dport: u16, payload: &[u8]) -> Vec<u8> {
    let udp_len = 8 + payload.len();
    let total = 20 + udp_len;
    let mut packet = vec![0u8; total];
    packet[0] = 0x45;
    packet[2..4].copy_from_slice(&(total as u16).to_be_bytes());
    packet[8] = 64;
    packet[9] = 17;
    packet[12..16].copy_from_slice(&source.octets());
    packet[16..20].copy_from_slice(&destination.octets());
    packet[20..22].copy_from_slice(&sport.to_be_bytes());
    packet[22..24].copy_from_slice(&dport.to_be_bytes());
    packet[24..26].copy_from_slice(&(udp_len as u16).to_be_bytes());
    packet[28..].copy_from_slice(payload);
    packet
}

fn authorized_publisher(action: &str) -> Arc<Publisher> {
    let text = format!(
        "[profiles.rules.private]\nname = \"private\"\naction = \"{action}\"\nmatch = 'network.mode == \"private\"'"
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

fn relay_at(dir: &tempfile::TempDir, own: Ipv4Addr, publisher: Arc<Publisher>, service: PathBuf) -> Arc<PrivateRelay> {
    Arc::new(PrivateRelay::new(
        dir.path().join("relay.sock"),
        own,
        publisher,
        service,
        "secret-a".into(),
        "vm-a".into(),
    ))
}

fn network() -> NetworkIdentity {
    NetworkIdentity::parse("6ba7b810-9dad-11d1-80b4-00c04fd430c8", "team".into()).unwrap()
}

fn member(id: &str, address: Ipv4Addr) -> (NetworkVm, Ipv4Addr) {
    (
        NetworkVm {
            id: id.into(),
            name: id.into(),
            generation: NonZeroU64::new(7).unwrap(),
        },
        address,
    )
}

/// A fake guest tun0: the owner's end goes to `attach_guest`, the test keeps
/// the guest's end and speaks frames on it.
fn guest_stream(relay: &Arc<PrivateRelay>) -> UnixStream {
    let (owner_end, guest_end) = std::os::unix::net::UnixStream::pair().unwrap();
    guest_end.set_nonblocking(true).unwrap();
    relay.attach_guest(VsockConnection::new(owner_end.as_raw_fd(), 5009, Box::new(owner_end)));
    UnixStream::from_std(guest_end).unwrap()
}

async fn next_frame(stream: &mut UnixStream) -> Option<Vec<u8>> {
    let mut packet = Vec::new();
    tokio::time::timeout(Duration::from_secs(2), read_frame(stream, &mut packet))
        .await
        .ok()?
        .ok()?
        .map(|length| packet[..length].to_vec())
}

#[tokio::test]
async fn an_unknown_token_or_an_unadmitted_frame_is_refused_and_the_stream_closed() {
    let dir = tempfile::tempdir().unwrap();
    let relay = relay_at(&dir, B, authorized_publisher("allow"), dir.path().join("service.sock"));
    let (owner_side, mut peer) = UnixStream::pair().unwrap();
    let taking = tokio::spawn({
        let relay = Arc::clone(&relay);
        async move { relay.take(owner_side).await }
    });
    write_frame(&mut peer, &0x99u64.to_be_bytes()).await.unwrap();
    let refused = taking.await.unwrap().unwrap_err();
    assert!(
        format!("{refused:#}").contains("unknown, reused or expired"),
        "{refused:#}"
    );
    assert_eq!(
        peer.read(&mut [0u8; 1]).await.unwrap(),
        0,
        "the stream is closed without a frame"
    );
}

#[tokio::test]
async fn a_blocked_rule_refuses_the_flow_before_any_frame_reaches_the_guest() {
    let dir = tempfile::tempdir().unwrap();
    let relay = relay_at(&dir, B, authorized_publisher("block"), dir.path().join("service.sock"));
    let (source, source_address) = member("vm-a", A);
    relay
        .expect(
            "00000000000000aa",
            network(),
            source,
            source_address,
            40000,
            5353,
            NetworkProtocol::Udp,
        )
        .unwrap();
    let (owner_side, mut peer) = UnixStream::pair().unwrap();
    let taking = tokio::spawn({
        let relay = Arc::clone(&relay);
        async move { relay.take(owner_side).await }
    });
    write_frame(&mut peer, &0xaau64.to_be_bytes()).await.unwrap();
    let refused = taking.await.unwrap().unwrap_err();
    assert!(
        format!("{refused:#}").contains("security decision: Block"),
        "{refused:#}"
    );
    assert_eq!(relay.engine.lock().unwrap().flows(), 0);
}

#[tokio::test]
async fn the_destination_seat_delivers_the_admitted_flow_and_returns_the_guests_answer() {
    let dir = tempfile::tempdir().unwrap();
    let relay = relay_at(&dir, B, authorized_publisher("allow"), dir.path().join("service.sock"));
    let mut guest = guest_stream(&relay);
    let (source, source_address) = member("vm-a", A);
    relay
        .expect(
            "00000000000000ab",
            network(),
            source,
            source_address,
            40000,
            5353,
            NetworkProtocol::Udp,
        )
        .unwrap();
    let (owner_side, mut peer) = UnixStream::pair().unwrap();
    let taking = tokio::spawn({
        let relay = Arc::clone(&relay);
        async move { relay.take(owner_side).await }
    });
    write_frame(&mut peer, &0xabu64.to_be_bytes()).await.unwrap();
    taking.await.unwrap().unwrap();
    let query = udp(A, 40000, B, 5353, b"query");
    write_frame(&mut peer, &query).await.unwrap();
    assert_eq!(next_frame(&mut guest).await.unwrap(), query);
    // A forged source and another socket's port never reach the guest.
    write_frame(&mut peer, &udp(STRANGER, 40000, B, 5353, b"forged"))
        .await
        .unwrap();
    write_frame(&mut peer, &udp(A, 40001, B, 5353, b"other")).await.unwrap();
    // The guest's answer returns on the same stream, no ask.
    let answer = udp(B, 5353, A, 40000, b"answer");
    write_frame(&mut guest, &answer).await.unwrap();
    assert_eq!(next_frame(&mut peer).await.unwrap(), answer);
    assert!(next_frame(&mut guest).await.is_none(), "dropped frames stay dropped");
    let counters = relay.engine.lock().unwrap().counters();
    assert_eq!(
        (counters.delivered, counters.forwarded, counters.dropped_forged),
        (1, 1, 1)
    );
    // The peer letting go ends the flow.
    drop(peer);
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(relay.engine.lock().unwrap().flows(), 0);
    assert!(relay.peers.lock().unwrap().is_empty());
}

/// The service the source seat asks, answering the datagram route as told.
async fn fake_service(socket: PathBuf, status: u16, answer: serde_json::Value) -> Arc<Mutex<Vec<serde_json::Value>>> {
    use axum::{routing::post, Router};
    let seen = Arc::new(Mutex::new(Vec::new()));
    let app = Router::new().route(
        "/networks/private/datagram",
        post({
            let seen = Arc::clone(&seen);
            move |axum::Json(body): axum::Json<serde_json::Value>| {
                let seen = Arc::clone(&seen);
                let answer = answer.clone();
                async move {
                    seen.lock().unwrap().push(body);
                    (axum::http::StatusCode::from_u16(status).unwrap(), axum::Json(answer))
                }
            }
        }),
    );
    let listener = UnixListener::bind(&socket).unwrap();
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    seen
}

#[tokio::test]
async fn the_source_seat_asks_once_relays_the_held_packets_and_returns_replies() {
    let dir = tempfile::tempdir().unwrap();
    let service = dir.path().join("service.sock");
    let destination_socket = dir.path().join("vm-b-relay.sock");
    let destination = UnixListener::bind(&destination_socket).unwrap();
    let seen = fake_service(
        service.clone(),
        200,
        serde_json::json!({
            "token": "00000000000000ee",
            "relay_socket": destination_socket.to_string_lossy(),
            "destination_vm": "vm-b",
            "network": "6ba7b810-9dad-11d1-80b4-00c04fd430c8",
        }),
    )
    .await;
    let relay = relay_at(&dir, A, Arc::new(Publisher::default()), service);
    let mut guest = guest_stream(&relay);
    let first = udp(A, 40000, B, 5353, b"first");
    let second = udp(A, 40000, B, 5353, b"second");
    write_frame(&mut guest, &first).await.unwrap();
    write_frame(&mut guest, &second).await.unwrap();
    // The destination owner's seat: the token, then the flow's frames.
    let (mut delivery, _) = destination.accept().await.unwrap();
    assert_eq!(next_frame(&mut delivery).await.unwrap(), 0xeeu64.to_be_bytes());
    assert_eq!(next_frame(&mut delivery).await.unwrap(), first);
    assert_eq!(next_frame(&mut delivery).await.unwrap(), second);
    let later = udp(A, 40000, B, 5353, b"later");
    write_frame(&mut guest, &later).await.unwrap();
    assert_eq!(next_frame(&mut delivery).await.unwrap(), later);
    let answer = udp(B, 5353, A, 40000, b"answer");
    write_frame(&mut delivery, &answer).await.unwrap();
    assert_eq!(next_frame(&mut guest).await.unwrap(), answer);
    let asks = seen.lock().unwrap().clone();
    assert_eq!(asks.len(), 1, "one ask per flow: {asks:?}");
    assert_eq!(asks[0]["protocol"], "udp");
    assert_eq!(asks[0]["destination"], B.to_string());
    assert_eq!(
        (asks[0]["port"].as_u64(), asks[0]["source_port"].as_u64()),
        (Some(5353), Some(40000))
    );
    assert_eq!(asks[0]["owner_secret"], "secret-a");
}

#[tokio::test]
async fn a_refused_ask_drops_the_flow_and_is_not_repeated_for_a_while() {
    let dir = tempfile::tempdir().unwrap();
    let service = dir.path().join("service.sock");
    let seen = fake_service(
        service.clone(),
        404,
        serde_json::json!({ "message": "no private path from vm-a to 10.128.0.9" }),
    )
    .await;
    let relay = relay_at(&dir, A, Arc::new(Publisher::default()), service);
    let mut guest = guest_stream(&relay);
    for n in 0..3u8 {
        write_frame(&mut guest, &udp(A, 40000, STRANGER, 53, &[n]))
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(next_frame(&mut guest).await.is_none());
    assert_eq!(seen.lock().unwrap().len(), 1, "the refusal is remembered");
    assert!(relay.peers.lock().unwrap().is_empty());
}
