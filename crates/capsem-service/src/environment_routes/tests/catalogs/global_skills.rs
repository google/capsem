use super::*;

#[tokio::test]
async fn global_skills_bind_pages_and_complete_files_to_an_exact_catalog_snapshot() {
    let fx = Fixture::new().await;
    let kind = CatalogKind::Skills;
    let mut original = document(kind, false);
    let support = "Fixture instructions: café.\n";
    original["entries"][0]["payload"]["files"]["references/fixture.md"] =
        json!({"content":support,"sha256":registry::digest(support.as_bytes())});
    let mut second = original["entries"][0].clone();
    second["metadata"]["version"] = "2.0.0+preview".into();
    second["metadata"]["name"] = "Second review".into();
    let mut incompatible = original["entries"][0].clone();
    incompatible["metadata"]["version"] = "3.0.0".into();
    incompatible["metadata"]["compatibility"]["min_runtime_contract"] = 999.into();
    let entries = original["entries"].as_array_mut().unwrap();
    entries.insert(0, second);
    entries.push(incompatible);
    let digest = fx.registry.publish(kind, original.clone());
    let app = crate::router_runtime::build_service_router(fx.state.clone());
    let get = |uri: String| {
        let app = app.clone();
        async move { route_request(app, Method::GET, &uri, None).await }
    };
    let (status, first) = get("/skills/list?search=REVIEW&limit=1".into()).await;
    assert_eq!(status, StatusCode::OK, "{first}");
    assert_eq!(first["catalog"]["reference"], fx.registry.reference(kind));
    assert_eq!(first["catalog"]["digest"], digest);
    assert_eq!(first["skills"][0]["version"], "1.0.0");
    assert!(first["skills"][0].get("files").is_none());
    let snapshot = first["catalog"]["snapshot"].as_str().unwrap();
    let after = first["next_cursor"].as_str().unwrap();
    let next = format!("/skills/list?search=REVIEW&limit=1&after={after}");
    let info = format!("/skills/review/info?version=1.0.0&snapshot={snapshot}");
    let (status, second) = get(next.clone()).await;
    assert_eq!(status, StatusCode::OK, "{second}");
    assert_eq!(second["skills"][0]["version"], "2.0.0+preview");
    assert!(second["next_cursor"].is_null());
    let (status, detail) = get(info.clone()).await;
    assert_eq!(status, StatusCode::OK, "{detail}");
    assert_eq!(detail["metadata"], first["skills"][0]);
    assert_eq!(detail["selection"]["resolved_digest"], digest);
    assert_eq!(
        detail["files"],
        json!({
            "SKILL.md":original["entries"][1]["payload"]["files"]["SKILL.md"]["content"],
            "references/fixture.md":support
        })
    );
    let absent = format!("/skills/review/info?version=0.0.1&snapshot={snapshot}");
    let incompatible = format!("/skills/review/info?version=3.0.0&snapshot={snapshot}");
    assert_eq!(get(absent).await.0, StatusCode::NOT_FOUND);
    assert_eq!(get(incompatible).await.0, StatusCode::FORBIDDEN);
    assert_eq!(
        get("/skills/list?after=bad.cursor".into()).await.0,
        StatusCode::BAD_REQUEST
    );

    let revoked = fx.registry.publish(kind, document(kind, true));
    assert_ne!(revoked, digest);
    assert_eq!(get(next).await.0, StatusCode::CONFLICT);
    assert_eq!(get(info.clone()).await.0, StatusCode::CONFLICT);

    assert_eq!(fx.registry.publish(kind, original), digest);
    let (status, rolled_back) = get("/skills/list".into()).await;
    assert_eq!(status, StatusCode::OK, "{rolled_back}");
    let skills = rolled_back["skills"].as_array().unwrap();
    assert_eq!(skills.len(), 1);
    assert_eq!(skills[0]["version"], "2.0.0+preview");
    assert_eq!(get(info).await.0, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn global_skills_follow_the_corporate_source_and_never_fall_back() {
    let fx = Fixture::new().await;
    let kind = CatalogKind::Skills;
    let digest = fx.registry.publish(kind, document(kind, false));
    let app = crate::router_runtime::build_service_router(fx.state.clone());
    let get = |uri: String| {
        let app = app.clone();
        async move { route_request(app, Method::GET, &uri, None).await }
    };
    let (status, public) = get("/skills/list".into()).await;
    assert_eq!(status, StatusCode::OK, "{public}");
    let public_snapshot = public["catalog"]["snapshot"].as_str().unwrap();

    fx.registry.alias("team/skills/", "team/private/");
    let corporate_reference = format!("{}/team/private:stable", fx.registry.address);
    fx.corporate.lock().unwrap().catalogs = Some(CatalogSources {
        skills: Some(descriptor(corporate_reference.clone())),
        ..Default::default()
    });
    fx.registry.requests.lock().unwrap().clear();
    let (status, corporate) = get("/skills/list".into()).await;
    assert_eq!(status, StatusCode::OK, "{corporate}");
    assert_eq!(corporate["catalog"]["reference"], corporate_reference);
    assert_eq!(corporate["catalog"]["digest"], digest);
    assert_eq!(
        get(format!("/skills/review/info?version=1.0.0&snapshot={public_snapshot}"))
            .await
            .0,
        StatusCode::CONFLICT
    );

    let broken_reference = format!("{}/team/missing:stable", fx.registry.address);
    fx.corporate.lock().unwrap().catalogs.as_mut().unwrap().skills = Some(descriptor(broken_reference));
    assert_eq!(get("/skills/list".into()).await.0, StatusCode::SERVICE_UNAVAILABLE);
    assert!(fx
        .registry
        .requests
        .lock()
        .unwrap()
        .iter()
        .all(|path| !path.starts_with("team/skills/")));
}

#[tokio::test]
async fn catalog_review_precondition_refuses_changed_bytes_and_reconciles_the_same_intent() {
    let fx = Fixture::new().await;
    let kind = CatalogKind::Skills;
    let original = document(kind, false);
    let digest = fx.registry.publish(kind, original.clone());
    let app = crate::router_runtime::build_service_router(fx.state.clone());
    let intent = json!({"request_id": id(389), "reason": "review exact skill", "specification": {
        "name": "exact-catalog-review", "skills": {"selected": [{
            "id": "review", "version": "1.0.0", "expected_catalog": {
                "reference": fx.registry.reference(kind), "digest": digest
            }
        }]}
    }});
    let mut changed = original.clone();
    changed["entries"][0]["metadata"]["name"] = "Replaced review".into();
    assert_ne!(fx.registry.publish(kind, changed), digest);
    let (status, body) = route_request(app.clone(), Method::POST, "/environments/create", Some(intent.clone())).await;
    assert_eq!(status, StatusCode::PRECONDITION_FAILED, "{body}");
    assert!(body["error"].as_str().unwrap().contains("catalog selection changed"));
    assert!(fx
        .state
        .host_ledger
        .environment_request_snapshot(&id(389))
        .await
        .unwrap()
        .is_none());

    assert_eq!(fx.registry.publish(kind, original), digest);
    let (status, saved) = route_request(app.clone(), Method::POST, "/environments/create", Some(intent.clone())).await;
    assert_eq!(status, StatusCode::OK, "{saved}");
    let result = &saved["result"];
    let route = format!("/environments/{}/info", result["environment_id"].as_str().unwrap());
    let (_, current) = route_request(app.clone(), Method::GET, &route, None).await;
    let pin = &current["document"]["specification"]["skills"]["selected"][0];
    assert_eq!(pin["resolved_digest"], digest);
    assert!(pin.get("expected_catalog").is_none());
    let calls = fx.registry.requests.lock().unwrap().len();
    let (status, replay) = route_request(app.clone(), Method::POST, "/environments/create", Some(intent.clone())).await;
    assert_eq!(status, StatusCode::OK, "{replay}");
    assert_eq!(saved, replay);
    assert_eq!(fx.registry.requests.lock().unwrap().len(), calls);
    let mut different = intent;
    different["specification"]["skills"]["selected"][0]["expected_catalog"]["digest"] =
        format!("sha256:{}", "0".repeat(64)).into();
    assert_eq!(
        route_request(app, Method::POST, "/environments/create", Some(different))
            .await
            .0,
        StatusCode::CONFLICT
    );
}
