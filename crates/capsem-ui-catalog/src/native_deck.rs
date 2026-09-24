use std::collections::BTreeMap;

use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

use crate::contract_matrix::{
    GENERATED_MEDIA_API_COVERAGE, PLOTLY_CHART_API_COVERAGE, RICH_ARTIFACT_SCHEMA_COVERAGE,
};

pub const NATIVE_ARTIFACT_SCHEMA_ID: &str = "https://capsem.org/schemas/ui/native-artifact.v1.json";

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
    GeneratedText,
    GeneratedImage,
    GeneratedEmbedding,
    Sheet,
    Table,
    Chart,
    Diagram,
    Timeline,
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
    #[serde(default)]
    pub caption: Option<String>,
    #[serde(default = "default_gemini_provider")]
    pub provider: String,
    #[serde(default)]
    pub model: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GenerateTextRequest {
    pub id: String,
    pub title: String,
    pub prompt: String,
    #[serde(default)]
    pub system: Option<String>,
    #[serde(default = "default_gemini_provider")]
    pub provider: String,
    #[serde(default)]
    pub model: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GenerateEmbeddingRequest {
    pub id: String,
    pub title: String,
    pub input: Vec<String>,
    #[serde(default = "default_gemini_provider")]
    pub provider: String,
    #[serde(default)]
    pub model: Option<String>,
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
    #[serde(default)]
    pub x_unit: Option<String>,
    pub y_label: String,
    pub y_unit: String,
    #[serde(default = "default_chart_stack")]
    pub stack: ChartStackMode,
    #[serde(default = "default_chart_direction")]
    pub direction: ChartDirection,
    #[serde(default)]
    pub legend: Option<LegendPosition>,
    #[serde(default)]
    pub second_axis: Option<ChartAxis>,
    #[serde(default)]
    pub fit: Option<ChartFit>,
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
    ScatterPlot,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ChartStackMode {
    None,
    Stacked,
    Grouped,
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
pub struct ChartFit {
    pub method: ChartFitMethod,
    #[serde(default = "default_true")]
    pub display: bool,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ChartFitMethod {
    Linear,
    Logarithmic,
    MovingAverage,
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
pub struct TimelineRequest {
    pub id: String,
    pub title: String,
    pub lanes: Vec<TimelineLane>,
    pub events: Vec<TimelineEvent>,
    #[serde(default = "default_timeline_exports")]
    pub export: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TimelineLane {
    pub id: String,
    pub title: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TimelineEvent {
    pub id: String,
    pub title: String,
    pub lane: String,
    pub start: String,
    #[serde(default)]
    pub end: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
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
    checked_artifact(
        request.id,
        NativeArtifactKind::GeneratedImage,
        request.title,
        json!({
            "component": "capsem-media",
            "media": "image",
            "provider": request.provider,
            "model": request.model,
            "prompt": request.prompt,
            "caption": request.caption,
            "revisedPrompt": null,
            "usage": null,
            "cost": null,
            "durationMs": null,
            "status": "planned"
        }),
    )
}

pub fn generate_text(request: GenerateTextRequest) -> Result<NativeArtifact, String> {
    require_non_empty("id", &request.id)?;
    require_non_empty("title", &request.title)?;
    require_non_empty("prompt", &request.prompt)?;
    require_non_empty("provider", &request.provider)?;
    checked_artifact(
        request.id,
        NativeArtifactKind::GeneratedText,
        request.title,
        json!({
            "component": "capsem-text",
            "media": "text",
            "provider": request.provider,
            "model": request.model,
            "system": request.system,
            "prompt": request.prompt,
            "usage": null,
            "cost": null,
            "durationMs": null,
            "status": "planned"
        }),
    )
}

pub fn generate_embedding(request: GenerateEmbeddingRequest) -> Result<NativeArtifact, String> {
    require_non_empty("id", &request.id)?;
    require_non_empty("title", &request.title)?;
    require_non_empty("provider", &request.provider)?;
    if request.input.is_empty() {
        return Err("input must contain at least one item".to_owned());
    }
    for value in &request.input {
        require_non_empty("input", value)?;
    }
    checked_artifact(
        request.id,
        NativeArtifactKind::GeneratedEmbedding,
        request.title,
        json!({
            "component": "capsem-embedding",
            "media": "embedding",
            "provider": request.provider,
            "model": request.model,
            "input": request.input,
            "usage": null,
            "cost": null,
            "durationMs": null,
            "status": "planned"
        }),
    )
}

pub fn create_sheet(request: SheetRequest) -> Result<NativeArtifact, String> {
    require_non_empty("id", &request.id)?;
    require_non_empty("title", &request.title)?;
    require_columns(&request.columns)?;
    checked_artifact(
        request.id,
        NativeArtifactKind::Sheet,
        request.title,
        json!({
            "component": "capsem-sheet",
            "columns": request.columns,
            "rows": request.rows,
            "source": request.source
        }),
    )
}

pub fn create_table(request: TableRequest) -> Result<NativeArtifact, String> {
    require_non_empty("id", &request.id)?;
    require_non_empty("title", &request.title)?;
    require_non_empty("sourceArtifact", &request.source_artifact)?;
    require_columns(&request.columns)?;
    if request.page_size == 0 {
        return Err("pageSize must be greater than zero".to_owned());
    }
    checked_artifact(
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
    )
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
    checked_artifact(
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
            "xUnit": request.x_unit,
            "yLabel": request.y_label,
            "yUnit": request.y_unit,
            "stack": request.stack,
            "direction": request.direction,
            "legend": request.legend,
            "secondAxis": request.second_axis,
            "fit": request.fit,
            "export": request.export
        }),
    )
}

pub fn create_diagram(request: DiagramRequest) -> Result<NativeArtifact, String> {
    require_non_empty("id", &request.id)?;
    require_non_empty("title", &request.title)?;
    require_non_empty("source", &request.source)?;
    checked_artifact(
        request.id,
        NativeArtifactKind::Diagram,
        request.title,
        json!({
            "component": "capsem-diagram",
            "kind": request.kind,
            "source": request.source,
            "export": request.export
        }),
    )
}

pub fn create_timeline(request: TimelineRequest) -> Result<NativeArtifact, String> {
    require_non_empty("id", &request.id)?;
    require_non_empty("title", &request.title)?;
    if request.lanes.is_empty() {
        return Err("timeline lanes must not be empty".to_owned());
    }
    if request.events.is_empty() {
        return Err("timeline events must not be empty".to_owned());
    }
    for lane in &request.lanes {
        require_non_empty("lanes.id", &lane.id)?;
        require_non_empty("lanes.title", &lane.title)?;
    }
    for event in &request.events {
        require_non_empty("events.id", &event.id)?;
        require_non_empty("events.title", &event.title)?;
        require_non_empty("events.lane", &event.lane)?;
        require_non_empty("events.start", &event.start)?;
    }
    checked_artifact(
        request.id,
        NativeArtifactKind::Timeline,
        request.title,
        json!({
            "component": "capsem-timeline",
            "lanes": request.lanes,
            "events": request.events,
            "export": request.export
        }),
    )
}

pub fn create_slide(request: SlideRequest) -> Result<NativeArtifact, String> {
    require_non_empty("id", &request.id)?;
    require_non_empty("title", &request.title)?;
    if request.blocks.is_empty() {
        return Err("slide blocks must not be empty".to_owned());
    }
    checked_artifact(
        request.id,
        NativeArtifactKind::Slide,
        request.title,
        json!({
            "component": "capsem-slide",
            "blocks": request.blocks
        }),
    )
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
    let artifact = checked_artifact(
        &deck.id,
        NativeArtifactKind::SlideDeck,
        &deck.title,
        json!({
            "component": "capsem-slide-deck",
            "slides": deck.slides,
            "export": request.export
        }),
    )?;
    Ok((deck, artifact))
}

pub fn validate_native_artifact(artifact: &NativeArtifact) -> Result<(), String> {
    require_non_empty("artifact.id", &artifact.id)?;
    require_non_empty("artifact.title", &artifact.title)?;
    if !artifact.handle.starts_with("capsem://artifact/") {
        return Err("artifact handle must use capsem://artifact/".to_owned());
    }
    match artifact.kind {
        NativeArtifactKind::GeneratedText
        | NativeArtifactKind::GeneratedImage
        | NativeArtifactKind::GeneratedEmbedding => validate_generated_artifact(artifact),
        NativeArtifactKind::Sheet
        | NativeArtifactKind::Chart
        | NativeArtifactKind::Diagram
        | NativeArtifactKind::Timeline
        | NativeArtifactKind::Slide
        | NativeArtifactKind::SlideDeck => validate_rich_artifact(artifact),
        NativeArtifactKind::Table => validate_table_artifact(artifact),
    }
}

pub fn native_artifact_schema() -> Value {
    let chart_values: Vec<&str> = PLOTLY_CHART_API_COVERAGE
        .iter()
        .map(|entry| entry.chart)
        .collect();
    json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "$id": NATIVE_ARTIFACT_SCHEMA_ID,
        "title": "Capsem Native Artifact",
        "type": "object",
        "additionalProperties": false,
        "required": ["id", "kind", "title", "handle", "spec"],
        "properties": {
            "id": string_schema(),
            "kind": {
                "enum": [
                    "generatedText",
                    "generatedImage",
                    "generatedEmbedding",
                    "sheet",
                    "table",
                    "chart",
                    "diagram",
                    "timeline",
                    "slide",
                    "slideDeck"
                ]
            },
            "title": string_schema(),
            "handle": {"type": "string", "pattern": "^capsem://artifact/[0-9a-f]+$"},
            "spec": {"type": "object"}
        },
        "allOf": [
            kind_spec_schema("generatedText", generated_spec_schema("capsem-text", "text", &["prompt"])),
            kind_spec_schema("generatedImage", generated_spec_schema("capsem-media", "image", &["prompt", "revisedPrompt"])),
            kind_spec_schema("generatedEmbedding", generated_spec_schema("capsem-embedding", "embedding", &["input"])),
            kind_spec_schema("sheet", spec_schema("capsem-sheet", &["columns", "rows"], json!({
                "columns": non_empty_string_array_schema(),
                "rows": array_schema(),
                "source": {}
            }))),
            kind_spec_schema("table", spec_schema("capsem-table", &["sourceArtifact", "columns", "rows", "pageSize"], json!({
                "sourceArtifact": string_schema(),
                "columns": non_empty_string_array_schema(),
                "rows": array_schema(),
                "searchable": {"type": "boolean"},
                "filterable": {"type": "boolean"},
                "pageSize": {"type": "integer", "minimum": 1}
            }))),
            kind_spec_schema("chart", spec_schema("capsem-chart", &["chart", "sourceArtifact", "data", "x", "series", "xLabel", "yLabel", "yUnit"], json!({
                "chart": {"enum": chart_values},
                "sourceArtifact": string_schema(),
                "data": array_schema(),
                "x": string_schema(),
                "series": {"type": "array", "minItems": 1, "items": {"type": "object"}},
                "xLabel": string_schema(),
                "xUnit": {"type": ["string", "null"]},
                "yLabel": string_schema(),
                "yUnit": string_schema(),
                "stack": {"enum": ["none", "stacked", "grouped"]},
                "direction": {"enum": ["vertical", "horizontal"]},
                "legend": {"enum": ["top", "right", "bottom", "left", "none", null]},
                "secondAxis": {"type": ["object", "null"]},
                "fit": {"type": ["object", "null"]},
                "export": export_schema(&["png", "svg"])
            }))),
            kind_spec_schema("diagram", spec_schema("capsem-diagram", &["kind", "source"], json!({
                "kind": {"const": "mermaid"},
                "source": string_schema(),
                "export": export_schema(&["svg", "png"])
            }))),
            kind_spec_schema("timeline", spec_schema("capsem-timeline", &["lanes", "events"], json!({
                "lanes": {"type": "array", "minItems": 1, "items": {"type": "object"}},
                "events": {"type": "array", "minItems": 1, "items": {"type": "object"}},
                "export": export_schema(&["html", "png", "svg"])
            }))),
            kind_spec_schema("slide", spec_schema("capsem-slide", &["blocks"], json!({
                "blocks": {"type": "array", "minItems": 1, "items": {"type": "object"}}
            }))),
            kind_spec_schema("slideDeck", spec_schema("capsem-slide-deck", &["slides"], json!({
                "slides": {"type": "array", "minItems": 1, "items": {"type": "object"}},
                "export": export_schema(&["html", "pdf"])
            })))
        ]
    })
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
        x_unit: None,
        y_label: "Velocity".to_owned(),
        y_unit: "score".to_owned(),
        stack: ChartStackMode::None,
        direction: ChartDirection::Vertical,
        legend: None,
        second_axis: None,
        fit: None,
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
        x_unit: None,
        y_label: "Reliability".to_owned(),
        y_unit: "score".to_owned(),
        stack: ChartStackMode::None,
        direction: ChartDirection::Vertical,
        legend: Some(LegendPosition::Bottom),
        second_axis: Some(ChartAxis {
            label: "Risk".to_owned(),
            unit: "score".to_owned(),
        }),
        fit: None,
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
        model: None,
        prompt: "editorial fantasy cartography of five software houses in a luminous secure code kingdom, premium slide deck style, no text".to_owned(),
        caption: Some("Generated hero image for the deck proof.".to_owned()),
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
            model: None,
            prompt: prompt.to_owned(),
            caption: Some(format!("{title} generated house portrait.")),
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

fn checked_artifact(
    id: impl Into<String>,
    kind: NativeArtifactKind,
    title: impl Into<String>,
    spec: Value,
) -> Result<NativeArtifact, String> {
    let artifact = artifact(id, kind, title, spec);
    validate_native_artifact(&artifact)?;
    Ok(artifact)
}

fn string_schema() -> Value {
    json!({"type": "string", "minLength": 1})
}

fn array_schema() -> Value {
    json!({"type": "array"})
}

fn non_empty_string_array_schema() -> Value {
    json!({
        "type": "array",
        "minItems": 1,
        "items": string_schema()
    })
}

fn export_schema(formats: &[&str]) -> Value {
    json!({
        "type": "array",
        "items": {"enum": formats}
    })
}

fn kind_spec_schema(kind: &str, spec_schema: Value) -> Value {
    json!({
        "if": {
            "properties": {"kind": {"const": kind}},
            "required": ["kind"]
        },
        "then": {
            "properties": {"spec": spec_schema}
        }
    })
}

fn generated_spec_schema(component: &str, media: &str, fields: &[&str]) -> Value {
    let mut required = vec![
        "component".to_owned(),
        "media".to_owned(),
        "provider".to_owned(),
        "status".to_owned(),
        "usage".to_owned(),
        "cost".to_owned(),
        "durationMs".to_owned(),
    ];
    required.extend(fields.iter().map(|field| (*field).to_owned()));

    json!({
        "type": "object",
        "required": required,
        "properties": {
            "component": {"const": component},
            "media": {"const": media},
            "provider": string_schema(),
            "model": {"type": ["string", "null"]},
            "system": {"type": ["string", "null"]},
            "prompt": string_schema(),
            "caption": {"type": ["string", "null"]},
            "revisedPrompt": {"type": ["string", "null"]},
            "input": non_empty_string_array_schema(),
            "usage": {},
            "cost": {},
            "durationMs": {"type": ["integer", "number", "null"], "minimum": 0},
            "status": string_schema()
        }
    })
}

fn spec_schema(component: &str, fields: &[&str], properties: Value) -> Value {
    let mut required = vec!["component".to_owned()];
    required.extend(fields.iter().map(|field| (*field).to_owned()));

    let mut property_map = match properties {
        Value::Object(map) => map,
        _ => Map::new(),
    };
    property_map.insert("component".to_owned(), json!({"const": component}));

    json!({
        "type": "object",
        "required": required,
        "properties": property_map
    })
}

fn validate_generated_artifact(artifact: &NativeArtifact) -> Result<(), String> {
    let media = match artifact.kind {
        NativeArtifactKind::GeneratedText => "text",
        NativeArtifactKind::GeneratedImage => "image",
        NativeArtifactKind::GeneratedEmbedding => "embedding",
        _ => return Err("artifact is not generated media".to_owned()),
    };
    let Some(row) = GENERATED_MEDIA_API_COVERAGE
        .iter()
        .find(|entry| entry.media == media)
    else {
        return Err(format!("missing generated media matrix row for {media}"));
    };
    require_component(artifact, row.renderer)?;
    require_matrix_fields(artifact, row.required_fields)?;
    require_spec_keys(artifact, row.provenance_fields)?;
    require_spec_keys(artifact, row.telemetry_fields)?;
    require_non_empty_spec_string(artifact, "status")?;
    match artifact.kind {
        NativeArtifactKind::GeneratedEmbedding => {
            require_non_empty_array(artifact, "input")?;
        }
        NativeArtifactKind::GeneratedText | NativeArtifactKind::GeneratedImage => {
            require_non_empty_spec_string(artifact, "prompt")?;
        }
        _ => {}
    }
    Ok(())
}

fn validate_rich_artifact(artifact: &NativeArtifact) -> Result<(), String> {
    let Some(name) = rich_artifact_name(artifact.kind) else {
        return Err(format!(
            "{} has no rich artifact schema",
            artifact.kind.kind_name()
        ));
    };
    let Some(row) = RICH_ARTIFACT_SCHEMA_COVERAGE
        .iter()
        .find(|entry| entry.artifact == name)
    else {
        return Err(format!("missing rich artifact matrix row for {name}"));
    };
    require_component(artifact, expected_component_for(name)?)?;
    require_matrix_fields(artifact, row.required_fields)?;
    match artifact.kind {
        NativeArtifactKind::Sheet => {
            require_non_empty_array(artifact, "columns")?;
            require_array(artifact, "rows")?;
        }
        NativeArtifactKind::Chart => validate_chart_artifact(artifact)?,
        NativeArtifactKind::Diagram => validate_diagram_artifact(artifact)?,
        NativeArtifactKind::Timeline => validate_timeline_artifact(artifact)?,
        NativeArtifactKind::Slide => validate_slide_artifact(artifact)?,
        NativeArtifactKind::SlideDeck => validate_slide_deck_artifact(artifact)?,
        _ => {}
    }
    Ok(())
}

fn validate_table_artifact(artifact: &NativeArtifact) -> Result<(), String> {
    require_component(artifact, "capsem-table")?;
    for field in ["sourceArtifact", "columns", "rows", "pageSize"] {
        require_spec_field(artifact, field)?;
    }
    require_non_empty_spec_string(artifact, "sourceArtifact")?;
    require_non_empty_array(artifact, "columns")?;
    require_array(artifact, "rows")?;
    let page_size = artifact.spec["pageSize"]
        .as_u64()
        .ok_or_else(|| "table pageSize must be an integer".to_owned())?;
    if page_size == 0 {
        return Err("table pageSize must be greater than zero".to_owned());
    }
    Ok(())
}

fn validate_chart_artifact(artifact: &NativeArtifact) -> Result<(), String> {
    require_array(artifact, "data")?;
    require_non_empty_array(artifact, "series")?;
    let chart = require_non_empty_spec_string(artifact, "chart")?;
    let Some(row) = PLOTLY_CHART_API_COVERAGE
        .iter()
        .find(|entry| entry.chart == chart)
    else {
        return Err(format!("unsupported chart kind `{chart}`"));
    };
    require_matrix_fields(artifact, row.required_fields)?;
    if !row.supports_direction && artifact.spec["direction"] == "horizontal" {
        return Err(format!("{chart} does not support horizontal direction"));
    }
    let stack = artifact.spec["stack"].as_str().unwrap_or("none");
    if stack != "none" && !row.supports_stack {
        return Err(format!("{chart} does not support stack mode `{stack}`"));
    }
    if artifact
        .spec
        .get("fit")
        .is_some_and(|value| !value.is_null())
        && !row.supports_fit
    {
        return Err(format!("{chart} does not support fit metadata"));
    }
    if artifact
        .spec
        .get("secondAxis")
        .is_some_and(|value| !value.is_null())
        && !row.supports_second_axis
    {
        return Err(format!("{chart} does not support secondAxis"));
    }
    require_exports(artifact, row.export_formats)
}

fn validate_diagram_artifact(artifact: &NativeArtifact) -> Result<(), String> {
    let kind = require_non_empty_spec_string(artifact, "kind")?;
    if kind != "mermaid" {
        return Err(format!("unsupported diagram kind `{kind}`"));
    }
    require_non_empty_spec_string(artifact, "source")?;
    require_exports(artifact, &["svg", "png"])
}

fn validate_timeline_artifact(artifact: &NativeArtifact) -> Result<(), String> {
    let lanes = require_non_empty_array(artifact, "lanes")?;
    for (index, lane) in lanes.iter().enumerate() {
        require_json_string(lane, "id", &format!("timeline lane {index}"))?;
        require_json_string(lane, "title", &format!("timeline lane {index}"))?;
    }
    let events = require_non_empty_array(artifact, "events")?;
    for (index, event) in events.iter().enumerate() {
        require_json_string(event, "id", &format!("timeline event {index}"))?;
        require_json_string(event, "title", &format!("timeline event {index}"))?;
        require_json_string(event, "lane", &format!("timeline event {index}"))?;
        require_json_string(event, "start", &format!("timeline event {index}"))?;
    }
    require_exports(artifact, &["html", "png", "svg"])
}

fn validate_slide_artifact(artifact: &NativeArtifact) -> Result<(), String> {
    let blocks = require_non_empty_array(artifact, "blocks")?;
    for (index, block) in blocks.iter().enumerate() {
        let kind = block["kind"]
            .as_str()
            .ok_or_else(|| format!("slide block {index} kind must be a string"))?;
        match kind {
            "text" => {
                require_json_string(block, "title", &format!("slide block {index}"))?;
                require_json_string(block, "body", &format!("slide block {index}"))?;
            }
            "image" | "diagram" | "table" | "sheet" | "chart" => {
                require_json_string(block, "artifactId", &format!("slide block {index}"))?;
            }
            _ => return Err(format!("unsupported slide block kind `{kind}`")),
        }
    }
    Ok(())
}

fn validate_slide_deck_artifact(artifact: &NativeArtifact) -> Result<(), String> {
    let slides = require_non_empty_array(artifact, "slides")?;
    for (index, slide) in slides.iter().enumerate() {
        require_json_string(slide, "artifactId", &format!("slideDeck slide {index}"))?;
        require_json_string(slide, "title", &format!("slideDeck slide {index}"))?;
    }
    require_exports(artifact, &["html", "pdf"])
}

fn rich_artifact_name(kind: NativeArtifactKind) -> Option<&'static str> {
    match kind {
        NativeArtifactKind::Sheet => Some("sheet"),
        NativeArtifactKind::Chart => Some("chart"),
        NativeArtifactKind::Diagram => Some("diagram"),
        NativeArtifactKind::Timeline => Some("timeline"),
        NativeArtifactKind::Slide => Some("slide"),
        NativeArtifactKind::SlideDeck => Some("slideDeck"),
        _ => None,
    }
}

fn expected_component_for(artifact_name: &str) -> Result<&'static str, String> {
    match artifact_name {
        "sheet" => Ok("capsem-sheet"),
        "chart" => Ok("capsem-chart"),
        "diagram" => Ok("capsem-diagram"),
        "timeline" => Ok("capsem-timeline"),
        "slide" => Ok("capsem-slide"),
        "slideDeck" => Ok("capsem-slide-deck"),
        _ => Err(format!("no expected component for {artifact_name}")),
    }
}

fn require_component(artifact: &NativeArtifact, expected: &str) -> Result<(), String> {
    let actual = artifact.spec["component"]
        .as_str()
        .ok_or_else(|| format!("{} component must be a string", artifact.id))?;
    if actual != expected {
        return Err(format!(
            "{} component must be `{expected}`, got `{actual}`",
            artifact.id
        ));
    }
    Ok(())
}

fn require_matrix_fields(artifact: &NativeArtifact, fields: &[&str]) -> Result<(), String> {
    for field in fields {
        match *field {
            "id" => require_non_empty("id", &artifact.id)?,
            "title" => require_non_empty("title", &artifact.title)?,
            field => {
                require_spec_field(artifact, field)?;
            }
        }
    }
    Ok(())
}

fn require_spec_keys(artifact: &NativeArtifact, fields: &[&str]) -> Result<(), String> {
    for field in fields {
        if artifact.spec.get(*field).is_none() {
            return Err(format!("{} spec missing `{field}`", artifact.id));
        }
    }
    Ok(())
}

fn require_spec_field<'a>(artifact: &'a NativeArtifact, field: &str) -> Result<&'a Value, String> {
    let value = artifact
        .spec
        .get(field)
        .ok_or_else(|| format!("{} spec missing `{field}`", artifact.id))?;
    if value.is_null() {
        return Err(format!(
            "{} spec field `{field}` must not be null",
            artifact.id
        ));
    }
    Ok(value)
}

fn require_non_empty_spec_string<'a>(
    artifact: &'a NativeArtifact,
    field: &str,
) -> Result<&'a str, String> {
    let value = require_spec_field(artifact, field)?;
    let string = value
        .as_str()
        .ok_or_else(|| format!("{} spec field `{field}` must be a string", artifact.id))?;
    require_non_empty(field, string)?;
    Ok(string)
}

fn require_array<'a>(artifact: &'a NativeArtifact, field: &str) -> Result<&'a Vec<Value>, String> {
    let value = require_spec_field(artifact, field)?;
    value
        .as_array()
        .ok_or_else(|| format!("{} spec field `{field}` must be an array", artifact.id))
}

fn require_non_empty_array<'a>(
    artifact: &'a NativeArtifact,
    field: &str,
) -> Result<&'a Vec<Value>, String> {
    let values = require_array(artifact, field)?;
    if values.is_empty() {
        return Err(format!(
            "{} spec field `{field}` must not be empty",
            artifact.id
        ));
    }
    Ok(values)
}

fn require_json_string<'a>(value: &'a Value, field: &str, scope: &str) -> Result<&'a str, String> {
    let string = value[field]
        .as_str()
        .ok_or_else(|| format!("{scope} `{field}` must be a string"))?;
    require_non_empty(field, string)?;
    Ok(string)
}

fn require_exports(artifact: &NativeArtifact, allowed: &[&str]) -> Result<(), String> {
    let Some(export) = artifact.spec.get("export") else {
        return Ok(());
    };
    if export.is_null() {
        return Ok(());
    }
    let formats = export
        .as_array()
        .ok_or_else(|| format!("{} export must be an array", artifact.id))?;
    for format in formats {
        let Some(format) = format.as_str() else {
            return Err(format!("{} export entries must be strings", artifact.id));
        };
        if !allowed.contains(&format) {
            return Err(format!(
                "{} export format `{format}` is not allowed",
                artifact.id
            ));
        }
    }
    Ok(())
}

impl NativeArtifactKind {
    fn kind_name(self) -> &'static str {
        match self {
            NativeArtifactKind::GeneratedText => "generatedText",
            NativeArtifactKind::GeneratedImage => "generatedImage",
            NativeArtifactKind::GeneratedEmbedding => "generatedEmbedding",
            NativeArtifactKind::Sheet => "sheet",
            NativeArtifactKind::Table => "table",
            NativeArtifactKind::Chart => "chart",
            NativeArtifactKind::Diagram => "diagram",
            NativeArtifactKind::Timeline => "timeline",
            NativeArtifactKind::Slide => "slide",
            NativeArtifactKind::SlideDeck => "slideDeck",
        }
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

fn default_chart_stack() -> ChartStackMode {
    ChartStackMode::None
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

fn default_timeline_exports() -> Vec<String> {
    vec!["html".to_owned()]
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
