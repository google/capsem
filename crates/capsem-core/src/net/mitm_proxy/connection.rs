//! Connection classification, TLS admission and owner-stop handling.
use super::*;

/// Read from a guest stream with a deadline, mapping a timeout to a TimedOut
/// io error so the caller tears the connection down instead of blocking.
pub(super) async fn classify_read<S>(
    stream: &mut S,
    buf: &mut [u8],
    timeout: std::time::Duration,
) -> std::io::Result<usize>
where
    S: tokio::io::AsyncRead + Unpin,
{
    match tokio::time::timeout(timeout, tokio::io::AsyncReadExt::read(stream, buf)).await {
        Ok(result) => result,
        Err(_) => Err(std::io::Error::new(
            std::io::ErrorKind::TimedOut,
            "guest did not send classification bytes before the deadline",
        )),
    }
}

async fn classify_until_stopped<S>(stream: &mut S, buf: &mut [u8], config: &MitmProxyConfig) -> std::io::Result<usize>
where
    S: tokio::io::AsyncRead + Unpin,
{
    tokio::select! {
        biased;
        _ = config.shutdown.cancelled() => Ok(0),
        result = classify_read(stream, buf, CLASSIFY_READ_TIMEOUT) => result,
    }
}

/// Returns the domain, or (domain, decision, reason); TelemetryHook records requests.
pub(super) async fn handle_inner(
    vsock_fd: OwnedFd,
    config: &Arc<MitmProxyConfig>,
) -> Result<String, (String, Decision, String)> {
    let std_fd = std::fs::File::from(vsock_fd);
    capsem_foundation::unix::fd::set_nonblocking(std_fd.as_fd(), true)
        .map_err(|e| (String::new(), Decision::Error, format!("set nonblocking: {e}")))?;
    // SAFETY: AsyncFdStream retains this owned file unchanged until deregistration.
    let async_fd = unsafe { tokio::io::unix::AsyncFd::register(std_fd) }
        .map_err(|e| (String::new(), Decision::Error, format!("async fd: {e}")))?;
    let mut vsock_stream = AsyncFdStream(async_fd);
    let classify_span = tracing::debug_span!(
        target: "capsem.mitm",
        spans::MITM_VSOCK_CLASSIFY,
        protocol = tracing::field::Empty,
        status = tracing::field::Empty,
        error_kind = tracing::field::Empty,
    );

    // 1. Read initial bytes (TLS ClientHello + potential metadata).
    let mut initial_buf = vec![0u8; MAX_HELLO_SIZE];
    let n = classify_until_stopped(&mut vsock_stream, &mut initial_buf, config)
        .instrument(classify_span.clone())
        .await
        .map_err(|e| {
            classify_span.record("status", "error");
            classify_span.record("error_kind", "read_client_hello");
            (String::new(), Decision::Error, format!("read ClientHello: {e}"))
        })?;
    if n == 0 {
        classify_span.record("status", "error");
        classify_span.record("error_kind", "empty_connection");
        return Err((String::new(), Decision::Error, "empty connection".into()));
    }
    initial_buf.truncate(n);

    let mut process_name: Option<String> = None;
    if initial_buf.starts_with(b"\0CAPSEM_META:") {
        // Metadata may arrive fragmented across multiple reads.
        // Keep reading until we find the terminating '\n' or hit the 4KB limit.
        const MAX_META_SIZE: usize = 4096;
        loop {
            if let Some(nl_idx) = initial_buf.iter().position(|&b| b == b'\n') {
                let proc_bytes = &initial_buf[13..nl_idx];
                process_name = String::from_utf8(proc_bytes.to_vec()).ok();
                initial_buf.drain(0..=nl_idx);
                break;
            }
            if initial_buf.len() >= MAX_META_SIZE {
                return Err((String::new(), Decision::Error, "metadata exceeded 4KB limit".into()));
            }
            let mut more = vec![0u8; 1024];
            let n2 = classify_until_stopped(&mut vsock_stream, &mut more, config)
                .instrument(classify_span.clone())
                .await
                .map_err(|e| {
                    classify_span.record("status", "error");
                    classify_span.record("error_kind", "read_metadata");
                    (String::new(), Decision::Error, format!("read metadata: {e}"))
                })?;
            if n2 == 0 {
                return Err((String::new(), Decision::Error, "EOF during metadata read".into()));
            }
            initial_buf.extend_from_slice(&more[..n2]);
        }

        // If initial_buf is empty after draining meta, we need to read
        // the wire payload (TLS ClientHello or HTTP request line).
        if initial_buf.is_empty() {
            let mut hello_buf = vec![0u8; MAX_HELLO_SIZE];
            let n2 = classify_until_stopped(&mut vsock_stream, &mut hello_buf, config)
                .instrument(classify_span.clone())
                .await
                .map_err(|e| {
                    classify_span.record("status", "error");
                    classify_span.record("error_kind", "read_payload_after_meta");
                    (String::new(), Decision::Error, format!("read payload after meta: {e}"))
                })?;
            if n2 == 0 {
                return Err((String::new(), Decision::Error, "empty connection after meta".into()));
            }
            hello_buf.truncate(n2);
            initial_buf = hello_buf;
        }
    }

    // Framed MCP starts with a four-byte length prefix followed by a
    // two-byte magic. If the guest's first write is split across the
    // hypervisor boundary, pull just enough bytes for the classifier.
    while initial_buf.first() == Some(&0) && initial_buf.len() < 6 {
        let mut more = vec![0u8; 6 - initial_buf.len()];
        let n2 = classify_until_stopped(&mut vsock_stream, &mut more, config)
            .instrument(classify_span.clone())
            .await
            .map_err(|e| {
                classify_span.record("status", "error");
                classify_span.record("error_kind", "read_protocol_prefix");
                (String::new(), Decision::Error, format!("read protocol prefix: {e}"))
            })?;
        if n2 == 0 {
            break;
        }
        initial_buf.extend_from_slice(&more[..n2]);
    }

    // 2. First-byte protocol sniff (T2.1). Classify the post-meta
    //    payload as TLS (0x16) or plain HTTP (uppercase ASCII method).
    //    Unrecognized first byte → connection-level error event.
    let detected = match protocol::detect(&initial_buf) {
        Some(p) => p,
        None => {
            classify_span.record("protocol", Protocol::Unknown.label());
            classify_span.record("status", "error");
            classify_span.record("error_kind", "unknown_protocol");
            ::metrics::counter!(m::CONNECTIONS_TOTAL,
                "protocol" => Protocol::Unknown.label())
            .increment(1);
            let first = initial_buf.first().copied().unwrap_or(0);
            return Err((
                String::new(),
                Decision::Error,
                format!("unknown protocol byte 0x{first:02x}"),
            ));
        }
    };
    ::metrics::counter!(m::CONNECTIONS_TOTAL,
        "protocol" => detected.label())
    .increment(1);
    classify_span.record("protocol", detected.label());
    classify_span.record("status", "ok");

    let process_name = Arc::new(process_name);

    match detected {
        Protocol::Tls => serve_tls(initial_buf, vsock_stream, config, process_name).await,
        Protocol::Http => serve_plain_http(initial_buf, vsock_stream, config, process_name).await,
        Protocol::McpFrame => mcp_frame::serve(initial_buf, vsock_stream, config).await,
        Protocol::Unknown => unreachable!("Protocol::Unknown returned Err earlier"),
    }
}

/// TLS-terminating MITM path. Runs the rustls acceptor on the vsock
/// stream (chained with the buffered ClientHello bytes), pulls the
/// SNI domain off the resolver, and serves a hyper HTTP/1.1 server
/// over the resulting TLS stream.
async fn serve_tls(
    initial_buf: Vec<u8>,
    vsock_stream: AsyncFdStream,
    config: &Arc<MitmProxyConfig>,
    process_name: Arc<Option<String>>,
) -> Result<String, (String, Decision, String)> {
    let acceptor = TlsAcceptor::from(Arc::clone(&config.server_tls));

    // Chain buffered ClientHello bytes with the remaining vsock stream.
    let replay = ReplayReader::new(initial_buf, vsock_stream);
    let handshake_start = Instant::now();
    let tls_span = tracing::debug_span!(
        target: "capsem.mitm",
        spans::MITM_TLS_GUEST_HANDSHAKE,
        protocol = "https",
        status = tracing::field::Empty,
        error_kind = tracing::field::Empty,
    );
    // The handshake reads the rest of the ClientHello from the guest; a guest
    // that sends one record byte and stalls must not hold the slot forever.
    let tls_stream = tokio::select! {
        biased;
        _ = config.shutdown.cancelled() => return Ok(String::new()),
        result = tokio::time::timeout(CLASSIFY_READ_TIMEOUT, acceptor.accept(replay)).instrument(tls_span.clone()) => result,
    }
        .unwrap_or_else(|_| {
            Err(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "guest TLS handshake stalled",
            ))
        })
        .map_err(|e| {
            tls_span.record("status", "error");
            tls_span.record("error_kind", "guest_tls_handshake");
            ::metrics::histogram!(m::TLS_HANDSHAKE_MS).record(handshake_start.elapsed().as_secs_f64() * 1000.0);
            (String::new(), Decision::Error, format!("TLS handshake: {e}"))
        })?;
    tls_span.record("status", "ok");
    ::metrics::histogram!(m::TLS_HANDSHAKE_MS).record(handshake_start.elapsed().as_secs_f64() * 1000.0);

    // The SNI is guest-controlled; normalize it so policy, dial and
    // telemetry all see the identity the leaf was minted for.
    let domain = tls_stream
        .get_ref()
        .1
        .server_name()
        .map(crate::net::hostname::normalize_host)
        .filter(|domain| !domain.is_empty())
        .ok_or_else(|| (String::new(), Decision::Denied, "no SNI in ClientHello".into()))?;

    let io = TokioIo::new(tls_stream);
    serve_pipeline(
        io,
        PipelineTarget::Dynamic {
            connection_domain: domain.clone(),
            protocol: Protocol::Tls,
        },
        config,
        process_name,
    )
    .await;
    Ok(domain)
}

/// Plain-HTTP MITM path (T2.2). Skips TLS termination entirely and
/// runs the hyper HTTP/1.1 server directly on the vsock stream
/// (`ReplayReader` carries the buffered first bytes plus the rest of
/// the stream). Per-request domain + upstream port are derived from
/// the inbound `Host` header inside the service closure.
async fn serve_plain_http(
    initial_buf: Vec<u8>,
    vsock_stream: AsyncFdStream,
    config: &Arc<MitmProxyConfig>,
    process_name: Arc<Option<String>>,
) -> Result<String, (String, Decision, String)> {
    let replay = ReplayReader::new(initial_buf, vsock_stream);
    let io = TokioIo::new(replay);
    serve_pipeline(
        io,
        PipelineTarget::Dynamic {
            connection_domain: String::new(),
            protocol: Protocol::Http,
        },
        config,
        process_name,
    )
    .await;
    // Per-request telemetry is emitted by `TelemetryHook`. The
    // connection-level `NetEvent` `handle_connection` would write on
    // an Err-return is intentionally skipped on this path -- there
    // is no connection-level domain to attribute (each request can
    // carry a different Host header).
    Ok(String::new())
}
