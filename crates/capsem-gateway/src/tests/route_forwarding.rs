use super::*;

fn service_proxy_app(uds_path: &str) -> axum::Router {
    let state = Arc::new(AppState {
        token: "test".into(),
        uds_path: uds_path.into(),
        service_client: ServiceClient::new(std::path::Path::new(uds_path)),
        status_cache: StatusCache::new(),
        auth_failures: AuthFailureTracker::new(),
        events_tx: tokio::sync::broadcast::channel(16).0,
    });
    service_proxy_routes().with_state(state)
}

#[tokio::test]
async fn gateway_unknown_paths_are_not_forwarded_to_service() {
    let app = service_proxy_app("/tmp/capsem-gateway-must-not-connect.sock");
    let resp = app
        .oneshot(
            http::Request::builder()
                .uri("/not-a-capsem-api")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), http::StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn gateway_profile_assets_edit_is_not_forwarded() {
    let app = service_proxy_app("/tmp/capsem-gateway-must-not-connect.sock");
    let resp = app
        .oneshot(
            http::Request::builder()
                .method("PATCH")
                .uri("/profiles/code/assets/edit")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), http::StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn gateway_profile_lifecycle_writes_are_not_forwarded() {
    let app = service_proxy_app("/tmp/capsem-gateway-must-not-connect.sock");
    for (method, uri) in [
        ("POST", "/profiles/create"),
        ("PATCH", "/profiles/code/edit"),
        ("DELETE", "/profiles/code/delete"),
        ("POST", "/profiles/code/clone"),
    ] {
        let resp = app
            .clone()
            .oneshot(
                http::Request::builder()
                    .method(method)
                    .uri(uri)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), http::StatusCode::NOT_FOUND, "{method} {uri}");
    }
}

#[tokio::test]
async fn gateway_update_status_route_is_get_only() {
    let app = service_proxy_app("/tmp/capsem-gateway-missing-service.sock");
    let get_resp = app
        .clone()
        .oneshot(
            http::Request::builder()
                .method("GET")
                .uri("/update/status")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(get_resp.status(), http::StatusCode::BAD_GATEWAY);

    let post_resp = app
        .oneshot(
            http::Request::builder()
                .method("POST")
                .uri("/update/status")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(post_resp.status(), http::StatusCode::METHOD_NOT_ALLOWED);
}

#[tokio::test]
async fn gateway_update_action_routes_are_post_only() {
    for uri in ["/update/check", "/update/apply"] {
        let app = service_proxy_app("/tmp/capsem-gateway-missing-service.sock");
        let post_resp = app
            .clone()
            .oneshot(
                http::Request::builder()
                    .method("POST")
                    .uri(uri)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(post_resp.status(), http::StatusCode::BAD_GATEWAY, "{uri}");

        let get_resp = app
            .oneshot(
                http::Request::builder()
                    .method("GET")
                    .uri(uri)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(get_resp.status(), http::StatusCode::METHOD_NOT_ALLOWED, "{uri}");
    }
}

#[tokio::test]
async fn gateway_fake_vm_mutation_routes_are_not_forwarded() {
    let app = service_proxy_app("/tmp/capsem-gateway-must-not-connect.sock");
    for (method, uri) in [
        ("PATCH", "/vms/test-vm/edit"),
        ("POST", "/vms/test-vm/restart"),
        ("POST", "/vms/test-vm/reload-profile"),
    ] {
        let resp = app
            .clone()
            .oneshot(
                http::Request::builder()
                    .method(method)
                    .uri(uri)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), http::StatusCode::NOT_FOUND, "{method} {uri}");
    }
}

#[tokio::test]
async fn gateway_security_routes_are_explicitly_forwarded() {
    for (method, uri) in [
        ("GET", "/vms/test-vm/security/latest"),
        ("GET", "/vms/test-vm/security/status"),
        ("GET", "/vms/test-vm/detection/latest"),
        ("GET", "/vms/test-vm/detection/status"),
        ("GET", "/vms/test-vm/enforcement/latest"),
        ("GET", "/vms/test-vm/enforcement/status"),
        ("GET", "/security/latest"),
        ("GET", "/security/status"),
        ("GET", "/enforcement/latest"),
        ("GET", "/enforcement/status"),
        ("GET", "/detection/latest"),
        ("GET", "/detection/status"),
        ("GET", "/profiles/list"),
        ("GET", "/profiles/status"),
        ("GET", "/update/status"),
        ("POST", "/update/check"),
        ("POST", "/update/apply"),
        ("POST", "/profiles/reload"),
        ("GET", "/profiles/code/info"),
        ("GET", "/profiles/code/obom"),
        ("POST", "/profiles/code/validate"),
        ("POST", "/vms/create"),
        ("GET", "/vms/list"),
        ("GET", "/vms/test-vm/info"),
        ("GET", "/vms/test-vm/status"),
        ("GET", "/vms/test-vm/snapshots/status"),
        ("GET", "/vms/test-vm/snapshots/list"),
        ("GET", "/vms/test-vm/logs"),
        ("POST", "/vms/test-vm/exec"),
        ("POST", "/vms/test-vm/files/write"),
        ("POST", "/vms/test-vm/files/read"),
        ("GET", "/vms/test-vm/files/list"),
        ("GET", "/vms/test-vm/files/content?path=/root/a.txt"),
        ("POST", "/vms/test-vm/files/content?path=/root/a.txt"),
        ("GET", "/vms/test-vm/history"),
        ("GET", "/vms/test-vm/history/processes"),
        ("GET", "/vms/test-vm/history/counts"),
        ("GET", "/vms/test-vm/history/transcript"),
        ("GET", "/vms/test-vm/stats/detail"),
        ("GET", "/vms/test-vm/stats/summary"),
        ("GET", "/vms/test-vm/timeline"),
        ("POST", "/vms/test-vm/stop"),
        ("POST", "/vms/test-vm/pause"),
        ("DELETE", "/vms/test-vm/delete"),
        ("POST", "/vms/test-vm/start"),
        ("POST", "/vms/test-vm/resume"),
        ("POST", "/vms/test-vm/save"),
        ("GET", "/vms/test-vm/save/status"),
        ("GET", "/vms/test-vm/fork/status"),
        ("POST", "/vms/test-vm/fork"),
        ("POST", "/profiles/code/enforcement/evaluate"),
        ("GET", "/profiles/code/enforcement/info"),
        ("PUT", "/profiles/code/enforcement/rules/eicar_block/edit"),
        ("DELETE", "/profiles/code/enforcement/rules/eicar_block/delete"),
        ("POST", "/profiles/code/enforcement/reload"),
        ("GET", "/profiles/code/enforcement/rules/list"),
        ("POST", "/profiles/code/detection/evaluate"),
        ("GET", "/profiles/code/detection/info"),
        ("PUT", "/profiles/code/detection/rules/eicar_detect/edit"),
        ("DELETE", "/profiles/code/detection/rules/eicar_detect/delete"),
        ("POST", "/profiles/code/detection/reload"),
        ("GET", "/profiles/code/detection/rules/list"),
        ("GET", "/profiles/code/assets/status"),
        ("GET", "/profiles/code/assets/info"),
        ("POST", "/profiles/code/assets/ensure"),
        ("GET", "/profiles/code/skills/info"),
        ("GET", "/profiles/code/skills/list"),
        ("POST", "/profiles/code/skills/add"),
        ("PATCH", "/profiles/code/skills/build/edit"),
        ("DELETE", "/profiles/code/skills/build/delete"),
        ("GET", "/profiles/code/plugins/list"),
        ("GET", "/profiles/code/plugins/info"),
        ("GET", "/profiles/code/plugins/dummy_pre_eicar/info"),
        ("PATCH", "/profiles/code/plugins/dummy_pre_eicar/edit"),
        ("GET", "/profiles/code/plugins/credential_broker/credentials/info"),
        ("POST", "/profiles/code/plugins/credential_broker/credentials/reload"),
        ("GET", "/profiles/code/mcp/info"),
        ("GET", "/profiles/code/mcp/servers/list"),
        ("GET", "/profiles/code/mcp/default/info"),
        ("PATCH", "/profiles/code/mcp/default/edit"),
        ("PUT", "/profiles/code/mcp/servers/local/edit"),
        ("DELETE", "/profiles/code/mcp/servers/local/delete"),
        ("GET", "/profiles/code/mcp/servers/local/tools/list"),
        ("POST", "/profiles/code/mcp/servers/local/refresh"),
        ("PATCH", "/profiles/code/mcp/servers/local/tools/echo/edit"),
        ("POST", "/profiles/code/mcp/servers/local/tools/echo/call"),
        ("PUT", "/corp/edit"),
        ("GET", "/settings/info"),
        ("PATCH", "/settings/edit"),
        ("POST", "/profiles/code/reload"),
        ("GET", "/corp/info"),
        ("POST", "/corp/validate"),
        ("POST", "/corp/reload"),
    ] {
        let app = service_proxy_app("/tmp/capsem-gateway-missing-service.sock");
        let resp = app
            .oneshot(
                http::Request::builder()
                    .method(method)
                    .uri(uri)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), http::StatusCode::BAD_GATEWAY, "{method} {uri}");
    }
}

#[tokio::test]
async fn gateway_does_not_forward_retired_vm_lifecycle_routes() {
    for (method, uri) in [
        ("POST", "/provision"),
        ("GET", "/list"),
        ("GET", "/info/test-vm"),
        ("POST", "/stop/test-vm"),
        ("GET", "/logs/test-vm"),
        ("POST", "/inspect/test-vm"),
        ("POST", "/exec/test-vm"),
        ("POST", "/write_file/test-vm"),
        ("POST", "/read_file/test-vm"),
        ("GET", "/files/test-vm"),
        ("GET", "/files/test-vm/content?path=/root/a.txt"),
        ("POST", "/files/test-vm/content?path=/root/a.txt"),
        ("GET", "/history/test-vm"),
        ("GET", "/history/test-vm/processes"),
        ("GET", "/history/test-vm/counts"),
        ("GET", "/history/test-vm/transcript"),
        ("GET", "/timeline/test-vm"),
        ("POST", "/suspend/test-vm"),
        ("DELETE", "/delete/test-vm"),
        ("POST", "/resume/test-vm"),
        ("POST", "/persist/test-vm"),
        ("POST", "/fork/test-vm"),
    ] {
        let app = service_proxy_app("/tmp/capsem-gateway-must-not-connect.sock");
        let resp = app
            .oneshot(
                http::Request::builder()
                    .method(method)
                    .uri(uri)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), http::StatusCode::NOT_FOUND, "{method} {uri}");
    }
}

#[tokio::test]
async fn gateway_does_not_forward_retired_plugin_authoring_routes() {
    for (method, uri) in [
        ("GET", "/plugins"),
        ("GET", "/plugins/test-vm"),
        ("GET", "/plugins/test-vm/dummy_pre_eicar"),
        ("POST", "/plugins/test-vm/dummy_pre_eicar"),
        ("GET", "/plugins/global/dummy_pre_eicar"),
        ("POST", "/plugins/global/dummy_pre_eicar"),
    ] {
        let app = service_proxy_app("/tmp/capsem-gateway-must-not-connect.sock");
        let resp = app
            .oneshot(
                http::Request::builder()
                    .method(method)
                    .uri(uri)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), http::StatusCode::NOT_FOUND, "{method} {uri}");
    }
}

#[tokio::test]
async fn gateway_does_not_forward_retired_profile_credential_routes() {
    for (method, uri) in [
        ("GET", "/profiles/code/credentials/info"),
        ("GET", "/profiles/code/credentials/status"),
        ("GET", "/profiles/code/credentials/list"),
        ("POST", "/profiles/code/credentials/reload"),
        ("GET", "/profiles/code/credentials/openai/info"),
        ("DELETE", "/profiles/code/credentials/openai/delete"),
    ] {
        let app = service_proxy_app("/tmp/capsem-gateway-must-not-connect.sock");
        let resp = app
            .oneshot(
                http::Request::builder()
                    .method(method)
                    .uri(uri)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), http::StatusCode::NOT_FOUND, "{method} {uri}");
    }
}

#[tokio::test]
async fn gateway_does_not_forward_retired_enforcement_authoring_routes() {
    for (method, uri) in [
        ("POST", "/enforcements/evaluate"),
        ("POST", "/enforcements/rules/eicar_block"),
        ("DELETE", "/enforcements/rules/eicar_block"),
        ("POST", "/enforcements/reload"),
    ] {
        let app = service_proxy_app("/tmp/capsem-gateway-must-not-connect.sock");
        let resp = app
            .oneshot(
                http::Request::builder()
                    .method(method)
                    .uri(uri)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), http::StatusCode::NOT_FOUND, "{method} {uri}");
    }
}

#[tokio::test]
async fn gateway_does_not_forward_retired_ledger_routes() {
    for (method, uri) in [
        ("GET", "/security/test-vm/latest"),
        ("GET", "/security/test-vm/info"),
        ("GET", "/detections/test-vm/latest"),
        ("GET", "/detections/test-vm/info"),
        ("GET", "/enforcements/test-vm/latest"),
        ("GET", "/enforcements/test-vm/info"),
    ] {
        let app = service_proxy_app("/tmp/capsem-gateway-must-not-connect.sock");
        let resp = app
            .oneshot(
                http::Request::builder()
                    .method(method)
                    .uri(uri)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), http::StatusCode::NOT_FOUND, "{method} {uri}");
    }
}

#[tokio::test]
async fn gateway_does_not_forward_retired_corp_config_route() {
    let app = service_proxy_app("/tmp/capsem-gateway-must-not-connect.sock");
    let resp = app
        .oneshot(
            http::Request::builder()
                .method("POST")
                .uri("/corp-config")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), http::StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn gateway_does_not_forward_retired_global_asset_routes() {
    for (method, uri) in [("GET", "/assets/status"), ("POST", "/assets/ensure")] {
        let app = service_proxy_app("/tmp/capsem-gateway-must-not-connect.sock");
        let resp = app
            .oneshot(
                http::Request::builder()
                    .method(method)
                    .uri(uri)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), http::StatusCode::NOT_FOUND, "{method} {uri}");
    }
}

#[tokio::test]
async fn gateway_does_not_forward_retired_magic_settings_route() {
    for (method, uri) in [("GET", "/settings"), ("POST", "/settings")] {
        let app = service_proxy_app("/tmp/capsem-gateway-must-not-connect.sock");
        let resp = app
            .oneshot(
                http::Request::builder()
                    .method(method)
                    .uri(uri)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), http::StatusCode::NOT_FOUND, "{method} {uri}");
    }
}

#[tokio::test]
async fn gateway_does_not_forward_retired_settings_utility_routes() {
    for (method, uri) in [
        ("GET", "/settings/presets"),
        ("POST", "/settings/presets/high"),
        ("POST", "/settings/lint"),
        ("POST", "/settings/validate-key"),
    ] {
        let app = service_proxy_app("/tmp/capsem-gateway-must-not-connect.sock");
        let resp = app
            .oneshot(
                http::Request::builder()
                    .method(method)
                    .uri(uri)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), http::StatusCode::NOT_FOUND, "{method} {uri}");
    }
}

#[tokio::test]
async fn gateway_does_not_forward_retired_global_reload_route() {
    let app = service_proxy_app("/tmp/capsem-gateway-must-not-connect.sock");
    let resp = app
        .oneshot(
            http::Request::builder()
                .method("POST")
                .uri("/reload-config")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), http::StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn gateway_does_not_forward_retired_mcp_policy_route() {
    for (method, uri) in [
        ("GET", "/mcp/policy"),
        ("GET", "/mcp/servers"),
        ("GET", "/mcp/tools"),
        ("POST", "/mcp/tools/refresh"),
        ("POST", "/mcp/tools/local__echo/approve"),
        ("POST", "/mcp/tools/local__echo/call"),
    ] {
        let app = service_proxy_app("/tmp/capsem-gateway-must-not-connect.sock");
        let resp = app
            .oneshot(
                http::Request::builder()
                    .method(method)
                    .uri(uri)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), http::StatusCode::NOT_FOUND, "{method} {uri}");
    }
}
