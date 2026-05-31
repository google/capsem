use capsem_ui_catalog::native_deck::{
    demo_deck_proof, query_demo_sql, NativeArtifactKind, SqliteQueryRequest,
};
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
