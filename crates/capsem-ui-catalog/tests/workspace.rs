use std::collections::BTreeMap;

use capsem_ui_catalog::native_deck::{generate_image, GenerateImageRequest};
use capsem_ui_catalog::workspace::{
    artifact_record, change_request_from_record, change_request_record, delete_artifact_record,
    resolve_task_record, select_record, style_patch_record, task_id_from_record, text_patch_record,
    text_record, text_selector_patch_record, title_patch_record, ElementStylePatch,
    ElementTextPatch, RenderDelta, WorkspaceAnnotationTarget, WorkspaceContent,
    WorkspaceContentType, WorkspaceProjector, WorkspaceRecord, WorkspaceRole, WorkspaceStatus,
    WorkspaceTaskStatus, WorkspaceVerb, ROOT_SLOT,
};

fn image(id: &str, title: &str) -> capsem_ui_catalog::native_deck::NativeArtifact {
    generate_image(GenerateImageRequest {
        id: id.to_owned(),
        title: title.to_owned(),
        prompt: "a generated image".to_owned(),
        caption: Some("Original caption".to_owned()),
        provider: "gemini".to_owned(),
        model: None,
    })
    .expect("image artifact")
}

#[test]
fn artifact_records_project_to_keyed_elements() {
    let artifact = image("generated-image-test", "Generated Image Test");
    let record = artifact_record(
        1,
        "2026-05-31T00:00:00Z".to_owned(),
        "local.generate.image",
        WorkspaceVerb::Create,
        artifact.clone(),
    );
    let mut projector = WorkspaceProjector::new();
    let deltas = projector.apply(&record).expect("record should project");

    assert_eq!(projector.projection().seq, 1);
    assert_eq!(
        projector.projection().topology.roots,
        vec!["generated-image-test"]
    );
    assert_eq!(
        projector.projection().topology.nodes["generated-image-test"].slot,
        ROOT_SLOT
    );
    assert_eq!(
        projector.projection().elements["generated-image-test"].component,
        "capsem-media"
    );
    assert_eq!(
        projector.projection().elements["generated-image-test"]
            .artifact
            .as_ref()
            .expect("artifact")
            .id,
        artifact.id
    );
    assert_eq!(deltas.len(), 3);
}

#[test]
fn replace_updates_same_id_without_topology_duplication() {
    let mut projector = WorkspaceProjector::new();
    let first = artifact_record(
        1,
        "2026-05-31T00:00:00Z".to_owned(),
        "local.generate.image",
        WorkspaceVerb::Create,
        image("generated-image-test", "Old"),
    );
    projector.apply(&first).expect("first record");

    let second = artifact_record(
        2,
        "2026-05-31T00:00:01Z".to_owned(),
        "local.generate.image",
        WorkspaceVerb::Replace,
        image("generated-image-test", "New"),
    );
    let deltas = projector.apply(&second).expect("replace record");

    assert_eq!(
        projector.projection().topology.roots,
        vec!["generated-image-test"]
    );
    assert_eq!(
        projector.projection().elements["generated-image-test"].title,
        "New"
    );
    assert_eq!(
        projector.projection().elements["generated-image-test"]
            .provenance
            .created_by,
        "local.generate.image"
    );
    assert_eq!(
        projector.projection().elements["generated-image-test"]
            .provenance
            .updated_seq,
        2
    );
    assert_eq!(deltas.len(), 1);
}

#[test]
fn checkpoint_collapses_current_projection() {
    let mut projector = WorkspaceProjector::new();
    projector
        .apply(&artifact_record(
            1,
            "2026-05-31T00:00:00Z".to_owned(),
            "local.generate.image",
            WorkspaceVerb::Create,
            image("generated-image-test", "Generated Image Test"),
        ))
        .expect("project");

    let checkpoint = projector.checkpoint("native-artifacts", "2026-05-31T00:00:02Z".to_owned());

    assert_eq!(checkpoint.checkpoint_seq, 1);
    assert_eq!(checkpoint.projection.elements.len(), 1);
    assert_eq!(checkpoint.records_compacted.expect("compacted").start, 1);
}

#[test]
fn content_type_mismatch_is_rejected() {
    let artifact = image("generated-image-test", "Generated Image Test");
    let record = WorkspaceRecord {
        seq: 1,
        id: "rec-1".to_owned(),
        timestamp: "2026-05-31T00:00:00Z".to_owned(),
        role: WorkspaceRole::Tool,
        principal: "local.generate.image".to_owned(),
        title: artifact.title.clone(),
        content_type: WorkspaceContentType::Text,
        verb: WorkspaceVerb::Create,
        status: WorkspaceStatus::Complete,
        target: Some(artifact.id.clone()),
        content: WorkspaceContent::Artifact { artifact },
    };

    let mut projector = WorkspaceProjector::new();
    let error = projector.apply(&record).expect_err("must reject mismatch");
    assert!(error.contains("content_type"));
}

#[test]
fn delete_prunes_element_topology_and_selection() {
    let mut projector = WorkspaceProjector::new();
    let first = image("generated-image-first", "First");
    let second = image("generated-image-second", "Second");
    projector
        .apply(&artifact_record(
            1,
            "2026-05-31T00:00:00Z".to_owned(),
            "local.generate.image",
            WorkspaceVerb::Create,
            first.clone(),
        ))
        .expect("first");
    projector
        .apply(&artifact_record(
            2,
            "2026-05-31T00:00:01Z".to_owned(),
            "local.generate.image",
            WorkspaceVerb::Create,
            second.clone(),
        ))
        .expect("second");
    projector
        .apply(&select_record(
            3,
            "2026-05-31T00:00:02Z".to_owned(),
            "local.workspace.select",
            Some(second.id.clone()),
        ))
        .expect("select second");

    let deltas = projector
        .apply(&delete_artifact_record(
            4,
            "2026-05-31T00:00:03Z".to_owned(),
            "local.workspace.delete",
            second,
        ))
        .expect("delete second");

    assert!(!projector
        .projection()
        .elements
        .contains_key("generated-image-second"));
    assert_eq!(
        projector.projection().topology.roots,
        vec!["generated-image-first"]
    );
    assert!(!projector
        .projection()
        .topology
        .nodes
        .contains_key("generated-image-second"));
    assert_eq!(
        projector.projection().selected.as_deref(),
        Some("generated-image-first")
    );
    assert_eq!(deltas.len(), 3);
}

#[test]
fn select_changes_focus_without_changing_elements() {
    let mut projector = WorkspaceProjector::new();
    let first = image("generated-image-first", "First");
    let second = image("generated-image-second", "Second");
    projector
        .apply(&artifact_record(
            1,
            "2026-05-31T00:00:00Z".to_owned(),
            "local.generate.image",
            WorkspaceVerb::Create,
            first,
        ))
        .expect("first");
    projector
        .apply(&artifact_record(
            2,
            "2026-05-31T00:00:01Z".to_owned(),
            "local.generate.image",
            WorkspaceVerb::Create,
            second.clone(),
        ))
        .expect("second");
    let before = projector.projection().elements.len();

    let deltas = projector
        .apply(&select_record(
            3,
            "2026-05-31T00:00:02Z".to_owned(),
            "local.workspace.select",
            Some(second.id),
        ))
        .expect("select");

    assert_eq!(projector.projection().elements.len(), before);
    assert_eq!(
        projector.projection().selected.as_deref(),
        Some("generated-image-second")
    );
    assert_eq!(deltas.len(), 1);
}

#[test]
fn title_patch_updates_display_title_without_mutating_artifact() {
    let mut projector = WorkspaceProjector::new();
    let artifact = image("generated-image-title-edit", "Original Artifact Title");
    projector
        .apply(&artifact_record(
            1,
            "2026-05-31T00:00:00Z".to_owned(),
            "local.generate.image",
            WorkspaceVerb::Create,
            artifact,
        ))
        .expect("create");

    let deltas = projector
        .apply(&title_patch_record(
            2,
            "2026-05-31T00:00:01Z".to_owned(),
            "local.workspace.title",
            "generated-image-title-edit".to_owned(),
            "Live Edited Card Title".to_owned(),
        ))
        .expect("title patch");

    let element = &projector.projection().elements["generated-image-title-edit"];
    assert_eq!(element.title, "Live Edited Card Title");
    assert_eq!(element.provenance.created_seq, 1);
    assert_eq!(element.provenance.updated_seq, 2);
    assert_eq!(element.provenance.updated_by, "local.workspace.title");
    assert_eq!(element.provenance.last_verb, WorkspaceVerb::Patch);
    assert_eq!(
        element.artifact.as_ref().expect("artifact").title,
        "Original Artifact Title"
    );
    assert_eq!(
        projector.projection().topology.roots,
        vec!["generated-image-title-edit"]
    );
    assert_eq!(deltas.len(), 1);
}

#[test]
fn text_record_and_patch_update_text_content() {
    let mut projector = WorkspaceProjector::new();
    projector
        .apply(&text_record(
            1,
            "2026-05-31T00:00:00Z".to_owned(),
            "local.ui.text",
            "text-summary".to_owned(),
            "Summary".to_owned(),
            "Original summary".to_owned(),
        ))
        .expect("text record");

    let deltas = projector
        .apply(&text_patch_record(
            2,
            "2026-05-31T00:00:01Z".to_owned(),
            "local.ui.mutate",
            "text-summary".to_owned(),
            "Edited summary".to_owned(),
        ))
        .expect("text patch");

    let element = &projector.projection().elements["text-summary"];
    assert_eq!(element.component, "capsem-text");
    assert_eq!(element.title, "Summary");
    assert_eq!(
        element.content.as_ref().expect("content")["text"],
        "Edited summary"
    );
    assert_eq!(element.provenance.created_seq, 1);
    assert_eq!(element.provenance.updated_seq, 2);
    assert_eq!(element.provenance.updated_by, "local.ui.mutate");
    assert!(matches!(
        deltas.as_slice(),
        [RenderDelta::UpsertElement { id, element }] if id == "text-summary"
            && element.content.as_ref().expect("content")["text"] == "Edited summary"
    ));
}

#[test]
fn invalid_text_patches_are_rejected() {
    let mut projector = WorkspaceProjector::new();
    projector
        .apply(&text_record(
            1,
            "2026-05-31T00:00:00Z".to_owned(),
            "local.ui.text",
            "text-summary".to_owned(),
            "Summary".to_owned(),
            "Original summary".to_owned(),
        ))
        .expect("text record");
    let empty_error = projector
        .apply(&text_patch_record(
            2,
            "2026-05-31T00:00:01Z".to_owned(),
            "local.ui.mutate",
            "text-summary".to_owned(),
            "   ".to_owned(),
        ))
        .expect_err("empty text patch should fail");
    assert!(empty_error.contains("patch text"));

    projector
        .apply(&artifact_record(
            3,
            "2026-05-31T00:00:02Z".to_owned(),
            "local.generate.image",
            WorkspaceVerb::Create,
            image("generated-image-not-text", "Not Text"),
        ))
        .expect("artifact record");
    let wrong_target_error = projector
        .apply(&text_patch_record(
            4,
            "2026-05-31T00:00:03Z".to_owned(),
            "local.ui.mutate",
            "generated-image-not-text".to_owned(),
            "Should fail".to_owned(),
        ))
        .expect_err("non-text target should fail");
    assert!(wrong_target_error.contains("text element"));
}

#[test]
fn style_patch_updates_artifact_style_patches() {
    let mut projector = WorkspaceProjector::new();
    projector
        .apply(&artifact_record(
            1,
            "2026-05-31T00:00:00Z".to_owned(),
            "local.generate.image",
            WorkspaceVerb::Create,
            image("generated-image-style-edit", "Style Edit"),
        ))
        .expect("create");

    let deltas = projector
        .apply(&style_patch_record(
            2,
            "2026-05-31T00:00:01Z".to_owned(),
            "local.workspace.style",
            "generated-image-style-edit".to_owned(),
            ElementStylePatch {
                selector: Some(
                    "capsem-media[data-capsem-artifact-id=\"generated-image-style-edit\"] >>> [data-capsem-node=\"6\"]"
                        .to_owned(),
                ),
                host_selector: Some(
                    "capsem-media[data-capsem-artifact-id=\"generated-image-style-edit\"]"
                        .to_owned(),
                ),
                shadow_selector: Some("[data-capsem-node=\"6\"]".to_owned()),
                styles: BTreeMap::from([("fontWeight".to_owned(), "700".to_owned())]),
                source_request_seq: Some(16),
            },
        ))
        .expect("style patch");

    let element = &projector.projection().elements["generated-image-style-edit"];
    let patches = element.artifact.as_ref().expect("artifact").spec["stylePatches"]
        .as_array()
        .expect("style patches");
    assert_eq!(patches.len(), 1);
    assert_eq!(patches[0]["shadowSelector"], "[data-capsem-node=\"6\"]");
    assert_eq!(patches[0]["styles"]["fontWeight"], "700");
    assert_eq!(element.provenance.updated_by, "local.workspace.style");
    assert_eq!(element.provenance.last_verb, WorkspaceVerb::Patch);
    assert_eq!(deltas.len(), 1);
}

#[test]
fn selector_text_patch_updates_artifact_text_patches() {
    let mut projector = WorkspaceProjector::new();
    projector
        .apply(&artifact_record(
            1,
            "2026-05-31T00:00:00Z".to_owned(),
            "local.generate.image",
            WorkspaceVerb::Create,
            image("generated-image-text-edit", "Text Edit"),
        ))
        .expect("create");

    let deltas = projector
        .apply(&text_selector_patch_record(
            2,
            "2026-05-31T00:00:01Z".to_owned(),
            "local.ui.mutate",
            "generated-image-text-edit".to_owned(),
            ElementTextPatch {
                selector: Some(
                    "capsem-media[data-capsem-artifact-id=\"generated-image-text-edit\"] >>> [data-capsem-node=\"generated-image-text-edit::caption\"]"
                        .to_owned(),
                ),
                host_selector: Some(
                    "capsem-media[data-capsem-artifact-id=\"generated-image-text-edit\"]"
                        .to_owned(),
                ),
                shadow_selector: Some(
                    "[data-capsem-node=\"generated-image-text-edit::caption\"]".to_owned(),
                ),
                text: "Sharper caption".to_owned(),
                source_request_seq: Some(18),
            },
        ))
        .expect("text patch");

    let element = &projector.projection().elements["generated-image-text-edit"];
    let patches = element.artifact.as_ref().expect("artifact").spec["textPatches"]
        .as_array()
        .expect("text patches");
    assert_eq!(patches.len(), 1);
    assert_eq!(
        patches[0]["shadowSelector"],
        "[data-capsem-node=\"generated-image-text-edit::caption\"]"
    );
    assert_eq!(patches[0]["text"], "Sharper caption");
    assert_eq!(element.provenance.updated_by, "local.ui.mutate");
    assert_eq!(element.provenance.last_verb, WorkspaceVerb::Patch);
    assert_eq!(deltas.len(), 1);
}

#[test]
fn selector_text_patch_rejects_empty_text_or_target_selector() {
    let mut projector = WorkspaceProjector::new();
    projector
        .apply(&artifact_record(
            1,
            "2026-05-31T00:00:00Z".to_owned(),
            "local.generate.image",
            WorkspaceVerb::Create,
            image("generated-image-text-deny", "Text Deny"),
        ))
        .expect("create");

    let empty_selector = projector
        .apply(&text_selector_patch_record(
            2,
            "2026-05-31T00:00:01Z".to_owned(),
            "local.ui.mutate",
            "generated-image-text-deny".to_owned(),
            ElementTextPatch {
                selector: None,
                host_selector: None,
                shadow_selector: None,
                text: "No selector".to_owned(),
                source_request_seq: None,
            },
        ))
        .expect_err("missing selector should fail");
    assert!(empty_selector.contains("selector"));

    let empty_text = projector
        .apply(&text_selector_patch_record(
            3,
            "2026-05-31T00:00:02Z".to_owned(),
            "local.ui.mutate",
            "generated-image-text-deny".to_owned(),
            ElementTextPatch {
                selector: None,
                host_selector: None,
                shadow_selector: Some(
                    "[data-capsem-node=\"generated-image-text-deny::caption\"]".to_owned(),
                ),
                text: " ".to_owned(),
                source_request_seq: None,
            },
        ))
        .expect_err("empty text should fail");
    assert!(empty_text.contains("text"));

    let unstable_selector = projector
        .apply(&text_selector_patch_record(
            4,
            "2026-05-31T00:00:03Z".to_owned(),
            "local.ui.mutate",
            "generated-image-text-deny".to_owned(),
            ElementTextPatch {
                selector: None,
                host_selector: None,
                shadow_selector: Some("[part~=\"body\"]".to_owned()),
                text: "Looks tempting but is not stable".to_owned(),
                source_request_seq: None,
            },
        ))
        .expect_err("unstable selector should fail");
    assert!(unstable_selector.contains("stable Capsem topology node"));
}

#[test]
fn light_dom_style_patch_can_target_shell_chrome() {
    let mut projector = WorkspaceProjector::new();
    projector
        .apply(&artifact_record(
            1,
            "2026-05-31T00:00:00Z".to_owned(),
            "local.generate.image",
            WorkspaceVerb::Create,
            image("generated-image-shell-style", "Shell Style"),
        ))
        .expect("create");

    projector
        .apply(&style_patch_record(
            2,
            "2026-05-31T00:00:01Z".to_owned(),
            "local.workspace.style",
            "generated-image-shell-style".to_owned(),
            ElementStylePatch {
                selector: Some(
                    "[data-capsem-element-id=\"generated-image-shell-style\"] [data-capsem-role=\"card-title\"]"
                        .to_owned(),
                ),
                host_selector: None,
                shadow_selector: None,
                styles: BTreeMap::from([("color".to_owned(), "var(--primary)".to_owned())]),
                source_request_seq: Some(4),
            },
        ))
        .expect("light DOM style patch");

    let element = &projector.projection().elements["generated-image-shell-style"];
    let patches = element.artifact.as_ref().expect("artifact").spec["stylePatches"]
        .as_array()
        .expect("style patches");
    assert_eq!(patches.len(), 1);
    assert_eq!(
        patches[0]["selector"],
        "[data-capsem-element-id=\"generated-image-shell-style\"] [data-capsem-role=\"card-title\"]"
    );
    assert!(patches[0].get("shadowSelector").is_none());
    assert_eq!(patches[0]["styles"]["color"], "var(--primary)");
}

#[test]
fn style_patch_rejects_unsupported_properties() {
    let mut projector = WorkspaceProjector::new();
    projector
        .apply(&artifact_record(
            1,
            "2026-05-31T00:00:00Z".to_owned(),
            "local.generate.image",
            WorkspaceVerb::Create,
            image("generated-image-style-deny", "Style Deny"),
        ))
        .expect("create");

    let error = projector
        .apply(&style_patch_record(
            2,
            "2026-05-31T00:00:01Z".to_owned(),
            "local.workspace.style",
            "generated-image-style-deny".to_owned(),
            ElementStylePatch {
                selector: Some(
                    "[data-capsem-element-id=\"generated-image-style-deny\"]".to_owned(),
                ),
                host_selector: None,
                shadow_selector: None,
                styles: BTreeMap::from([("position".to_owned(), "fixed".to_owned())]),
                source_request_seq: Some(4),
            },
        ))
        .expect_err("unsupported style should fail");

    assert!(error.contains("style property is not allowed"));
}

#[test]
fn style_patch_rejects_dangerous_values() {
    let mut projector = WorkspaceProjector::new();
    projector
        .apply(&artifact_record(
            1,
            "2026-05-31T00:00:00Z".to_owned(),
            "local.generate.image",
            WorkspaceVerb::Create,
            image("generated-image-style-danger", "Style Danger"),
        ))
        .expect("create");

    let error = projector
        .apply(&style_patch_record(
            2,
            "2026-05-31T00:00:01Z".to_owned(),
            "local.workspace.style",
            "generated-image-style-danger".to_owned(),
            ElementStylePatch {
                selector: Some(
                    "[data-capsem-element-id=\"generated-image-style-danger\"]".to_owned(),
                ),
                host_selector: None,
                shadow_selector: None,
                styles: BTreeMap::from([(
                    "backgroundColor".to_owned(),
                    "url(https://example.invalid/pixel)".to_owned(),
                )]),
                source_request_seq: Some(4),
            },
        ))
        .expect_err("dangerous style should fail");

    assert!(error.contains("style value is not allowed"));
}

#[test]
fn change_request_creates_durable_task_without_mutating_element() {
    let mut projector = WorkspaceProjector::new();
    projector
        .apply(&artifact_record(
            1,
            "2026-05-31T00:00:00Z".to_owned(),
            "local.generate.image",
            WorkspaceVerb::Create,
            image("generated-image-change-request", "Original"),
        ))
        .expect("create");

    let record = change_request_record(
        2,
        "2026-05-31T00:00:01Z".to_owned(),
        "chat.ui",
        "generated-image-change-request".to_owned(),
        "make the title sharper".to_owned(),
        Some(WorkspaceAnnotationTarget {
            kind: "cardTitle".to_owned(),
            label: "Card title".to_owned(),
            path: vec!["card".to_owned(), "title".to_owned()],
            topology_id: Some("generated-image-change-request::title".to_owned()),
            topology_role: Some("title".to_owned()),
            selector: Some("button:nth-child(1)".to_owned()),
            host_selector: None,
            shadow_selector: None,
            selector_verified: Some(true),
            metadata: Some(serde_json::json!({
                "tag": "button",
                "capsemNode": "1"
            })),
        }),
    );
    let request = change_request_from_record(&record).expect("typed request");
    let deltas = projector.apply(&record).expect("request");

    assert_eq!(request.instruction, "make the title sharper");
    assert_eq!(
        request.annotation.as_ref().expect("annotation").path,
        vec!["card", "title"]
    );
    assert_eq!(
        request
            .annotation
            .as_ref()
            .expect("annotation")
            .topology_id
            .as_deref(),
        Some("generated-image-change-request::title")
    );
    assert_eq!(
        request
            .annotation
            .as_ref()
            .expect("annotation")
            .topology_role
            .as_deref(),
        Some("title")
    );
    assert_eq!(projector.projection().seq, 2);
    assert_eq!(
        projector.projection().elements["generated-image-change-request"].title,
        "Original"
    );
    let task_id = task_id_from_record(&record);
    let task = &projector.projection().tasks[&task_id];
    assert_eq!(task.target, "generated-image-change-request");
    assert_eq!(task.instruction, "make the title sharper");
    assert_eq!(task.status, WorkspaceTaskStatus::Open);
    assert_eq!(task.created_by, "chat.ui");
    assert_eq!(task.source_record_id, "rec-2");
    assert_eq!(
        task.annotation.as_ref().expect("annotation").selector,
        Some("button:nth-child(1)".to_owned())
    );
    assert!(matches!(
        deltas.as_slice(),
        [RenderDelta::UpsertTask { id, task }] if id == &task_id
            && task.status == WorkspaceTaskStatus::Open
            && task.target == "generated-image-change-request"
    ));

    let checkpoint = projector.checkpoint("native-artifacts", "2026-05-31T00:00:02Z".to_owned());
    assert_eq!(
        checkpoint.projection.tasks[&task_id].instruction,
        "make the title sharper"
    );
}

#[test]
fn resolve_task_closes_task_without_mutating_element() {
    let mut projector = WorkspaceProjector::new();
    projector
        .apply(&artifact_record(
            1,
            "2026-05-31T00:00:00Z".to_owned(),
            "local.generate.image",
            WorkspaceVerb::Create,
            image("generated-image-resolve-request", "Original"),
        ))
        .expect("create");
    let request_record = change_request_record(
        2,
        "2026-05-31T00:00:01Z".to_owned(),
        "chat.ui",
        "generated-image-resolve-request".to_owned(),
        "make it quieter".to_owned(),
        None,
    );
    projector.apply(&request_record).expect("request");
    let task_id = task_id_from_record(&request_record);

    let deltas = projector
        .apply(&resolve_task_record(
            3,
            "2026-05-31T00:00:02Z".to_owned(),
            "assistant.ui",
            task_id.clone(),
        ))
        .expect("resolve");

    let task = &projector.projection().tasks[&task_id];
    assert_eq!(task.status, WorkspaceTaskStatus::Resolved);
    assert_eq!(task.updated_seq, 3);
    assert_eq!(task.updated_by, "assistant.ui");
    assert_eq!(task.resolved_seq, Some(3));
    assert_eq!(task.resolved_by.as_deref(), Some("assistant.ui"));
    assert_eq!(
        projector.projection().elements["generated-image-resolve-request"].title,
        "Original"
    );
    assert!(matches!(
        deltas.as_slice(),
        [RenderDelta::UpsertTask { id, task }] if id == &task_id
            && task.status == WorkspaceTaskStatus::Resolved
    ));
}

#[test]
fn resolve_missing_task_is_rejected() {
    let mut projector = WorkspaceProjector::new();
    let error = projector
        .apply(&resolve_task_record(
            1,
            "2026-05-31T00:00:00Z".to_owned(),
            "assistant.ui",
            "task-missing".to_owned(),
        ))
        .expect_err("missing task should fail");

    assert!(error.contains("missing task"));
}

#[test]
fn change_request_for_missing_element_is_rejected() {
    let mut projector = WorkspaceProjector::new();
    let error = projector
        .apply(&change_request_record(
            1,
            "2026-05-31T00:00:00Z".to_owned(),
            "chat.ui",
            "missing-element".to_owned(),
            "make it visible".to_owned(),
            None,
        ))
        .expect_err("missing target should fail");

    assert!(error.contains("missing element"));
}

#[test]
fn malformed_ui_change_payload_is_rejected() {
    let mut projector = WorkspaceProjector::new();
    projector
        .apply(&artifact_record(
            1,
            "2026-05-31T00:00:00Z".to_owned(),
            "local.generate.image",
            WorkspaceVerb::Create,
            image("generated-image-invalid-change", "Invalid Change"),
        ))
        .expect("create");

    let record = WorkspaceRecord {
        seq: 2,
        id: "rec-2".to_owned(),
        timestamp: "2026-05-31T00:00:01Z".to_owned(),
        role: WorkspaceRole::User,
        principal: "chat.ui".to_owned(),
        title: "Broken change".to_owned(),
        content_type: WorkspaceContentType::Action,
        verb: WorkspaceVerb::Request,
        status: WorkspaceStatus::Pending,
        target: Some("generated-image-invalid-change".to_owned()),
        content: WorkspaceContent::Action {
            name: "ui.change".to_owned(),
            payload: serde_json::json!({ "annotation": null }),
        },
    };
    let error = projector.apply(&record).expect_err("invalid payload");

    assert!(error.contains("invalid ui.change payload"));
}

#[test]
fn malformed_ui_resolve_payload_is_rejected() {
    let mut projector = WorkspaceProjector::new();
    let record = WorkspaceRecord {
        seq: 1,
        id: "rec-1".to_owned(),
        timestamp: "2026-05-31T00:00:00Z".to_owned(),
        role: WorkspaceRole::Assistant,
        principal: "assistant.ui".to_owned(),
        title: "Broken resolve".to_owned(),
        content_type: WorkspaceContentType::Action,
        verb: WorkspaceVerb::Respond,
        status: WorkspaceStatus::Complete,
        target: Some("task-1".to_owned()),
        content: WorkspaceContent::Action {
            name: "ui.resolve".to_owned(),
            payload: serde_json::json!({ "id": "task-1" }),
        },
    };
    let error = projector.apply(&record).expect_err("invalid payload");

    assert!(error.contains("invalid ui.resolve payload"));
}
