use super::*;
use crate::tests::{insert_fake_instance_with_session_dir, make_test_state, route_request, test_persistent_entry};
use axum::http::{Method, StatusCode};
use capsem_config::environment::{EnvironmentImage, EnvironmentStartup};
use capsem_logger::events::PreparedEnvironmentRevision;
use serde_json::json;

fn id(n: u64) -> String {
    format!("00000000-0000-4000-8000-{n:012x}")
}

fn selected(revision: u64) -> EnvironmentRevisionParent {
    EnvironmentRevisionParent {
        environment_id: id(1),
        revision_id: id(revision),
    }
}

async fn commit(state: &ServiceState, revision: u64, previous: Option<u64>, seconds: u32) {
    let mut specification: capsem_config::environment::EnvironmentSpecification =
        toml::from_str("name='lifecycle-proof'\n[variables]\nPRIVATE_VALUE='never-return'").unwrap();
    specification.image = Some(EnvironmentImage {
        reference: "registry.example/app:1".into(),
        resolved_digest: format!("sha256:{}", "a".repeat(64)),
        update_policy: Default::default(),
    });
    specification.startup = Some(EnvironmentStartup {
        script: "printf script-must-stay-private".into(),
        timeout_seconds: seconds,
    });
    state
        .environments
        .commit(
            PreparedEnvironmentRevision {
                environment_id: id(1),
                revision_id: id(revision),
                previous_revision_id: previous.map(id),
                fork_parent: None,
                timestamp_unix_ms: revision as i64,
                actor: "operator".into(),
                reason: "approved lifecycle wait".into(),
                request_id: None,
                payload_event_id: format!("{revision:012x}"),
                specification: Box::new(specification),
            },
            revision as i64,
        )
        .wait()
        .await
        .unwrap();
}

fn saved(state: &ServiceState, environment: Option<EnvironmentRevisionParent>) -> std::path::PathBuf {
    let session = state.run_dir.join("persistent/vm-1");
    std::fs::create_dir_all(&session).unwrap();
    let mut entry = test_persistent_entry("saved-name", session.clone());
    entry.id = "vm-1".into();
    entry.environment = environment;
    state.persistent_registry.lock().unwrap().register(entry).unwrap();
    session
}

fn write_identity(session: &std::path::Path, vm: &str, generation: u64) {
    let session = ContainedDir::open_root(session).unwrap();
    capsem_core::session::write_spawn_identity(
        &session,
        &capsem_core::managed_sessions::VmBinding::new(vm.into(), uuid::Uuid::parse_str(&id(generation)).unwrap())
            .unwrap(),
    )
    .unwrap();
}

#[tokio::test]
async fn original_active_owner_is_selected_without_exposing_workload_content() {
    let state = make_test_state();
    insert_fake_instance_with_session_dir(&state, "vm-1", std::process::id(), state.run_dir.join("sessions/vm-1"));
    let generation = state.instances.lock().unwrap()["vm-1"].generation.to_string();
    let app = crate::router_runtime::build_service_router(state);
    let (status, body) = route_request(app, Method::GET, "/vms/vm-1/readiness?intent=start", None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body,
        json!({"selection": {"id": "vm-1", "generation": generation,
                         "running": true, "environment": null}, "startup_timeout_seconds": 0})
    );
}

#[tokio::test]
async fn each_intent_uses_its_exact_retained_revision_during_preparation() {
    let state = make_test_state();
    commit(&state, 2, None, 600).await;
    commit(&state, 3, Some(2), u32::MAX).await;
    commit(&state, 4, Some(3), 1).await;
    let session = saved(&state, Some(selected(4)));
    write_identity(&session, "vm-1", 8);
    insert_fake_instance_with_session_dir(&state, "vm-1", std::process::id(), session);
    let generation = {
        let mut instances = state.instances.lock().unwrap();
        let instance = instances.get_mut("vm-1").unwrap();
        let mut binding = crate::environment_launch::EnvironmentBinding::starting(selected(2), "pending".into());
        binding.desired = selected(3);
        assert!(!binding.acknowledged, "a preparing owner still has a wait selection");
        instance.environment = Some(binding);
        let generation = instance.generation.to_string();
        drop(instances);
        generation
    };
    std::fs::write(
        state
            .run_dir
            .parent()
            .unwrap()
            .join("environments")
            .join(id(1))
            .join("environment.toml"),
        "broken",
    )
    .unwrap();
    let app = crate::router_runtime::build_service_router(state);
    for (target, intent, revision, seconds) in [
        ("vm-1", "start", 2, 600),
        ("vm-1", "fork", 3, u32::MAX),
        ("vm-1", "clone", 4, 1),
        ("saved-name", "clone", 4, 1),
    ] {
        let (status, body) = route_request(
            app.clone(),
            Method::GET,
            &format!("/vms/{target}/readiness?intent={intent}"),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(
            body,
            json!({"selection": {"id":"vm-1", "generation": generation, "running":true,
            "environment": {"environment_id":id(1), "revision_id":id(revision)}}, "startup_timeout_seconds":seconds})
        );
    }
}

#[tokio::test]
async fn stopped_owner_retains_durable_generation_and_legacy_absence_is_explicit() {
    let state = make_test_state();
    let session = saved(&state, None);
    let app = crate::router_runtime::build_service_router(state);
    for generation in [None, Some(9)] {
        if let Some(generation) = generation {
            write_identity(&session, "vm-1", generation);
        }
        for intent in ["start", "fork", "clone"] {
            let (status, body) = route_request(
                app.clone(),
                Method::GET,
                &format!("/vms/vm-1/readiness?intent={intent}"),
                None,
            )
            .await;
            assert_eq!(status, StatusCode::OK, "{body}");
            assert_eq!(
                body,
                json!({"selection":{"id":"vm-1", "generation":generation.map(id), "running":false,
                "environment":null}, "startup_timeout_seconds":0})
            );
        }
    }
}

#[tokio::test]
async fn malformed_or_misbound_durable_generation_never_becomes_a_wait_selection() {
    let state = make_test_state();
    let session = saved(&state, None);
    write_identity(&session, "other-vm", 9);
    let app = crate::router_runtime::build_service_router(state);
    assert_eq!(
        route_request(app.clone(), Method::GET, "/vms/vm-1/readiness?intent=start", None)
            .await
            .0,
        StatusCode::SERVICE_UNAVAILABLE
    );
    std::fs::write(session.join("spawn-identity.json"), "broken").unwrap();
    assert_eq!(
        route_request(app, Method::GET, "/vms/vm-1/readiness?intent=start", None)
            .await
            .0,
        StatusCode::SERVICE_UNAVAILABLE
    );
}

#[tokio::test]
async fn missing_history_and_invalid_registered_generation_fail_closed() {
    let state = make_test_state();
    insert_fake_instance_with_session_dir(&state, "vm-1", std::process::id(), state.run_dir.join("sessions/vm-1"));
    state.instances.lock().unwrap().get_mut("vm-1").unwrap().environment = Some(
        crate::environment_launch::EnvironmentBinding::starting(selected(2), "pending".into()),
    );
    let app = crate::router_runtime::build_service_router(state.clone());
    assert_eq!(
        route_request(app.clone(), Method::GET, "/vms/vm-1/readiness?intent=start", None)
            .await
            .0,
        StatusCode::SERVICE_UNAVAILABLE
    );
    state.instances.lock().unwrap().get_mut("vm-1").unwrap().generation = uuid::Uuid::nil();
    assert_eq!(
        route_request(app, Method::GET, "/vms/vm-1/readiness?intent=start", None)
            .await
            .0,
        StatusCode::SERVICE_UNAVAILABLE
    );
}

#[tokio::test]
async fn original_selection_is_rechecked_after_history_reads_before_publishing_timing() {
    let state = make_test_state();
    commit(&state, 2, None, 600).await;
    commit(&state, 3, Some(2), u32::MAX).await;
    let session = saved(&state, Some(selected(2)));
    write_identity(&session, "vm-1", 9);
    insert_fake_instance_with_session_dir(&state, "vm-1", std::process::id(), session);
    {
        let mut instances = state.instances.lock().unwrap();
        let instance = instances.get_mut("vm-1").unwrap();
        instance.generation = uuid::Uuid::parse_str(&id(9)).unwrap();
        instance.environment = Some(crate::environment_launch::EnvironmentBinding::starting(
            selected(2),
            "pending".into(),
        ));
        drop(instances);
    }
    let before = state.lifecycle_selection("vm-1", VmLifecycleIntent::Start).unwrap();
    state.instances.lock().unwrap().get_mut("vm-1").unwrap().generation = uuid::Uuid::parse_str(&id(10)).unwrap();
    assert_eq!(
        finish_view(&state, "vm-1".into(), VmLifecycleIntent::Start, before)
            .await
            .unwrap_err()
            .status,
        StatusCode::CONFLICT
    );
    let before = state.lifecycle_selection("vm-1", VmLifecycleIntent::Start).unwrap();
    state
        .instances
        .lock()
        .unwrap()
        .get_mut("vm-1")
        .unwrap()
        .environment
        .as_mut()
        .unwrap()
        .boot = selected(3);
    assert_eq!(
        finish_view(&state, "vm-1".into(), VmLifecycleIntent::Start, before)
            .await
            .unwrap_err()
            .status,
        StatusCode::CONFLICT
    );
    let before = state.lifecycle_selection("vm-1", VmLifecycleIntent::Fork).unwrap();
    state
        .instances
        .lock()
        .unwrap()
        .get_mut("vm-1")
        .unwrap()
        .environment
        .as_mut()
        .unwrap()
        .desired = selected(3);
    assert_eq!(
        finish_view(&state, "vm-1".into(), VmLifecycleIntent::Fork, before)
            .await
            .unwrap_err()
            .status,
        StatusCode::CONFLICT
    );
    // The last sidecar generation can equal the active one; stopping still
    // changes selection and cannot be laundered into the original plan.
    state.instances.lock().unwrap().get_mut("vm-1").unwrap().generation = uuid::Uuid::parse_str(&id(9)).unwrap();
    state
        .instances
        .lock()
        .unwrap()
        .get_mut("vm-1")
        .unwrap()
        .environment
        .as_mut()
        .unwrap()
        .boot = selected(2);
    let before = state.lifecycle_selection("vm-1", VmLifecycleIntent::Start).unwrap();
    state.instances.lock().unwrap().remove("vm-1");
    assert_eq!(
        finish_view(&state, "vm-1".into(), VmLifecycleIntent::Start, before)
            .await
            .unwrap_err()
            .status,
        StatusCode::CONFLICT
    );
}

#[tokio::test]
async fn absent_legacy_generation_is_not_a_wildcard_and_clone_alias_cannot_retarget() {
    let state = make_test_state();
    let session = saved(&state, None);
    let before = state.lifecycle_selection("vm-1", VmLifecycleIntent::Start).unwrap();
    write_identity(&session, "vm-1", 9);
    assert_eq!(
        finish_view(&state, "vm-1".into(), VmLifecycleIntent::Start, before)
            .await
            .unwrap_err()
            .status,
        StatusCode::CONFLICT
    );
    let before = state
        .lifecycle_selection("saved-name", VmLifecycleIntent::Clone)
        .unwrap();
    state
        .persistent_registry
        .lock()
        .unwrap()
        .unregister("saved-name")
        .unwrap();
    let replacement_dir = state.run_dir.join("persistent/vm-2");
    std::fs::create_dir_all(&replacement_dir).unwrap();
    let mut replacement = test_persistent_entry("saved-name", replacement_dir);
    replacement.id = "vm-2".into();
    state.persistent_registry.lock().unwrap().register(replacement).unwrap();
    assert_eq!(
        finish_view(&state, "saved-name".into(), VmLifecycleIntent::Clone, before)
            .await
            .unwrap_err()
            .status,
        StatusCode::CONFLICT
    );
}

#[tokio::test]
async fn readiness_requires_operator_transport_and_a_known_explicit_intent() {
    use axum::{body::Body, http::Request};
    use tower::ServiceExt;
    let state = make_test_state();
    insert_fake_instance_with_session_dir(&state, "vm-1", std::process::id(), state.run_dir.join("sessions/vm-1"));
    let app = crate::router_runtime::build_service_router(state.clone());
    for suffix in [
        "",
        "?intent=resume",
        "?intent=operator",
        "?intent=start&actor=operator",
        "?intent=start&intent=fork",
    ] {
        assert_eq!(
            route_request(app.clone(), Method::GET, &format!("/vms/vm-1/readiness{suffix}"), None)
                .await
                .0,
            StatusCode::BAD_REQUEST
        );
    }
    assert_eq!(
        route_request(app.clone(), Method::GET, "/vms/unknown/readiness?intent=start", None)
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        app.oneshot(
            Request::get("/vms/vm-1/readiness?intent=start")
                .header("x-capsem-actor", "operator")
                .body(Body::empty())
                .unwrap()
        )
        .await
        .unwrap()
        .status(),
        StatusCode::FORBIDDEN
    );
    let owner = crate::owner_connection::OwnerConnection::capture(&state.instances.lock().unwrap()["vm-1"]).unwrap();
    assert_eq!(
        handle(
            AuthenticatedActor::vm_owner(&owner),
            State(state),
            Path("vm-1".into()),
            Query(ReadinessQuery {
                intent: VmLifecycleIntent::Start
            })
        )
        .await
        .unwrap_err()
        .status,
        StatusCode::FORBIDDEN
    );
}
