use super::*;
use rustls::pki_types::ServerName;
use rustls::{ClientConfig, ClientConnection, HandshakeKind, RootCertStore, ServerConnection};

fn ca() -> Arc<CertAuthority> {
    Arc::new(
        CertAuthority::load(
            include_str!("../../../../resources/ca/capsem-ca.key"),
            include_str!("../../../../resources/ca/capsem-ca.crt"),
        )
        .expect("bundled CA loads"),
    )
}

fn client_config(ca: &CertAuthority) -> Arc<ClientConfig> {
    let mut roots = RootCertStore::empty();
    roots.add(ca.ca_cert_der().clone()).unwrap();
    let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
    Arc::new(
        ClientConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_root_certificates(roots)
            .with_no_client_auth(),
    )
}

/// Pump TLS records between an in-memory client and server until the
/// handshake completes on both sides.
fn handshake(client: &mut ClientConnection, server: &mut ServerConnection) {
    for _ in 0..32 {
        if !client.is_handshaking() && !server.is_handshaking() {
            return;
        }
        let mut to_server = Vec::new();
        client.write_tls(&mut to_server).unwrap();
        if !to_server.is_empty() {
            server.read_tls(&mut &to_server[..]).unwrap();
            server.process_new_packets().unwrap();
        }
        let mut to_client = Vec::new();
        server.write_tls(&mut to_client).unwrap();
        if !to_client.is_empty() {
            client.read_tls(&mut &to_client[..]).unwrap();
            client.process_new_packets().unwrap();
        }
    }
    panic!("handshake did not complete");
}

#[test]
fn a_second_connection_resumes_the_session_the_first_one_established() {
    let ca = ca();
    let server_config = make_server_tls_config(&ca);
    let client_config = client_config(&ca);
    let name = ServerName::try_from("resume.example.com").unwrap();

    let mut server = ServerConnection::new(Arc::clone(&server_config)).unwrap();
    let mut client = ClientConnection::new(Arc::clone(&client_config), name.clone()).unwrap();
    handshake(&mut client, &mut server);
    assert_eq!(client.handshake_kind(), Some(HandshakeKind::Full));
    assert_eq!(server.server_name(), Some("resume.example.com"));
    // TLS 1.3 sends the session ticket after the handshake; let it flow.
    let mut ticket = Vec::new();
    server.write_tls(&mut ticket).unwrap();
    client.read_tls(&mut &ticket[..]).unwrap();
    client.process_new_packets().unwrap();

    let mut server = ServerConnection::new(server_config).unwrap();
    let mut client = ClientConnection::new(client_config, name).unwrap();
    handshake(&mut client, &mut server);
    assert_eq!(
        client.handshake_kind(),
        Some(HandshakeKind::Resumed),
        "one shared server config keeps the session cache the second connection resumes from"
    );
    assert_eq!(server.server_name(), Some("resume.example.com"));
}

#[test]
fn concurrent_connections_each_see_their_own_sni() {
    let ca = ca();
    let server_config = make_server_tls_config(&ca);
    let client_config = client_config(&ca);
    let handles: Vec<_> = (0..8)
        .map(|i| {
            let server_config = Arc::clone(&server_config);
            let client_config = Arc::clone(&client_config);
            std::thread::spawn(move || {
                let host = format!("host{i}.example.com");
                let mut server = ServerConnection::new(server_config).unwrap();
                let mut client =
                    ClientConnection::new(client_config, ServerName::try_from(host.clone()).unwrap()).unwrap();
                handshake(&mut client, &mut server);
                assert_eq!(server.server_name(), Some(host.as_str()));
            })
        })
        .collect();
    for handle in handles {
        handle.join().unwrap();
    }
    assert_eq!(ca.cache_size(), 8, "one leaf per distinct name");
}

#[test]
fn a_mixed_case_sni_is_served_the_lowercase_leaf() {
    let ca = ca();
    let server_config = make_server_tls_config(&ca);
    let client_config = client_config(&ca);
    let mut server = ServerConnection::new(server_config).unwrap();
    let mut client = ClientConnection::new(client_config, ServerName::try_from("Mixed.Example.COM").unwrap()).unwrap();
    handshake(&mut client, &mut server);
    assert_eq!(ca.cache_size(), 1);
    ca.certified_key_for_domain("mixed.example.com").unwrap();
    assert_eq!(ca.cache_size(), 1, "the leaf was minted under the normalized name");
}
