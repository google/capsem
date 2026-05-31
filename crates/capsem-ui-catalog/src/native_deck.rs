use std::collections::BTreeMap;

use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NativeDeckProof {
    pub ok: bool,
    pub title: String,
    pub summary: NativeDeckSummary,
    pub artifacts: Vec<NativeArtifact>,
    pub deck: SlideDeckSpec,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NativeDeckSummary {
    pub sqlite_rows: usize,
    pub chart_count: usize,
    pub individual_artifact_count: usize,
    pub required_artifacts_present: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NativeArtifact {
    pub id: String,
    pub kind: NativeArtifactKind,
    pub title: String,
    pub handle: String,
    pub spec: Value,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum NativeArtifactKind {
    GeneratedImage,
    Sheet,
    Table,
    Chart,
    Diagram,
    Slide,
    SlideDeck,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SlideDeckSpec {
    pub id: String,
    pub title: String,
    pub slides: Vec<SlideRef>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SlideRef {
    pub artifact_id: String,
    pub title: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SqliteQueryRequest {
    pub sql: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SqliteQueryResponse {
    pub ok: bool,
    pub columns: Vec<String>,
    pub rows: Vec<BTreeMap<String, Value>>,
}

pub fn demo_deck_proof() -> Result<NativeDeckProof, String> {
    let rows = query_demo_rows()?;
    let sheet = artifact(
        "sheet-quarterly-metrics",
        NativeArtifactKind::Sheet,
        "Quarterly Metrics Sheet",
        json!({
            "component": "capsem-sheet",
            "columns": ["quarter", "revenue", "margin", "riskScore"],
            "rows": rows,
            "source": {
                "kind": "sqlite",
                "query": "select quarter, revenue, margin, risk_score from quarterly_metrics order by quarter"
            }
        }),
    );
    let table = artifact(
        "table-quarterly-metrics",
        NativeArtifactKind::Table,
        "Quarterly Metrics Table",
        json!({
            "component": "capsem-table",
            "sourceArtifact": sheet.id,
            "searchable": true,
            "filterable": true,
            "pageSize": 4
        }),
    );
    let revenue_chart = artifact(
        "chart-revenue-by-quarter",
        NativeArtifactKind::Chart,
        "Revenue By Quarter",
        json!({
            "component": "capsem-chart",
            "chart": "barChart",
            "sourceArtifact": "sheet-quarterly-metrics",
            "x": "quarter",
            "series": [{"name": "revenue", "field": "revenue"}],
            "xLabel": "Quarter",
            "yLabel": "Revenue",
            "yUnit": "USDm",
            "stack": false,
            "direction": "vertical",
            "export": ["png", "svg"]
        }),
    );
    let margin_chart = artifact(
        "chart-margin-risk-trend",
        NativeArtifactKind::Chart,
        "Margin And Risk Trend",
        json!({
            "component": "capsem-chart",
            "chart": "lineChart",
            "sourceArtifact": "sheet-quarterly-metrics",
            "x": "quarter",
            "series": [
                {"name": "margin", "field": "margin", "axis": "left"},
                {"name": "riskScore", "field": "riskScore", "axis": "right"}
            ],
            "xLabel": "Quarter",
            "yLabel": "Margin",
            "yUnit": "%",
            "secondAxis": {"label": "Risk score", "unit": "score"},
            "legend": "bottom",
            "export": ["png", "svg"]
        }),
    );
    let diagram = artifact(
        "diagram-review-flow",
        NativeArtifactKind::Diagram,
        "Review Flow Diagram",
        json!({
            "component": "capsem-diagram",
            "kind": "mermaid",
            "source": "flowchart LR\n  A[SQLite data] --> B[Charts]\n  B --> C[Slides]\n  C --> D[Deck export]\n  D --> E[Capsem preview]",
            "export": ["svg", "png"]
        }),
    );
    let hero = artifact(
        "generated-image-hero",
        NativeArtifactKind::GeneratedImage,
        "Generated Hero Image",
        json!({
            "component": "capsem-media",
            "media": "image",
            "provider": "gemini",
            "prompt": "clean technical board-deck hero for a secure AI workspace",
            "status": "planned",
            "note": "Gemini call is wired in the generate spike; this artifact proves the typed handle path."
        }),
    );

    let intro_slide = artifact(
        "slide-intro",
        NativeArtifactKind::Slide,
        "Executive Intro",
        json!({
            "component": "capsem-slide",
            "blocks": [
                {"kind": "text", "title": "Capsem Native Artifact Proof", "body": "A staged deck built from typed data, media, charts, and diagrams."},
                {"kind": "image", "artifactId": hero.id}
            ]
        }),
    );
    let data_slide = artifact(
        "slide-data",
        NativeArtifactKind::Slide,
        "Data Foundation",
        json!({
            "component": "capsem-slide",
            "blocks": [
                {"kind": "sheet", "artifactId": sheet.id},
                {"kind": "table", "artifactId": table.id}
            ]
        }),
    );
    let charts_slide = artifact(
        "slide-charts",
        NativeArtifactKind::Slide,
        "Charts",
        json!({
            "component": "capsem-slide",
            "blocks": [
                {"kind": "chart", "artifactId": revenue_chart.id},
                {"kind": "chart", "artifactId": margin_chart.id}
            ]
        }),
    );
    let diagram_slide = artifact(
        "slide-diagram",
        NativeArtifactKind::Slide,
        "Flow",
        json!({
            "component": "capsem-slide",
            "blocks": [
                {"kind": "diagram", "artifactId": diagram.id}
            ]
        }),
    );

    let deck = SlideDeckSpec {
        id: "deck-native-artifact-proof".to_owned(),
        title: "Native Artifact Proof Deck".to_owned(),
        slides: vec![
            slide_ref(&intro_slide),
            slide_ref(&data_slide),
            slide_ref(&charts_slide),
            slide_ref(&diagram_slide),
        ],
    };
    let deck_artifact = artifact(
        &deck.id,
        NativeArtifactKind::SlideDeck,
        &deck.title,
        json!({
            "component": "capsem-slide-deck",
            "slides": deck.slides,
            "export": ["html", "pdf"]
        }),
    );

    let mut artifacts = vec![
        hero,
        sheet,
        table,
        revenue_chart,
        margin_chart,
        diagram,
        intro_slide,
        data_slide,
        charts_slide,
        diagram_slide,
        deck_artifact,
    ];
    artifacts.sort_by(|a, b| a.id.cmp(&b.id));

    let chart_count = artifacts
        .iter()
        .filter(|artifact| artifact.kind == NativeArtifactKind::Chart)
        .count();
    let required_artifacts_present = required_kinds()
        .iter()
        .all(|kind| artifacts.iter().any(|artifact| artifact.kind == *kind));
    Ok(NativeDeckProof {
        ok: chart_count >= 2 && required_artifacts_present,
        title: deck.title.clone(),
        summary: NativeDeckSummary {
            sqlite_rows: rows.len(),
            chart_count,
            individual_artifact_count: artifacts.len(),
            required_artifacts_present,
        },
        artifacts,
        deck,
    })
}

pub fn artifact_by_id(id: &str) -> Result<Option<NativeArtifact>, String> {
    Ok(demo_deck_proof()?
        .artifacts
        .into_iter()
        .find(|artifact| artifact.id == id))
}

pub fn query_demo_sql(request: SqliteQueryRequest) -> Result<SqliteQueryResponse, String> {
    if !request
        .sql
        .trim_start()
        .to_ascii_lowercase()
        .starts_with("select")
    {
        return Err("only read-only SELECT queries are allowed in the proof workspace".to_owned());
    }
    let conn = demo_connection()?;
    let mut statement = conn
        .prepare(&request.sql)
        .map_err(|error| error.to_string())?;
    let columns: Vec<String> = statement
        .column_names()
        .into_iter()
        .map(ToOwned::to_owned)
        .collect();
    let rows = statement
        .query_map([], |row| row_to_map(row, &columns))
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    Ok(SqliteQueryResponse {
        ok: true,
        columns,
        rows,
    })
}

fn query_demo_rows() -> Result<Vec<BTreeMap<String, Value>>, String> {
    query_demo_sql(SqliteQueryRequest {
        sql: "select quarter, revenue, margin, risk_score as riskScore from quarterly_metrics order by quarter".to_owned(),
    })
    .map(|response| response.rows)
}

fn demo_connection() -> Result<Connection, String> {
    let conn = Connection::open_in_memory().map_err(|error| error.to_string())?;
    conn.execute(
        "create table quarterly_metrics (
            quarter text primary key,
            revenue real not null,
            margin real not null,
            risk_score real not null
        )",
        [],
    )
    .map_err(|error| error.to_string())?;
    for (quarter, revenue, margin, risk_score) in [
        ("Q1", 12.4, 38.0, 42.0),
        ("Q2", 14.8, 41.5, 37.0),
        ("Q3", 16.2, 39.0, 31.0),
        ("Q4", 19.6, 44.2, 24.0),
    ] {
        conn.execute(
            "insert into quarterly_metrics (quarter, revenue, margin, risk_score) values (?1, ?2, ?3, ?4)",
            params![quarter, revenue, margin, risk_score],
        )
        .map_err(|error| error.to_string())?;
    }
    Ok(conn)
}

fn row_to_map(
    row: &rusqlite::Row<'_>,
    columns: &[String],
) -> Result<BTreeMap<String, Value>, rusqlite::Error> {
    let mut map = BTreeMap::new();
    for (index, column) in columns.iter().enumerate() {
        let value = row.get_ref(index)?;
        let value = match value {
            rusqlite::types::ValueRef::Null => Value::Null,
            rusqlite::types::ValueRef::Integer(value) => json!(value),
            rusqlite::types::ValueRef::Real(value) => json!(value),
            rusqlite::types::ValueRef::Text(value) => {
                json!(String::from_utf8_lossy(value).to_string())
            }
            rusqlite::types::ValueRef::Blob(value) => json!(format!("<{} bytes>", value.len())),
        };
        map.insert(column.clone(), value);
    }
    Ok(map)
}

fn artifact(
    id: impl Into<String>,
    kind: NativeArtifactKind,
    title: impl Into<String>,
    spec: Value,
) -> NativeArtifact {
    let id = id.into();
    let title = title.into();
    let hash_input = json!({
        "id": id,
        "kind": kind,
        "title": title,
        "spec": spec,
    });
    let bytes = serde_json::to_vec(&hash_input).expect("artifact hash input is serializable");
    let hash = blake3::hash(&bytes).to_hex().to_string();
    NativeArtifact {
        id,
        kind,
        title,
        handle: format!("capsem://artifact/{hash}"),
        spec,
    }
}

fn slide_ref(artifact: &NativeArtifact) -> SlideRef {
    SlideRef {
        artifact_id: artifact.id.clone(),
        title: artifact.title.clone(),
    }
}

fn required_kinds() -> &'static [NativeArtifactKind] {
    &[
        NativeArtifactKind::GeneratedImage,
        NativeArtifactKind::Sheet,
        NativeArtifactKind::Table,
        NativeArtifactKind::Chart,
        NativeArtifactKind::Diagram,
        NativeArtifactKind::Slide,
        NativeArtifactKind::SlideDeck,
    ]
}
