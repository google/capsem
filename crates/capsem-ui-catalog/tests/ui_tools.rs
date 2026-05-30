use std::{fs, path::Path};

use capsem_ui_catalog::ui_tools::{
    acceptance_program, check_template, run_tool_program, typed_ui_recipe_components, TemplateSpec,
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
fn ui_tools_build_card_and_inline_ask_drafts() {
    let result = run_tool_program(capsem_ui_catalog::ui_tools::UiToolProgram {
        calls: vec![
            capsem_ui_catalog::ui_tools::UiToolCall {
                tool: "ui.surface.create".to_owned(),
                args: json!({ "id": "agent-draft" }),
            },
            capsem_ui_catalog::ui_tools::UiToolCall {
                tool: "ui.card".to_owned(),
                args: json!({
                    "surfaceId": "agent-draft",
                    "id": "ship-card",
                    "title": "Release gate",
                    "description": "All plugin UI must pass through structured tools."
                }),
            },
            capsem_ui_catalog::ui_tools::UiToolCall {
                tool: "ui.surface.validate".to_owned(),
                args: json!({ "surfaceId": "agent-draft" }),
            },
            capsem_ui_catalog::ui_tools::UiToolCall {
                tool: "ui.ask".to_owned(),
                args: json!({
                    "surfaceId": "agent-draft",
                    "id": "approve-release",
                    "text": "Allow this plugin UI to render?",
                    "yes": "Allow",
                    "no": "Deny"
                }),
            },
            capsem_ui_catalog::ui_tools::UiToolCall {
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
        Some("ask")
    );
    let serialized = serde_json::to_string(&result.surfaces[0].messages).unwrap();
    assert!(serialized.contains("Allow this plugin UI"));
    assert!(!serialized.contains("\"component\":\"Modal\""));
    assert!(!serialized.contains("Open question"));
}

#[test]
fn ui_tools_preserve_alert_tone_and_card_link_contract_fields() {
    let alert = run_tool_program(capsem_ui_catalog::ui_tools::UiToolProgram {
        calls: vec![
            capsem_ui_catalog::ui_tools::UiToolCall {
                tool: "ui.surface.create".to_owned(),
                args: json!({ "id": "warning" }),
            },
            capsem_ui_catalog::ui_tools::UiToolCall {
                tool: "ui.alert".to_owned(),
                args: json!({
                    "surfaceId": "warning",
                    "message": "all your base beling to us",
                    "tone": "warning",
                    "variant": "soft"
                }),
            },
        ],
    });
    assert_eq!(
        alert.surfaces[0]
            .recipe
            .as_ref()
            .and_then(|recipe| recipe.tone.as_deref()),
        Some("warning")
    );

    let card = run_tool_program(capsem_ui_catalog::ui_tools::UiToolProgram {
        calls: vec![
            capsem_ui_catalog::ui_tools::UiToolCall {
                tool: "ui.surface.create".to_owned(),
                args: json!({ "id": "model" }),
            },
            capsem_ui_catalog::ui_tools::UiToolCall {
                tool: "ui.card".to_owned(),
                args: json!({
                    "surfaceId": "model",
                    "title": "model",
                    "description": "Gemini",
                    "imageUrl": "https://example.test/gemini.png",
                    "imageAlt": "Gemini product mark",
                    "linkLabel": "visit homepage",
                    "linkHref": "https://gemini.google.com/",
                    "actions": [
                        { "label": "Use model", "action": "model.use", "variant": "primary" }
                    ]
                }),
            },
        ],
    });
    let link = card.surfaces[0]
        .recipe
        .as_ref()
        .and_then(|recipe| recipe.link.as_ref())
        .expect("link survives");
    assert_eq!(link.label, "visit homepage");
    assert_eq!(link.href, "https://gemini.google.com/");

    let serialized = serde_json::to_string(&card.surfaces[0].messages).unwrap();
    assert!(
        !serialized.contains("A2UI Basic component"),
        "user-facing card output leaked internal protocol label: {serialized}"
    );
    assert!(serialized.contains("https://example.test/gemini.png"));
    assert!(serialized.contains("model.use"));
}

#[test]
fn ui_pack_01_lowers_notice_facts_and_inline_choice_ask() {
    let result = run_tool_program(capsem_ui_catalog::ui_tools::UiToolProgram {
        calls: vec![
            capsem_ui_catalog::ui_tools::UiToolCall {
                tool: "ui.surface.create".to_owned(),
                args: json!({ "id": "pack" }),
            },
            capsem_ui_catalog::ui_tools::UiToolCall {
                tool: "ui.notice".to_owned(),
                args: json!({
                    "surfaceId": "pack",
                    "id": "notice",
                    "title": "Review required",
                    "message": "This model call needs approval.",
                    "tone": "warning",
                    "actions": [{ "label": "Review", "action": "notice.review" }]
                }),
            },
            capsem_ui_catalog::ui_tools::UiToolCall {
                tool: "ui.facts".to_owned(),
                args: json!({
                    "surfaceId": "pack",
                    "id": "facts",
                    "title": "Repository",
                    "items": [
                        { "label": "Project", "value": "capsem" },
                        { "label": "Branch", "value": "main" }
                    ]
                }),
            },
            capsem_ui_catalog::ui_tools::UiToolCall {
                tool: "ui.ask".to_owned(),
                args: json!({
                    "surfaceId": "pack",
                    "id": "choice",
                    "title": "Policy decision",
                    "detail": "How should this call proceed?",
                    "buttonLabel": "Choose policy",
                    "choices": [
                        { "label": "Allow once", "action": "policy.allow_once", "variant": "primary" },
                        { "label": "Deny", "action": "policy.deny", "variant": "default" }
                    ]
                }),
            },
            capsem_ui_catalog::ui_tools::UiToolCall {
                tool: "ui.surface.validate".to_owned(),
                args: json!({ "surfaceId": "pack" }),
            },
        ],
    });

    assert!(result.ok, "{:#?}", result.observations);
    let serialized = serde_json::to_string(&result.surfaces[0].messages).unwrap();
    assert!(serialized.contains("Review required"));
    assert!(serialized.contains("Repository"));
    assert!(serialized.contains("Policy decision"));
    assert!(serialized.contains("How should this call proceed?"));
    assert!(serialized.contains("policy.allow_once"));
    assert!(!serialized.contains("Choose policy"));
    assert!(!serialized.contains("\"component\":\"Modal\""));
    assert!(!serialized.contains("<"));
}

#[test]
fn ui_ask_rejects_missing_prompt_and_extra_choices() {
    let missing_prompt = run_tool_program(capsem_ui_catalog::ui_tools::UiToolProgram {
        calls: vec![
            capsem_ui_catalog::ui_tools::UiToolCall {
                tool: "ui.surface.create".to_owned(),
                args: json!({ "id": "bad-ask" }),
            },
            capsem_ui_catalog::ui_tools::UiToolCall {
                tool: "ui.ask".to_owned(),
                args: json!({ "surfaceId": "bad-ask" }),
            },
        ],
    });
    assert!(!missing_prompt.ok);
    assert!(missing_prompt.observations.iter().any(|observation| {
        observation
            .errors
            .iter()
            .any(|error| error.contains("title or text is required"))
    }));

    let extra_choices = run_tool_program(capsem_ui_catalog::ui_tools::UiToolProgram {
        calls: vec![
            capsem_ui_catalog::ui_tools::UiToolCall {
                tool: "ui.surface.create".to_owned(),
                args: json!({ "id": "bad-choices" }),
            },
            capsem_ui_catalog::ui_tools::UiToolCall {
                tool: "ui.ask".to_owned(),
                args: json!({
                    "surfaceId": "bad-choices",
                    "title": "Policy decision",
                    "choices": [
                        { "label": "Allow", "action": "allow" },
                        { "label": "Deny", "action": "deny" },
                        { "label": "Details", "action": "details" }
                    ]
                }),
            },
        ],
    });
    assert!(!extra_choices.ok);
    assert!(extra_choices.observations.iter().any(|observation| {
        observation
            .errors
            .iter()
            .any(|error| error.contains("exactly two actions"))
    }));
}

#[test]
fn typed_ui_recipe_components_have_explicit_svelte_renderers() {
    let renderer = fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("ui-preview/src/A2Node.svelte"),
    )
    .expect("renderer exists");

    for component in typed_ui_recipe_components() {
        let recipe_check = format!("recipe?.component === \"{component}\"");
        assert!(
            renderer.contains(&recipe_check),
            "typed UI recipe component `{component}` has no explicit Svelte renderer branch"
        );
    }
}

#[test]
fn ui_table_lowers_to_conformant_a2ui_and_recipe() {
    let result = run_tool_program(capsem_ui_catalog::ui_tools::UiToolProgram {
        calls: vec![
            capsem_ui_catalog::ui_tools::UiToolCall {
                tool: "ui.surface.create".to_owned(),
                args: json!({ "id": "agent-draft" }),
            },
            capsem_ui_catalog::ui_tools::UiToolCall {
                tool: "ui.table".to_owned(),
                args: json!({
                    "surfaceId": "agent-draft",
                    "id": "houses",
                    "title": "Great Houses of Westeros",
                    "columns": ["House", "Motto", "Arms"],
                    "searchable": true,
                    "filterable": true,
                    "pageSize": 2,
                    "rows": [
                        ["Stark", "Winter Is Coming", "Direwolf"],
                        ["Lannister", "Hear Me Roar!", "Golden lion"],
                        ["Targaryen", "Fire and Blood", "Three-headed dragon"]
                    ]
                }),
            },
            capsem_ui_catalog::ui_tools::UiToolCall {
                tool: "ui.surface.validate".to_owned(),
                args: json!({ "surfaceId": "agent-draft" }),
            },
        ],
    });

    assert!(result.ok, "{:#?}", result.observations);
    let surface = result.surfaces.first().expect("surface exists");
    assert_eq!(
        surface
            .recipe
            .as_ref()
            .map(|recipe| recipe.component.as_str()),
        Some("table")
    );
    let table = surface
        .recipe
        .as_ref()
        .and_then(|recipe| recipe.table.as_ref())
        .expect("table options survive");
    assert!(table.searchable);
    assert!(table.filterable);
    assert_eq!(table.page_size, 2);
    let serialized = serde_json::to_string(&surface.messages).unwrap();
    assert!(serialized.contains("Winter Is Coming"));
    assert!(serialized.contains("Golden lion"));
    assert!(!serialized.contains("class"));
    assert!(!serialized.contains("<table"));
}

#[test]
fn ui_table_rejects_rows_with_wrong_cell_count() {
    let result = run_tool_program(capsem_ui_catalog::ui_tools::UiToolProgram {
        calls: vec![
            capsem_ui_catalog::ui_tools::UiToolCall {
                tool: "ui.surface.create".to_owned(),
                args: json!({ "id": "bad-table" }),
            },
            capsem_ui_catalog::ui_tools::UiToolCall {
                tool: "ui.table".to_owned(),
                args: json!({
                    "surfaceId": "bad-table",
                    "columns": ["House", "Motto", "Arms"],
                    "rows": [["Stark", "Winter Is Coming"]]
                }),
            },
        ],
    });

    assert!(!result.ok);
    assert!(result.observations.iter().any(|observation| {
        observation
            .errors
            .iter()
            .any(|error| error.contains("has 2 cells but columns has 3"))
    }));
}

#[test]
fn ui_tools_reject_raw_renderer_inputs() {
    let result = run_tool_program(capsem_ui_catalog::ui_tools::UiToolProgram {
        calls: vec![
            capsem_ui_catalog::ui_tools::UiToolCall {
                tool: "ui.surface.create".to_owned(),
                args: json!({ "id": "bad" }),
            },
            capsem_ui_catalog::ui_tools::UiToolCall {
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
        bindings: vec![capsem_ui_catalog::ui_tools::TemplateBinding {
            prop: "notAProp".to_owned(),
            kind: capsem_ui_catalog::ui_tools::TemplateBindingKind::Text,
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

fn check_template_dir(path: &str) -> capsem_ui_catalog::ui_tools::TemplateCheckReport {
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
