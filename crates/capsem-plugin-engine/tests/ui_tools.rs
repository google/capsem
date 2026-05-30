use std::{fs, path::Path};

use capsem_plugin_engine::ui_tools::{
    acceptance_program, check_template, run_tool_program, TemplateSpec,
};
use serde_json::json;

#[test]
fn ui_tool_acceptance_program_builds_renderable_surface() {
    let result = run_tool_program(acceptance_program());
    assert!(result.ok, "{:#?}", result.observations);
    assert_eq!(result.surfaces.len(), 1);
    assert_eq!(result.surfaces[0].surface_id, "tool-acceptance");
    assert!(result.surfaces[0].component_count >= 4);
    assert!(result
        .observations
        .iter()
        .any(|observation| observation.tool == "ui.surface.preview"
            && !observation.messages.is_empty()));
}

#[test]
fn ui_tools_build_card_and_modal_drafts() {
    let result = run_tool_program(capsem_plugin_engine::ui_tools::UiToolProgram {
        calls: vec![
            capsem_plugin_engine::ui_tools::UiToolCall {
                tool: "ui.surface.create".to_owned(),
                args: json!({ "id": "agent-draft" }),
            },
            capsem_plugin_engine::ui_tools::UiToolCall {
                tool: "ui.card".to_owned(),
                args: json!({
                    "surfaceId": "agent-draft",
                    "id": "ship-card",
                    "title": "Release gate",
                    "description": "All plugin UI must pass through structured tools."
                }),
            },
            capsem_plugin_engine::ui_tools::UiToolCall {
                tool: "ui.surface.validate".to_owned(),
                args: json!({ "surfaceId": "agent-draft" }),
            },
            capsem_plugin_engine::ui_tools::UiToolCall {
                tool: "ui.ask".to_owned(),
                args: json!({
                    "surfaceId": "agent-draft",
                    "id": "approve-release",
                    "text": "Allow this plugin UI to render?",
                    "yes": "Allow",
                    "no": "Deny"
                }),
            },
            capsem_plugin_engine::ui_tools::UiToolCall {
                tool: "ui.surface.preview".to_owned(),
                args: json!({ "surfaceId": "agent-draft" }),
            },
        ],
    });

    assert!(result.ok, "{:#?}", result.observations);
    assert_eq!(result.surfaces.len(), 1);
    assert_eq!(result.surfaces[0].surface_id, "agent-draft");
    assert_eq!(
        result.surfaces[0]
            .recipe
            .as_ref()
            .map(|recipe| recipe.component.as_str()),
        Some("modal")
    );
    assert!(result.surfaces[0]
        .messages
        .iter()
        .any(|message| serde_json::to_string(message)
            .unwrap()
            .contains("Allow this plugin UI")));
}

#[test]
fn ui_tools_reject_raw_renderer_inputs() {
    let result = run_tool_program(capsem_plugin_engine::ui_tools::UiToolProgram {
        calls: vec![
            capsem_plugin_engine::ui_tools::UiToolCall {
                tool: "ui.surface.create".to_owned(),
                args: json!({ "id": "bad" }),
            },
            capsem_plugin_engine::ui_tools::UiToolCall {
                tool: "ui.component.add".to_owned(),
                args: json!({
                    "surfaceId": "bad",
                    "component": {
                        "id": "root",
                        "component": "Text",
                        "text": "owned",
                        "class": "text-red-500"
                    }
                }),
            },
        ],
    });

    assert!(!result.ok);
    assert!(result.observations.iter().any(|observation| {
        observation
            .errors
            .iter()
            .any(|error| error.contains("raw html"))
    }));
}

#[test]
fn promoted_templates_match_their_binding_specs() {
    for path in [
        "templates/capsem-ui/Alert/soft",
        "templates/capsem-ui/Modal/basic",
        "templates/capsem-ui/Card/simple",
        "templates/capsem-ui/Button/primary",
        "templates/capsem-ui/Card/top-border",
    ] {
        let report = check_template_dir(path);
        assert!(report.ok, "{path}: {:#?}", report.errors);
    }
}

#[test]
fn template_checker_rejects_unknown_props() {
    let spec = TemplateSpec {
        schema: "capsem.ui-template.v1".to_owned(),
        component: "Alert".to_owned(),
        variant: "soft".to_owned(),
        structural: Vec::new(),
        bindings: vec![capsem_plugin_engine::ui_tools::TemplateBinding {
            prop: "notAProp".to_owned(),
            kind: capsem_plugin_engine::ui_tools::TemplateBindingKind::Text,
            selector: "[data-capui-text='message']".to_owned(),
        }],
    };
    let report = check_template(
        &spec,
        r#"<div data-capui-text="message">Security review required</div>"#,
    );
    assert!(!report.ok);
    assert!(report
        .errors
        .iter()
        .any(|error| error.contains("unknown binding prop")));
}

fn check_template_dir(path: &str) -> capsem_plugin_engine::ui_tools::TemplateCheckReport {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join(path);
    let spec: TemplateSpec = serde_json::from_str(
        &fs::read_to_string(root.join("template.capui.json")).expect("template spec exists"),
    )
    .expect("template spec parses");
    let html = fs::read_to_string(root.join("template.html")).expect("template html exists");
    check_template(&spec, &html)
}
