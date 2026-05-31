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
    let rows = query_house_rows()?;
    let sheet = artifact(
        "sheet-realms-of-code",
        NativeArtifactKind::Sheet,
        "Realms Of Code Sheet",
        json!({
            "component": "capsem-sheet",
            "columns": ["house", "motto", "armory", "domain", "velocity", "reliability", "risk"],
            "rows": rows.clone(),
            "source": {
                "kind": "sqlite",
                "query": "select house, motto, armory, domain, velocity, reliability, risk from code_houses order by house"
            }
        }),
    );
    let table = artifact(
        "table-house-overview",
        NativeArtifactKind::Table,
        "House Overview Table",
        json!({
            "component": "capsem-table",
            "sourceArtifact": sheet.id,
            "columns": ["house", "motto", "armory", "domain"],
            "rows": rows.clone(),
            "searchable": true,
            "filterable": true,
            "pageSize": 5
        }),
    );
    let velocity_chart = artifact(
        "chart-house-velocity",
        NativeArtifactKind::Chart,
        "House Delivery Velocity",
        json!({
            "component": "capsem-chart",
            "chart": "barChart",
            "sourceArtifact": "sheet-realms-of-code",
            "data": rows.clone(),
            "x": "house",
            "series": [{"name": "velocity", "field": "velocity"}],
            "xLabel": "House",
            "yLabel": "Velocity",
            "yUnit": "score",
            "stack": false,
            "direction": "vertical",
            "export": ["png", "svg"]
        }),
    );
    let balance_chart = artifact(
        "chart-house-balance",
        NativeArtifactKind::Chart,
        "Reliability And Risk Balance",
        json!({
            "component": "capsem-chart",
            "chart": "lineChart",
            "sourceArtifact": "sheet-realms-of-code",
            "data": rows.clone(),
            "x": "house",
            "series": [
                {"name": "reliability", "field": "reliability", "axis": "left"},
                {"name": "risk", "field": "risk", "axis": "right"}
            ],
            "xLabel": "House",
            "yLabel": "Reliability",
            "yUnit": "score",
            "secondAxis": {"label": "Risk", "unit": "score"},
            "legend": "bottom",
            "export": ["png", "svg"]
        }),
    );
    let house_map = artifact(
        "diagram-realm-map",
        NativeArtifactKind::Diagram,
        "Realm Map Diagram",
        json!({
            "component": "capsem-diagram",
            "kind": "mermaid",
            "source": "flowchart TB\n  Crown[The Realms of Code] --> Compiler[House Compiler]\n  Crown --> Runtime[House Runtime]\n  Crown --> Sandbox[House Sandbox]\n  Crown --> Telemetry[House Telemetry]\n  Crown --> Interface[House Interface]",
            "export": ["svg", "png"]
        }),
    );
    let workflow = artifact(
        "diagram-deck-workflow",
        NativeArtifactKind::Diagram,
        "Deck Build Workflow",
        json!({
            "component": "capsem-diagram",
            "kind": "mermaid",
            "source": "flowchart LR\n  Data[SQLite house data] --> Table[Overview table]\n  Data --> Charts[Charts]\n  Data --> Images[Generated house images]\n  Table --> Slides[One slide per house]\n  Charts --> Slides\n  Images --> Slides\n  Slides --> Deck[Capsem slide deck]",
            "export": ["svg", "png"]
        }),
    );
    let hero = artifact(
        "generated-image-hero",
        NativeArtifactKind::GeneratedImage,
        "The Realms Of Code Hero Image",
        json!({
            "component": "capsem-media",
            "media": "image",
            "provider": "gemini",
            "prompt": "editorial fantasy cartography of five software houses in a luminous secure code kingdom, premium slide deck style, no text",
            "status": "planned",
            "note": "Gemini call is wired in the generate spike; this artifact proves the typed handle path."
        }),
    );
    let house_images = house_image_artifacts();

    let intro_slide = artifact(
        "slide-intro",
        NativeArtifactKind::Slide,
        "The Realms Of Code",
        json!({
            "component": "capsem-slide",
            "blocks": [
                {"kind": "text", "title": "The Realms Of Code", "body": "A composed deck proving SQLite data, generated media, diagrams, charts, and typed slide blocks can travel through the same Capsem artifact lane."},
                {"kind": "image", "artifactId": hero.id},
                {"kind": "diagram", "artifactId": house_map.id}
            ]
        }),
    );
    let overview_slide = artifact(
        "slide-overview",
        NativeArtifactKind::Slide,
        "House Overview",
        json!({
            "component": "capsem-slide",
            "blocks": [
                {"kind": "table", "artifactId": table.id},
                {"kind": "sheet", "artifactId": sheet.id}
            ]
        }),
    );
    let charts_slide = artifact(
        "slide-metrics",
        NativeArtifactKind::Slide,
        "Realm Metrics",
        json!({
            "component": "capsem-slide",
            "blocks": [
                {"kind": "chart", "artifactId": velocity_chart.id},
                {"kind": "chart", "artifactId": balance_chart.id}
            ]
        }),
    );
    let workflow_slide = artifact(
        "slide-workflow",
        NativeArtifactKind::Slide,
        "Artifact Workflow",
        json!({
            "component": "capsem-slide",
            "blocks": [
                {"kind": "diagram", "artifactId": workflow.id}
            ]
        }),
    );
    let house_slides = house_slide_artifacts();

    let deck = SlideDeckSpec {
        id: "deck-realms-of-code".to_owned(),
        title: "The Realms Of Code".to_owned(),
        slides: [
            slide_ref(&intro_slide),
            slide_ref(&overview_slide),
            slide_ref(&charts_slide),
            slide_ref(&workflow_slide),
        ]
        .into_iter()
        .chain(house_slides.iter().map(slide_ref))
        .collect(),
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
        velocity_chart,
        balance_chart,
        house_map,
        workflow,
        intro_slide,
        overview_slide,
        charts_slide,
        workflow_slide,
        deck_artifact,
    ];
    artifacts.extend(house_images);
    artifacts.extend(house_slides);
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

fn query_house_rows() -> Result<Vec<BTreeMap<String, Value>>, String> {
    query_demo_sql(SqliteQueryRequest {
        sql: "select house, motto, armory, domain, velocity, reliability, risk from code_houses order by house".to_owned(),
    })
    .map(|response| response.rows)
}

fn demo_connection() -> Result<Connection, String> {
    let conn = Connection::open_in_memory().map_err(|error| error.to_string())?;
    conn.execute(
        "create table code_houses (
            house text primary key,
            motto text not null,
            armory text not null,
            domain text not null,
            velocity real not null,
            reliability real not null,
            risk real not null
        )",
        [],
    )
    .map_err(|error| error.to_string())?;
    for (house, motto, armory, domain, velocity, reliability, risk) in [
        (
            "House Compiler",
            "Types before triumph",
            "Silver parser helm over a crimson AST",
            "Language and contracts",
            74.0,
            92.0,
            18.0,
        ),
        (
            "House Runtime",
            "Fast paths pay their debts",
            "Black flame over a bronze event loop",
            "Execution and scheduling",
            88.0,
            83.0,
            29.0,
        ),
        (
            "House Sandbox",
            "No trust crosses the wall",
            "Iron gate around a white capability key",
            "Isolation and policy",
            62.0,
            96.0,
            12.0,
        ),
        (
            "House Telemetry",
            "What is measured is remembered",
            "Golden signal tower over a blue ledger",
            "Tracing and evidence",
            70.0,
            89.0,
            21.0,
        ),
        (
            "House Interface",
            "The user sees the realm",
            "Glass window over a green command ribbon",
            "UI and workflow",
            81.0,
            78.0,
            34.0,
        ),
    ] {
        conn.execute(
            "insert into code_houses (house, motto, armory, domain, velocity, reliability, risk) values (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![house, motto, armory, domain, velocity, reliability, risk],
        )
        .map_err(|error| error.to_string())?;
    }
    Ok(conn)
}

fn house_image_artifacts() -> Vec<NativeArtifact> {
    [
        (
            "generated-image-house-compiler",
            "House Compiler Image",
            "noble software house crest, silver parser helm over crimson abstract syntax tree, premium editorial fantasy portrait, no text",
        ),
        (
            "generated-image-house-runtime",
            "House Runtime Image",
            "noble software house crest, black flame over bronze event loop, fast execution energy, premium editorial fantasy portrait, no text",
        ),
        (
            "generated-image-house-sandbox",
            "House Sandbox Image",
            "noble software house crest, iron gate around white capability key, secure isolation fortress, premium editorial fantasy portrait, no text",
        ),
        (
            "generated-image-house-telemetry",
            "House Telemetry Image",
            "noble software house crest, golden signal tower over blue ledger, observability realm, premium editorial fantasy portrait, no text",
        ),
        (
            "generated-image-house-interface",
            "House Interface Image",
            "noble software house crest, glass window over green command ribbon, elegant user interface realm, premium editorial fantasy portrait, no text",
        ),
    ]
    .into_iter()
    .map(|(id, title, prompt)| {
        artifact(
            id,
            NativeArtifactKind::GeneratedImage,
            title,
            json!({
                "component": "capsem-media",
                "media": "image",
                "provider": "gemini",
                "prompt": prompt,
                "status": "planned"
            }),
        )
    })
    .collect()
}

fn house_slide_artifacts() -> Vec<NativeArtifact> {
    [
        (
            "slide-house-compiler",
            "House Compiler",
            "generated-image-house-compiler",
            "Types before triumph",
            "Owns the contract language, schema checks, and compiler discipline before anything reaches runtime.",
        ),
        (
            "slide-house-runtime",
            "House Runtime",
            "generated-image-house-runtime",
            "Fast paths pay their debts",
            "Owns callback execution, scheduling, and the hot path where plugin and UI artifacts must stay quick.",
        ),
        (
            "slide-house-sandbox",
            "House Sandbox",
            "generated-image-house-sandbox",
            "No trust crosses the wall",
            "Owns capability boundaries, process isolation, and the rule that only typed objects cross trust edges.",
        ),
        (
            "slide-house-telemetry",
            "House Telemetry",
            "generated-image-house-telemetry",
            "What is measured is remembered",
            "Owns traces, audit records, artifact handles, and performance evidence for every generated surface.",
        ),
        (
            "slide-house-interface",
            "House Interface",
            "generated-image-house-interface",
            "The user sees the realm",
            "Owns chat, side panels, decks, and the Capsem component renderer that makes model output visible.",
        ),
    ]
    .into_iter()
    .map(|(id, title, image_id, motto, body)| {
        artifact(
            id,
            NativeArtifactKind::Slide,
            title,
            json!({
                "component": "capsem-slide",
                "blocks": [
                    {"kind": "image", "artifactId": image_id},
                    {"kind": "text", "title": motto, "body": body}
                ]
            }),
        )
    })
    .collect()
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
