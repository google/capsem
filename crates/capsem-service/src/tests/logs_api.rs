use super::*;

#[tokio::test]
async fn host_logs_offer_typed_json_and_preserve_plain_text() {
    let (state, _dir) = make_test_state_with_tempdir();
    std::fs::write(
        state.run_dir.join("service.log"),
        "start\nerror first\nokay\nerror last\n",
    )
    .unwrap();
    for accept in ["application/json", "text/plain"] {
        let response = build_service_router(state.clone())
            .oneshot(
                axum::http::Request::builder()
                    .uri("/host-logs/service?grep=error&tail=1")
                    .header("accept", accept)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let content_type = response.headers()["content-type"].to_str().unwrap().to_owned();
        let bytes = to_bytes(response.into_body(), 1024).await.unwrap();
        if accept == "application/json" {
            assert!(content_type.starts_with("application/json"));
            let body: capsem_api::HostLogsResponse = serde_json::from_slice(&bytes).unwrap();
            assert_eq!(body.source, capsem_api::HostLogSource::Service);
            assert_eq!(body.text, "error last");
        } else {
            assert!(content_type.starts_with("text/plain"));
            assert_eq!(bytes, "error last");
        }
    }
}

#[tokio::test]
async fn host_log_sources_read_rotated_streams_and_enforce_byte_limits() {
    let (state, _dir) = make_test_state_with_tempdir();
    for source in ["service", "mcp", "gateway", "tray"] {
        std::fs::write(
            state.run_dir.join(format!("{source}.2026-09-10.log")),
            "old line\nlast\n",
        )
        .unwrap();
        let (status, body) = route_request(
            build_service_router(state.clone()),
            axum::http::Method::GET,
            &format!("/host-logs/{source}?max_bytes=7"),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["raw"], "last\n");
    }
    for query in ["max_bytes=0", "tail=0"] {
        let (status, body) = route_request(
            build_service_router(state.clone()),
            axum::http::Method::GET,
            &format!("/host-logs/service?{query}"),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert!(body.is_null());
    }
    for uri in [
        "/host-logs/secret",
        "/host-logs/service.log",
        "/host-logs/service?max_bytes=-1",
    ] {
        let (status, _) = route_request(build_service_router(state.clone()), axum::http::Method::GET, uri, None).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{uri}");
    }
}

#[tokio::test]
async fn host_logs_cap_large_requested_reads_and_honor_json_opt_out() {
    let (state, _dir) = make_test_state_with_tempdir();
    std::fs::write(state.run_dir.join("service.log"), "line\n".repeat(1_100_000)).unwrap();
    let response = build_service_router(state)
        .oneshot(
            axum::http::Request::builder()
                .uri("/host-logs/service?max_bytes=18446744073709551615")
                .header("accept", "application/json;q=0, text/plain")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["vary"], "Accept");
    assert!(response.headers()["content-type"]
        .to_str()
        .unwrap()
        .starts_with("text/plain"));
    let bytes = to_bytes(response.into_body(), 6 * 1024 * 1024).await.unwrap();
    assert!(bytes.len() <= 5 * 1024 * 1024);
    assert!(bytes.ends_with(b"line\n"));
}

#[tokio::test]
async fn vm_log_filters_apply_to_both_retained_failed_boot_streams() {
    let (state, _dir) = make_test_state_with_tempdir();
    let session = state.run_dir.join("sessions/vm-failed-1");
    std::fs::create_dir_all(&session).unwrap();
    std::fs::write(session.join("serial.log"), "error old\nokay\nerror serial\n").unwrap();
    std::fs::write(session.join("process.log"), "error old\nerror process\n").unwrap();
    let (status, body) = route_request(
        build_service_router(state),
        axum::http::Method::GET,
        "/vms/vm/logs?grep=error&tail=1",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["logs"], "error serial");
    assert_eq!(body["serial_logs"], "error serial");
    assert_eq!(body["process_logs"], "error process");
}
