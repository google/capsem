use capsem_ui_catalog::native_deck::{
    demo_deck_proof, query_demo_sql, NativeArtifactKind, SqliteQueryRequest,
};
use std::collections::BTreeSet;

#[test]
fn demo_deck_materializes_artifacts_individually() {
    let proof = demo_deck_proof().expect("demo proof should build from SQLite data");

    assert!(proof.ok);
    assert_eq!(proof.summary.sqlite_rows, 4);
    assert!(proof.summary.required_artifacts_present);
    assert!(proof.summary.individual_artifact_count >= 8);

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
        sql: "select quarter, revenue from quarterly_metrics order by quarter".to_owned(),
    })
    .expect("select should be accepted");

    assert_eq!(response.columns, vec!["quarter", "revenue"]);
    assert_eq!(response.rows.len(), 4);

    let error = query_demo_sql(SqliteQueryRequest {
        sql: "delete from quarterly_metrics".to_owned(),
    })
    .expect_err("non-select SQL should be rejected");
    assert!(error.contains("SELECT"));
}
