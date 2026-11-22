use super::*;

#[tokio::test]
async fn gateway_overview_reports_only_the_current_sources_cached_verification() {
    let fx = Fixture::new().await;
    let kind = CatalogKind::Skills;
    let digest = fx.registry.publish(kind, document(kind, false));
    let app = crate::router_runtime::build_service_router(fx.state.clone());
    let refresh_sources = || super::super::super::refresh_catalog_sources(&fx.state);
    let status = || {
        let app = app.clone();
        async move {
            let (code, body) = route_request(app, Method::GET, "/internal/gateway/status", None).await;
            assert_eq!(code, StatusCode::OK, "{body}");
            body["catalogs"].clone()
        }
    };
    refresh_sources().await;
    let initial = status().await;
    assert!(fx.registry.requests.lock().unwrap().is_empty());
    assert_eq!(
        initial[0],
        json!({
            "kind":"skills", "availability":"unverified", "reference":fx.registry.reference(kind),
            "origin":"manifest", "digest":null, "last_successful_refresh":null, "error":null
        })
    );
    assert_eq!(initial[1]["availability"], "unverified");
    assert_eq!(initial[2]["error"], "missing_source");

    let fetch = || route_request(app.clone(), Method::GET, "/skills/list", None);
    assert_eq!(fetch().await.0, StatusCode::OK);
    let calls = fx.registry.requests.lock().unwrap().len();
    assert!(calls > 0);
    let available = status().await;
    let time = available[0]["last_successful_refresh"].as_i64().unwrap();
    assert!(time > 0);
    assert_eq!(available[0]["availability"], "available");
    assert_eq!(available[0]["digest"], digest);
    for _ in 0..5 {
        assert_eq!(status().await, available);
    }
    assert_eq!(fx.registry.requests.lock().unwrap().len(), calls);

    fx.registry.unpublish_tag(kind);
    assert_eq!(fetch().await.0, StatusCode::SERVICE_UNAVAILABLE);
    let stale = status().await;
    assert_eq!(stale[0]["availability"], "stale");
    assert_eq!(stale[0]["error"], "registry_unavailable");
    assert_eq!(stale[0]["digest"], digest);
    assert_eq!(stale[0]["last_successful_refresh"], time);

    let mut invalid = document(kind, false);
    invalid["entries"][0]["payload"]["files"]["SKILL.md"]["sha256"] = format!("sha256:{}", "0".repeat(64)).into();
    fx.registry.publish(kind, invalid);
    assert_eq!(fetch().await.0, StatusCode::BAD_GATEWAY);
    let invalid = status().await;
    assert_eq!(invalid[0]["availability"], "stale");
    assert_eq!(invalid[0]["error"], "verification_failed");
    assert_eq!(invalid[0]["digest"], digest);

    let reference = format!("{}/team/corporate:stable", fx.registry.address);
    fx.corporate.lock().unwrap().catalogs = Some(CatalogSources {
        skills: Some(descriptor(reference.clone())),
        ..Default::default()
    });
    refresh_sources().await;
    let overridden = status().await;
    assert_eq!(overridden[0]["reference"], reference);
    assert_eq!(overridden[0]["origin"], "corporate");
    assert_eq!(overridden[0]["availability"], "unverified");
    assert!(overridden[0]["digest"].is_null());
    assert!(overridden[0]["last_successful_refresh"].is_null());
    assert_eq!(fetch().await.0, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(status().await[0]["availability"], "unavailable");

    fx.corporate.lock().unwrap().catalogs.as_mut().unwrap().skills = Some(descriptor("invalid source".into()));
    refresh_sources().await;
    let refused = status().await;
    assert_eq!(refused[0]["origin"], "corporate");
    assert_eq!(refused[0]["error"], "invalid_source");
    assert!(refused[0]["reference"].is_null());
    *fx.broken.lock().unwrap() = true;
    refresh_sources().await;
    let broken = status().await;
    assert_eq!(broken[0]["error"], "policy_unavailable");
    assert!(broken[0]["origin"].is_null());
}
