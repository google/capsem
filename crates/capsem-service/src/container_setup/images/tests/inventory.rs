use super::*;

#[tokio::test]
async fn inventory_detail_requires_a_catalog_name_and_the_exact_selected_pin() {
    let fx = Fixture::default_policy();
    let (_, list) = fx.list().await;
    let image = list["images"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["name"] == "codex-cli")
        .unwrap()["image"]
        .as_str()
        .unwrap();
    let uri = format!("/images/codex-cli/inventory?image={image}");
    let (status, body) = fx.call(axum::http::Method::GET, &uri, None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["image"], image);
    assert!(body.get("inventory").is_none());
    assert!(fx.pulls().is_empty(), "opening Info never prepares application layers");
    let (status, _) = fx
        .call(
            axum::http::Method::GET,
            &uri.replace(image, "registry.example/other@sha256:bad"),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT);
    let (status, _) = fx
        .call(
            axum::http::Method::GET,
            &uri.replace("/codex-cli/", "/not-listed/"),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn declared_but_broken_inventory_is_an_error_instead_of_absent() {
    let fx = Fixture::default_policy();
    fx.catalog.lock().unwrap().as_mut().unwrap()["entries"]["codex-cli"]["versions"][1]["evidence"] = json!({host():[{"kind":"package_inventory","reference":format!("ghcr.io/google/capsem/codex-cli@{}",digest('5')),
            "media_type":"application/vnd.capsem.package-inventory.v1+json"}]});
    let (_, list) = fx.list().await;
    let image = list["images"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["name"] == "codex-cli")
        .unwrap()["image"]
        .as_str()
        .unwrap();
    let (status, body) = fx
        .call(
            axum::http::Method::GET,
            &format!("/images/codex-cli/inventory?image={image}"),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::BAD_GATEWAY, "{body}");
    assert!(fx.pulls().is_empty());
}

#[tokio::test]
async fn inventory_routes_only_the_selected_platform_evidence_and_returns_typed_rows() {
    for foreign in [false, true] {
        let fx = Fixture::default_policy();
        let reference = format!(
            "{}/codex-cli@{}",
            if foreign {
                "ghcr.io/foreign"
            } else {
                "ghcr.io/google/capsem"
            },
            digest('6')
        );
        fx.catalog.lock().unwrap().as_mut().unwrap()["entries"]["codex-cli"]["versions"][1]["evidence"] = json!({host():[{"kind":"package_inventory","reference":reference,"media_type":"application/vnd.capsem.package-inventory.v1+json"}]});
        let image = format!("ghcr.io/google/capsem/codex-cli@{}", digest('b'));
        let (status, body) = fx
            .call(
                axum::http::Method::GET,
                &format!("/images/codex-cli/inventory?image={image}"),
                None,
            )
            .await;
        assert_eq!(
            status,
            if foreign { StatusCode::FORBIDDEN } else { StatusCode::OK },
            "{body}"
        );
        if !foreign {
            assert_eq!(body["artifact_reference"], reference);
            assert_eq!(body["inventory"]["packages"][0]["name"], "git");
            assert_eq!(body["inventory"]["tools"][0]["name"], "gh");
            assert_eq!(body["inventory"]["architecture"], host());
        }
        assert!(fx.pulls().is_empty());
    }
}
