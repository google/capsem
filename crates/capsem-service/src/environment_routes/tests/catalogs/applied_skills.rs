use super::*;
use crate::{environment_launch::EnvironmentBinding, lifecycle_readiness::SelectionRefusal};
use capsem_logger::{
    events::{
        environment::{EnvironmentSandboxReferenceKind, EnvironmentSandboxReferenceMutation},
        EnvironmentRevisionParent,
    },
    WriteOp,
};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn applied_skills_require_a_boot_receipt_and_block_revoked_files_and_new_boots() {
    let _lock = crate::tests::SETTINGS_ENV_LOCK.lock().await;
    let directory = tempfile::tempdir().unwrap();
    let (_paths, _, _) = crate::tests::install_empty_settings_env(&directory);
    let fx = Fixture::new().await;
    let registry = &fx.registry;
    let kind = CatalogKind::Skills;
    let contents = document(kind, false);
    let retained = registry.publish(kind, contents.clone());
    let app = crate::router_runtime::build_service_router(fx.state.clone());
    let (status, saved) = route_request(
        app.clone(),
        Method::POST,
        "/environments/create",
        Some(json!({
            "request_id":id(681), "reason":"applied skill fixture", "specification":{
                "name":"applied-skills", "skills":{"selected":[{"id":"review","version":"1.0.0"}]}
            }
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{saved}");
    let saved: capsem_api::EnvironmentMutationResponse = serde_json::from_value(saved).unwrap();
    let boot = saved.result.unwrap();
    let reference = EnvironmentRevisionParent {
        environment_id: boot.environment_id.clone(),
        revision_id: boot.revision_id.clone(),
    };
    let session = directory.path().join("prepared");
    let prepare = || {
        let selection = reference.clone();
        let session = session.clone();
        fx.state
            .off_worker(move |state| state.prepare_environment_policy(&session, Some(&selection), Some(&selection)))
    };
    prepare().await.unwrap().unwrap();

    let vm = id(682);
    crate::tests::insert_fake_instance_with_session_dir(&fx.state, &vm, std::process::id(), session.clone());
    let generation = {
        let mut instances = fx.state.instances.lock().unwrap();
        let instance = instances.get_mut(&vm).unwrap();
        instance.environment = Some(EnvironmentBinding::starting(reference.clone(), "fixture".into()));
        instance.generation.to_string()
    };
    let list = format!("/vms/{vm}/skills/list");
    let detail = format!("/vms/{vm}/skills/review/info");
    assert_eq!(
        route_request(app.clone(), Method::GET, &list, None).await.0,
        StatusCode::CONFLICT
    );
    fx.state
        .instances
        .lock()
        .unwrap()
        .get_mut(&vm)
        .unwrap()
        .environment
        .as_mut()
        .unwrap()
        .acknowledged = true;
    assert_eq!(
        route_request(app.clone(), Method::GET, &list, None).await.0,
        StatusCode::SERVICE_UNAVAILABLE
    );
    fx.state
        .host_ledger
        .write(WriteOp::EnvironmentSandboxReference(
            EnvironmentSandboxReferenceMutation {
                session_id: vm.clone(),
                generation: generation.clone(),
                kind: EnvironmentSandboxReferenceKind::Boot,
                expected: None,
                revision: Some(reference.clone()),
                timestamp_unix_ms: crate::policy_mutation::unix_timestamp_ms(),
                actor: "system".into(),
            },
        ))
        .await
        .unwrap();
    fx.state.host_ledger.flush().await.unwrap();

    let (status, advanced) = route_request(
        app.clone(),
        Method::POST,
        &format!("/environments/{}/changes", boot.environment_id),
        Some(json!({
            "request_id":id(683), "base_revision_id":boot.revision_id, "reason":"deselect for a later generation",
            "specification":{"name":"applied-skills"}
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{advanced}");
    let advanced: capsem_api::EnvironmentMutationResponse = serde_json::from_value(advanced).unwrap();
    let desired = advanced.result.unwrap();
    fx.state
        .instances
        .lock()
        .unwrap()
        .get_mut(&vm)
        .unwrap()
        .environment
        .as_mut()
        .unwrap()
        .desired = EnvironmentRevisionParent {
        environment_id: desired.environment_id,
        revision_id: desired.revision_id,
    };

    registry.unpublish_tag(kind);
    prepare().await.unwrap().unwrap();
    let (status, page) = route_request(app.clone(), Method::GET, &list, None).await;
    assert_eq!(status, StatusCode::OK, "{page}");
    assert_eq!(page["revision"], serde_json::to_value(&boot).unwrap());
    assert_eq!(page["owner_generation"], generation);
    assert_eq!(page["discovery_fresh"], false);
    assert_eq!(page["skills"][0]["selection"]["resolved_digest"], retained);
    let (status, payload) = route_request(app.clone(), Method::GET, &detail, None).await;
    assert_eq!(status, StatusCode::OK, "{payload}");
    assert_eq!(
        payload["skill"]["files"]["SKILL.md"],
        contents["entries"][0]["payload"]["files"]["SKILL.md"]["content"]
    );

    registry.publish(kind, document(kind, true));
    assert_eq!(
        route_request(app.clone(), Method::GET, &detail, None).await.0,
        StatusCode::FORBIDDEN
    );
    let (status, page) = route_request(app, Method::GET, &list, None).await;
    assert_eq!(status, StatusCode::OK, "{page}");
    assert_eq!(page["skills"][0]["availability"], "revoked");
    let error = prepare()
        .await
        .unwrap()
        .err()
        .unwrap()
        .downcast::<SelectionRefusal>()
        .unwrap();
    assert_eq!(error.0.status, StatusCode::FORBIDDEN);

    // Rule and policy refresh can still tighten an already-running sandbox.
    fx.state
        .off_worker(move |state| state.materialize_environment_policy(&session, Some(&reference), Some(&reference)))
        .await
        .unwrap()
        .unwrap();
}
