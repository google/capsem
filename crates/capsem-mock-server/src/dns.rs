//! Deterministic DNS fixtures: known names answer, everything else is NXDOMAIN.
use super::{now_unix, State};
use serde_json::json;
use std::io::Write as _;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, UdpSocket};

/// Answered with a routable TEST-NET-2 address instead of loopback: a guest
/// behind a gateway (a container) must be able to route to it.
pub(super) const ROUTABLE_DNS_FIXTURE: &str = "egress.capsem.test";

const QTYPE_A: u16 = 1;
const LOOPBACK_ANSWER: [u8; 4] = [127, 0, 0, 1];
/// TEST-NET-2: never a real host, and routed by a container's default route
/// through the VM, whose proxies intercept it by port.
const ROUTABLE_ANSWER: [u8; 4] = [198, 51, 100, 10];

/// The address the DNS fixtures answer.
///
/// `Loopback` (the default) is what a VM-root client needs: the VM proxies
/// intercept its loopback ports. A container workload has its own loopback,
/// so its clients need `Routable`, where every fixture answers TEST-NET-2 and
/// reaches the same proxies through the container's gateway.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, clap::ValueEnum)]
pub(super) enum DnsAnswers {
    #[default]
    Loopback,
    Routable,
}

impl DnsAnswers {
    fn answer(self, name: &str) -> [u8; 4] {
        if self == Self::Routable || name == ROUTABLE_DNS_FIXTURE {
            ROUTABLE_ANSWER
        } else {
            LOOPBACK_ANSWER
        }
    }

    /// What the provider and model fixtures resolve to in this mode.
    pub(super) fn provider_answer(self) -> String {
        std::net::Ipv4Addr::from(self.answer("model.capsem.test")).to_string()
    }
}

pub(super) const DNS_FIXTURES: &[&str] = &[
    "fixture.capsem.test",
    ROUTABLE_DNS_FIXTURE,
    "model.capsem.test",
    "ollama.capsem.test",
    "mcp.capsem.test",
    "api.openai.com",
    "api.anthropic.com",
    "daily-cloudcode-pa.googleapis.com",
    "generativelanguage.googleapis.com",
    "www.googleapis.com",
    "play.googleapis.com",
    "antigravity-unleash.goog",
    "deb.debian.org",
];

struct DnsExchange {
    qname: String,
    qtype: u16,
    qclass: u16,
    rcode: u8,
    request_bytes: usize,
    response_bytes: usize,
}

pub(super) async fn serve_dns_udp(socket: UdpSocket, state: State) {
    let mut buf = vec![0_u8; 1500];
    loop {
        let Ok((len, peer)) = socket.recv_from(&mut buf).await else {
            continue;
        };
        if let Some((response, exchange)) = dns_response_with_exchange(&buf[..len], state.dns_answers) {
            log_dns_request(&state, "udp", &exchange);
            let _ = socket.send_to(&response, peer).await;
        }
    }
}

pub(super) async fn serve_dns_tcp(listener: TcpListener, state: State) {
    loop {
        let Ok((mut stream, _)) = listener.accept().await else {
            continue;
        };
        let state = state.clone();
        tokio::spawn(async move {
            loop {
                let mut len_bytes = [0_u8; 2];
                if stream.read_exact(&mut len_bytes).await.is_err() {
                    return;
                }
                let len = usize::from(u16::from_be_bytes(len_bytes));
                let mut query = vec![0_u8; len];
                if stream.read_exact(&mut query).await.is_err() {
                    return;
                }
                let Some((response, exchange)) = dns_response_with_exchange(&query, state.dns_answers) else {
                    return;
                };
                let Ok(response_len) = u16::try_from(response.len()) else {
                    return;
                };
                log_dns_request(&state, "tcp", &exchange);
                if stream.write_all(&response_len.to_be_bytes()).await.is_err() {
                    return;
                }
                if stream.write_all(&response).await.is_err() {
                    return;
                }
            }
        });
    }
}

#[cfg(test)]
pub(super) fn dns_response(query: &[u8]) -> Option<Vec<u8>> {
    dns_response_for(query, DnsAnswers::default())
}

#[cfg(test)]
pub(super) fn dns_response_for(query: &[u8], answers: DnsAnswers) -> Option<Vec<u8>> {
    dns_response_with_exchange(query, answers).map(|(response, _)| response)
}

fn dns_response_with_exchange(query: &[u8], answers: DnsAnswers) -> Option<(Vec<u8>, DnsExchange)> {
    if query.len() < 12 {
        return None;
    }
    let query_id = &query[..2];
    let mut offset = 12;
    let mut labels = Vec::new();
    while offset < query.len() {
        let len = usize::from(query[offset]);
        offset += 1;
        if len == 0 {
            break;
        }
        if offset + len > query.len() {
            return None;
        }
        labels.push(String::from_utf8_lossy(&query[offset..offset + len]).to_string());
        offset += len;
    }
    if offset + 4 > query.len() {
        return None;
    }
    let qtype = u16::from_be_bytes([query[offset], query[offset + 1]]);
    let qclass = u16::from_be_bytes([query[offset + 2], query[offset + 3]]);
    let question_end = offset + 4;
    let name = labels.join(".").to_ascii_lowercase();
    let known = DNS_FIXTURES.iter().any(|fixture| *fixture == name);
    // The fixtures are IPv4 addresses: only an A question gets one. Any other
    // type for a known name is NOERROR with no records, never an A record in
    // an AAAA answer.
    let answered = known && qtype == QTYPE_A;
    let mut response = Vec::with_capacity(query.len() + 32);
    response.extend_from_slice(query_id);
    response.extend_from_slice(if known { &[0x81, 0x80] } else { &[0x81, 0x83] });
    response.extend_from_slice(&[0x00, 0x01]);
    response.extend_from_slice(if answered { &[0x00, 0x01] } else { &[0x00, 0x00] });
    response.extend_from_slice(&[0x00, 0x00, 0x00, 0x00]);
    response.extend_from_slice(&query[12..question_end]);
    if answered {
        response.extend_from_slice(&[
            0xC0, 0x0C, // name pointer
            0x00, 0x01, // A
            0x00, 0x01, // IN
            0x00, 0x00, 0x00, 0x3C, // ttl 60
            0x00, 0x04, // len
        ]);
        response.extend_from_slice(&answers.answer(&name));
    }
    let exchange = DnsExchange {
        qname: name,
        qtype,
        qclass,
        rcode: if known { 0 } else { 3 },
        request_bytes: query.len(),
        response_bytes: response.len(),
    };
    Some((response, exchange))
}

fn log_dns_request(state: &State, source_proto: &str, exchange: &DnsExchange) {
    let Some(file) = &state.request_log else {
        return;
    };
    let record = json!({
        "timestamp": now_unix(),
        "kind": "dns",
        "source_proto": source_proto,
        "qname": exchange.qname,
        "qtype": exchange.qtype,
        "qclass": exchange.qclass,
        "rcode": exchange.rcode,
        "request_bytes": exchange.request_bytes,
        "response_bytes": exchange.response_bytes,
    });
    if let Ok(mut file) = file.lock() {
        let _ = writeln!(file, "{record}");
        let _ = file.flush();
    }
}
