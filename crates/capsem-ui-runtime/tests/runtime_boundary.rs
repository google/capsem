use capsem_ui_catalog::native_deck::{generate_image, GenerateImageRequest};
use capsem_ui_runtime::editable_state::run_editable_state_audit;
use capsem_ui_runtime::workspace::{
    artifact_record, title_patch_record, WorkspaceProjector, WorkspaceVerb, ROOT_SLOT,
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

#[test]
fn runtime_crate_projects_records_and_checkpoints_without_server_code() {
    let mut projector = WorkspaceProjector::new();
    projector
        .apply(&artifact_record(
            1,
            "2026-06-05T00:00:00Z".to_owned(),
            "local.generate.image",
            WorkspaceVerb::Create,
            image("generated-image-runtime", "Runtime Image"),
        ))
        .expect("create record projects");
    projector
        .apply(&title_patch_record(
            2,
            "2026-06-05T00:00:01Z".to_owned(),
            "local.ui.mutate",
            "generated-image-runtime".to_owned(),
            "Runtime Image Edited".to_owned(),
        ))
        .expect("title patch projects");

    let projection = projector.projection();
    assert_eq!(projection.seq, 2);
    assert_eq!(
        projection.topology.roots,
        vec!["generated-image-runtime".to_owned()]
    );
    assert_eq!(
        projection.topology.nodes["generated-image-runtime"].slot,
        ROOT_SLOT
    );
    assert_eq!(
        projection.elements["generated-image-runtime"].title,
        "Runtime Image Edited"
    );
    assert_eq!(
        projection.elements["generated-image-runtime"]
            .provenance
            .updated_by,
        "local.ui.mutate"
    );

    let checkpoint = projector.checkpoint("runtime-test", "2026-06-05T00:00:02Z".to_owned());
    assert_eq!(checkpoint.workspace_id, "runtime-test");
    assert_eq!(checkpoint.checkpoint_seq, 2);
    assert_eq!(
        checkpoint.records_compacted.expect("compacted").end,
        projection.seq
    );
}

#[test]
fn runtime_crate_exposes_editable_loro_state_audit() {
    let audit = run_editable_state_audit().expect("loro audit fixture");
    assert_eq!(audit.record.payload.engine, "loro");
    assert!(audit.topology_ids.iter().any(|id| id == "card:model"));
}
