use super::*;

#[tokio::test]
async fn global_rules_return_exact_typed_templates_and_enforce_catalog_admission() {
    let fx = Fixture::new().await;
    let kind = CatalogKind::Rules;
    let mut original = document(kind, false);
    let host = json!({"type":"text","default":"api.example","max_bytes":70});
    let access = &mut original["entries"][0];
    access["parameters"]["host"] = host.clone();
    access["definition"]["rule"]["match"] = "http.host == {{host}}".into();
    let mut detection = access.clone();
    detection["metadata"]["id"] = "witness".into();
    detection["metadata"]["name"] = "Review observations".into();
    detection["definition"] = json!({"kind":"detection","rule":{
        "name":"Observe endpoint", "level":"high", "match":"http.host == {{host}}"
    }});
    original["entries"].as_array_mut().unwrap().push(detection);
    let digest = fx.registry.publish(kind, original.clone());
    let app = crate::router_runtime::build_service_router(fx.state.clone());
    let get = |uri: String| {
        let app = app.clone();
        async move { route_request(app, Method::GET, &uri, None).await }
    };
    let (status, public) = get("/rules/list?search=REVIEW&limit=1".into()).await;
    assert_eq!(status, StatusCode::OK, "{public}");
    assert_eq!(public["rules"][0]["kind"], "access");
    assert_eq!(public["rules"][0]["metadata"]["id"], "review");
    assert!(public["rules"][0].get("definition").is_none());
    assert_eq!(public["catalog"]["digest"], digest);
    let snapshot = public["catalog"]["snapshot"].as_str().unwrap();
    let cursor = public["next_cursor"].as_str().unwrap();
    let next = format!("/rules/list?search=REVIEW&after={cursor}&limit=1");
    let (status, page) = get(next.clone()).await;
    assert_eq!(status, StatusCode::OK, "{page}");
    assert_eq!(page["rules"][0]["kind"], "detection");
    assert_eq!(page["rules"][0]["metadata"]["id"], "witness");
    assert!(page["next_cursor"].is_null());
    let info = |id: &str, snapshot: &str| format!("/rules/{id}/info?version=1.0.0&snapshot={snapshot}");
    for (id, expected_kind) in [("review", "access"), ("witness", "detection")] {
        let (status, detail) = get(info(id, snapshot)).await;
        assert_eq!(status, StatusCode::OK, "{detail}");
        assert_eq!(detail["definition"]["kind"], expected_kind);
        assert_eq!(detail["definition"]["rule"]["match"], "http.host == {{host}}");
        assert_eq!(detail["parameters"]["host"]["default"], host["default"]);
        assert_eq!(detail["parameters"]["host"]["max_bytes"], host["max_bytes"]);
        assert_eq!(detail["selection"]["resolved_digest"], digest);
    }
    assert_eq!(get(info("unknown", snapshot)).await.0, StatusCode::NOT_FOUND);

    fx.registry.alias("team/rules/", "team/private/");
    let corporate_reference = format!("{}/team/private:stable", fx.registry.address);
    fx.corporate.lock().unwrap().catalogs = Some(CatalogSources {
        rules: Some(descriptor(corporate_reference.clone())),
        ..Default::default()
    });
    let (status, overridden) = get("/rules/list".into()).await;
    assert_eq!(status, StatusCode::OK, "{overridden}");
    assert_eq!(overridden["catalog"]["digest"], digest);
    assert_eq!(overridden["catalog"]["reference"], corporate_reference);
    assert_eq!(get(next.clone()).await.0, StatusCode::CONFLICT);
    assert_eq!(get(info("review", snapshot)).await.0, StatusCode::CONFLICT);

    // A failed override must not try the public tag. Returning to the public
    // source allows the separately namespaced revocation check below.
    fx.registry.requests.lock().unwrap().clear();
    fx.corporate.lock().unwrap().catalogs.as_mut().unwrap().rules =
        Some(descriptor(format!("{}/team/missing:stable", fx.registry.address)));
    assert_eq!(get("/rules/list".into()).await.0, StatusCode::SERVICE_UNAVAILABLE);
    assert!(fx
        .registry
        .requests
        .lock()
        .unwrap()
        .iter()
        .all(|path| !path.starts_with("team/rules/")));
    fx.corporate.lock().unwrap().catalogs = None;

    assert_ne!(fx.registry.publish(kind, document(kind, true)), digest);
    assert_eq!(get(next).await.0, StatusCode::CONFLICT);
    assert_eq!(get(info("review", snapshot)).await.0, StatusCode::CONFLICT);
    assert_eq!(fx.registry.publish(kind, original), digest);
    let (status, rolled_back) = get("/rules/list".into()).await;
    assert_eq!(status, StatusCode::OK, "{rolled_back}");
    let rules = rolled_back["rules"].as_array().unwrap();
    assert_eq!(rules.len(), 1);
    assert_eq!(rules[0]["metadata"]["id"], "witness");
    assert_eq!(get(info("review", snapshot)).await.0, StatusCode::FORBIDDEN);
}
