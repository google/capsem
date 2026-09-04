use super::*;
use capsem_core::net::dns::{DnsHandler, DnsResolver};
use std::os::fd::AsRawFd;
use std::os::unix::net::UnixStream;

fn handler() -> Arc<DnsHandler> {
    let policy = Arc::new(std::sync::RwLock::new(Arc::new(
        capsem_core::net::policy::NetworkMechanics::default(),
    )));
    let rules = Arc::new(std::sync::RwLock::new(Arc::new(
        capsem_core::net::policy_config::SecurityRuleSet::new(Vec::new()),
    )));
    let plugins = Arc::new(std::sync::RwLock::new(std::collections::BTreeMap::new().into()));
    Arc::new(DnsHandler::new(
        policy,
        rules,
        plugins,
        Arc::new(DnsResolver::with_upstreams(Vec::new())),
    ))
}

fn rules() -> SecurityRulesHandle {
    Arc::new(std::sync::RwLock::new(Arc::new(
        capsem_core::net::policy_config::SecurityRuleSet::new(Vec::new()),
    )))
}

/// A `.capsem-bogus` A query the handler answers locally with NXDOMAIN.
fn query(id: u16, label: &str) -> Vec<u8> {
    let mut msg = vec![0u8; 12];
    msg[0..2].copy_from_slice(&id.to_be_bytes());
    msg[2] = 0x01;
    msg[5] = 1;
    for part in [label, "capsem-bogus"] {
        msg.push(part.len() as u8);
        msg.extend_from_slice(part.as_bytes());
    }
    msg.push(0);
    msg.extend_from_slice(&[0, 1, 0, 1]);
    msg
}

fn request_frame(id: u32, raw: Vec<u8>) -> Vec<u8> {
    capsem_proto::encode_dns_request(&capsem_proto::DnsRequest {
        id,
        raw,
        proto: "udp".into(),
        process_name: None,
    })
    .unwrap()
}

fn read_response(guest: &mut UnixStream) -> Option<capsem_proto::DnsResponse> {
    let mut len_buf = [0u8; 4];
    guest.read_exact(&mut len_buf).ok()?;
    let mut payload = vec![0u8; u32::from_be_bytes(len_buf) as usize];
    guest.read_exact(&mut payload).ok()?;
    Some(capsem_proto::decode_dns_response(&payload).unwrap())
}

/// Serve one session over a socket pair; returns the guest end and the
/// server task.
fn session(db: Arc<capsem_logger::DbWriter>) -> (UnixStream, tokio::task::JoinHandle<()>) {
    let (guest, host) = UnixStream::pair().unwrap();
    let conn = VsockConnection::new(host.as_raw_fd(), capsem_proto::VSOCK_PORT_DNS_PROXY, Box::new(host));
    let task = tokio::spawn(serve_dns_session(conn, handler(), db, rules()));
    (guest, task)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn thirty_two_queries_in_flight_are_all_answered_under_their_own_ids() {
    let db = Arc::new(capsem_logger::DbWriter::open_in_memory(64).unwrap());
    let (mut guest, task) = session(db);
    let queries: Vec<(u32, Vec<u8>)> = (1..=32u32)
        .map(|id| (id, query(id as u16, &format!("q{id}"))))
        .collect();
    let mut writer = guest.try_clone().unwrap();
    let frames: Vec<Vec<u8>> = queries
        .iter()
        .map(|(id, raw)| request_frame(*id, raw.clone()))
        .collect();
    let send = tokio::task::spawn_blocking(move || {
        for frame in frames {
            writer.write_all(&frame).unwrap();
        }
    });
    let answers = tokio::task::spawn_blocking(move || {
        let mut seen = std::collections::HashMap::new();
        for _ in 0..32 {
            let response = read_response(&mut guest).expect("an answer for every query");
            seen.insert(response.id, response);
        }
        (guest, seen)
    });
    send.await.unwrap();
    let (guest, seen) = answers.await.unwrap();
    assert_eq!(seen.len(), 32, "every correlation id answered exactly once");
    for (id, raw) in &queries {
        let response = &seen[id];
        assert_eq!(response.rcode, 3, "local NXDOMAIN fixture");
        assert_eq!(
            &response.raw[0..2],
            &raw[0..2],
            "answer keeps the query's transaction id"
        );
        assert_eq!(&response.raw[12..], &raw[12..], "answer echoes the query's question");
    }
    drop(guest);
    tokio::time::timeout(std::time::Duration::from_secs(5), task)
        .await
        .expect("session ends at EOF")
        .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_oversized_frame_ends_the_session() {
    let db = Arc::new(capsem_logger::DbWriter::open_in_memory(16).unwrap());
    let (mut guest, task) = session(db);
    let mut writer = guest.try_clone().unwrap();
    tokio::task::spawn_blocking(move || {
        writer
            .write_all(&(capsem_proto::MAX_FRAME_SIZE + 1).to_be_bytes())
            .unwrap();
        let _ = writer.write_all(&[0u8; 32]);
    })
    .await
    .unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(5), task)
        .await
        .expect("session refuses the frame and ends")
        .unwrap();
    let eof = tokio::task::spawn_blocking(move || read_response(&mut guest))
        .await
        .unwrap();
    assert!(eof.is_none(), "nothing is answered after the connection is dropped");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_truncated_frame_ends_the_session_without_an_answer() {
    let db = Arc::new(capsem_logger::DbWriter::open_in_memory(16).unwrap());
    let (guest, task) = session(db);
    let mut writer = guest.try_clone().unwrap();
    tokio::task::spawn_blocking(move || {
        writer.write_all(&64u32.to_be_bytes()).unwrap();
        writer.write_all(&[1, 2, 3]).unwrap();
        drop(writer);
    })
    .await
    .unwrap();
    // Close our end fully so the reader sees EOF mid-frame.
    let guest_fd_holder = guest.try_clone().unwrap();
    drop(guest);
    drop(guest_fd_holder);
    tokio::time::timeout(std::time::Duration::from_secs(5), task)
        .await
        .expect("session ends on the cut frame")
        .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn every_answered_query_leaves_a_dns_event_row_before_its_answer() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("session.db");
    let db = Arc::new(capsem_logger::DbWriter::open(&db_path, 64).unwrap());
    let (mut guest, task) = session(Arc::clone(&db));
    let mut writer = guest.try_clone().unwrap();
    tokio::task::spawn_blocking(move || {
        for id in 1..=8u32 {
            writer.write_all(&request_frame(id, query(id as u16, "row"))).unwrap();
        }
    })
    .await
    .unwrap();
    let guest = tokio::task::spawn_blocking(move || {
        for _ in 0..8 {
            read_response(&mut guest).expect("answer");
        }
        guest
    })
    .await
    .unwrap();
    drop(guest);
    task.await.unwrap();
    let db_for_flush = Arc::clone(&db);
    tokio::task::spawn_blocking(move || db_for_flush.shutdown_blocking())
        .await
        .unwrap();
    let reader = capsem_logger::DbReader::open(&db_path).unwrap();
    let rows: serde_json::Value =
        serde_json::from_str(&reader.query_raw("SELECT COUNT(*) FROM dns_events").unwrap()).unwrap();
    assert_eq!(rows["rows"][0][0].as_i64(), Some(8));
}
