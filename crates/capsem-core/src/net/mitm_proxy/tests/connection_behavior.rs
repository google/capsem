use super::*;

// ---------------------------------------------------------------
// Metadata fragmentation tests
// ---------------------------------------------------------------

#[tokio::test]
async fn fragmented_metadata_is_reassembled() {
    let config = make_config_dev();
    let (s1, s2) = UnixStream::pair().unwrap();

    let proxy_fd = s2.into_raw_fd();
    let proxy_config = Arc::clone(&config);
    let proxy_task = tokio::spawn(async move {
        handle_connection(proxy_fd, proxy_config).await;
    });

    // Write metadata in two fragments: first the prefix, then the rest + newline + client hello.
    s1.set_nonblocking(false).unwrap();
    let mut writer = s1;
    // Fragment 1: metadata prefix without the newline
    std::io::Write::write_all(&mut writer, b"\0CAPSEM_META:my_proc").unwrap();
    // Small delay so the proxy reads the first fragment before the rest arrives.
    std::thread::sleep(std::time::Duration::from_millis(50));
    // Fragment 2: rest of metadata with newline, then the TLS ClientHello
    let mut frag2 = b"ess_name\n".to_vec();
    frag2.extend_from_slice(&make_client_hello(TEST_DOMAIN));
    std::io::Write::write_all(&mut writer, &frag2).unwrap();
    drop(writer);

    // The proxy should have reassembled metadata and completed TLS handshake.
    // It will fail after handshake (no real TLS client), but the key check
    // is that it didn't error during metadata parsing.
    let _ = proxy_task.await;

    tokio::time::sleep(std::time::Duration::from_millis(DB_FLUSH_MS)).await;

    let reader = config.db.reader().unwrap();
    let events = reader.recent_net_events(10).unwrap();
    // Should have an event (error from failed TLS with raw bytes, not metadata error).
    // The important thing is we didn't get "metadata exceeded 4KB" or "EOF during metadata".
    if !events.is_empty() {
        let rule = events[0].matched_rule.as_deref().unwrap_or("");
        assert!(
            !rule.contains("metadata"),
            "Fragmented metadata should be reassembled, got: {rule}"
        );
    }
}

#[tokio::test]
async fn oversized_metadata_rejected() {
    let config = make_config_dev();
    let (s1, s2) = UnixStream::pair().unwrap();

    let proxy_fd = s2.into_raw_fd();
    let proxy_config = Arc::clone(&config);
    let proxy_task = tokio::spawn(async move {
        handle_connection(proxy_fd, proxy_config).await;
    });

    // Write >4KB metadata without a newline terminator.
    let mut oversized = b"\0CAPSEM_META:".to_vec();
    oversized.extend_from_slice(&vec![b'A'; 5000]);
    let mut writer = s1;
    std::io::Write::write_all(&mut writer, &oversized).unwrap();
    drop(writer);

    let _ = proxy_task.await;

    tokio::time::sleep(std::time::Duration::from_millis(DB_FLUSH_MS)).await;

    let reader = config.db.reader().unwrap();
    let events = reader.recent_net_events(10).unwrap();
    assert!(
        !events.is_empty(),
        "oversized metadata should produce error event"
    );
    assert_eq!(events[0].decision, Decision::Error);
    let rule = events[0].matched_rule.as_deref().unwrap_or("");
    assert!(
        rule.contains("4KB"),
        "Should mention 4KB limit, got: {rule}"
    );
}

// ---------------------------------------------------------------
// Existing connection-level tests (unchanged behavior)
// ---------------------------------------------------------------

#[tokio::test]
async fn no_sni_records_error() {
    let config = make_config_dev();
    let (mut s1, s2) = UnixStream::pair().unwrap();

    std::io::Write::write_all(&mut s1, b"not a client hello").unwrap();
    drop(s1);

    handle_connection(s2.into_raw_fd(), config.clone()).await;

    // Give writer thread time to flush.
    tokio::time::sleep(std::time::Duration::from_millis(DB_FLUSH_MS)).await;

    let reader = config.db.reader().unwrap();
    let events = reader.recent_net_events(10).unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].domain, "<unknown>");
    // Without valid TLS, it's an error (handshake failure)
    assert!(matches!(
        events[0].decision,
        Decision::Error | Decision::Denied
    ));
}

#[tokio::test]
async fn empty_connection_records_error() {
    let config = make_config_dev();
    let (_s1, s2) = UnixStream::pair().unwrap();
    drop(_s1);

    handle_connection(s2.into_raw_fd(), config.clone()).await;

    tokio::time::sleep(std::time::Duration::from_millis(DB_FLUSH_MS)).await;

    let reader = config.db.reader().unwrap();
    let events = reader.recent_net_events(10).unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].decision, Decision::Error);
}

#[test]
fn replay_reader_drains_buffer_then_inner() {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    rt.block_on(async {
        let buffer = b"hello".to_vec();
        let inner_data: &[u8] = b" world";
        let mut reader = ReplayReader::new(buffer, inner_data);

        let mut output = Vec::new();
        tokio::io::AsyncReadExt::read_to_end(&mut reader, &mut output)
            .await
            .unwrap();
        assert_eq!(&output, b"hello world");
    });
}

// ---------------------------------------------------------------
// AsyncFdStream tests
// ---------------------------------------------------------------

fn wrap_fd_like_handle_inner(raw_fd: RawFd) -> AsyncFdStream {
    let file = ManuallyDrop::new(unsafe { std::fs::File::from_raw_fd(raw_fd) });
    let cloned = file.try_clone().expect("try_clone (dup) failed");
    set_nonblocking(raw_fd).expect("set_nonblocking failed");
    let async_fd = tokio::io::unix::AsyncFd::new(cloned).expect("AsyncFd::new failed");
    AsyncFdStream(async_fd)
}

#[tokio::test]
async fn async_fd_stream_basic_read_write() {
    let (s1, s2) = UnixStream::pair().unwrap();
    let fd1 = s1.into_raw_fd();
    let fd2 = s2.into_raw_fd();
    let mut stream1 = wrap_fd_like_handle_inner(fd1);
    let mut stream2 = wrap_fd_like_handle_inner(fd2);

    tokio::io::AsyncWriteExt::write_all(&mut stream1, b"hello vsock")
        .await
        .unwrap();
    let mut buf = vec![0u8; 64];
    let n = tokio::io::AsyncReadExt::read(&mut stream2, &mut buf)
        .await
        .unwrap();
    assert_eq!(&buf[..n], b"hello vsock");

    unsafe {
        libc::close(fd1);
        libc::close(fd2);
    }
}

#[tokio::test]
async fn async_fd_stream_large_transfer() {
    let (s1, s2) = UnixStream::pair().unwrap();
    let fd1 = s1.into_raw_fd();
    let fd2 = s2.into_raw_fd();
    let mut stream1 = wrap_fd_like_handle_inner(fd1);
    let mut stream2 = wrap_fd_like_handle_inner(fd2);

    let data: Vec<u8> = (0..131072).map(|i| (i % 251) as u8).collect();
    let send_data = data.clone();
    let writer = tokio::spawn(async move {
        tokio::io::AsyncWriteExt::write_all(&mut stream1, &send_data)
            .await
            .unwrap();
        drop(stream1);
        unsafe {
            libc::close(fd1);
        }
    });
    let mut received = Vec::new();
    tokio::io::AsyncReadExt::read_to_end(&mut stream2, &mut received)
        .await
        .unwrap();
    writer.await.unwrap();

    assert_eq!(received.len(), data.len());
    assert_eq!(received, data);

    unsafe {
        libc::close(fd2);
    }
}

#[tokio::test]
async fn async_fd_stream_eof_on_close() {
    let (s1, s2) = UnixStream::pair().unwrap();
    let fd1 = s1.into_raw_fd();
    let fd2 = s2.into_raw_fd();
    let mut stream2 = wrap_fd_like_handle_inner(fd2);

    {
        let mut stream1 = wrap_fd_like_handle_inner(fd1);
        tokio::io::AsyncWriteExt::write_all(&mut stream1, b"before eof")
            .await
            .unwrap();
    }
    unsafe {
        libc::close(fd1);
    }

    let mut buf = Vec::new();
    tokio::io::AsyncReadExt::read_to_end(&mut stream2, &mut buf)
        .await
        .unwrap();
    assert_eq!(&buf, b"before eof");

    unsafe {
        libc::close(fd2);
    }
}

#[tokio::test]
async fn async_fd_stream_bidirectional() {
    let (s1, s2) = UnixStream::pair().unwrap();
    let fd1 = s1.into_raw_fd();
    let fd2 = s2.into_raw_fd();
    let mut stream1 = wrap_fd_like_handle_inner(fd1);
    let mut stream2 = wrap_fd_like_handle_inner(fd2);

    tokio::io::AsyncWriteExt::write_all(&mut stream1, b"ping")
        .await
        .unwrap();
    let mut buf = vec![0u8; 32];
    let n = tokio::io::AsyncReadExt::read(&mut stream2, &mut buf)
        .await
        .unwrap();
    assert_eq!(&buf[..n], b"ping");

    tokio::io::AsyncWriteExt::write_all(&mut stream2, b"pong")
        .await
        .unwrap();
    let n = tokio::io::AsyncReadExt::read(&mut stream1, &mut buf)
        .await
        .unwrap();
    assert_eq!(&buf[..n], b"pong");

    unsafe {
        libc::close(fd1);
        libc::close(fd2);
    }
}

#[tokio::test]
async fn async_fd_stream_replay_then_live() {
    let (s1, s2) = UnixStream::pair().unwrap();
    let fd2 = s2.into_raw_fd();
    let mut stream2 = wrap_fd_like_handle_inner(fd2);

    let mut writer = s1;
    std::io::Write::write_all(&mut writer, b"INITIAL").unwrap();
    std::io::Write::write_all(&mut writer, b"REMAINING").unwrap();
    drop(writer);

    let mut initial = vec![0u8; 7];
    tokio::io::AsyncReadExt::read_exact(&mut stream2, &mut initial)
        .await
        .unwrap();
    assert_eq!(&initial, b"INITIAL");

    let mut replay = ReplayReader::new(initial, stream2);
    let mut all = Vec::new();
    tokio::io::AsyncReadExt::read_to_end(&mut replay, &mut all)
        .await
        .unwrap();
    assert_eq!(&all, b"INITIALREMAINING");

    unsafe {
        libc::close(fd2);
    }
}

/// Full TLS handshake through handle_connection using a real rustls client.
#[tokio::test]
async fn tls_handshake_completes_without_global_provider() {
    let config = make_config_dev();
    let (s1, s2) = UnixStream::pair().unwrap();

    let proxy_fd = s2.into_raw_fd();
    let proxy_config = Arc::clone(&config);
    let proxy_task = tokio::spawn(async move {
        handle_connection(proxy_fd, proxy_config).await;
    });

    let mut root_store = rustls::RootCertStore::empty();
    let ca_certs: Vec<_> = rustls_pemfile::certs(&mut CA_CERT.as_bytes())
        .collect::<Result<_, _>>()
        .unwrap();
    for cert in ca_certs {
        root_store.add(cert).unwrap();
    }
    let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
    let client_config = rustls::ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_root_certificates(root_store)
        .with_no_client_auth();
    let connector = tokio_rustls::TlsConnector::from(Arc::new(client_config));

    s1.set_nonblocking(true).unwrap();
    let stream = tokio::net::UnixStream::from_std(s1).unwrap();
    let domain = rustls::pki_types::ServerName::try_from(TEST_DOMAIN).unwrap();
    let tls_result = connector.connect(domain, stream).await;

    assert!(
        tls_result.is_ok(),
        "TLS handshake failed: {:?}",
        tls_result.err()
    );

    drop(tls_result);
    let _ = proxy_task.await;
}
