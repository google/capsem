use std::collections::{BTreeMap, BTreeSet};

use capsem_ui_catalog::contract_matrix::{
    Coverage, A2UI_BASIC_COVERAGE, CAPSEM_ARTIFACT_COVERAGE, CAPSEM_BLOCK_COVERAGE,
    GENERATED_MEDIA_API_COVERAGE, LOCAL_TOOL_COVERAGE, MERMAID_DIAGRAM_API_COVERAGE,
    PLOTLY_CHART_API_COVERAGE, RICH_ARTIFACT_SCHEMA_COVERAGE, TIMELINE_API_COVERAGE,
};
use capsem_ui_catalog::ui::{A2uiServerMessage, A2UI_BASIC_CATALOG_ID};
use capsem_ui_catalog::ui_tools::{
    run_tool_program, typed_ui_recipe_components, UiToolCall, UiToolProgram,
};
use serde_json::json;

#[test]
fn a2ui_basic_matrix_tracks_every_rust_component_variant() {
    let expected = BTreeSet::from([
        "Text",
        "Image",
        "Icon",
        "Video",
        "AudioPlayer",
        "Row",
        "Column",
        "List",
        "Card",
        "Tabs",
        "Modal",
        "Divider",
        "Button",
        "TextField",
        "CheckBox",
        "ChoicePicker",
        "Slider",
        "DateTimeInput",
    ]);
    let actual: BTreeSet<_> = A2UI_BASIC_COVERAGE
        .iter()
        .map(|entry| entry.component)
        .collect();

    assert_eq!(actual, expected);
    assert!(A2UI_BASIC_COVERAGE
        .iter()
        .all(|entry| !entry.component.trim().is_empty()));
}

#[test]
fn pack_01_blocks_are_shipped_helpers_with_renderer_topology_and_mutations() {
    let expected = BTreeSet::from(["alert", "ask", "card", "facts", "notice", "table"]);
    let actual: BTreeSet<_> = CAPSEM_BLOCK_COVERAGE
        .iter()
        .map(|entry| entry.block)
        .collect();

    assert_eq!(actual, expected);
    for entry in CAPSEM_BLOCK_COVERAGE {
        assert!(
            entry.api.starts_with("ui."),
            "{} must have a public ui.* helper",
            entry.block
        );
        assert_eq!(
            entry.schema, A2UI_BASIC_CATALOG_ID,
            "{} must declare the A2UI Basic wire schema",
            entry.block
        );
        assert!(
            entry.lowers_to_a2ui,
            "{} must lower through A2UI",
            entry.block
        );
        assert!(
            !entry.renderer_recipe.trim().is_empty(),
            "{} must name a renderer recipe",
            entry.block
        );
        assert!(
            entry.renderer_adapter.starts_with("A2Node."),
            "{} must name its trusted Svelte renderer adapter",
            entry.block
        );
        assert!(
            !entry.topology_targets.is_empty(),
            "{} must expose topology targets",
            entry.block
        );
        assert!(
            !entry.expected_component_suffixes.is_empty(),
            "{} must declare concrete emitted component ids",
            entry.block
        );
        assert!(
            !entry.mutations.is_empty(),
            "{} must declare typed mutation surface",
            entry.block
        );
    }
}

#[test]
fn pack_01_helpers_lower_to_wire_renderer_and_stable_component_ids() {
    for entry in CAPSEM_BLOCK_COVERAGE {
        let id = format!("contract-{}", entry.block);
        let result = run_tool_program(UiToolProgram {
            calls: vec![UiToolCall {
                tool: entry.api.to_owned(),
                args: sample_args(entry.block, &id),
            }],
        });

        assert!(result.ok, "{}: {:#?}", entry.api, result.observations);
        let surface = result.surfaces.first().expect("surface exists");
        let recipe = surface.recipe.as_ref().expect("recipe exists");
        assert_eq!(recipe.component, entry.block);
        let expected_recipe = format!("{}/{}", recipe.component, recipe.variant);
        assert_eq!(
            entry.renderer_recipe, expected_recipe,
            "{} renderer recipe must match the emitted recipe exactly",
            entry.block
        );
        assert_eq!(
            surface.catalog_id, entry.schema,
            "{} surface schema must match the matrix",
            entry.block
        );

        let components = surface_components(surface);
        for suffix in entry.expected_component_suffixes {
            let expected_id = expected_component_id(&id, suffix);
            assert!(
                components.contains(&expected_id),
                "{} must emit component id `{}`; emitted {:?}",
                entry.block,
                expected_id,
                components
            );
        }
    }
}

#[test]
fn renderer_recipe_registry_matches_declared_typed_renderers() {
    let mut declared: BTreeSet<_> = CAPSEM_BLOCK_COVERAGE
        .iter()
        .map(|entry| entry.block)
        .collect();
    declared.insert("modal");

    let registered: BTreeSet<_> = typed_ui_recipe_components().iter().copied().collect();
    assert_eq!(registered, declared);
}

fn sample_args(block: &str, id: &str) -> serde_json::Value {
    match block {
        "alert" => json!({
            "surfaceId": format!("{id}-surface"),
            "id": id,
            "message": "Security review required",
            "tone": "warning",
            "variant": "soft"
        }),
        "notice" => json!({
            "surfaceId": format!("{id}-surface"),
            "id": id,
            "title": "Review required",
            "message": "This model call needs approval.",
            "tone": "warning",
            "actions": [{ "label": "Review", "action": "notice.review" }]
        }),
        "card" => json!({
            "surfaceId": format!("{id}-surface"),
            "id": id,
            "title": "Model",
            "description": "Gemini model card",
            "imageUrl": "https://example.test/model.png",
            "imageAlt": "Model mark",
            "linkLabel": "visit homepage",
            "linkHref": "https://gemini.google.com/",
            "actions": [{ "label": "Use model", "action": "model.use", "variant": "primary" }]
        }),
        "facts" => json!({
            "surfaceId": format!("{id}-surface"),
            "id": id,
            "title": "Repository",
            "items": [
                { "label": "Project", "value": "capsem" },
                { "label": "Branch", "value": "main" }
            ]
        }),
        "table" => json!({
            "surfaceId": format!("{id}-surface"),
            "id": id,
            "title": "Great Houses",
            "columns": ["House", "Motto"],
            "rows": [
                ["Stark", "Winter Is Coming"],
                ["Lannister", "Hear Me Roar"]
            ],
            "searchable": true,
            "filterable": true,
            "pageSize": 2
        }),
        "ask" => json!({
            "surfaceId": format!("{id}-surface"),
            "id": id,
            "title": "Policy decision",
            "detail": "How should this call proceed?",
            "choices": [
                { "label": "Allow once", "action": "policy.allow_once", "variant": "primary" },
                { "label": "Deny", "action": "policy.deny", "variant": "default" }
            ]
        }),
        unknown => panic!("missing sample args for {unknown}"),
    }
}

fn surface_components(
    surface: &capsem_ui_catalog::ui_tools::UiToolSurfaceSnapshot,
) -> BTreeSet<String> {
    surface
        .messages
        .iter()
        .flat_map(|message| match message {
            A2uiServerMessage::UpdateComponents {
                update_components, ..
            } => update_components
                .components
                .iter()
                .map(|component| component.id().to_owned())
                .collect::<Vec<_>>(),
            _ => Vec::new(),
        })
        .collect()
}

fn expected_component_id(root_id: &str, suffix: &str) -> String {
    if suffix == "root" {
        "root".to_owned()
    } else {
        format!("{root_id}-{suffix}")
    }
}

#[test]
fn rich_artifact_matrix_covers_day_one_extension_families() {
    let expected = BTreeSet::from([
        "audio",
        "barChart",
        "boxPlot",
        "form",
        "generatedMediaCard",
        "heatmapChart",
        "image",
        "lineChart",
        "mermaidDiagram",
        "page",
        "pluginToolInstallationCard",
        "scatterPlot",
        "securityDecisionPanel",
        "sheet",
        "slide",
        "slideDeck",
        "spreadsheet",
        "timeline",
        "video",
        "website",
    ]);
    let actual: BTreeSet<_> = CAPSEM_ARTIFACT_COVERAGE
        .iter()
        .map(|entry| entry.artifact)
        .collect();

    assert_eq!(actual, expected);
    for entry in CAPSEM_ARTIFACT_COVERAGE {
        assert!(
            !entry.renderer.trim().is_empty(),
            "{} must declare renderer backend",
            entry.artifact
        );
        assert!(
            !entry.topology_targets.is_empty(),
            "{} must declare topology targets",
            entry.artifact
        );
        assert!(
            !entry.mutations.is_empty(),
            "{} must declare mutation surface",
            entry.artifact
        );
    }
}

#[test]
fn rich_artifact_schema_matrix_freezes_day_one_schema_families() {
    let expected = BTreeSet::from([
        "audio",
        "chart",
        "diagram",
        "form",
        "image",
        "page",
        "sheet",
        "slide",
        "slideDeck",
        "spreadsheet",
        "timeline",
        "video",
        "website",
    ]);
    let actual: BTreeSet<_> = RICH_ARTIFACT_SCHEMA_COVERAGE
        .iter()
        .map(|entry| entry.artifact)
        .collect();

    assert_eq!(actual, expected);
    for entry in RICH_ARTIFACT_SCHEMA_COVERAGE {
        assert!(
            entry.schema.starts_with("capsem.artifact.") && entry.schema.ends_with(".v1"),
            "{} must declare a versioned Capsem artifact schema",
            entry.artifact
        );
        assert!(
            entry.required_fields.contains(&"id") && entry.required_fields.contains(&"title"),
            "{} must require id and title",
            entry.artifact
        );
        assert!(
            !entry.renderer.trim().is_empty(),
            "{} must declare renderer backend",
            entry.artifact
        );
        assert!(
            !entry.topology_targets.is_empty(),
            "{} must declare topology targets",
            entry.artifact
        );
        assert!(
            !entry.mutations.is_empty(),
            "{} must declare mutation surface",
            entry.artifact
        );
    }
}

#[test]
fn rich_artifact_topology_snapshot_matches_matrix() {
    let snapshot: serde_json::Value = serde_json::from_str(include_str!(
        "../../../schemas/capsem-ui/artifacts/topology-targets.v1.json"
    ))
    .expect("topology snapshot JSON");
    let expected = json!({
        "$id": "https://capsem.org/schemas/ui/artifact-topology-targets.v1.json",
        "artifactTopologyTargets": RICH_ARTIFACT_SCHEMA_COVERAGE
            .iter()
            .map(|entry| (entry.artifact, entry.topology_targets))
            .collect::<BTreeMap<_, _>>()
    });

    assert_eq!(snapshot, expected);
}

#[test]
fn plotly_chart_api_matrix_freezes_science_chart_surface() {
    let expected = BTreeSet::from([
        "ui.chart.barChart",
        "ui.chart.boxPlot",
        "ui.chart.heatmap",
        "ui.chart.lineChart",
        "ui.chart.scatterPlot",
    ]);
    let actual: BTreeSet<_> = PLOTLY_CHART_API_COVERAGE
        .iter()
        .map(|entry| entry.api)
        .collect();

    assert_eq!(actual, expected);
    assert!(
        PLOTLY_CHART_API_COVERAGE
            .iter()
            .any(|entry| entry.supports_fit),
        "at least one chart family must support fit/trend metadata"
    );
    assert!(
        PLOTLY_CHART_API_COVERAGE
            .iter()
            .any(|entry| entry.supports_stack),
        "at least one chart family must support stacked/grouped bars"
    );
    assert!(
        PLOTLY_CHART_API_COVERAGE
            .iter()
            .any(|entry| entry.supports_second_axis),
        "at least one chart family must support double-axis charts"
    );
    for entry in PLOTLY_CHART_API_COVERAGE {
        assert!(entry.api.starts_with("ui.chart."));
        assert!(!entry.plotly_trace.trim().is_empty());
        assert!(entry.required_fields.contains(&"id"));
        assert!(entry.required_fields.contains(&"title"));
        assert!(entry.required_fields.contains(&"data"));
        assert!(entry.export_formats.contains(&"png"));
        assert!(entry.export_formats.contains(&"svg"));
    }
}

#[test]
fn diagram_and_timeline_api_matrices_freeze_structured_surfaces() {
    let diagram = MERMAID_DIAGRAM_API_COVERAGE
        .first()
        .expect("diagram API coverage");
    assert_eq!(diagram.api, "local.diagram.render");
    assert_eq!(diagram.renderer, "mermaid");
    assert!(diagram.required_fields.contains(&"source"));
    assert!(diagram.supported_diagrams.contains(&"flowchart"));
    assert!(diagram.supported_diagrams.contains(&"sequence"));
    assert!(diagram.export_formats.contains(&"svg"));
    assert_eq!(diagram.sandbox, "mermaid.securityLevel.strict");

    let timeline = TIMELINE_API_COVERAGE
        .first()
        .expect("timeline API coverage");
    assert_eq!(timeline.api, "local.ui.timeline");
    assert_eq!(timeline.renderer, "svelte/preline");
    assert!(timeline.required_fields.contains(&"lanes"));
    assert!(timeline.required_fields.contains(&"events"));
    assert!(timeline.event_fields.contains(&"lane"));
    assert!(timeline.event_fields.contains(&"start"));
    assert!(timeline.topology_targets.contains(&"event"));
    assert!(timeline.mutations.contains(&"events"));
}

#[test]
fn generated_media_api_matrix_freezes_provenance_and_render_paths() {
    let expected = BTreeSet::from([
        "local.generate.audio",
        "local.generate.embedding",
        "local.generate.image",
        "local.generate.text",
        "local.generate.video",
    ]);
    let actual: BTreeSet<_> = GENERATED_MEDIA_API_COVERAGE
        .iter()
        .map(|entry| entry.api)
        .collect();

    assert_eq!(actual, expected);
    for entry in GENERATED_MEDIA_API_COVERAGE {
        assert!(entry.api.starts_with("local.generate."));
        assert!(entry.required_fields.contains(&"id"));
        assert!(entry.required_fields.contains(&"title"));
        assert!(entry.provenance_fields.contains(&"provider"));
        assert!(entry.provenance_fields.contains(&"model"));
        assert!(entry.telemetry_fields.contains(&"usage"));
        assert!(entry.telemetry_fields.contains(&"cost"));
        assert!(!entry.renderer.trim().is_empty());
    }

    let explicit: BTreeSet<_> = GENERATED_MEDIA_API_COVERAGE
        .iter()
        .filter(|entry| entry.status == Coverage::Explicit)
        .map(|entry| entry.api)
        .collect();
    assert_eq!(
        explicit,
        BTreeSet::from([
            "local.generate.embedding",
            "local.generate.image",
            "local.generate.text"
        ])
    );
}

#[test]
fn local_tool_matrix_freezes_the_mcp_surface() {
    let expected = BTreeSet::from([
        "local.data.sheet",
        "local.data.spreadsheet",
        "local.data.sqlite",
        "local.diagram.render",
        "local.export.chart",
        "local.export.diagram",
        "local.export.pdf",
        "local.export.slideDeck",
        "local.export.spreadsheet",
        "local.generate.audio",
        "local.generate.embedding",
        "local.generate.image",
        "local.generate.text",
        "local.generate.video",
        "local.ui.comment",
        "local.ui.info",
        "local.ui.mutate",
        "local.ui.render",
        "local.ui.resolve",
        "local.ui.timeline",
        "local.web.preview",
        "local.website.form",
        "local.website.render",
        "local.workspace.checkpoint",
        "local.workspace.snapshot",
        "local.workspace.stream",
    ]);
    let actual: BTreeSet<_> = LOCAL_TOOL_COVERAGE.iter().map(|entry| entry.tool).collect();

    assert_eq!(actual, expected);
    assert!(LOCAL_TOOL_COVERAGE
        .iter()
        .all(|entry| entry.tool.starts_with("local.")));
}
