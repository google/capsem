//! Real tracked children and production service effects, without a VM boot.
use super::*;
use crate::managed_sessions::ManagedLifecycle;
use capsem_core::managed_sessions::{
    controller::{BindingReporter, EffectFuture, Effects},
    Ticket, VmBinding,
};
use capsem_core::net::policy_config::ManagedSessionPolicy;
use tokio_util::sync::CancellationToken;

struct ChildEffects {
    state: Arc<ServiceState>,
    binding: VmBinding,
    lifecycle: tokio::sync::OnceCell<ManagedLifecycle>,
    reaper: tokio::sync::Mutex<Option<tokio::task::JoinHandle<()>>>,
}

struct SpawnEffects(Arc<ChildEffects>);

impl Effects for SpawnEffects {
    fn is_live(&self, binding: &VmBinding) -> Result<bool> {
        self.0
            .lifecycle
            .get()
            .context("fixture creation not complete")?
            .is_live(binding)
    }

    fn create(
        &self,
        _ticket: Ticket,
        _cancel: CancellationToken,
        reporter: BindingReporter,
    ) -> EffectFuture<VmBinding> {
        let owner = self.0.clone();
        Box::pin(async move {
            let prepared = reporter.prepare(owner.binding.clone()).await?;
            let lifecycle = ManagedLifecycle::for_recovery_with_broker(
                owner.state.clone(),
                &prepared,
                owner.state.managed.grants.clone(),
            )?;
            owner
                .lifecycle
                .set(lifecycle)
                .map_err(|_| anyhow::anyhow!("fixture create replay"))?;
            let id = owner.binding.id().to_owned();
            let generation = owner.binding.generation();
            let session = owner.state.run_dir.join("sessions").join(&id);
            let directory = session.clone();
            owner
                .state
                .off_worker(move |_| -> Result<()> {
                    std::fs::create_dir_all(&directory)?;
                    crate::instance::persist_spawn_identity(&directory, &id, generation)?;
                    Ok(())
                })
                .await
                .map_err(|_| anyhow::anyhow!("fixture session preparation failed"))??;
            let retirement = owner.state.retirements.register(owner.binding.id(), generation)?;
            let child = tokio::process::Command::new("sh")
                .args(["-c", "exec sleep 30"])
                .env_clear()
                .env("PATH", "/usr/bin:/bin")
                .kill_on_drop(true)
                .spawn()?;
            crate::tests::insert_fake_instance_with_session_dir(
                &owner.state,
                owner.binding.id(),
                child.id().unwrap(),
                session.clone(),
            );
            owner
                .state
                .instances
                .lock()
                .unwrap()
                .get_mut(owner.binding.id())
                .unwrap()
                .generation = generation;
            *owner.reaper.lock().await = Some(crate::instance_reaper::spawn_exit_reaper(
                child,
                owner.binding.id().into(),
                owner.binding.id().into(),
                owner.state.clone(),
                owner.state.instance_socket_path(owner.binding.id()).unwrap(),
                session,
                retirement,
            ));
            reporter.registered(owner.binding.clone()).await?;
            Ok(owner.binding.clone())
        })
    }

    fn cleanup(&self, ticket: Ticket, binding: VmBinding) -> EffectFuture<()> {
        self.0
            .lifecycle
            .get()
            .expect("prepared adapter")
            .cleanup(ticket, binding)
    }

    fn reconcile(&self, ticket: Ticket, intent: Option<VmBinding>) -> EffectFuture<Option<VmBinding>> {
        self.0
            .lifecycle
            .get()
            .expect("prepared adapter")
            .reconcile(ticket, intent)
    }
}

async fn active(state: &Arc<ServiceState>) -> (Arc<ChildEffects>, capsem_core::managed_sessions::Snapshot) {
    let effects = Arc::new(ChildEffects {
        state: state.clone(),
        binding: VmBinding::new(new_persistent_vm_id(), capsem_crypto::uuid_v4()).unwrap(),
        lifecycle: tokio::sync::OnceCell::new(),
        reaper: tokio::sync::Mutex::new(None),
    });
    let snapshot = state
        .managed
        .controller
        .create(
            uuid::Uuid::parse_str(REQUEST).unwrap(),
            capsem_core::managed_sessions::Capability::from_bytes([0xaf; 32]),
            capsem_core::managed_sessions::LeasePolicy::new(std::time::Duration::from_secs(300)).unwrap(),
            Arc::new(SpawnEffects(effects.clone())),
        )
        .await
        .unwrap();
    (effects, snapshot)
}

#[tokio::test]
async fn ordinary_stop_and_delete_converge_managed_claim_to_original_closed_ownership() {
    for action in ["stop", "delete"] {
        let state = crate::tests::make_test_state();
        let (effects, original) = active(&state).await;
        let app = build_service_router(state.clone());
        let claim = format!("/vms/managed/{REQUEST}/claim");
        let token = "af".repeat(32);
        let (status, live) = post(&app, &claim, json!({"ownership_token":token})).await;
        assert_eq!(status, StatusCode::OK, "{live}");
        assert_eq!(live["state"], "active");
        assert_eq!(live["session"]["id"], effects.binding.id());
        let (status, body) = request_json(
            &app,
            &format!("/vms/{}/{action}", effects.binding.id()),
            if action == "delete" { "DELETE" } else { "POST" },
            json!({}),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let (status, closed) = post(&app, &claim, json!({"ownership_token":token})).await;
        assert_eq!(status, StatusCode::OK, "{closed}");
        assert_eq!(closed["state"], "closed", "{action} left ownership renewable");
        assert_eq!(closed["ownership"]["generation"], original.generation().to_string());
        assert!(closed.get("session").is_none());
        assert!(state.instances.lock().unwrap().is_empty());
        assert!(!state.run_dir.join("sessions").join(effects.binding.id()).exists());
        assert!(state
            .retirements
            .wait(effects.binding.id(), effects.binding.generation())
            .await
            .unwrap());
        effects.reaper.lock().await.take().unwrap().await.unwrap();
        let (status, repeated) = post(&app, &claim, json!({"ownership_token":token})).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(repeated["ownership"]["expires_at"], closed["ownership"]["expires_at"]);
    }
}

#[tokio::test]
async fn reloaded_policy_reports_actual_narrowed_lease_and_cannot_extend_old_reservation() {
    let state = crate::tests::make_test_state();
    let (effects, original) = active(&state).await;
    let app = build_service_router(state.clone());
    let policy = ManagedSessionPolicy {
        lease_seconds: 120,
        renew_after_seconds: 30,
        tombstone_seconds: 86400,
    };
    let mutation = state.policy_mutation.begin().await;
    state.managed.reload_policy(policy, &mutation).await.unwrap();
    let claim = format!("/vms/managed/{REQUEST}/claim");
    let (status, narrowed) = post(&app, &claim, json!({"ownership_token":"af".repeat(32)})).await;
    assert_eq!(status, StatusCode::OK, "{narrowed}");
    assert_eq!(narrowed["state"], "active");
    assert_eq!(narrowed["ownership"]["lease_seconds"], 120);
    assert_eq!(narrowed["ownership"]["renew_after_seconds"], 30);
    assert_eq!(narrowed["ownership"]["generation"], original.generation().to_string());
    state
        .managed
        .reload_policy(
            ManagedSessionPolicy {
                lease_seconds: 600,
                renew_after_seconds: 150,
                ..policy
            },
            &mutation,
        )
        .await
        .unwrap();
    drop(mutation);
    let (status, retained) = post(&app, &claim, json!({"ownership_token":"af".repeat(32)})).await;
    assert_eq!(status, StatusCode::OK, "{retained}");
    assert_eq!(retained["ownership"]["lease_seconds"], 120);
    assert_eq!(retained["ownership"]["renew_after_seconds"], 119);
    let close = format!("/vms/managed/{REQUEST}/close");
    let (status, closed) = post(
        &app,
        &close,
        json!({"ownership_token":"af".repeat(32),"outcome":"completed"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{closed}");
    effects.reaper.lock().await.take().unwrap().await.unwrap();
}

#[tokio::test]
async fn changed_spawn_cannot_be_adopted_by_claim_or_killed_by_original_cleanup() {
    let state = crate::tests::make_test_state();
    let (effects, original) = active(&state).await;
    let generation = capsem_crypto::uuid_v4();
    state
        .instances
        .lock()
        .unwrap()
        .get_mut(effects.binding.id())
        .unwrap()
        .generation = generation;
    let app = build_service_router(state.clone());
    let claim = format!("/vms/managed/{REQUEST}/claim");
    let (status, body) = post(&app, &claim, json!({"ownership_token":"af".repeat(32)})).await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(body["code"], "cleanup_failed");
    assert_eq!(
        state
            .instances
            .lock()
            .unwrap()
            .get(effects.binding.id())
            .unwrap()
            .generation,
        generation
    );
    let observed = state
        .managed
        .controller
        .inspect(
            original.request(),
            capsem_core::managed_sessions::Capability::from_bytes([0xaf; 32]),
        )
        .await
        .unwrap()
        .unwrap();
    assert_eq!(observed.state(), capsem_core::managed_sessions::State::Closing);
    assert_eq!(observed.expires_wall_ms(), original.expires_wall_ms());
    // Restore the fixture's original authority solely to reap its real child.
    state
        .instances
        .lock()
        .unwrap()
        .get_mut(effects.binding.id())
        .unwrap()
        .generation = effects.binding.generation();
    state
        .managed
        .controller
        .close(
            original.request(),
            capsem_core::managed_sessions::Capability::from_bytes([0xaf; 32]),
        )
        .await
        .unwrap();
    effects.reaper.lock().await.take().unwrap().await.unwrap();
}
