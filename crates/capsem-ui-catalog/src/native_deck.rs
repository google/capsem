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

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GenerateImageRequest {
    pub id: String,
    pub title: String,
    pub prompt: String,
    #[serde(default = "default_gemini_provider")]
    pub provider: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SheetRequest {
    pub id: String,
    pub title: String,
    pub columns: Vec<String>,
    pub rows: Vec<BTreeMap<String, Value>>,
    #[serde(default)]
    pub source: Option<Value>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TableRequest {
    pub id: String,
    pub title: String,
    pub source_artifact: String,
    pub columns: Vec<String>,
    pub rows: Vec<BTreeMap<String, Value>>,
    #[serde(default = "default_true")]
    pub searchable: bool,
    #[serde(default = "default_true")]
    pub filterable: bool,
    #[serde(default = "default_page_size")]
    pub page_size: usize,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChartRequest {
    pub id: String,
    pub title: String,
    pub chart: ChartKind,
    pub source_artifact: String,
    pub data: Vec<BTreeMap<String, Value>>,
    pub x: String,
    pub series: Vec<ChartSeries>,
    pub x_label: String,
    pub y_label: String,
    pub y_unit: String,
    #[serde(default)]
    pub stack: bool,
    #[serde(default = "default_chart_direction")]
    pub direction: ChartDirection,
    #[serde(default)]
    pub legend: Option<LegendPosition>,
    #[serde(default)]
    pub second_axis: Option<ChartAxis>,
    #[serde(default = "default_chart_exports")]
    pub export: Vec<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ChartKind {
    BarChart,
    LineChart,
    HeatmapChart,
    BoxPlot,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ChartDirection {
    Vertical,
    Horizontal,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum LegendPosition {
    Top,
    Right,
    Bottom,
    Left,
    None,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChartSeries {
    pub name: String,
    pub field: String,
    #[serde(default)]
    pub axis: Option<ChartSeriesAxis>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ChartSeriesAxis {
    Left,
    Right,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChartAxis {
    pub label: String,
    pub unit: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagramRequest {
    pub id: String,
    pub title: String,
    #[serde(default = "default_mermaid_diagram")]
    pub kind: DiagramKind,
    pub source: String,
    #[serde(default = "default_diagram_exports")]
    pub export: Vec<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum DiagramKind {
    Mermaid,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SlideRequest {
    pub id: String,
    pub title: String,
    pub blocks: Vec<SlideBlock>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum SlideBlock {
    Text {
        title: String,
        body: String,
    },
    Image {
        #[serde(rename = "artifactId")]
        artifact_id: String,
    },
    Diagram {
        #[serde(rename = "artifactId")]
        artifact_id: String,
    },
    Table {
        #[serde(rename = "artifactId")]
        artifact_id: String,
    },
    Sheet {
        #[serde(rename = "artifactId")]
        artifact_id: String,
    },
    Chart {
        #[serde(rename = "artifactId")]
        artifact_id: String,
    },
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SlideDeckRequest {
    pub id: String,
    pub title: String,
    pub slides: Vec<SlideRef>,
    #[serde(default = "default_deck_exports")]
    pub export: Vec<String>,
}

pub fn generate_image(request: GenerateImageRequest) -> Result<NativeArtifact, String> {
    require_non_empty("id", &request.id)?;
    require_non_empty("title", &request.title)?;
    require_non_empty("prompt", &request.prompt)?;
    require_non_empty("provider", &request.provider)?;
    Ok(artifact(
        request.id,
        NativeArtifactKind::GeneratedImage,
        request.title,
        json!({
            "component": "capsem-media",
            "media": "image",
            "provider": request.provider,
            "prompt": request.prompt,
            "status": "planned"
        }),
    ))
}

pub fn create_sheet(request: SheetRequest) -> Result<NativeArtifact, String> {
    require_non_empty("id", &request.id)?;
    require_non_empty("title", &request.title)?;
    require_columns(&request.columns)?;
    Ok(artifact(
        request.id,
        NativeArtifactKind::Sheet,
        request.title,
        json!({
            "component": "capsem-sheet",
            "columns": request.columns,
            "rows": request.rows,
            "source": request.source
        }),
    ))
}

pub fn create_table(request: TableRequest) -> Result<NativeArtifact, String> {
    require_non_empty("id", &request.id)?;
    require_non_empty("title", &request.title)?;
    require_non_empty("sourceArtifact", &request.source_artifact)?;
    require_columns(&request.columns)?;
    if request.page_size == 0 {
        return Err("pageSize must be greater than zero".to_owned());
    }
    Ok(artifact(
        request.id,
        NativeArtifactKind::Table,
        request.title,
        json!({
            "component": "capsem-table",
            "sourceArtifact": request.source_artifact,
            "columns": request.columns,
            "rows": request.rows,
            "searchable": request.searchable,
            "filterable": request.filterable,
            "pageSize": request.page_size
        }),
    ))
}

pub fn create_chart(request: ChartRequest) -> Result<NativeArtifact, String> {
    require_non_empty("id", &request.id)?;
    require_non_empty("title", &request.title)?;
    require_non_empty("sourceArtifact", &request.source_artifact)?;
    require_non_empty("x", &request.x)?;
    require_non_empty("xLabel", &request.x_label)?;
    require_non_empty("yLabel", &request.y_label)?;
    require_non_empty("yUnit", &request.y_unit)?;
    if request.series.is_empty() {
        return Err("chart series must not be empty".to_owned());
    }
    for series in &request.series {
        require_non_empty("series.name", &series.name)?;
        require_non_empty("series.field", &series.field)?;
    }
    Ok(artifact(
        request.id,
        NativeArtifactKind::Chart,
        request.title,
        json!({
            "component": "capsem-chart",
            "chart": request.chart,
            "sourceArtifact": request.source_artifact,
            "data": request.data,
            "x": request.x,
            "series": request.series,
            "xLabel": request.x_label,
            "yLabel": request.y_label,
            "yUnit": request.y_unit,
            "stack": request.stack,
            "direction": request.direction,
            "legend": request.legend,
            "secondAxis": request.second_axis,
            "export": request.export
        }),
    ))
}

pub fn create_diagram(request: DiagramRequest) -> Result<NativeArtifact, String> {
    require_non_empty("id", &request.id)?;
    require_non_empty("title", &request.title)?;
    require_non_empty("source", &request.source)?;
    Ok(artifact(
        request.id,
        NativeArtifactKind::Diagram,
        request.title,
        json!({
            "component": "capsem-diagram",
            "kind": request.kind,
            "source": request.source,
            "export": request.export
        }),
    ))
}

pub fn create_slide(request: SlideRequest) -> Result<NativeArtifact, String> {
    require_non_empty("id", &request.id)?;
    require_non_empty("title", &request.title)?;
    if request.blocks.is_empty() {
        return Err("slide blocks must not be empty".to_owned());
    }
    Ok(artifact(
        request.id,
        NativeArtifactKind::Slide,
        request.title,
        json!({
            "component": "capsem-slide",
            "blocks": request.blocks
        }),
    ))
}

pub fn create_slide_deck(
    request: SlideDeckRequest,
) -> Result<(SlideDeckSpec, NativeArtifact), String> {
    require_non_empty("id", &request.id)?;
    require_non_empty("title", &request.title)?;
    if request.slides.is_empty() {
        return Err("slideDeck slides must not be empty".to_owned());
    }
    for slide in &request.slides {
        require_non_empty("slides.artifactId", &slide.artifact_id)?;
        require_non_empty("slides.title", &slide.title)?;
    }
    let deck = SlideDeckSpec {
        id: request.id,
        title: request.title,
        slides: request.slides,
    };
    let artifact = artifact(
        &deck.id,
        NativeArtifactKind::SlideDeck,
        &deck.title,
        json!({
            "component": "capsem-slide-deck",
            "slides": deck.slides,
            "export": request.export
        }),
    );
    Ok((deck, artifact))
}

pub fn demo_deck_proof() -> Result<NativeDeckProof, String> {
    let rows = query_house_rows()?;
    let columns = vec![
        "house".to_owned(),
        "motto".to_owned(),
        "armory".to_owned(),
        "domain".to_owned(),
        "velocity".to_owned(),
        "reliability".to_owned(),
        "risk".to_owned(),
    ];
    let sheet = create_sheet(SheetRequest {
        id: "sheet-realms-of-code".to_owned(),
        title: "Realms Of Code Sheet".to_owned(),
        columns: columns.clone(),
        rows: rows.clone(),
        source: Some(json!({
            "kind": "sqlite",
            "query": "select house, motto, armory, domain, velocity, reliability, risk from code_houses order by house"
        })),
    })?;
    let table = create_table(TableRequest {
        id: "table-house-overview".to_owned(),
        title: "House Overview Table".to_owned(),
        source_artifact: sheet.id.clone(),
        columns: vec![
            "house".to_owned(),
            "motto".to_owned(),
            "armory".to_owned(),
            "domain".to_owned(),
        ],
        rows: rows.clone(),
        searchable: true,
        filterable: true,
        page_size: 5,
    })?;
    let velocity_chart = create_chart(ChartRequest {
        id: "chart-house-velocity".to_owned(),
        title: "House Delivery Velocity".to_owned(),
        chart: ChartKind::BarChart,
        source_artifact: sheet.id.clone(),
        data: rows.clone(),
        x: "house".to_owned(),
        series: vec![ChartSeries {
            name: "velocity".to_owned(),
            field: "velocity".to_owned(),
            axis: None,
        }],
        x_label: "House".to_owned(),
        y_label: "Velocity".to_owned(),
        y_unit: "score".to_owned(),
        stack: false,
        direction: ChartDirection::Vertical,
        legend: None,
        second_axis: None,
        export: default_chart_exports(),
    })?;
    let balance_chart = create_chart(ChartRequest {
        id: "chart-house-balance".to_owned(),
        title: "Reliability And Risk Balance".to_owned(),
        chart: ChartKind::LineChart,
        source_artifact: sheet.id.clone(),
        data: rows.clone(),
        x: "house".to_owned(),
        series: vec![
            ChartSeries {
                name: "reliability".to_owned(),
                field: "reliability".to_owned(),
                axis: Some(ChartSeriesAxis::Left),
            },
            ChartSeries {
                name: "risk".to_owned(),
                field: "risk".to_owned(),
                axis: Some(ChartSeriesAxis::Right),
            },
        ],
        x_label: "House".to_owned(),
        y_label: "Reliability".to_owned(),
        y_unit: "score".to_owned(),
        stack: false,
        direction: ChartDirection::Vertical,
        legend: Some(LegendPosition::Bottom),
        second_axis: Some(ChartAxis {
            label: "Risk".to_owned(),
            unit: "score".to_owned(),
        }),
        export: default_chart_exports(),
    })?;
    let house_map = create_diagram(DiagramRequest {
        id: "diagram-realm-map".to_owned(),
        title: "Realm Map Diagram".to_owned(),
        kind: DiagramKind::Mermaid,
        source: "flowchart TB\n  Crown[The Realms of Code] --> Compiler[House Compiler]\n  Crown --> Runtime[House Runtime]\n  Crown --> Sandbox[House Sandbox]\n  Crown --> Telemetry[House Telemetry]\n  Crown --> Interface[House Interface]".to_owned(),
        export: default_diagram_exports(),
    })?;
    let workflow = create_diagram(DiagramRequest {
        id: "diagram-deck-workflow".to_owned(),
        title: "Deck Build Workflow".to_owned(),
        kind: DiagramKind::Mermaid,
        source: "flowchart LR\n  Data[SQLite house data] --> Table[Overview table]\n  Data --> Charts[Charts]\n  Data --> Images[Generated house images]\n  Table --> Slides[One slide per house]\n  Charts --> Slides\n  Images --> Slides\n  Slides --> Deck[Capsem slide deck]".to_owned(),
        export: default_diagram_exports(),
    })?;
    let hero = generate_image(GenerateImageRequest {
        id: "generated-image-hero".to_owned(),
        title: "The Realms Of Code Hero Image".to_owned(),
        provider: default_gemini_provider(),
        prompt: "editorial fantasy cartography of five software houses in a luminous secure code kingdom, premium slide deck style, no text".to_owned(),
    })?;
    let house_images = house_image_artifacts()?;

    let intro_slide = create_slide(SlideRequest {
        id: "slide-intro".to_owned(),
        title: "The Realms Of Code".to_owned(),
        blocks: vec![
            SlideBlock::Text {
                title: "The Realms Of Code".to_owned(),
                body: "A composed deck proving SQLite data, generated media, diagrams, charts, and typed slide blocks can travel through the same Capsem artifact lane.".to_owned(),
            },
            SlideBlock::Image {
                artifact_id: hero.id.clone(),
            },
            SlideBlock::Diagram {
                artifact_id: house_map.id.clone(),
            },
        ],
    })?;
    let overview_slide = create_slide(SlideRequest {
        id: "slide-overview".to_owned(),
        title: "House Overview".to_owned(),
        blocks: vec![
            SlideBlock::Table {
                artifact_id: table.id.clone(),
            },
            SlideBlock::Sheet {
                artifact_id: sheet.id.clone(),
            },
        ],
    })?;
    let charts_slide = create_slide(SlideRequest {
        id: "slide-metrics".to_owned(),
        title: "Realm Metrics".to_owned(),
        blocks: vec![
            SlideBlock::Chart {
                artifact_id: velocity_chart.id.clone(),
            },
            SlideBlock::Chart {
                artifact_id: balance_chart.id.clone(),
            },
        ],
    })?;
    let workflow_slide = create_slide(SlideRequest {
        id: "slide-workflow".to_owned(),
        title: "Artifact Workflow".to_owned(),
        blocks: vec![SlideBlock::Diagram {
            artifact_id: workflow.id.clone(),
        }],
    })?;
    let house_slides = house_slide_artifacts()?;

    let deck_slides: Vec<SlideRef> = [
        slide_ref(&intro_slide),
        slide_ref(&overview_slide),
        slide_ref(&charts_slide),
        slide_ref(&workflow_slide),
    ]
    .into_iter()
    .chain(house_slides.iter().map(slide_ref))
    .collect();
    let (deck, deck_artifact) = create_slide_deck(SlideDeckRequest {
        id: "deck-realms-of-code".to_owned(),
        title: "The Realms Of Code".to_owned(),
        slides: deck_slides,
        export: default_deck_exports(),
    })?;

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

fn house_image_artifacts() -> Result<Vec<NativeArtifact>, String> {
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
        generate_image(GenerateImageRequest {
            id: id.to_owned(),
            title: title.to_owned(),
            provider: default_gemini_provider(),
            prompt: prompt.to_owned(),
        })
    })
    .collect()
}

fn house_slide_artifacts() -> Result<Vec<NativeArtifact>, String> {
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
        create_slide(SlideRequest {
            id: id.to_owned(),
            title: title.to_owned(),
            blocks: vec![
                SlideBlock::Image {
                    artifact_id: image_id.to_owned(),
                },
                SlideBlock::Text {
                    title: motto.to_owned(),
                    body: body.to_owned(),
                },
            ],
        })
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

fn require_non_empty(name: &str, value: &str) -> Result<(), String> {
    if value.trim().is_empty() {
        return Err(format!("{name} must not be empty"));
    }
    Ok(())
}

fn require_columns(columns: &[String]) -> Result<(), String> {
    if columns.is_empty() {
        return Err("columns must not be empty".to_owned());
    }
    for column in columns {
        require_non_empty("column", column)?;
    }
    Ok(())
}

fn default_true() -> bool {
    true
}

fn default_page_size() -> usize {
    10
}

fn default_gemini_provider() -> String {
    "gemini".to_owned()
}

fn default_chart_direction() -> ChartDirection {
    ChartDirection::Vertical
}

fn default_mermaid_diagram() -> DiagramKind {
    DiagramKind::Mermaid
}

fn default_chart_exports() -> Vec<String> {
    vec!["png".to_owned(), "svg".to_owned()]
}

fn default_diagram_exports() -> Vec<String> {
    vec!["svg".to_owned(), "png".to_owned()]
}

fn default_deck_exports() -> Vec<String> {
    vec!["html".to_owned(), "pdf".to_owned()]
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
