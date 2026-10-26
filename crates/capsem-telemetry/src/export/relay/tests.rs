use super::*;

fn env_of<'a>(vars: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + 'a {
    move |key| vars.iter().find_map(|(k, v)| (*k == key).then(|| v.to_string()))
}

#[test]
fn trusted_service_configuration_resolves_corp_and_standard_signal_precedence() {
    assert!(Collector::resolve(None, env_of(&[])).unwrap().is_none());
    let collector = Collector::resolve(Some("https://corp.example/base/"), env_of(&[]))
        .unwrap()
        .unwrap();
    assert_eq!(collector.endpoint.as_str(), "https://corp.example/base/v1/metrics");
    let collector = Collector::resolve(
        Some("https://corp.example"),
        env_of(&[
            ("OTEL_EXPORTER_OTLP_ENDPOINT", "http://base.example/root/"),
            ("OTEL_EXPORTER_OTLP_METRICS_ENDPOINT", "https://signal.example/exact"),
            ("OTEL_EXPORTER_OTLP_HEADERS", "authorization=base-secret"),
            (
                "OTEL_EXPORTER_OTLP_METRICS_HEADERS",
                "authorization=Bearer%20signal-secret,x-tag=a%2Cb",
            ),
        ]),
    )
    .unwrap()
    .unwrap();
    assert_eq!(collector.endpoint.as_str(), "https://signal.example/exact");
    assert_eq!(collector.headers["authorization"], "Bearer signal-secret");
    assert_eq!(collector.headers["x-tag"], "a,b");
    assert_eq!(collector.headers.len(), 2);
    assert!(!format!("{collector:?}").contains("secret"));
    let collector = Collector::resolve(
        None,
        env_of(&[("OTEL_EXPORTER_OTLP_ENDPOINT", "http://base.example/root")]),
    )
    .unwrap()
    .unwrap();
    assert_eq!(collector.endpoint.as_str(), "http://base.example/root/v1/metrics");
}

#[test]
fn refuses_unbounded_or_invalid_configuration_without_printing_values() {
    for endpoint in [
        "file:///tmp/private",
        "unix:///socket",
        "https://",
        "http://localhost/#private",
    ] {
        let result = Collector::resolve(Some(endpoint), env_of(&[]));
        assert!(result.is_err(), "{endpoint}");
        assert_eq!(
            result.unwrap_err().to_string(),
            "metric collector configuration or delivery failed"
        );
    }
    for (key, value) in [
        ("OTEL_EXPORTER_OTLP_HEADERS", "authorization=private\nheader"),
        ("OTEL_EXPORTER_OTLP_HEADERS", "authorization=%GG"),
        ("OTEL_EXPORTER_OTLP_PROTOCOL", "grpc"),
        ("OTEL_EXPORTER_OTLP_METRICS_COMPRESSION", "gzip"),
    ] {
        assert!(Collector::resolve(Some("https://collector.example"), env_of(&[(key, value)])).is_err());
    }
    assert!(Collector::resolve(Some(&format!("https://example.com/{}", "a".repeat(2048))), env_of(&[])).is_err());
    let headers = format!("authorization={}", "a".repeat(16 * 1024));
    assert!(Collector::resolve(
        Some("https://collector.example"),
        env_of(&[("OTEL_EXPORTER_OTLP_HEADERS", &headers)])
    )
    .is_err());
}

fn real_sdk_body() -> Bytes {
    use super::super::tests::CapturingHttpClient;
    use opentelemetry::metrics::MeterProvider as _;
    let client = CapturingHttpClient::default();
    let received = std::sync::Arc::clone(&client.0);
    let provider = super::super::gateway_provider_with_http_client(client).unwrap();
    let recorder = super::super::OtelRecorder::new(provider.meter("capsem"));
    metrics::with_local_recorder(&recorder, || {
        crate::describe_all();
        metrics::counter!(crate::http::REQUESTS_TOTAL, "component" => "gateway", "method" => "GET", "route" => "/vms/list").increment(1);
    });
    provider.force_flush().unwrap();
    let body = received.lock().unwrap().pop().unwrap().into_body();
    provider.shutdown().unwrap();
    body
}

#[tokio::test]
async fn actual_collector_receives_original_bytes_and_auth_without_returning_unbounded_body() {
    use std::io::Write;
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let (tx, rx) = tokio::sync::oneshot::channel();
    let received = std::thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        socket
            .set_read_timeout(Some(std::time::Duration::from_secs(2)))
            .unwrap();
        let (lines, body) = read_request(&socket);
        // Only response headers are relevant. A malicious declared response
        // length cannot allocate that body or leak bytes back to the gateway.
        socket
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 4294967295\r\n\r\n")
            .unwrap();
        tx.send((lines, body)).unwrap();
    });
    let body = real_sdk_body();
    let expected = body.clone();
    let pointer = body.as_ptr();
    let admitted = AdmittedBody::admit(body).unwrap();
    assert_eq!(admitted.0.as_ptr(), pointer);
    let collector = Collector::resolve(
        Some(&endpoint),
        env_of(&[(
            "OTEL_EXPORTER_OTLP_HEADERS",
            "authorization=Bearer%20fake-collector-canary",
        )]),
    )
    .unwrap()
    .unwrap();
    assert!(collector.headers["authorization"].is_sensitive());
    tokio::time::timeout(std::time::Duration::from_secs(2), collector.send(admitted))
        .await
        .unwrap()
        .unwrap();
    let (lines, received_body) = rx.await.unwrap();
    assert_eq!(received_body.as_slice(), expected.as_ref());
    assert!(lines[0].starts_with("POST /v1/metrics HTTP/1.1"));
    assert!(lines
        .iter()
        .any(|line| line == "authorization: Bearer fake-collector-canary\r\n"));
    assert!(lines
        .iter()
        .any(|line| line == "content-type: application/x-protobuf\r\n"));
    received.join().unwrap();
}

#[tokio::test]
async fn redirect_and_transport_failure_are_not_relayed_and_have_static_errors() {
    use std::io::Write;
    let target = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    target.set_nonblocking(true).unwrap();
    let redirect = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", redirect.local_addr().unwrap());
    let destination = format!("http://{}/secret-canary", target.local_addr().unwrap());
    let task = std::thread::spawn(move || {
        let (mut socket, _) = redirect.accept().unwrap();
        socket
            .set_read_timeout(Some(std::time::Duration::from_secs(2)))
            .unwrap();
        let _ = read_request(&socket);
        socket
            .write_all(
                format!("HTTP/1.1 307 Temporary Redirect\r\nLocation: {destination}\r\nContent-Length: 0\r\n\r\n")
                    .as_bytes(),
            )
            .unwrap();
    });
    let collector = Collector::resolve(Some(&base), env_of(&[])).unwrap().unwrap();
    let error = collector
        .send(AdmittedBody::admit(real_sdk_body()).unwrap())
        .await
        .unwrap_err();
    assert_eq!(error.to_string(), "metric collector configuration or delivery failed");
    assert_eq!(target.accept().unwrap_err().kind(), std::io::ErrorKind::WouldBlock);
    task.join().unwrap();
    drop(target);
    let collector = Collector::resolve(Some("http://127.0.0.1:1"), env_of(&[]))
        .unwrap()
        .unwrap();
    assert_eq!(
        collector
            .send(AdmittedBody::admit(real_sdk_body()).unwrap())
            .await
            .unwrap_err(),
        CollectorError
    );
    assert!(AdmittedBody::admit(Bytes::from_static(b"secret-canary-not-protobuf")).is_err());
}

#[test]
fn service_configuration_matches_actual_sdk_environment_resolution() {
    use std::process::Command;
    for case in ["base", "signal", "corp"] {
        let mut child = Command::new(std::env::current_exe().unwrap());
        child
            .env_clear()
            .env("CAPSEM_OTLP_CONFIG_TEST", case)
            .env("OTEL_EXPORTER_OTLP_HEADERS", "authorization=Bearer%20base-canary")
            .args(["--exact", "export::relay::tests::sdk_configuration_probe"]);
        // Instrumented reexecs must retain the gate-owned profile destination.
        if let Some(profile) = std::env::var_os("LLVM_PROFILE_FILE") {
            child.env("LLVM_PROFILE_FILE", profile);
        }
        if case != "corp" {
            child.env("OTEL_EXPORTER_OTLP_ENDPOINT", "https://base.example/prefix/");
        }
        if case == "signal" {
            child
                .env("OTEL_EXPORTER_OTLP_METRICS_ENDPOINT", "https://signal.example/exact")
                .env(
                    "OTEL_EXPORTER_OTLP_METRICS_HEADERS",
                    "authorization=Bearer%20signal-canary",
                );
        }
        let output = child.output().unwrap();
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    }
}

#[test]
fn sdk_configuration_probe() {
    use super::super::tests::CapturingHttpClient;
    use opentelemetry::metrics::MeterProvider as _;
    use opentelemetry_otlp::{WithExportConfig as _, WithHttpConfig as _};
    let Ok(case) = std::env::var("CAPSEM_OTLP_CONFIG_TEST") else {
        return;
    };
    let client = CapturingHttpClient::default();
    let received = std::sync::Arc::clone(&client.0);
    let mut builder = opentelemetry_otlp::MetricExporter::builder()
        .with_http()
        .with_http_client(client);
    if case == "corp" {
        builder = builder.with_endpoint("https://corp.example/v1/metrics");
    }
    let provider = super::super::build_resource_provider(
        builder,
        opentelemetry_sdk::Resource::builder_empty()
            .with_service_name("capsem-gateway")
            .build(),
    )
    .unwrap();
    let recorder = super::super::OtelRecorder::new(provider.meter("capsem"));
    metrics::with_local_recorder(&recorder, || {
        metrics::counter!(crate::http::REQUESTS_TOTAL).increment(1);
    });
    provider.force_flush().unwrap();
    let request = received.lock().unwrap().pop().unwrap();
    let collector = Collector::resolve(Some("https://corp.example"), |name| std::env::var(name).ok())
        .unwrap()
        .unwrap();
    assert_eq!(request.uri().to_string(), collector.endpoint.as_str());
    assert_eq!(request.headers()["authorization"], collector.headers["authorization"]);
}

#[tokio::test]
async fn slow_collector_times_out_without_changing_error_or_requiring_response_body() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let mut collector = Collector::resolve(Some(&base), env_of(&[])).unwrap().unwrap();
    assert_eq!(EXPORT_TIMEOUT, std::time::Duration::from_secs(10));
    collector.client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_millis(20))
        .build()
        .unwrap();
    // The kernel accepts the TCP handshake but no collector consumes/responds.
    assert_eq!(
        collector
            .send(AdmittedBody::admit(real_sdk_body()).unwrap())
            .await
            .unwrap_err(),
        CollectorError
    );
}

#[test]
fn header_count_and_empty_signal_override_do_not_inherit_general_auth() {
    let collector = Collector::resolve(
        Some("https://collector.example"),
        env_of(&[
            ("OTEL_EXPORTER_OTLP_HEADERS", "authorization=general-secret"),
            ("OTEL_EXPORTER_OTLP_METRICS_HEADERS", ""),
        ]),
    )
    .unwrap()
    .unwrap();
    assert!(collector.headers.is_empty());
    let headers = (0..65)
        .map(|n| format!("x-label-{n}=value"))
        .collect::<Vec<_>>()
        .join(",");
    assert!(Collector::resolve(
        Some("https://collector.example"),
        env_of(&[("OTEL_EXPORTER_OTLP_HEADERS", &headers)])
    )
    .is_err());
}

fn read_request(socket: &std::net::TcpStream) -> (Vec<String>, Vec<u8>) {
    use std::io::{BufRead, BufReader, Read};
    let mut input = BufReader::new(socket.try_clone().unwrap());
    let mut lines = Vec::new();
    loop {
        let mut line = String::new();
        assert!(input.read_line(&mut line).unwrap() > 0);
        if line == "\r\n" {
            break;
        }
        lines.push(line);
    }
    let length: usize = lines
        .iter()
        .find_map(|line| {
            line.to_ascii_lowercase()
                .strip_prefix("content-length:")
                .map(|n| n.trim().parse().unwrap())
        })
        .unwrap();
    assert!(length < 4096);
    let mut body = vec![0; length];
    input.read_exact(&mut body).unwrap();
    (lines, body)
}
