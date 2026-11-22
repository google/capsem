use super::*;
use crate::environment_routes::launch_policy;

#[tokio::test]
async fn environment_mcp_launch_rechecks_the_exact_catalog_and_revocations() {
    let fx = Fixture::new().await;
    let digest = fx.registry.publish(CatalogKind::Mcp, document(CatalogKind::Mcp, false));
    fx.select_mcp_digest(&digest);
    let app = crate::router_runtime::build_service_router(fx.state.clone());
    let (status, response) = route_request(
        app.clone(),
        Method::POST,
        "/environments/create",
        Some(json!({
            "request_id":id(614),"reason":"approved remote MCP","specification":{"name":"catalog-runtime","mcps":{
                "catalog":{"selected":[{"id":"review","version":"1.0.0"}]}
            }}
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{response}");
    let id = response["result"]["environment_id"].as_str().unwrap();
    let saved = fx
        .state
        .environments
        .load_current(id)
        .await
        .unwrap()
        .document
        .specification;
    let corporate = SettingsFile::default();
    let compile = async |specification: &capsem_config::environment::EnvironmentSpecification| {
        let inputs = capsem_core::environment::EnvironmentPolicyInputs::new(specification, specification, &corporate);
        launch_policy(&fx.state, &inputs).await
    };
    let policy = compile(&saved).await.unwrap().compile_runtime().unwrap();
    assert_eq!(policy.mcp.servers[0].url, "https://mcp.example.invalid/service");
    assert!(policy.scoped_mcp_servers.contains("review"));
    let mut altered = saved.clone();
    let configuration = altered.mcps.catalog_configuration.as_mut().unwrap();
    let capsem_config::environment::EnvironmentMcpDefinition::Remote { url, .. } =
        configuration.get_mut("review").unwrap()
    else {
        panic!("remote template changed kind")
    };
    *url = "https://unapproved.invalid/mcp".into();
    assert_eq!(
        compile(&altered).await.err(),
        Some(EnvironmentResolutionError::InvalidPin)
    );
    let revoked = fx.registry.publish(CatalogKind::Mcp, document(CatalogKind::Mcp, true));
    fx.select_mcp_digest(&revoked);
    assert_eq!(compile(&saved).await.err(), Some(EnvironmentResolutionError::Refused));
    assert_eq!(
        route_request(app, Method::GET, "/mcps/list", None).await.0,
        StatusCode::OK
    );
    fx.select_mcp_digest(&digest);
    assert_eq!(
        compile(&saved).await.err(),
        Some(EnvironmentResolutionError::UnusableCatalogEntry)
    );
}

#[tokio::test]
async fn saved_mcp_catalog_configuration_records_the_verified_launch_definition() {
    let fx = Fixture::new().await;
    let mut catalog = document(CatalogKind::Mcp, false);
    catalog["entries"][0]["definition"]["headers"] =
        json!({"X-Workspace":{"description":"Workspace identity","required":true}});
    catalog["entries"][0]["definition"]["secret_binding"] =
        json!({"description":"Workspace credential","required":true});
    let digest = fx.registry.publish(CatalogKind::Mcp, catalog);
    let reference = fx.select_mcp_digest(&digest);
    let app = crate::router_runtime::build_service_router(fx.state.clone());
    let request = json!({"request_id":id(610),"reason":"approved MCP catalog template","specification":{
        "name":"catalog-mcp","mcps":{
            "catalog":{"selected":[{"id":"review","version":"1.0.0",
                "expected_catalog":{"reference":reference,"digest":digest}}]},
            "catalog_configuration":{"review":{"values":{"X-Workspace":"example"},"binding_id":id(611)}}
        }
    }});
    let create = |body: serde_json::Value| {
        let app = app.clone();
        async move { route_request(app, Method::POST, "/environments/create", Some(body)).await }
    };
    for (field, value, expected) in [
        (
            "url",
            json!("https://unreviewed.invalid"),
            StatusCode::UNPROCESSABLE_ENTITY,
        ),
        ("binding_id", serde_json::Value::Null, StatusCode::BAD_REQUEST),
        ("values", json!({"X-Undeclared":"example"}), StatusCode::BAD_REQUEST),
    ] {
        let mut invalid = request.clone();
        invalid["specification"]["mcps"]["catalog_configuration"]["review"][field] = value;
        assert_eq!(create(invalid).await.0, expected);
    }
    let mut stale = request.clone();
    stale["specification"]["mcps"]["catalog"]["selected"][0]["expected_catalog"]["digest"] =
        format!("sha256:{}", "a".repeat(64)).into();
    assert_eq!(create(stale).await.0, StatusCode::PRECONDITION_FAILED);
    let review_path = format!("/requests/{}/info", id(610));
    assert_eq!(
        route_request(app.clone(), Method::GET, &review_path, None).await.0,
        StatusCode::NOT_FOUND
    );
    let (status, response) = create(request.clone()).await;
    assert_eq!(status, StatusCode::OK, "{response}");
    let revision: capsem_api::environments::EnvironmentRevisionReference =
        serde_json::from_value(response["result"].clone()).unwrap();
    let environment = &revision.environment_id;
    let current_path = format!("/environments/{environment}/info");
    let (status, current) = route_request(app.clone(), Method::GET, &current_path, None).await;
    assert_eq!(status, StatusCode::OK, "{current}");
    let config = &current["document"]["specification"]["mcps"]["catalog_configuration"]["review"];
    assert_eq!(config["kind"], "remote");
    assert_eq!(config["url"], "https://mcp.example.invalid/service");
    assert_eq!(config["headers"]["X-Workspace"], "example");
    assert_eq!(config["binding_id"], id(611));
    let (status, review) = route_request(app.clone(), Method::GET, &review_path, None).await;
    assert_eq!(status, StatusCode::OK, "{review}");
    assert_eq!(
        review["proposed"]["specification"]["mcps"]["catalog_configuration"]["review"],
        *config
    );
    let refusal = crate::environment_launch::validate_selection(&fx.state, &revision, false)
        .await
        .unwrap_err();
    assert_eq!(refusal.status, StatusCode::NOT_IMPLEMENTED);
    assert!(refusal
        .body
        .error
        .contains("credential binding requires scoped admission"));

    let requests = fx.registry.requests.lock().unwrap().len();
    assert_eq!(create(request.clone()).await, (StatusCode::OK, response));
    assert_eq!(fx.registry.requests.lock().unwrap().len(), requests);
    let canonical: capsem_config::environment::EnvironmentDocument =
        serde_json::from_value(current["document"].clone()).unwrap();
    let canonical_toml = canonical.to_toml().unwrap();
    assert!(canonical
        .draft_from_toml(&canonical_toml.replace("https://mcp.example.invalid/service", "https://unreviewed.invalid"))
        .is_err());
    let toml = canonical_toml.replace("X-Workspace = \"example\"", "X-Workspace = \"updated\"");
    let draft = canonical.draft_from_toml(&toml).unwrap();
    let changes = format!("/environments/{environment}/changes");
    let edit = |n| {
        json!({"request_id":id(n),"reason":"reconfigure saved entry",
        "base_revision_id":revision.revision_id,"specification":draft})
    };
    let (status, changed) = route_request(app.clone(), Method::POST, &changes, Some(edit(612))).await;
    assert_eq!(status, StatusCode::OK, "{changed}");
    assert_eq!(fx.registry.requests.lock().unwrap().len(), requests);
    let changed_revision = changed["result"]["revision_id"].as_str().unwrap();
    let revoked = fx.registry.publish(CatalogKind::Mcp, document(CatalogKind::Mcp, true));
    fx.select_mcp_digest(&revoked);
    assert_eq!(
        route_request(app.clone(), Method::GET, "/mcps/list", None).await.0,
        StatusCode::OK
    );
    fx.select_mcp_digest(&digest);
    let mut revoked_edit = edit(613);
    revoked_edit["base_revision_id"] = changed_revision.into();
    assert_eq!(
        route_request(app.clone(), Method::POST, &changes, Some(revoked_edit))
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    let (_, retained) = route_request(app, Method::GET, &current_path, None).await;
    assert_eq!(retained["document"]["identity"]["revision_id"], changed_revision);
    assert_eq!(
        retained["document"]["specification"]["mcps"]["catalog_configuration"]["review"]["headers"]["X-Workspace"],
        "updated"
    );
}

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
