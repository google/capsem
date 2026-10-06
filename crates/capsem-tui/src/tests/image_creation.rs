use super::*;

#[test]
fn create_dialog_offers_an_explicit_image_choice() {
    let mut app = App::new(fixture_state());
    app.handle_key(key(KeyCode::Char('n'), KeyModifiers::ALT));
    let snapshot = render_app_snapshot(&app, 100, 30).unwrap();
    assert!(snapshot.contains("image"));
    assert!(snapshot.contains("Plain VM"));
    assert!(snapshot.contains("loading image catalog"));
}

fn catalog() -> capsem_sdk::models::ImageListResponse {
    use capsem_sdk::models::{ImageCacheState, ImageInfo, ImageListResponse};
    ImageListResponse {
        catalog: None,
        images: vec![
            ImageInfo {
                name: "code".into(),
                description: "Development tools".into(),
                architectures: vec!["amd64".into()],
                image: Some(format!("registry.example/code@sha256:{}", "a".repeat(64))),
                cached: ImageCacheState::Unknown,
            },
            ImageInfo {
                name: "other".into(),
                description: "Other architecture".into(),
                architectures: vec!["arm64".into()],
                image: None,
                cached: ImageCacheState::Unknown,
            },
        ],
    }
}

#[test]
fn image_selection_blocks_incompatible_entries_and_preserves_failed_drafts() {
    let mut app = App::new(fixture_state());
    app.handle_key(key(KeyCode::Char('n'), KeyModifiers::ALT));
    let generation = app.take_catalog_request().unwrap();
    assert!(app.take_catalog_request().is_none());
    app.apply_image_catalog(generation, Ok(catalog()));
    app.handle_key(key(KeyCode::Tab, KeyModifiers::NONE));
    app.handle_key(key(KeyCode::Down, KeyModifiers::NONE));
    let snapshot = render_app_snapshot(&app, 100, 30).unwrap();
    for text in ["code", "compatible", "cache", "unknown", "Development tools"] {
        assert!(snapshot.contains(text), "{text}");
    }
    app.handle_key(key(KeyCode::Down, KeyModifiers::NONE));
    let snapshot = render_app_snapshot(&app, 100, 30).unwrap();
    assert!(snapshot.contains("incompatible architecture/runtime"));
    assert_eq!(
        app.handle_key(key(KeyCode::Enter, KeyModifiers::NONE)),
        AppAction::Consumed
    );
    assert_eq!(app.overlay(), AppOverlay::Create);
    app.handle_key(key(KeyCode::Up, KeyModifiers::NONE));
    assert_eq!(
        app.handle_key(key(KeyCode::Enter, KeyModifiers::NONE)),
        AppAction::Invoke(ControlAction::CreateSession {
            name: None,
            image: catalog().images[0].image.clone()
        })
    );
    app.set_control_message("create refused by policy");
    app.complete_create(false);
    assert_eq!(app.overlay(), AppOverlay::Create);
    assert_eq!(app.create_draft().unwrap().selected_image().unwrap().name, "code");
    let snapshot = render_app_snapshot(&app, 100, 30).unwrap();
    assert!(snapshot.contains("create refused by policy"));
}

#[test]
fn catalog_failures_and_late_results_never_replace_a_new_dialog() {
    let mut app = App::new(fixture_state());
    app.handle_key(key(KeyCode::Char('n'), KeyModifiers::ALT));
    let original = app.take_catalog_request().unwrap();
    app.handle_key(key(KeyCode::Esc, KeyModifiers::NONE));
    app.handle_key(key(KeyCode::Char('n'), KeyModifiers::ALT));
    let current = app.take_catalog_request().unwrap();
    assert_ne!(original, current);
    app.apply_image_catalog(original, Ok(catalog()));
    assert!(matches!(
        app.create_draft().unwrap().catalog,
        crate::app::ImageCatalog::Loading
    ));
    app.apply_image_catalog(current, Err("catalog denied".into()));
    let snapshot = render_app_snapshot(&app, 100, 30).unwrap();
    assert!(snapshot.contains("catalog denied"));
    assert_eq!(app.state().service.status, ServiceStatus::Online);
    assert_eq!(
        app.handle_key(key(KeyCode::Enter, KeyModifiers::NONE)),
        AppAction::Invoke(ControlAction::CreateSession {
            name: None,
            image: None
        })
    );
    app.complete_create(true);
    assert!(app.create_draft().is_none());
}

#[test]
fn pending_create_keeps_its_catalog_result_and_prevents_duplicate_dialogs() {
    let mut app = App::new(fixture_state());
    app.handle_key(key(KeyCode::Char('n'), KeyModifiers::ALT));
    let generation = app.take_catalog_request().unwrap();
    assert!(matches!(
        app.handle_key(key(KeyCode::Enter, KeyModifiers::NONE)),
        AppAction::Invoke(_)
    ));
    app.handle_key(key(KeyCode::Char('n'), KeyModifiers::ALT));
    assert!(app.create_draft().is_none(), "only one outstanding create intent");
    app.apply_image_catalog(generation, Ok(catalog()));
    app.complete_create(false);
    assert!(
        matches!(app.create_draft().unwrap().catalog, crate::app::ImageCatalog::Loaded(_)),
        "the failed draft retains its late catalog result"
    );
}

#[tokio::test]
async fn catalog_and_oci_create_use_authenticated_typed_gateway_calls() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let pin = catalog().images[0].image.clone().unwrap();
    let expected = pin.clone();
    let server = tokio::spawn(async move {
        for path in ["/token", "/status", "/images?refresh=false", "/vms/create"] {
            let (mut stream, _) = listener.accept().await.unwrap();
            let request = read_http_request(&mut stream).await;
            let method = if path == "/vms/create" { "POST" } else { "GET" };
            assert!(request.starts_with(&format!("{method} {path} ")), "{request}");
            if path != "/token" {
                assert!(request
                    .to_ascii_lowercase()
                    .contains("authorization: bearer test-token"));
            }
            match path {
                "/token" => write_json_response(&mut stream, r#"{"token":"test-token"}"#).await,
                "/status" => write_json_response(&mut stream, gateway_status_body()).await,
                "/images?refresh=false" => {
                    write_json_response(&mut stream, &serde_json::to_string(&catalog()).unwrap()).await
                }
                _ => {
                    let body: serde_json::Value =
                        serde_json::from_str(request.split_once("\r\n\r\n").unwrap().1).unwrap();
                    assert_eq!(
                        body,
                        serde_json::json!({"name": null, "persistent": true, "container": {"image": expected, "env": {}, "attach": false}})
                    );
                    write_json_response(
                        &mut stream,
                        r#"{"id":"created-id","name":"vm-3","status":"Running","available_actions":[]}"#,
                    )
                    .await;
                }
            }
        }
    });
    let provider = GatewayProvider::new(url);
    let mut app = App::new(provider.load_async().await.unwrap());
    app.handle_key(key(KeyCode::Char('n'), KeyModifiers::ALT));
    let generation = app.take_catalog_request().unwrap();
    app.apply_image_catalog(generation, Ok(provider.list_images_async().await.unwrap()));
    app.handle_key(key(KeyCode::Tab, KeyModifiers::NONE));
    app.handle_key(key(KeyCode::Down, KeyModifiers::NONE));
    let AppAction::Invoke(action) = app.handle_key(key(KeyCode::Enter, KeyModifiers::NONE)) else {
        panic!("create intent")
    };
    assert_eq!(
        action,
        ControlAction::CreateSession {
            name: None,
            image: Some(pin)
        }
    );
    let outcome = provider.invoke_async(&action).await.unwrap();
    assert_eq!(outcome.focus_session.as_deref(), Some("created-id"));
    server.await.unwrap();
}

#[tokio::test]
async fn catalog_errors_are_bounded_and_redact_gateway_credentials() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        for path in ["/token", "/images?refresh=false"] {
            let (mut stream, _) = listener.accept().await.unwrap();
            let request = read_http_request(&mut stream).await;
            assert!(request.starts_with(&format!("GET {path} ")));
            if path == "/token" {
                write_json_response(&mut stream, r#"{"token":"test-token"}"#).await;
            } else {
                write_response(
                    &mut stream,
                    "403 Forbidden",
                    &format!("catalog refused test-token\u{1b}\n{}", "x".repeat(1000)),
                )
                .await;
            }
        }
    });
    let error = GatewayProvider::new(url)
        .list_images_async()
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("403") && error.contains("catalog refused"), "{error}");
    assert!(!error.contains("test-token"));
    assert!(error.chars().count() <= 512);
    assert!(!error.chars().any(char::is_control));
    server.await.unwrap();
}
