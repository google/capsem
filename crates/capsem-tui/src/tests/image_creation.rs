use super::*;

#[test]
fn image_picker_excludes_graphical_and_unknown_surfaces_and_refuses_default_create() {
    let mut app = App::new(fixture_state());
    app.handle_key(key(KeyCode::Char('n'), KeyModifiers::ALT));
    assert_eq!(
        app.handle_key(key(KeyCode::Enter, KeyModifiers::NONE)),
        AppAction::Consumed
    );
    let generation = app.take_catalog_request().unwrap();
    let mut response = catalog();
    response.images[0].surface = Some(capsem_sdk::models::ImageSurface::Xpra);
    response.images[1].surface = None;
    app.apply_image_catalog(generation, Ok(response));
    assert!(
        matches!(&app.create_draft().unwrap().catalog, crate::app::ImageCatalog::Loaded(images) if images.is_empty())
    );
    assert_eq!(
        app.handle_key(key(KeyCode::Enter, KeyModifiers::NONE)),
        AppAction::Consumed
    );
}

#[test]
fn periodic_state_refresh_follows_pending_image_verification() {
    let mut app = App::new(fixture_state());
    app.handle_key(key(KeyCode::Char('n'), KeyModifiers::ALT));
    let generation = app.take_catalog_request().unwrap();
    assert!(generation.refresh);
    app.apply_image_catalog(generation, Ok(catalog()));
    app.handle_key(key(KeyCode::Tab, KeyModifiers::NONE));
    app.handle_key(key(KeyCode::Down, KeyModifiers::NONE));
    let name = app.create_draft().unwrap().name.clone();
    app.replace_state(app.state().clone());
    let poll = app
        .take_catalog_request()
        .expect("follow the cache owner's pending verification");
    assert_eq!(poll.generation, generation.generation);
    assert!(
        !poll.refresh,
        "observing local cache verification must not refetch the registry catalog"
    );
    let mut ready = catalog();
    ready.images[0].cached = capsem_sdk::models::ImageCacheState::Ready;
    app.apply_image_catalog(poll, Ok(ready));
    assert_eq!(app.create_draft().unwrap().name, name);
    assert_eq!(app.create_draft().unwrap().selected_image().unwrap().name, "code");
    app.replace_state(app.state().clone());
    assert!(
        app.take_catalog_request().is_none(),
        "incompatible unknown entries must not cause endless polling"
    );
    app.handle_key(key(KeyCode::Char('r'), KeyModifiers::CONTROL));
    let refresh = app.take_catalog_request().unwrap();
    assert!(refresh.refresh);
    assert_ne!(refresh.generation, poll.generation);
    app.handle_key(key(KeyCode::Esc, KeyModifiers::NONE));
    app.replace_state(app.state().clone());
    assert!(
        app.take_catalog_request().is_none(),
        "closing the chooser stops polling"
    );
}

#[test]
fn active_cache_and_transfer_progress_keep_the_existing_draft_observed() {
    use capsem_sdk::models::{ImageCacheState, ImageDownloadPhase, ImageDownloadProgress};
    let mut app = App::new(fixture_state());
    app.handle_key(key(KeyCode::Char('n'), KeyModifiers::ALT));
    let generation = app.take_catalog_request().unwrap();
    let mut response = catalog();
    response.images[0].cached = ImageCacheState::Missing;
    app.apply_image_catalog(generation, Ok(response.clone()));
    app.handle_key(key(KeyCode::Tab, KeyModifiers::NONE));
    app.handle_key(key(KeyCode::Down, KeyModifiers::NONE));
    app.replace_state(app.state().clone());
    assert!(
        app.take_catalog_request().is_none(),
        "resolved missing entries do not need a poll"
    );
    app.handle_key(key(KeyCode::Char('p'), KeyModifiers::CONTROL));
    app.replace_state(app.state().clone());
    let pending = app
        .take_catalog_request()
        .expect("observe an active pull before its progress arrives");
    assert!(!pending.refresh);
    app.complete_image_cache(false);
    response.images[0].progress = Some(ImageDownloadProgress {
        phase: ImageDownloadPhase::Downloading,
        completed_bytes: 1,
        total_bytes: Some(2),
    });
    app.apply_image_catalog(pending, Ok(response));
    app.replace_state(app.state().clone());
    assert!(
        !app.take_catalog_request()
            .expect("follow another client's active transfer")
            .refresh
    );
    let mut state = app.state().clone();
    state.service.status = ServiceStatus::Offline;
    app.replace_state(state);
    assert!(
        app.take_catalog_request().is_none(),
        "an offline service is handled by its own reconnect flow"
    );
}

#[test]
fn periodic_catalog_observation_preserves_explicit_refresh_and_retry_requests() {
    let mut app = App::new(fixture_state());
    app.handle_key(key(KeyCode::Char('n'), KeyModifiers::ALT));
    let first = app.take_catalog_request().unwrap();
    app.apply_image_catalog(first, Ok(catalog()));
    app.handle_key(key(KeyCode::Char('r'), KeyModifiers::CONTROL));
    app.replace_state(app.state().clone());
    let manual = app.take_catalog_request().unwrap();
    assert!(
        manual.refresh,
        "periodic observation cannot replace a queued explicit refresh"
    );
    app.retry_catalog_request(manual);
    assert_eq!(app.take_catalog_request(), Some(manual));
    app.apply_image_catalog(manual, Err("registry unavailable".into()));
    app.replace_state(app.state().clone());
    assert!(
        app.take_catalog_request().is_none(),
        "failed registry reads retain the explicit retry action"
    );
}

#[test]
fn selected_image_can_be_cached_without_creating_a_session() {
    let mut app = App::new(fixture_state());
    app.handle_key(key(KeyCode::Char('n'), KeyModifiers::ALT));
    let generation = app.take_catalog_request().unwrap();
    app.apply_image_catalog(generation, Ok(catalog()));
    app.handle_key(key(KeyCode::Tab, KeyModifiers::NONE));
    app.handle_key(key(KeyCode::Down, KeyModifiers::NONE));
    assert_eq!(
        app.handle_key(key(KeyCode::Char('p'), KeyModifiers::CONTROL)),
        AppAction::Invoke(ControlAction::CacheImage {
            image: catalog().images[0].image.clone().unwrap(),
            name: "code".into(),
        })
    );
    assert_eq!(app.overlay(), AppOverlay::Create);
    assert_eq!(
        app.handle_key(key(KeyCode::Enter, KeyModifiers::NONE)),
        AppAction::Consumed
    );
    app.complete_image_cache(false);
    assert_eq!(app.create_draft().unwrap().selected_image().unwrap().name, "code");
    assert!(matches!(
        app.handle_key(key(KeyCode::Enter, KeyModifiers::NONE)),
        AppAction::Invoke(ControlAction::CreateSession { image: Some(_), .. })
    ));
}

#[test]
fn catalog_retry_preserves_the_choice_and_never_silently_creates_a_plain_vm() {
    let mut app = App::new(fixture_state());
    app.handle_key(key(KeyCode::Char('n'), KeyModifiers::ALT));
    let generation = app.take_catalog_request().unwrap();
    app.apply_image_catalog(generation, Ok(catalog()));
    app.handle_key(key(KeyCode::Tab, KeyModifiers::NONE));
    app.handle_key(key(KeyCode::Down, KeyModifiers::NONE));
    app.handle_key(key(KeyCode::Char('r'), KeyModifiers::CONTROL));
    let refreshed = app.take_catalog_request().expect("retry request");
    assert_ne!(generation, refreshed);
    app.apply_image_catalog(generation, Ok(catalog()));
    assert!(matches!(
        app.create_draft().unwrap().catalog,
        crate::app::ImageCatalog::Loading
    ));
    let mut empty = catalog();
    empty.images.clear();
    app.apply_image_catalog(refreshed, Ok(empty));
    assert_eq!(
        app.handle_key(key(KeyCode::Enter, KeyModifiers::NONE)),
        AppAction::Consumed
    );
}

#[test]
fn successful_cache_refreshes_readiness_without_losing_the_selected_image() {
    let mut app = App::new(fixture_state());
    app.handle_key(key(KeyCode::Char('n'), KeyModifiers::ALT));
    let generation = app.take_catalog_request().unwrap();
    app.apply_image_catalog(generation, Ok(catalog()));
    app.handle_key(key(KeyCode::Tab, KeyModifiers::NONE));
    app.handle_key(key(KeyCode::Down, KeyModifiers::NONE));
    app.handle_key(key(KeyCode::Char('p'), KeyModifiers::CONTROL));
    app.complete_image_cache(true);
    let fresh = app.take_catalog_request().unwrap();
    let mut response = catalog();
    response.images[0].cached = capsem_sdk::models::ImageCacheState::Ready;
    response.images.reverse();
    app.apply_image_catalog(fresh, Ok(response));
    let selected = app.create_draft().unwrap().selected_image().unwrap();
    assert_eq!(selected.name, "code");
    assert_eq!(selected.cached, capsem_sdk::models::ImageCacheState::Ready);
}

#[test]
fn create_dialog_offers_an_explicit_image_choice() {
    let mut app = App::new(fixture_state());
    app.handle_key(key(KeyCode::Char('n'), KeyModifiers::ALT));
    let snapshot = render_app_snapshot(&app, 100, 30).unwrap();
    assert!(snapshot.contains("image"));
    assert!(!snapshot.contains("Plain VM"));
    assert!(snapshot.contains("loading image catalog"));
}

pub(super) fn catalog() -> capsem_sdk::models::ImageListResponse {
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
                previous_image: None,
                app: None,
                surface: Some(capsem_sdk::models::ImageSurface::Terminal),
                progress: None,
            },
            ImageInfo {
                name: "other".into(),
                description: "Other architecture".into(),
                architectures: vec!["arm64".into()],
                image: None,
                cached: ImageCacheState::Unknown,
                previous_image: None,
                app: None,
                surface: Some(capsem_sdk::models::ImageSurface::Terminal),
                progress: None,
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
        AppAction::Consumed
    );
    assert!(app.create_draft().is_some());
}

#[test]
fn pending_create_keeps_its_catalog_result_and_prevents_duplicate_dialogs() {
    let mut app = App::new(fixture_state());
    app.handle_key(key(KeyCode::Char('n'), KeyModifiers::ALT));
    let generation = app.take_catalog_request().unwrap();
    app.apply_image_catalog(generation, Ok(catalog()));
    app.handle_key(key(KeyCode::Tab, KeyModifiers::NONE));
    app.handle_key(key(KeyCode::Down, KeyModifiers::NONE));
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
        for path in ["/status", "/images?refresh=false", "/vms/create"] {
            let (mut stream, _) = listener.accept().await.unwrap();
            let request = read_http_request(&mut stream).await;
            let method = if path == "/vms/create" { "POST" } else { "GET" };
            assert!(request.starts_with(&format!("{method} {path} ")), "{request}");
            assert!(request
                .to_ascii_lowercase()
                .contains("authorization: bearer test-token"));
            match path {
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
    let provider = GatewayProvider::with_test_token(url, "test-token");
    let mut app = App::new(provider.load_async().await.unwrap());
    app.handle_key(key(KeyCode::Char('n'), KeyModifiers::ALT));
    let generation = app.take_catalog_request().unwrap();
    app.apply_image_catalog(generation, Ok(provider.list_images_async(false).await.unwrap()));
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
        for path in ["/images?refresh=false"] {
            let (mut stream, _) = listener.accept().await.unwrap();
            let request = read_http_request(&mut stream).await;
            assert!(request.starts_with(&format!("GET {path} ")));
            write_response(
                &mut stream,
                "403 Forbidden",
                &format!("catalog refused test-token\u{1b}\n{}", "x".repeat(1000)),
            )
            .await;
        }
    });
    let error = GatewayProvider::with_test_token(url, "test-token")
        .list_images_async(false)
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("403") && error.contains("catalog refused"), "{error}");
    assert!(!error.contains("test-token"));
    assert!(error.chars().count() <= 512);
    assert!(!error.chars().any(char::is_control));
    server.await.unwrap();
}
