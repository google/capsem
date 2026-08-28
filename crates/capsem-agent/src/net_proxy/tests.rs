use super::*;
use std::os::unix::io::IntoRawFd;
use std::os::unix::net::UnixStream;

#[test]
fn vsock_port_matches_host() {
    assert_eq!(VSOCK_PORT_SNI_PROXY, 5002);
}

#[test]
fn listen_port_is_10443() {
    assert_eq!(LISTEN_PORT_HTTPS, 10443);
}

#[test]
fn http_listen_port_is_10080() {
    assert_eq!(LISTEN_PORT_HTTP, 10080);
}

#[test]
fn http_and_https_listen_ports_are_distinct() {
    // Same vsock target on the host, but distinct guest-side
    // listen ports so iptables can route 80/443 to the right
    // localhost socket without collision.
    assert_ne!(LISTEN_PORT_HTTP, LISTEN_PORT_HTTPS);
}

#[test]
fn async_vsock_from_socketpair() {
    // Verify AsyncVsock wraps a raw fd from a unix socketpair
    let (a, _b) = UnixStream::pair().unwrap();
    let fd = a.into_raw_fd();
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    rt.block_on(async {
        let vsock = AsyncVsock::new(fd);
        assert!(vsock.is_ok(), "AsyncVsock should wrap a socketpair fd");
        // Drop will close the fd
    });
}

#[test]
fn test_sanitize_process_name() {
    assert_eq!(sanitize_process_name("gemini"), "gemini");
    assert_eq!(sanitize_process_name("my proc"), "my_proc");
    assert_eq!(sanitize_process_name("my\nproc"), "my_proc");
    assert_eq!(sanitize_process_name("my\rproc"), "my_proc");
    assert_eq!(sanitize_process_name("my\0proc"), "my_proc");

    let long_name = "A".repeat(200);
    assert_eq!(sanitize_process_name(&long_name).len(), 128);
}

#[test]
fn sanitize_truncates_multibyte_names_on_a_character_boundary() {
    let name = format!("{}é", "a".repeat(127));
    let result = sanitize_process_name(&name);

    assert_eq!(result.chars().count(), 128);
    assert!(result.ends_with('é'));
}

#[test]
fn sanitize_blocks_meta_line_injection() {
    // Newline in process name would split the \0CAPSEM_META:...\n frame
    let evil = "evil\nCAPS_META:spoof";
    let sanitized = sanitize_process_name(evil);
    assert!(!sanitized.contains('\n'));
    assert!(!sanitized.contains('\0'));
    let meta = format!("\0CAPSEM_META:{}\n", sanitized);
    assert_eq!(meta.matches('\n').count(), 1);
}

#[test]
fn port_hex_parsing_extracts_exact_port() {
    // Simulate /proc/net/tcp format: local_address is "HEX_IP:HEX_PORT".
    // Verify rsplit(':') extracts the port portion correctly.
    let port_hex = format!("{:04X}", 443u16); // "01BB"

    let line_match = "  0: 0100007F:01BB 00000000:0000 0A 00000000:00000000 00:00000000 00000000  1000 0 12345 1";
    let line_no_match = "  1: 0100007F:1234 00000000:0000 0A 00000000:00000000 00:00000000 00000000  1000 0 99999 1";

    let parts: Vec<&str> = line_match.split_whitespace().collect();
    let port_part = parts[1].rsplit(':').next().unwrap();
    assert_eq!(port_part, port_hex, "should match port 443");

    let parts2: Vec<&str> = line_no_match.split_whitespace().collect();
    let port_part2 = parts2[1].rsplit(':').next().unwrap();
    assert_ne!(port_part2, port_hex, "should not match different port");
    assert_eq!(port_part2, "1234");
}

#[test]
fn port_hex_parsing_ipv6_format() {
    // /proc/net/tcp6 has longer IP hex but same colon-delimited port.
    let port_hex = format!("{:04X}", 8080u16); // "1F90"
    let line = "  0: 00000000000000000000000001000000:1F90 00000000000000000000000000000000:0000 0A 00000000:00000000 00:00000000 00000000  1000 0 54321 1";

    let parts: Vec<&str> = line.split_whitespace().collect();
    let port_part = parts[1].rsplit(':').next().unwrap();
    assert_eq!(port_part, port_hex);
}

#[tokio::test]
async fn tcp_bind_accept_localhost() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let client = TcpStream::connect(addr).await.unwrap();
    let (server, peer) = listener.accept().await.unwrap();

    assert_eq!(peer.ip(), std::net::Ipv4Addr::LOCALHOST);
    assert!(client.peer_addr().is_ok());
    drop(server);
    drop(client);
}

#[tokio::test]
async fn meta_line_injected_before_data() {
    // Simulate the meta line injection that handle_connection does
    let (a, b) = UnixStream::pair().unwrap();
    let fd = a.into_raw_fd();
    let mut vsock = AsyncVsock::new(fd).unwrap();

    let meta = "\0CAPSEM_META:test-agent\n".to_string();
    tokio::io::AsyncWriteExt::write_all(&mut vsock, meta.as_bytes())
        .await
        .unwrap();

    // Read from the other end
    let mut buf = vec![0u8; meta.len()];
    use std::io::Read;
    let mut reader = b;
    reader.read_exact(&mut buf).unwrap();
    assert_eq!(buf[0], 0); // NUL prefix
    assert!(String::from_utf8_lossy(&buf).contains("CAPSEM_META:test-agent"));
}

#[tokio::test]
async fn async_vsock_write_then_read() {
    let (a, b) = UnixStream::pair().unwrap();
    let fd_a = a.into_raw_fd();
    let fd_b = b.into_raw_fd();

    let mut va = AsyncVsock::new(fd_a).unwrap();
    let mut vb = AsyncVsock::new(fd_b).unwrap();

    // Write from a, read fixed-size from b
    tokio::io::AsyncWriteExt::write_all(&mut va, b"ping")
        .await
        .unwrap();

    let mut buf = [0u8; 4];
    tokio::io::AsyncReadExt::read_exact(&mut vb, &mut buf)
        .await
        .unwrap();
    assert_eq!(&buf, b"ping");
}

#[tokio::test]
async fn async_vsock_large_transfer() {
    let (a, b) = UnixStream::pair().unwrap();
    let fd_a = a.into_raw_fd();
    let fd_b = b.into_raw_fd();

    let mut va = AsyncVsock::new(fd_a).unwrap();
    let mut vb = AsyncVsock::new(fd_b).unwrap();

    let data: Vec<u8> = (0..65536).map(|i| (i % 256) as u8).collect();
    let data_clone = data.clone();

    let (write_res, read_res) = tokio::join!(
        async {
            let r = tokio::io::AsyncWriteExt::write_all(&mut va, &data_clone).await;
            tokio::io::AsyncWriteExt::shutdown(&mut va).await.unwrap();
            r
        },
        async {
            let mut received = Vec::new();
            let r = tokio::io::AsyncReadExt::read_to_end(&mut vb, &mut received).await;
            (r, received)
        }
    );

    write_res.unwrap();
    let (_, received) = read_res;
    assert_eq!(received.len(), 65536);
    assert_eq!(received, data);
}
