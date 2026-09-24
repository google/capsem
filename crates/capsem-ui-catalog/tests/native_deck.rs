use capsem_ui_catalog::native_deck::{
    create_chart, create_slide, create_slide_deck, create_table, create_timeline, demo_deck_proof,
    generate_image, native_artifact_schema, query_demo_sql, validate_native_artifact,
    ChartDirection, ChartKind, ChartRequest, ChartSeries, ChartStackMode, GenerateImageRequest,
    NativeArtifact, NativeArtifactKind, SlideBlock, SlideDeckRequest, SlideRef, SlideRequest,
    SqliteQueryRequest, TableRequest, TimelineEvent, TimelineLane, TimelineRequest,
    NATIVE_ARTIFACT_SCHEMA_ID,
};
use serde_json::json;
use std::collections::BTreeSet;

#[test]
fn demo_deck_materializes_artifacts_individually() {
    let proof = demo_deck_proof().expect("demo proof should build from SQLite data");

    assert!(proof.ok);
    assert_eq!(proof.title, "The Realms Of Code");
    assert_eq!(proof.summary.sqlite_rows, 5);
    assert!(proof.summary.required_artifacts_present);
    assert!(proof.summary.individual_artifact_count >= 18);
    assert!(proof
        .deck
        .slides
        .iter()
        .any(|slide| slide.title == "House Compiler"));

    let kinds: BTreeSet<NativeArtifactKind> = proof
        .artifacts
        .iter()
        .map(|artifact| artifact.kind)
        .collect();
    assert!(kinds.contains(&NativeArtifactKind::GeneratedImage));
    assert!(kinds.contains(&NativeArtifactKind::Sheet));
    assert!(kinds.contains(&NativeArtifactKind::Table));
    assert!(kinds.contains(&NativeArtifactKind::Chart));
    assert!(kinds.contains(&NativeArtifactKind::Diagram));
    assert!(kinds.contains(&NativeArtifactKind::Slide));
    assert!(kinds.contains(&NativeArtifactKind::SlideDeck));
}

#[test]
fn demo_deck_requires_multiple_charts() {
    let proof = demo_deck_proof().expect("demo proof should build");
    let charts: Vec<_> = proof
        .artifacts
        .iter()
        .filter(|artifact| artifact.kind == NativeArtifactKind::Chart)
        .collect();

    assert!(
        charts.len() >= 2,
        "deck proof must exercise more than one chart"
    );
    assert!(charts
        .iter()
        .any(|artifact| artifact.spec["chart"] == "barChart"));
    assert!(charts
        .iter()
        .any(|artifact| artifact.spec["chart"] == "lineChart"));
    assert!(charts
        .iter()
        .all(|artifact| artifact.spec["sourceArtifact"] == "sheet-realms-of-code"));
}

#[test]
fn demo_deck_has_one_slide_and_image_per_house() {
    let proof = demo_deck_proof().expect("demo proof should build");
    let expected_house_slide_ids: BTreeSet<_> = [
        "slide-house-compiler",
        "slide-house-runtime",
        "slide-house-sandbox",
        "slide-house-telemetry",
        "slide-house-interface",
    ]
    .into_iter()
    .collect();
    let house_slides: Vec<_> = proof
        .artifacts
        .iter()
        .filter(|artifact| {
            artifact.kind == NativeArtifactKind::Slide
                && expected_house_slide_ids.contains(artifact.id.as_str())
        })
        .collect();
    let house_images: Vec<_> = proof
        .artifacts
        .iter()
        .filter(|artifact| {
            artifact.kind == NativeArtifactKind::GeneratedImage
                && artifact.id.starts_with("generated-image-house-")
        })
        .collect();

    assert_eq!(house_slides.len(), 5);
    assert_eq!(house_images.len(), 5);
    for slide in house_slides {
        let blocks = slide.spec["blocks"].as_array().expect("slide blocks");
        assert!(blocks.iter().any(|block| block["kind"] == "image"));
        assert!(blocks.iter().any(|block| block["kind"] == "text"));
    }
}

#[test]
fn artifact_handles_are_stable_and_unique() {
    let proof = demo_deck_proof().expect("demo proof should build");
    let handles: BTreeSet<_> = proof
        .artifacts
        .iter()
        .map(|artifact| artifact.handle.as_str())
        .collect();

    assert_eq!(handles.len(), proof.artifacts.len());
    assert!(handles
        .iter()
        .all(|handle| handle.starts_with("capsem://artifact/")));
}

#[test]
fn demo_deck_artifacts_validate_against_runtime_contracts() {
    let proof = demo_deck_proof().expect("demo proof should build");

    for artifact in &proof.artifacts {
        validate_native_artifact(artifact).unwrap_or_else(|error| {
            panic!(
                "{} should satisfy the runtime artifact contract: {error}",
                artifact.id
            )
        });
    }
}

#[test]
fn native_artifact_schema_snapshot_matches_rust_contract() {
    let snapshot: serde_json::Value = serde_json::from_str(include_str!(
        "../../../schemas/capsem-ui/artifacts/native-artifact.v1.schema.json"
    ))
    .expect("schema snapshot is valid JSON");

    assert_eq!(native_artifact_schema(), snapshot);
    assert_eq!(snapshot["$id"], NATIVE_ARTIFACT_SCHEMA_ID);
}

#[test]
fn generated_media_artifacts_carry_planned_usage_and_cost_shape() {
    let image = generate_image(GenerateImageRequest {
        id: "generated-image-telemetry".to_owned(),
        title: "Generated Image Telemetry".to_owned(),
        prompt: "planned image".to_owned(),
        caption: Some("Telemetry fixture caption".to_owned()),
        provider: "gemini".to_owned(),
        model: None,
    })
    .expect("image artifact");

    assert_eq!(image.spec["status"], "planned");
    assert!(image.spec.get("usage").expect("usage field").is_null());
    assert!(image.spec.get("cost").expect("cost field").is_null());
    assert!(image
        .spec
        .get("durationMs")
        .expect("duration field")
        .is_null());
    assert!(image
        .spec
        .get("revisedPrompt")
        .expect("revised prompt field")
        .is_null());
    assert_eq!(image.spec["caption"], "Telemetry fixture caption");
    validate_native_artifact(&image).expect("generated image contract");
}

#[test]
fn timeline_artifact_validates_against_runtime_contract() {
    let artifact = create_timeline(TimelineRequest {
        id: "timeline-build-plan".to_owned(),
        title: "Build Plan Timeline".to_owned(),
        lanes: vec![
            TimelineLane {
                id: "compiler".to_owned(),
                title: "Compiler".to_owned(),
            },
            TimelineLane {
                id: "runtime".to_owned(),
                title: "Runtime".to_owned(),
            },
        ],
        events: vec![
            TimelineEvent {
                id: "contracts".to_owned(),
                title: "Freeze contracts".to_owned(),
                lane: "compiler".to_owned(),
                start: "2026-06-06".to_owned(),
                end: None,
                description: Some("Schema and validators land first.".to_owned()),
            },
            TimelineEvent {
                id: "renderer".to_owned(),
                title: "Render timeline".to_owned(),
                lane: "runtime".to_owned(),
                start: "2026-06-07".to_owned(),
                end: Some("2026-06-08".to_owned()),
                description: None,
            },
        ],
        export: vec!["html".to_owned(), "svg".to_owned()],
    })
    .expect("timeline artifact");

    assert_eq!(artifact.kind, NativeArtifactKind::Timeline);
    assert_eq!(artifact.spec["component"], "capsem-timeline");
    assert_eq!(artifact.spec["events"].as_array().expect("events").len(), 2);
    validate_native_artifact(&artifact).expect("timeline contract");
}

#[test]
fn validation_rejects_unsupported_plotly_chart_kind() {
    let artifact = NativeArtifact {
        id: "chart-unsupported".to_owned(),
        kind: NativeArtifactKind::Chart,
        title: "Unsupported Chart".to_owned(),
        handle: "capsem://artifact/test".to_owned(),
        spec: json!({
            "component": "capsem-chart",
            "chart": "pieChart",
            "sourceArtifact": "sheet-test",
            "data": [{"house": "Compiler", "score": 74}],
            "x": "house",
            "series": [{"name": "score", "field": "score"}],
            "xLabel": "House",
            "yLabel": "Score",
            "yUnit": "score",
            "stack": "none",
            "direction": "vertical",
            "export": ["png"]
        }),
    };

    let error = validate_native_artifact(&artifact).expect_err("pie chart is not in the matrix");
    assert!(error.contains("unsupported chart kind"));
}

#[test]
fn chart_constructor_rejects_matrix_incompatible_options() {
    let error = create_chart(ChartRequest {
        id: "line-chart-stacked".to_owned(),
        title: "Line Chart Stacked".to_owned(),
        chart: ChartKind::LineChart,
        source_artifact: "sheet-test".to_owned(),
        data: vec![std::collections::BTreeMap::from([
            ("house".to_owned(), json!("House Compiler")),
            ("score".to_owned(), json!(74.0)),
        ])],
        x: "house".to_owned(),
        series: vec![ChartSeries {
            name: "score".to_owned(),
            field: "score".to_owned(),
            axis: None,
        }],
        x_label: "House".to_owned(),
        x_unit: None,
        y_label: "Score".to_owned(),
        y_unit: "score".to_owned(),
        stack: ChartStackMode::Stacked,
        direction: ChartDirection::Vertical,
        legend: None,
        second_axis: None,
        fit: None,
        export: vec!["png".to_owned()],
    })
    .expect_err("line charts do not support stacked mode");

    assert!(error.contains("lineChart does not support stack mode"));
}

#[test]
fn sqlite_query_lane_is_read_only() {
    let response = query_demo_sql(SqliteQueryRequest {
        sql: "select house, motto from code_houses order by house".to_owned(),
    })
    .expect("select should be accepted");

    assert_eq!(response.columns, vec!["house", "motto"]);
    assert_eq!(response.rows.len(), 5);

    let error = query_demo_sql(SqliteQueryRequest {
        sql: "delete from quarterly_metrics".to_owned(),
    })
    .expect_err("non-select SQL should be rejected");
    assert!(error.contains("SELECT"));
}

#[test]
fn primitive_calls_compose_slide_deck_artifact() {
    let rows = vec![std::collections::BTreeMap::from([
        ("house".to_owned(), json!("House Compiler")),
        ("score".to_owned(), json!(74.0)),
    ])];
    let image = generate_image(GenerateImageRequest {
        id: "image-test".to_owned(),
        title: "Image Test".to_owned(),
        prompt: "test image".to_owned(),
        caption: Some("Image fixture caption".to_owned()),
        provider: "gemini".to_owned(),
        model: None,
    })
    .expect("image artifact");
    let table = create_table(TableRequest {
        id: "table-test".to_owned(),
        title: "Table Test".to_owned(),
        source_artifact: "sheet-test".to_owned(),
        columns: vec!["house".to_owned(), "score".to_owned()],
        rows: rows.clone(),
        searchable: true,
        filterable: true,
        page_size: 10,
    })
    .expect("table artifact");
    let chart = create_chart(ChartRequest {
        id: "chart-test".to_owned(),
        title: "Chart Test".to_owned(),
        chart: ChartKind::BarChart,
        source_artifact: "sheet-test".to_owned(),
        data: rows,
        x: "house".to_owned(),
        series: vec![ChartSeries {
            name: "score".to_owned(),
            field: "score".to_owned(),
            axis: None,
        }],
        x_label: "House".to_owned(),
        x_unit: None,
        y_label: "Score".to_owned(),
        y_unit: "score".to_owned(),
        stack: ChartStackMode::None,
        direction: ChartDirection::Vertical,
        legend: None,
        second_axis: None,
        fit: None,
        export: vec!["png".to_owned(), "svg".to_owned()],
    })
    .expect("chart artifact");
    let slide = create_slide(SlideRequest {
        id: "slide-test".to_owned(),
        title: "Slide Test".to_owned(),
        blocks: vec![
            SlideBlock::Image {
                artifact_id: image.id,
            },
            SlideBlock::Table {
                artifact_id: table.id,
            },
            SlideBlock::Chart {
                artifact_id: chart.id,
            },
        ],
    })
    .expect("slide artifact");
    let (_, deck) = create_slide_deck(SlideDeckRequest {
        id: "deck-test".to_owned(),
        title: "Deck Test".to_owned(),
        slides: vec![SlideRef {
            artifact_id: slide.id,
            title: slide.title,
        }],
        export: vec!["html".to_owned(), "pdf".to_owned()],
    })
    .expect("deck artifact");

    assert_eq!(deck.kind, NativeArtifactKind::SlideDeck);
    assert_eq!(deck.spec["component"], "capsem-slide-deck");
    assert_eq!(deck.spec["slides"].as_array().expect("slides").len(), 1);
}
