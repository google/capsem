use capsem_ui_catalog::native_deck::{generate_image, GenerateImageRequest};
use capsem_ui_runtime::storage::SqliteWorkspaceStore;
use capsem_ui_runtime::workspace::{
    artifact_record, change_request_record, resolve_task_record, task_id_from_record,
    title_patch_record, WorkspaceProjector, WorkspaceRecord, WorkspaceTaskStatus, WorkspaceVerb,
};

fn image(id: &str, title: &str) -> capsem_ui_catalog::native_deck::NativeArtifact {
    generate_image(GenerateImageRequest {
        id: id.to_owned(),
        title: title.to_owned(),
        prompt: "a generated image".to_owned(),
        provider: "gemini".to_owned(),
        model: None,
    })
    .expect("image artifact")
}

fn apply_and_store(
    store: &SqliteWorkspaceStore,
    workspace_id: &str,
    projector: &mut WorkspaceProjector,
    record: WorkspaceRecord,
) {
    projector.apply(&record).expect("record projects");
    store
        .append_record(workspace_id, &record)
        .expect("record persists");
}

#[test]
fn sqlite_store_replays_records_when_no_checkpoint_exists() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let db_path = tempdir.path().join("workspace.sqlite");
    let workspace_id = "runtime-storage-no-checkpoint";
    {
        let store = SqliteWorkspaceStore::open(&db_path).expect("store");
        let mut projector = WorkspaceProjector::new();
        apply_and_store(
            &store,
            workspace_id,
            &mut projector,
            artifact_record(
                1,
                "2026-06-05T00:00:00Z".to_owned(),
                "local.generate.image",
                WorkspaceVerb::Create,
                image("generated-image-storage", "Storage Image"),
            ),
        );
        apply_and_store(
            &store,
            workspace_id,
            &mut projector,
            title_patch_record(
                2,
                "2026-06-05T00:00:01Z".to_owned(),
                "local.ui.mutate",
                "generated-image-storage".to_owned(),
                "Storage Image Edited".to_owned(),
            ),
        );
    }

    let restored = SqliteWorkspaceStore::open(&db_path)
        .expect("reopen")
        .restore(workspace_id)
        .expect("restore");

    assert!(restored.checkpoint.is_none());
    assert_eq!(restored.frames.len(), 2);
    assert_eq!(restored.projector.projection().seq, 2);
    assert_eq!(
        restored.projector.projection().elements["generated-image-storage"].title,
        "Storage Image Edited"
    );
}

#[test]
fn sqlite_store_restores_checkpoint_and_replays_tail_tasks() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let db_path = tempdir.path().join("workspace.sqlite");
    let workspace_id = "runtime-storage-checkpoint-tail";
    let task_id;
    {
        let store = SqliteWorkspaceStore::open(&db_path).expect("store");
        let mut projector = WorkspaceProjector::new();
        apply_and_store(
            &store,
            workspace_id,
            &mut projector,
            artifact_record(
                1,
                "2026-06-05T00:00:00Z".to_owned(),
                "local.generate.image",
                WorkspaceVerb::Create,
                image("generated-image-task-storage", "Task Storage"),
            ),
        );
        apply_and_store(
            &store,
            workspace_id,
            &mut projector,
            title_patch_record(
                2,
                "2026-06-05T00:00:01Z".to_owned(),
                "local.ui.mutate",
                "generated-image-task-storage".to_owned(),
                "Task Storage Edited".to_owned(),
            ),
        );
        let request_record = change_request_record(
            3,
            "2026-06-05T00:00:02Z".to_owned(),
            "chat.ui",
            "generated-image-task-storage".to_owned(),
            "make it more direct".to_owned(),
            None,
        );
        task_id = task_id_from_record(&request_record);
        apply_and_store(&store, workspace_id, &mut projector, request_record);
        let checkpoint = projector.checkpoint(workspace_id, "2026-06-05T00:00:03Z".to_owned());
        store
            .save_checkpoint(workspace_id, &checkpoint)
            .expect("checkpoint persists");

        apply_and_store(
            &store,
            workspace_id,
            &mut projector,
            resolve_task_record(
                4,
                "2026-06-05T00:00:04Z".to_owned(),
                "assistant.ui",
                task_id.clone(),
            ),
        );
    }

    let restored = SqliteWorkspaceStore::open(&db_path)
        .expect("reopen")
        .restore(workspace_id)
        .expect("restore");
    let projection = restored.projector.projection();
    let snapshot = restored.snapshot("2026-06-05T00:00:05Z".to_owned());

    assert_eq!(
        restored
            .checkpoint
            .as_ref()
            .expect("checkpoint")
            .checkpoint_seq,
        3
    );
    assert_eq!(restored.frames.len(), 1);
    assert_eq!(restored.frames[0].record.seq, 4);
    assert_eq!(projection.seq, 4);
    assert_eq!(
        projection.elements["generated-image-task-storage"].title,
        "Task Storage Edited"
    );
    assert_eq!(
        projection.tasks[&task_id].status,
        WorkspaceTaskStatus::Resolved
    );
    assert_eq!(
        projection.tasks[&task_id].resolved_by.as_deref(),
        Some("assistant.ui")
    );
    assert_eq!(snapshot.checkpoint.checkpoint_seq, 3);
    assert_eq!(snapshot.projection.seq, 4);
    assert_eq!(snapshot.tail.len(), 1);
}

#[test]
fn sqlite_store_rejects_duplicate_and_gap_record_sequences() {
    let store = SqliteWorkspaceStore::open_in_memory().expect("store");
    let workspace_id = "runtime-storage-sequence-rejects";
    let first = artifact_record(
        1,
        "2026-06-05T00:00:00Z".to_owned(),
        "local.generate.image",
        WorkspaceVerb::Create,
        image("generated-image-seq", "Sequence"),
    );
    store
        .append_record(workspace_id, &first)
        .expect("first record persists");

    let duplicate = title_patch_record(
        1,
        "2026-06-05T00:00:01Z".to_owned(),
        "local.ui.mutate",
        "generated-image-seq".to_owned(),
        "Duplicate".to_owned(),
    );
    let duplicate_error = store
        .append_record(workspace_id, &duplicate)
        .expect_err("duplicate seq should fail");
    assert!(duplicate_error.to_string().contains("expected 2"));

    let gap = title_patch_record(
        3,
        "2026-06-05T00:00:02Z".to_owned(),
        "local.ui.mutate",
        "generated-image-seq".to_owned(),
        "Gap".to_owned(),
    );
    let gap_error = store
        .append_record(workspace_id, &gap)
        .expect_err("gap seq should fail");
    assert!(gap_error.to_string().contains("expected 2"));
}

#[test]
fn sqlite_store_rejects_ahead_and_stale_checkpoints() {
    let store = SqliteWorkspaceStore::open_in_memory().expect("store");
    let workspace_id = "runtime-storage-checkpoint-rejects";
    let mut projector = WorkspaceProjector::new();
    let first = artifact_record(
        1,
        "2026-06-05T00:00:00Z".to_owned(),
        "local.generate.image",
        WorkspaceVerb::Create,
        image("generated-image-checkpoint", "Checkpoint"),
    );
    projector.apply(&first).expect("first projects");
    let checkpoint_one = projector.checkpoint(workspace_id, "2026-06-05T00:00:01Z".to_owned());
    let ahead_error = store
        .save_checkpoint(workspace_id, &checkpoint_one)
        .expect_err("checkpoint ahead of records should fail");
    assert!(ahead_error.to_string().contains("ahead"));

    store
        .append_record(workspace_id, &first)
        .expect("first persists");
    store
        .save_checkpoint(workspace_id, &checkpoint_one)
        .expect("checkpoint one persists");
    let second = title_patch_record(
        2,
        "2026-06-05T00:00:02Z".to_owned(),
        "local.ui.mutate",
        "generated-image-checkpoint".to_owned(),
        "Checkpoint Edited".to_owned(),
    );
    projector.apply(&second).expect("second projects");
    store
        .append_record(workspace_id, &second)
        .expect("second persists");
    let checkpoint_two = projector.checkpoint(workspace_id, "2026-06-05T00:00:03Z".to_owned());
    store
        .save_checkpoint(workspace_id, &checkpoint_two)
        .expect("checkpoint two persists");

    let stale_error = store
        .save_checkpoint(workspace_id, &checkpoint_one)
        .expect_err("stale checkpoint should fail");
    assert!(stale_error.to_string().contains("older than latest"));
}
