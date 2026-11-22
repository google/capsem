use super::*;

#[tokio::test]
async fn global_mcps_browse_the_exact_release_authorized_catalog() {
    let fx = Fixture::new().await;
    let mut original = document(CatalogKind::Mcp, false);
    let endpoint = format!("https://{}/v2/advertised-mcp", fx.registry.address);
    let remote = &mut original["entries"][0];
    remote["definition"]["url"] = endpoint.clone().into();
    remote["definition"]["headers"] = json!({"X-Workspace":{"description":"Workspace identity","required":true}});
    remote["definition"]["secret_binding"] = json!({"description":"Workspace token","required":true});
    remote["published_tools"] = json!([{"name":"review_file","description":"Summarize a file"}]);
    let mut sandbox = remote.clone();
    sandbox["metadata"]["id"] = "review-local".into();
    sandbox["definition"] = json!({"kind":"sandbox","executable":"/usr/bin/reviewer","args":["serve","--stdio"],
        "workdir":"/workspace","variables":{"WORKSPACE_ID":{"description":"Workspace identity","required":true}},
        "secret_variables":{"WORKSPACE_TOKEN":{"description":"Workspace token","required":false}}});
    sandbox.as_object_mut().unwrap().remove("published_tools");
    original["entries"].as_array_mut().unwrap().push(sandbox);
    let digest = fx.registry.publish(CatalogKind::Mcp, original.clone());
    let reference = fx.select_mcp_digest(&digest);
    let app = crate::router_runtime::build_service_router(fx.state.clone());
    let get = |uri: String| {
        let app = app.clone();
        async move { route_request(app, Method::GET, &uri, None).await }
    };
    let overview = || async {
        let (status, view) = route_request(app.clone(), Method::GET, "/internal/gateway/status", None).await;
        assert_eq!(status, StatusCode::OK, "{view}");
        view["catalogs"][2].clone()
    };
    super::super::super::refresh_catalog_sources(&fx.state).await;
    assert_eq!(overview().await["availability"], "unverified");
    assert!(fx.registry.requests.lock().unwrap().is_empty());
    fx.registry.unpublish_tag(CatalogKind::Mcp);
    let (status, first) = get("/mcps/list?search=REVIEW&limit=1".into()).await;
    assert_eq!(status, StatusCode::OK, "{first}");
    assert_eq!(first["catalog"]["reference"], reference);
    assert_eq!(first["catalog"]["digest"], digest);
    assert_eq!(first["mcps"][0]["metadata"]["id"], "review");
    assert_eq!(first["mcps"][0]["kind"], "remote");
    assert!(first["mcps"][0].get("definition").is_none());
    let snapshot = first["catalog"]["snapshot"].as_str().unwrap();
    let cursor = first["next_cursor"].as_str().unwrap();
    let next = format!("/mcps/list?after={cursor}&limit=1");
    let (status, second) = get(next.clone()).await;
    assert_eq!(status, StatusCode::OK, "{second}");
    assert_eq!(second["mcps"][0]["metadata"]["id"], "review-local");
    assert_eq!(second["mcps"][0]["kind"], "sandbox");
    let info = |id: &str, snapshot: &str| format!("/mcps/{id}/info?version=1.0.0&snapshot={snapshot}");
    for (index, id) in ["review", "review-local"].into_iter().enumerate() {
        let (status, detail) = get(info(id, snapshot)).await;
        assert_eq!(status, StatusCode::OK, "{detail}");
        assert_eq!(detail["selection"]["resolved_digest"], digest);
        assert_eq!(detail["definition"], original["entries"][index]["definition"]);
        assert_eq!(detail["published_tools"], original["entries"][index]["published_tools"]);
    }
    assert_eq!(get(info("missing", snapshot)).await.0, StatusCode::NOT_FOUND);
    let available = overview().await;
    assert_eq!(available["availability"], "available");
    assert_eq!(available["origin"], "manifest");
    assert_eq!(available["digest"], digest);
    assert!(fx
        .registry
        .requests
        .lock()
        .unwrap()
        .iter()
        .all(|path| path.starts_with("team/mcp/") && !path.ends_with("/stable")));

    fx.registry.alias("team/mcp/", "team/corporate/");
    let corporate = format!("{}/team/corporate@{digest}", fx.registry.address);
    fx.corporate.lock().unwrap().catalogs = Some(CatalogSources {
        mcp: Some(descriptor(corporate.clone())),
        ..Default::default()
    });
    let (status, replaced) = get("/mcps/list".into()).await;
    assert_eq!(status, StatusCode::OK, "{replaced}");
    assert_eq!(replaced["catalog"]["reference"], corporate);
    assert_eq!(overview().await["origin"], "corporate");
    assert_eq!(get(next).await.0, StatusCode::CONFLICT);
    assert_eq!(get(info("review", snapshot)).await.0, StatusCode::CONFLICT);
    fx.registry.requests.lock().unwrap().clear();
    fx.corporate.lock().unwrap().catalogs.as_mut().unwrap().mcp =
        Some(descriptor(format!("{}/team/missing@{digest}", fx.registry.address)));
    assert_eq!(get("/mcps/list".into()).await.0, StatusCode::SERVICE_UNAVAILABLE);
    assert!(fx
        .registry
        .requests
        .lock()
        .unwrap()
        .iter()
        .all(|path| path.starts_with("team/missing/")));
    fx.corporate.lock().unwrap().catalogs = None;

    let mut malformed = original.clone();
    malformed["entries"][0]["definition"]["url"] = "file:///workspace/mcp".into();
    let invalid = fx.registry.publish(CatalogKind::Mcp, malformed);
    fx.select_mcp_digest(&invalid);
    assert_eq!(get("/mcps/list".into()).await.0, StatusCode::BAD_GATEWAY);
    assert_eq!(overview().await["error"], "verification_failed");
    let revoked = fx.registry.publish(CatalogKind::Mcp, document(CatalogKind::Mcp, true));
    fx.select_mcp_digest(&revoked);
    let (status, listings) = get("/mcps/list".into()).await;
    assert_eq!(status, StatusCode::OK, "{listings}");
    assert_eq!(listings["mcps"], json!([]));
    assert_eq!(get(info("review", snapshot)).await.0, StatusCode::CONFLICT);
    fx.select_mcp_digest(&digest);
    let (status, restored) = get("/mcps/list".into()).await;
    assert_eq!(status, StatusCode::OK, "{restored}");
    assert_eq!(restored["mcps"].as_array().unwrap().len(), 1);
    assert_eq!(restored["mcps"][0]["metadata"]["id"], "review-local");
    assert_eq!(get(info("review", snapshot)).await.0, StatusCode::FORBIDDEN);
}
