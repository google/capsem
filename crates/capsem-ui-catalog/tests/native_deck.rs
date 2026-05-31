use capsem_ui_catalog::native_deck::{
    create_chart, create_slide, create_slide_deck, create_table, demo_deck_proof, generate_image,
    query_demo_sql, ChartDirection, ChartKind, ChartRequest, ChartSeries, GenerateImageRequest,
    NativeArtifactKind, SlideBlock, SlideDeckRequest, SlideRef, SlideRequest, SqliteQueryRequest,
    TableRequest,
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
        y_label: "Score".to_owned(),
        y_unit: "score".to_owned(),
        stack: false,
        direction: ChartDirection::Vertical,
        legend: None,
        second_axis: None,
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
