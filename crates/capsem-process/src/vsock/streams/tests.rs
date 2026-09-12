use super::*;
use std::net::Ipv4Addr;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[tokio::test]
async fn the_preamble_is_read_exactly_and_the_payload_stays_in_the_stream() {
    let (mut client, mut owner) = tokio::net::UnixStream::pair().unwrap();
    let header = ConnectHeader {
        destination: Ipv4Addr::new(10, 128, 0, 3),
        port: 5201,
        source_port: 40001,
    };
    let mut segment = header.encode().to_vec();
    segment.extend_from_slice(b"\0CAPSEM_META:redis-cli\n");
    segment.extend_from_slice(b"PING the first request, in the same segment");
    client.write_all(&segment).await.unwrap();
    let (decoded, name) = read_private_preamble(&mut owner).await.unwrap();
    assert_eq!(decoded, header);
    assert_eq!(name, "redis-cli");
    let mut payload = vec![0u8; segment.len() - HEADER_BYTES - b"\0CAPSEM_META:redis-cli\n".len()];
    owner.read_exact(&mut payload).await.unwrap();
    assert_eq!(payload, b"PING the first request, in the same segment");
}

#[tokio::test]
async fn a_missing_or_endless_meta_line_is_refused() {
    let (mut client, mut owner) = tokio::net::UnixStream::pair().unwrap();
    let header = ConnectHeader {
        destination: Ipv4Addr::new(10, 128, 0, 3),
        port: 5201,
        source_port: 40001,
    };
    let mut segment = header.encode().to_vec();
    segment.extend_from_slice(&[b'x'; META_LINE_MAX + 2]);
    client.write_all(&segment).await.unwrap();
    assert!(read_private_preamble(&mut owner)
        .await
        .unwrap_err()
        .contains("too long"));
    let (mut client, mut owner) = tokio::net::UnixStream::pair().unwrap();
    client.write_all(&header.encode()[..4]).await.unwrap();
    drop(client);
    assert!(
        read_private_preamble(&mut owner).await.is_err(),
        "a truncated header is refused"
    );
}
