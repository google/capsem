use super::*;

#[test]
fn deterministic_bytes_are_cached_and_correct() {
    let first = deterministic_bytes("10mb").expect("10mb fixture");
    let second = deterministic_bytes("10mb").expect("10mb fixture");
    assert_eq!(first.len(), 10 * 1024 * 1024);
    assert_eq!(first, second);
    assert_eq!(&first[..26], b"abcdefghijklmnopqrstuvwxyz");
}

#[test]
fn dns_fixture_answers_known_names_and_rejects_unknown() {
    let query = test_dns_query("fixture.capsem.test", 0xCAFE);
    let response = dns_response(&query).expect("dns response");
    assert_eq!(&response[..2], b"\xCA\xFE");
    assert_eq!(response[3] & 0x0F, 0);
    assert_eq!(&response[response.len() - 4..], &[127, 0, 0, 1]);

    let query = test_dns_query("unknown.capsem.invalid", 0xBEEF);
    let response = dns_response(&query).expect("dns response");
    assert_eq!(&response[..2], b"\xBE\xEF");
    assert_eq!(response[3] & 0x0F, 3);
}

#[test]
fn websocket_accept_matches_rfc_fixture() {
    assert_eq!(
        websocket_accept("dGhlIHNhbXBsZSBub25jZQ=="),
        "s3pPLMBiTxaQ9kYGzzhZRbK+xOo="
    );
}

fn test_dns_query(name: &str, id: u16) -> Vec<u8> {
    let mut query = Vec::new();
    query.extend_from_slice(&id.to_be_bytes());
    query.extend_from_slice(&[0x01, 0x00, 0x00, 0x01, 0, 0, 0, 0, 0, 0]);
    for part in name.split('.') {
        query.push(u8::try_from(part.len()).expect("label fits"));
        query.extend_from_slice(part.as_bytes());
    }
    query.extend_from_slice(&[0, 0, 1, 0, 1]);
    query
}
