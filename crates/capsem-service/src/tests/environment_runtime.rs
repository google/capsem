use super::*;
use capsem_config::environment::{EnvironmentDocument, EnvironmentIdentity};
use capsem_logger::{db::EnvironmentRevisionQuery, events::PreparedEnvironmentRevision, WriteOp};

fn id(n: u64) -> String {
    format!("00000000-0000-4000-8000-{n:012x}")
}
fn revision() -> PreparedEnvironmentRevision {
    PreparedEnvironmentRevision {
        environment_id: id(1),
        revision_id: id(2),
        previous_revision_id: None,
        fork_parent: None,
        timestamp_unix_ms: 2,
        actor: "operator".into(),
        reason: "state integration fixture".into(),
        request_id: None,
        payload_event_id: "abcdef012345".into(),
        specification: Box::new(toml::from_str("name='development'").unwrap()),
    }
}

#[tokio::test]
async fn service_environment_owner_uses_the_same_host_ledger_and_home_directory() {
    let state = make_test_state();
    let committed = state.environments.commit(revision(), 10).wait().await.unwrap();
    let rows = state
        .host_ledger
        .environment_revisions(EnvironmentRevisionQuery::Get {
            environment_id: id(1),
            revision_id: id(2),
        })
        .await
        .unwrap();
    assert_eq!(rows, [committed]);
    assert!(state
        .run_dir
        .parent()
        .unwrap()
        .join("environments")
        .join(id(1))
        .join("environment.toml")
        .is_file());
    assert!(!state.run_dir.join("environments").exists());
    assert_eq!(
        state
            .environments
            .load_current(&id(1))
            .await
            .unwrap()
            .revision
            .revision_id,
        id(2)
    );
}

#[tokio::test]
async fn service_environment_owner_reconciles_durable_startup_rows() {
    let state = make_test_state();
    let prepared = revision();
    let document = EnvironmentDocument {
        identity: EnvironmentIdentity {
            schema_version: 1,
            id: id(1),
            revision_id: id(2),
            parent_environment_id: None,
            parent_revision_id: None,
        },
        specification: *prepared.specification.clone(),
    };
    state
        .host_ledger
        .write(WriteOp::PrepareEnvironmentRevision(prepared))
        .await
        .unwrap();
    state.host_ledger.flush().await.unwrap();
    let directory = state.run_dir.parent().unwrap().join("environments").join(id(1));
    capsem_foundation::unix::fs::ensure_private_dir(&directory).unwrap();
    capsem_foundation::unix::fs::atomic_write_private(
        &directory.join("environment.toml"),
        document.to_toml().unwrap().as_bytes(),
    )
    .unwrap();
    let recovery = state.environments.reconcile_pending(10).wait().await.unwrap();
    assert_eq!(recovery.committed, 1);
    assert_eq!(recovery.failed, 0);
    assert_eq!(
        state.environments.load_current(&id(1)).await.unwrap().document,
        document
    );
}
