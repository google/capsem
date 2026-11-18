use super::*;
use crate::tests::*;
use capsem_logger::events::PreparedEnvironmentRevision;

fn id(n: u64) -> String {
    format!("00000000-0000-4000-8000-{n:012x}")
}
fn reference(revision: u64) -> EnvironmentRevisionParent {
    EnvironmentRevisionParent {
        environment_id: id(1),
        revision_id: id(revision),
    }
}
fn revision(number: u64, previous: Option<u64>, action: &str) -> PreparedEnvironmentRevision {
    PreparedEnvironmentRevision {
        environment_id: id(1), revision_id: id(number), previous_revision_id: previous.map(id), fork_parent: None,
        timestamp_unix_ms: number as i64, actor: "operator".into(), reason: "launch binding proof".into(), request_id: None,
        payload_event_id: format!("{number:012x}"),
        specification: Box::new(toml::from_str(&format!("name='development'\n[variables]\nREVISION='revision-{number}'\n[access.custom.endpoint]\nname='Endpoint'\naction='{action}'\nmatch='http.host == \"scoped.example\"'\n[plugins.pii_filter.outbound]\nmode='block'\n")).unwrap()),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn generation_policy_materializes_bound_history_without_ambient_user_rules() {
    let _lock = SETTINGS_ENV_LOCK.lock().await;
    let directory = tempfile::tempdir().unwrap();
    let (_paths, settings, corporate) = install_empty_settings_env(&directory);
    // Malformed global workload settings cannot replace a selected environment.
    std::fs::write(settings, "[profiles.rules.inapplicable]\ninvalid = true").unwrap();
    let state = make_test_state();
    state
        .environments
        .commit(revision(2, None, "block"), 10)
        .wait()
        .await
        .unwrap();
    state
        .environments
        .commit(revision(3, Some(2), "allow"), 11)
        .wait()
        .await
        .unwrap();
    let session = directory.path().join("session");
    let prepared = {
        let state = state.clone();
        let session = session.clone();
        tokio::task::spawn_blocking(move || {
            state.prepare_environment_policy(&session, Some(&reference(2)), Some(&reference(2)))
        })
        .await
        .unwrap()
        .unwrap()
    };
    assert_eq!(prepared.1.unwrap()["REVISION"], "revision-2");
    assert!(!allowed(&prepared.0));
    std::fs::write(
        corporate,
        "[corp.rules.mandatory]\nname='mandatory'\naction='block'\nmatch='http.host == \"scoped.example\"'",
    )
    .unwrap();
    let (boot, live) = (reference(2), reference(3));
    let updated = state
        .off_worker(move |state| state.materialize_environment_policy(&session, Some(&boot), Some(&live)))
        .await
        .unwrap()
        .unwrap();
    assert!(!allowed(&updated), "current corporate block wins over live allow");
}
fn allowed(published: &PublishedActivePolicy) -> bool {
    let event = capsem_core::security_engine::SecurityEvent::new(
        capsem_core::security_engine::RuntimeSecurityEventType::HttpRequest,
    )
    .with_http(capsem_core::security_engine::HttpSecurityEvent {
        host: Some("scoped.example".into()),
        ..Default::default()
    });
    capsem_core::security_engine::evaluate_security_boundary(
        &published.runtime.security_rules,
        published.runtime.plugins.clone(),
        event,
    )
    .unwrap()
    .enforcement
    .is_allowed()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn environment_launch_receipts_require_original_generation_and_exact_acknowledgement() {
    use capsem_logger::db::{EnvironmentReferenceQuery, EnvironmentReferenceRecord};
    let _lock = SETTINGS_ENV_LOCK.lock().await;
    let directory = tempfile::tempdir().unwrap();
    let (_paths, _, _) = install_empty_settings_env(&directory);
    let state = make_test_state();
    state
        .environments
        .commit(revision(2, None, "block"), 10)
        .wait()
        .await
        .unwrap();
    for (index, behavior) in ["exact", "wrong-digest", "replaced"].into_iter().enumerate() {
        let vm = id(20 + index as u64);
        let session = directory.path().join(&vm);
        std::fs::create_dir_all(&session).unwrap();
        insert_fake_instance_with_session_dir(&state, &vm, std::process::id(), session.clone());
        let (generation, socket) = {
            let mut instances = state.instances.lock().unwrap();
            let instance = instances.get_mut(&vm).unwrap();
            instance.environment = Some(EnvironmentBinding::starting(reference(2), String::new()));
            let captured = (instance.generation, instance.uds_path.clone());
            drop(instances);
            captured
        };
        state
            .register_proxy_worker(&vm, generation, crate::proxy_worker::ProxyWorker::test_stub())
            .unwrap();
        let peer = spawn_fake_process(&socket, 1, {
            let state = state.clone();
            let vm = vm.clone();
            move |message| {
                let crate::ServiceToProcess::ReloadConfig { id, active_policy } = message else {
                    panic!("reload expected");
                };
                let digest = if behavior == "wrong-digest" {
                    "blake3:wrong".into()
                } else {
                    capsem_core::net::policy_config::active_policy_digest(active_policy)
                };
                if behavior == "replaced" {
                    insert_fake_instance_with_session_dir(&state, &vm, std::process::id(), session.clone());
                }
                let reply = crate::ProcessToService::ConfigReloadResult {
                    id: *id,
                    active_policy_digest: Some(digest),
                    error: None,
                };
                Box::pin(async { Some(reply) })
            }
        });
        let result = tokio::time::timeout(std::time::Duration::from_secs(5), initialize(&state, &vm))
            .await
            .unwrap();
        assert_eq!(result.is_ok(), behavior == "exact", "{behavior}: {result:?}");
        peer.await.unwrap();
        let rows = state
            .host_ledger
            .environment_references(EnvironmentReferenceQuery::Sandbox {
                session_id: vm.clone(),
                generation: generation.to_string(),
            })
            .await
            .unwrap();
        if behavior == "exact" {
            assert_eq!(rows.len(), 3);
            assert!(rows.iter().all(
                |row| matches!(row, EnvironmentReferenceRecord::Sandbox { revision, .. } if *revision == reference(2))
            ));
            tokio::time::timeout(std::time::Duration::from_secs(1), initialize(&state, &vm))
                .await
                .unwrap()
                .unwrap();
            assert!(
                state.instances.lock().unwrap()[&vm]
                    .environment
                    .as_ref()
                    .unwrap()
                    .acknowledged
            );
        } else {
            assert!(
                rows.is_empty(),
                "failed acknowledgement cannot create applied references"
            );
        }
    }
}
