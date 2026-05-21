//! End-to-end tests for the DNS handler + resolver, using a fake
//! UDP upstream bound on `127.0.0.1:0`. No system DNS, no internet.

use std::net::{Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use capsem_logger::events::Decision;
use hickory_proto::op::{Message, MessageType, OpCode, Query, ResponseCode};
use hickory_proto::rr::{Name, RData, Record, RecordType};
use tokio::net::UdpSocket;

use super::resolver::DnsResolver;
use super::server::DnsHandler;

fn build_query_bytes(name: &str, qtype: RecordType, id: u16) -> Vec<u8> {
    let mut msg = Message::new(id, MessageType::Query, OpCode::Query);
    msg.metadata.recursion_desired = true;
    let n = Name::from_ascii(name).unwrap();
    msg.add_query(Query::query(n, qtype));
    msg.to_vec().unwrap()
}

/// Spawn a fake DNS upstream that answers any A query with `answer_ip`
/// after an optional delay. Returns the bound socket address.
async fn spawn_fake_upstream(answer_ip: [u8; 4], delay: Duration) -> SocketAddr {
    let sock = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let addr = sock.local_addr().unwrap();
    tokio::spawn(async move {
        let mut buf = vec![0u8; 4096];
        loop {
            let (n, peer) = match sock.recv_from(&mut buf).await {
                Ok(x) => x,
                Err(_) => break,
            };
            let req = Message::from_vec(&buf[..n]).unwrap();
            let mut resp = Message::new(req.metadata.id, MessageType::Response, OpCode::Query);
            resp.metadata.recursion_desired = req.metadata.recursion_desired;
            resp.metadata.recursion_available = true;
            resp.metadata.response_code = ResponseCode::NoError;
            for q in &req.queries {
                resp.add_query(q.clone());
                if q.query_type() == RecordType::A {
                    let rec = Record::from_rdata(
                        q.name().clone(),
                        60,
                        RData::A(Ipv4Addr::from(answer_ip).into()),
                    );
                    resp.add_answer(rec);
                }
            }
            if !delay.is_zero() {
                tokio::time::sleep(delay).await;
            }
            let _ = sock.send_to(&resp.to_vec().unwrap(), peer).await;
        }
    });
    addr
}

/// Spawn a black-hole upstream that accepts queries but never replies.
/// Returns the bound socket address.
async fn spawn_blackhole_upstream() -> SocketAddr {
    let sock = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let addr = sock.local_addr().unwrap();
    tokio::spawn(async move {
        let mut buf = vec![0u8; 4096];
        loop {
            if sock.recv_from(&mut buf).await.is_err() {
                break;
            }
            // Intentionally drop the query.
        }
    });
    addr
}

mod resolver_behavior;

mod metrics_behavior;

mod cache_behavior;
