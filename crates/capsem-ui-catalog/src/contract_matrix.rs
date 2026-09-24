use serde::{Deserialize, Serialize};

use crate::ui::A2UI_BASIC_CATALOG_ID;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ContractPack {
    Pack01,
    Pack02,
    CapsemExtension,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Coverage {
    Explicit,
    Generic,
    Deferred,
    NotApplicable,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MutationCoverage {
    Typed,
    StructuralOnly,
    Deferred,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct A2uiPrimitiveCoverage {
    pub component: &'static str,
    pub rust_type: Coverage,
    pub helper: Option<&'static str>,
    pub renderer: Coverage,
    pub topology: Coverage,
    pub mutation: MutationCoverage,
    pub pack: ContractPack,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CapsemBlockCoverage {
    pub block: &'static str,
    pub api: &'static str,
    pub schema: &'static str,
    pub lowers_to_a2ui: bool,
    pub renderer_recipe: &'static str,
    pub renderer_adapter: &'static str,
    pub topology_targets: &'static [&'static str],
    pub expected_component_suffixes: &'static [&'static str],
    pub mutations: &'static [&'static str],
    pub pack: ContractPack,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArtifactCoverage {
    pub artifact: &'static str,
    pub renderer: &'static str,
    pub topology_targets: &'static [&'static str],
    pub mutations: &'static [&'static str],
    pub pack: ContractPack,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RichArtifactSchemaCoverage {
    pub artifact: &'static str,
    pub schema: &'static str,
    pub renderer: &'static str,
    pub required_fields: &'static [&'static str],
    pub optional_fields: &'static [&'static str],
    pub topology_targets: &'static [&'static str],
    pub mutations: &'static [&'static str],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlotlyChartApiCoverage {
    pub api: &'static str,
    pub chart: &'static str,
    pub plotly_trace: &'static str,
    pub required_fields: &'static [&'static str],
    pub supports_direction: bool,
    pub supports_stack: bool,
    pub supports_fit: bool,
    pub supports_second_axis: bool,
    pub export_formats: &'static [&'static str],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MermaidDiagramApiCoverage {
    pub api: &'static str,
    pub artifact: &'static str,
    pub renderer: &'static str,
    pub required_fields: &'static [&'static str],
    pub supported_diagrams: &'static [&'static str],
    pub export_formats: &'static [&'static str],
    pub sandbox: &'static str,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TimelineApiCoverage {
    pub api: &'static str,
    pub artifact: &'static str,
    pub renderer: &'static str,
    pub required_fields: &'static [&'static str],
    pub event_fields: &'static [&'static str],
    pub topology_targets: &'static [&'static str],
    pub mutations: &'static [&'static str],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GeneratedMediaApiCoverage {
    pub api: &'static str,
    pub artifact: &'static str,
    pub media: &'static str,
    pub renderer: &'static str,
    pub status: Coverage,
    pub required_fields: &'static [&'static str],
    pub provenance_fields: &'static [&'static str],
    pub telemetry_fields: &'static [&'static str],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalToolCoverage {
    pub tool: &'static str,
    pub purpose: &'static str,
    pub status: Coverage,
}

pub const A2UI_BASIC_COVERAGE: &[A2uiPrimitiveCoverage] = &[
    shipped("Text", None, Coverage::Generic, MutationCoverage::Typed),
    shipped("Image", None, Coverage::Generic, MutationCoverage::Typed),
    shipped("Icon", None, Coverage::Generic, MutationCoverage::Typed),
    deferred("Video"),
    deferred("AudioPlayer"),
    shipped(
        "Row",
        None,
        Coverage::Generic,
        MutationCoverage::StructuralOnly,
    ),
    shipped(
        "Column",
        None,
        Coverage::Generic,
        MutationCoverage::StructuralOnly,
    ),
    deferred("List"),
    shipped(
        "Card",
        Some("ui.card"),
        Coverage::Explicit,
        MutationCoverage::Typed,
    ),
    deferred("Tabs"),
    promoted_template("Modal"),
    shipped(
        "Divider",
        None,
        Coverage::Generic,
        MutationCoverage::StructuralOnly,
    ),
    shipped("Button", None, Coverage::Generic, MutationCoverage::Typed),
    deferred("TextField"),
    deferred("CheckBox"),
    deferred("ChoicePicker"),
    deferred("Slider"),
    deferred("DateTimeInput"),
];

pub const CAPSEM_BLOCK_COVERAGE: &[CapsemBlockCoverage] = &[
    CapsemBlockCoverage {
        block: "alert",
        api: "ui.alert",
        schema: A2UI_BASIC_CATALOG_ID,
        lowers_to_a2ui: true,
        renderer_recipe: "alert/soft",
        renderer_adapter: "A2Node.alert",
        topology_targets: &["root", "message"],
        expected_component_suffixes: &["root", "row", "icon", "text"],
        mutations: &["tone", "message"],
        pack: ContractPack::Pack01,
    },
    CapsemBlockCoverage {
        block: "notice",
        api: "ui.notice",
        schema: A2UI_BASIC_CATALOG_ID,
        lowers_to_a2ui: true,
        renderer_recipe: "notice/basic",
        renderer_adapter: "A2Node.notice",
        topology_targets: &["root", "title", "message", "actions"],
        expected_component_suffixes: &[
            "root",
            "body",
            "title",
            "message",
            "actions",
            "action-0",
            "action-0-text",
        ],
        mutations: &["title", "message", "tone", "actions"],
        pack: ContractPack::Pack01,
    },
    CapsemBlockCoverage {
        block: "card",
        api: "ui.card",
        schema: A2UI_BASIC_CATALOG_ID,
        lowers_to_a2ui: true,
        renderer_recipe: "card/simple",
        renderer_adapter: "A2Node.card",
        topology_targets: &["root", "image", "title", "description", "actions", "link"],
        expected_component_suffixes: &[
            "root",
            "body",
            "image",
            "title",
            "description",
            "actions",
            "action-0",
            "action-0-text",
        ],
        mutations: &["title", "description", "image", "link", "actions"],
        pack: ContractPack::Pack01,
    },
    CapsemBlockCoverage {
        block: "facts",
        api: "ui.facts",
        schema: A2UI_BASIC_CATALOG_ID,
        lowers_to_a2ui: true,
        renderer_recipe: "facts/basic",
        renderer_adapter: "A2Node.facts",
        topology_targets: &["root", "title", "fact.label", "fact.value"],
        expected_component_suffixes: &[
            "root",
            "body",
            "title",
            "row-0",
            "row-0-label",
            "row-0-value",
        ],
        mutations: &["title", "items"],
        pack: ContractPack::Pack01,
    },
    CapsemBlockCoverage {
        block: "table",
        api: "ui.table",
        schema: A2UI_BASIC_CATALOG_ID,
        lowers_to_a2ui: true,
        renderer_recipe: "table/basic",
        renderer_adapter: "A2Node.table",
        topology_targets: &["root", "title", "header.cell", "row", "cell", "controls"],
        expected_component_suffixes: &[
            "root",
            "table",
            "title",
            "header",
            "header-cell-0",
            "row-0",
            "row-0-cell-0",
        ],
        mutations: &[
            "title",
            "columns",
            "rows",
            "searchable",
            "filterable",
            "pageSize",
        ],
        pack: ContractPack::Pack01,
    },
    CapsemBlockCoverage {
        block: "ask",
        api: "ui.ask",
        schema: A2UI_BASIC_CATALOG_ID,
        lowers_to_a2ui: true,
        renderer_recipe: "ask/inline",
        renderer_adapter: "A2Node.ask",
        topology_targets: &["root", "title", "detail", "choice"],
        expected_component_suffixes: &[
            "root",
            "content",
            "title",
            "detail",
            "actions",
            "action-0",
            "action-0-text",
        ],
        mutations: &["title", "detail", "choices"],
        pack: ContractPack::Pack01,
    },
];

pub const CAPSEM_ARTIFACT_COVERAGE: &[ArtifactCoverage] = &[
    ArtifactCoverage {
        artifact: "barChart",
        renderer: "plotly",
        topology_targets: CHART_TARGETS,
        mutations: CHART_MUTATIONS,
        pack: ContractPack::CapsemExtension,
    },
    ArtifactCoverage {
        artifact: "lineChart",
        renderer: "plotly",
        topology_targets: CHART_TARGETS,
        mutations: CHART_MUTATIONS,
        pack: ContractPack::CapsemExtension,
    },
    ArtifactCoverage {
        artifact: "heatmapChart",
        renderer: "plotly",
        topology_targets: CHART_TARGETS,
        mutations: CHART_MUTATIONS,
        pack: ContractPack::CapsemExtension,
    },
    ArtifactCoverage {
        artifact: "boxPlot",
        renderer: "plotly",
        topology_targets: CHART_TARGETS,
        mutations: CHART_MUTATIONS,
        pack: ContractPack::CapsemExtension,
    },
    ArtifactCoverage {
        artifact: "scatterPlot",
        renderer: "plotly",
        topology_targets: CHART_TARGETS,
        mutations: CHART_MUTATIONS,
        pack: ContractPack::CapsemExtension,
    },
    ArtifactCoverage {
        artifact: "mermaidDiagram",
        renderer: "mermaid",
        topology_targets: &["title", "source", "node", "edge"],
        mutations: &["title", "source"],
        pack: ContractPack::CapsemExtension,
    },
    ArtifactCoverage {
        artifact: "timeline",
        renderer: "svelte/preline",
        topology_targets: &["title", "lane", "event", "date", "annotation"],
        mutations: &["title", "lanes", "events"],
        pack: ContractPack::CapsemExtension,
    },
    ArtifactCoverage {
        artifact: "spreadsheet",
        renderer: "svelte/grid",
        topology_targets: &[
            "workbook",
            "sheetTab",
            "cell",
            "range",
            "formula",
            "embeddedArtifact",
        ],
        mutations: &["title", "sheets", "cells", "ranges"],
        pack: ContractPack::CapsemExtension,
    },
    ArtifactCoverage {
        artifact: "sheet",
        renderer: "svelte/grid",
        topology_targets: &["sheet", "cell", "range", "row", "column"],
        mutations: &["name", "cells", "rows", "columns"],
        pack: ContractPack::CapsemExtension,
    },
    ArtifactCoverage {
        artifact: "slideDeck",
        renderer: "svelte/deck",
        topology_targets: &["deckTitle", "slide"],
        mutations: &["title", "slides", "order"],
        pack: ContractPack::CapsemExtension,
    },
    ArtifactCoverage {
        artifact: "slide",
        renderer: "svelte/deck",
        topology_targets: &[
            "slideTitle",
            "region",
            "textBlock",
            "image",
            "chart",
            "diagram",
        ],
        mutations: &["title", "regions", "blocks"],
        pack: ContractPack::CapsemExtension,
    },
    ArtifactCoverage {
        artifact: "website",
        renderer: "svelte/preline",
        topology_targets: &["siteTitle", "page", "navigation"],
        mutations: &["title", "pages", "navigation"],
        pack: ContractPack::CapsemExtension,
    },
    ArtifactCoverage {
        artifact: "page",
        renderer: "svelte/preline",
        topology_targets: &["pageTitle", "section", "block", "action"],
        mutations: &["title", "sections", "blocks"],
        pack: ContractPack::CapsemExtension,
    },
    ArtifactCoverage {
        artifact: "form",
        renderer: "a2ui/preline",
        topology_targets: &["form", "field", "label", "validation", "action"],
        mutations: &["title", "fields", "labels", "validation", "actions"],
        pack: ContractPack::CapsemExtension,
    },
    ArtifactCoverage {
        artifact: "image",
        renderer: "capsem-media",
        topology_targets: &["title", "media", "caption", "source"],
        mutations: &["title", "caption", "source"],
        pack: ContractPack::CapsemExtension,
    },
    ArtifactCoverage {
        artifact: "video",
        renderer: "capsem-media",
        topology_targets: &["title", "media", "caption", "controls", "source"],
        mutations: &["title", "caption", "source"],
        pack: ContractPack::CapsemExtension,
    },
    ArtifactCoverage {
        artifact: "audio",
        renderer: "capsem-media",
        topology_targets: &["title", "media", "caption", "controls", "source"],
        mutations: &["title", "caption", "source"],
        pack: ContractPack::CapsemExtension,
    },
    ArtifactCoverage {
        artifact: "generatedMediaCard",
        renderer: "capsem-media-card",
        topology_targets: &["title", "prompt", "media", "provider", "cost"],
        mutations: &["title", "caption", "prompt"],
        pack: ContractPack::CapsemExtension,
    },
    ArtifactCoverage {
        artifact: "securityDecisionPanel",
        renderer: "svelte/preline",
        topology_targets: &["title", "decision", "reason", "actions"],
        mutations: &["decision", "reason", "actions"],
        pack: ContractPack::CapsemExtension,
    },
    ArtifactCoverage {
        artifact: "pluginToolInstallationCard",
        renderer: "svelte/preline",
        topology_targets: &["title", "description", "permissions", "actions"],
        mutations: &["title", "description", "permissions", "actions"],
        pack: ContractPack::CapsemExtension,
    },
];

pub const RICH_ARTIFACT_SCHEMA_COVERAGE: &[RichArtifactSchemaCoverage] = &[
    RichArtifactSchemaCoverage {
        artifact: "slideDeck",
        schema: "capsem.artifact.slideDeck.v1",
        renderer: "svelte/deck",
        required_fields: &["id", "title", "slides"],
        optional_fields: &["export", "theme", "speakerNotes"],
        topology_targets: &["deckTitle", "slide"],
        mutations: &["title", "slides", "order"],
    },
    RichArtifactSchemaCoverage {
        artifact: "slide",
        schema: "capsem.artifact.slide.v1",
        renderer: "svelte/deck",
        required_fields: &["id", "title", "blocks"],
        optional_fields: &["layout", "notes", "background"],
        topology_targets: &[
            "slideTitle",
            "region",
            "textBlock",
            "image",
            "chart",
            "diagram",
        ],
        mutations: &["title", "regions", "blocks"],
    },
    RichArtifactSchemaCoverage {
        artifact: "spreadsheet",
        schema: "capsem.artifact.spreadsheet.v1",
        renderer: "svelte/grid",
        required_fields: &["id", "title", "sheets"],
        optional_fields: &["charts", "namedRanges", "export"],
        topology_targets: &[
            "workbook",
            "sheetTab",
            "cell",
            "range",
            "formula",
            "embeddedArtifact",
        ],
        mutations: &["title", "sheets", "cells", "ranges"],
    },
    RichArtifactSchemaCoverage {
        artifact: "sheet",
        schema: "capsem.artifact.sheet.v1",
        renderer: "svelte/grid",
        required_fields: &["id", "title", "columns", "rows"],
        optional_fields: &["source", "formulas", "ranges"],
        topology_targets: &["sheet", "cell", "range", "row", "column"],
        mutations: &["name", "cells", "rows", "columns"],
    },
    RichArtifactSchemaCoverage {
        artifact: "website",
        schema: "capsem.artifact.website.v1",
        renderer: "svelte/preline",
        required_fields: &["id", "title", "pages"],
        optional_fields: &["navigation", "theme", "forms"],
        topology_targets: &["siteTitle", "page", "navigation"],
        mutations: &["title", "pages", "navigation"],
    },
    RichArtifactSchemaCoverage {
        artifact: "page",
        schema: "capsem.artifact.page.v1",
        renderer: "svelte/preline",
        required_fields: &["id", "title", "sections"],
        optional_fields: &["route", "actions", "metadata"],
        topology_targets: &["pageTitle", "section", "block", "action"],
        mutations: &["title", "sections", "blocks"],
    },
    RichArtifactSchemaCoverage {
        artifact: "form",
        schema: "capsem.artifact.form.v1",
        renderer: "a2ui/preline",
        required_fields: &["id", "title", "fields", "actions"],
        optional_fields: &["validation", "submission", "description"],
        topology_targets: &["form", "field", "label", "validation", "action"],
        mutations: &["title", "fields", "labels", "validation", "actions"],
    },
    RichArtifactSchemaCoverage {
        artifact: "image",
        schema: "capsem.artifact.image.v1",
        renderer: "capsem-media",
        required_fields: &["id", "title", "media", "source"],
        optional_fields: &["caption", "prompt", "provider", "model", "usage", "cost"],
        topology_targets: &["title", "media", "caption", "source"],
        mutations: &["title", "caption", "source"],
    },
    RichArtifactSchemaCoverage {
        artifact: "video",
        schema: "capsem.artifact.video.v1",
        renderer: "capsem-media",
        required_fields: &["id", "title", "media", "source"],
        optional_fields: &[
            "caption", "prompt", "provider", "model", "usage", "cost", "controls",
        ],
        topology_targets: &["title", "media", "caption", "controls", "source"],
        mutations: &["title", "caption", "source"],
    },
    RichArtifactSchemaCoverage {
        artifact: "audio",
        schema: "capsem.artifact.audio.v1",
        renderer: "capsem-media",
        required_fields: &["id", "title", "media", "source"],
        optional_fields: &[
            "caption", "prompt", "provider", "model", "usage", "cost", "controls",
        ],
        topology_targets: &["title", "media", "caption", "controls", "source"],
        mutations: &["title", "caption", "source"],
    },
    RichArtifactSchemaCoverage {
        artifact: "chart",
        schema: "capsem.artifact.chart.v1",
        renderer: "plotly",
        required_fields: &[
            "id",
            "title",
            "chart",
            "sourceArtifact",
            "data",
            "x",
            "series",
            "xLabel",
            "yLabel",
            "yUnit",
        ],
        optional_fields: &[
            "stack",
            "direction",
            "legend",
            "secondAxis",
            "fit",
            "export",
        ],
        topology_targets: CHART_TARGETS,
        mutations: CHART_MUTATIONS,
    },
    RichArtifactSchemaCoverage {
        artifact: "diagram",
        schema: "capsem.artifact.diagram.v1",
        renderer: "mermaid",
        required_fields: &["id", "title", "kind", "source"],
        optional_fields: &["export", "theme", "metadata"],
        topology_targets: &["title", "source", "node", "edge"],
        mutations: &["title", "source"],
    },
    RichArtifactSchemaCoverage {
        artifact: "timeline",
        schema: "capsem.artifact.timeline.v1",
        renderer: "svelte/preline",
        required_fields: &["id", "title", "lanes", "events"],
        optional_fields: &["filters", "scale", "export"],
        topology_targets: &["title", "lane", "event", "date", "annotation"],
        mutations: &["title", "lanes", "events"],
    },
];

pub const PLOTLY_CHART_API_COVERAGE: &[PlotlyChartApiCoverage] = &[
    PlotlyChartApiCoverage {
        api: "ui.chart.barChart",
        chart: "barChart",
        plotly_trace: "bar",
        required_fields: CHART_REQUIRED_FIELDS,
        supports_direction: true,
        supports_stack: true,
        supports_fit: false,
        supports_second_axis: true,
        export_formats: CHART_EXPORT_FORMATS,
    },
    PlotlyChartApiCoverage {
        api: "ui.chart.lineChart",
        chart: "lineChart",
        plotly_trace: "scatter",
        required_fields: CHART_REQUIRED_FIELDS,
        supports_direction: false,
        supports_stack: false,
        supports_fit: true,
        supports_second_axis: true,
        export_formats: CHART_EXPORT_FORMATS,
    },
    PlotlyChartApiCoverage {
        api: "ui.chart.heatmap",
        chart: "heatmapChart",
        plotly_trace: "heatmap",
        required_fields: &["id", "title", "data", "x", "series", "xLabel", "yLabel"],
        supports_direction: false,
        supports_stack: false,
        supports_fit: false,
        supports_second_axis: false,
        export_formats: CHART_EXPORT_FORMATS,
    },
    PlotlyChartApiCoverage {
        api: "ui.chart.boxPlot",
        chart: "boxPlot",
        plotly_trace: "box",
        required_fields: &["id", "title", "data", "series", "xLabel", "yLabel"],
        supports_direction: false,
        supports_stack: false,
        supports_fit: false,
        supports_second_axis: false,
        export_formats: CHART_EXPORT_FORMATS,
    },
    PlotlyChartApiCoverage {
        api: "ui.chart.scatterPlot",
        chart: "scatterPlot",
        plotly_trace: "scatter",
        required_fields: CHART_REQUIRED_FIELDS,
        supports_direction: false,
        supports_stack: false,
        supports_fit: true,
        supports_second_axis: true,
        export_formats: CHART_EXPORT_FORMATS,
    },
];

pub const MERMAID_DIAGRAM_API_COVERAGE: &[MermaidDiagramApiCoverage] =
    &[MermaidDiagramApiCoverage {
        api: "local.diagram.render",
        artifact: "diagram",
        renderer: "mermaid",
        required_fields: &["id", "title", "kind", "source"],
        supported_diagrams: &[
            "flowchart",
            "sequence",
            "class",
            "state",
            "er",
            "gantt",
            "timeline",
            "mindmap",
        ],
        export_formats: &["svg", "png"],
        sandbox: "mermaid.securityLevel.strict",
    }];

pub const TIMELINE_API_COVERAGE: &[TimelineApiCoverage] = &[TimelineApiCoverage {
    api: "local.ui.timeline",
    artifact: "timeline",
    renderer: "svelte/preline",
    required_fields: &["id", "title", "lanes", "events"],
    event_fields: &["id", "title", "lane", "start", "end", "description"],
    topology_targets: &["title", "lane", "event", "date", "annotation"],
    mutations: &["title", "lanes", "events"],
}];

pub const GENERATED_MEDIA_API_COVERAGE: &[GeneratedMediaApiCoverage] = &[
    GeneratedMediaApiCoverage {
        api: "local.generate.text",
        artifact: "generatedText",
        media: "text",
        renderer: "capsem-text",
        status: Coverage::Explicit,
        required_fields: &["id", "title", "prompt"],
        provenance_fields: &["provider", "model", "system", "prompt"],
        telemetry_fields: &["usage", "cost", "durationMs", "status"],
    },
    GeneratedMediaApiCoverage {
        api: "local.generate.image",
        artifact: "generatedImage",
        media: "image",
        renderer: "capsem-media",
        status: Coverage::Explicit,
        required_fields: &["id", "title", "prompt"],
        provenance_fields: &["provider", "model", "prompt", "revisedPrompt"],
        telemetry_fields: &["usage", "cost", "durationMs", "status"],
    },
    GeneratedMediaApiCoverage {
        api: "local.generate.embedding",
        artifact: "generatedEmbedding",
        media: "embedding",
        renderer: "capsem-embedding",
        status: Coverage::Explicit,
        required_fields: &["id", "title", "input"],
        provenance_fields: &["provider", "model", "input"],
        telemetry_fields: &["usage", "cost", "durationMs", "status"],
    },
    GeneratedMediaApiCoverage {
        api: "local.generate.video",
        artifact: "generatedVideo",
        media: "video",
        renderer: "capsem-media",
        status: Coverage::Deferred,
        required_fields: &["id", "title", "prompt"],
        provenance_fields: &["provider", "model", "prompt", "sourceMedia"],
        telemetry_fields: &["usage", "cost", "durationMs", "status"],
    },
    GeneratedMediaApiCoverage {
        api: "local.generate.audio",
        artifact: "generatedAudio",
        media: "audio",
        renderer: "capsem-media",
        status: Coverage::Deferred,
        required_fields: &["id", "title", "prompt"],
        provenance_fields: &["provider", "model", "prompt", "sourceMedia"],
        telemetry_fields: &["usage", "cost", "durationMs", "status"],
    },
];

pub const LOCAL_TOOL_COVERAGE: &[LocalToolCoverage] = &[
    explicit_tool(
        "local.ui.info",
        "Return surfaces, topology, selected comments, tasks, and mutation affordances.",
    ),
    explicit_tool(
        "local.ui.render",
        "Render checked Loro-backed UI state into A2UI or Capsem component specs.",
    ),
    explicit_tool("local.ui.comment", "Attach feedback to a topology target."),
    explicit_tool("local.ui.resolve", "Resolve a comment or task."),
    explicit_tool(
        "local.ui.mutate",
        "Apply typed UI/artifact mutations through the Rust allowlist.",
    ),
    explicit_tool(
        "local.workspace.snapshot",
        "Return compact workspace state.",
    ),
    explicit_tool(
        "local.workspace.stream",
        "Subscribe to or replay workspace records.",
    ),
    explicit_tool(
        "local.workspace.checkpoint",
        "Compact records into a replay-safe checkpoint.",
    ),
    explicit_tool("local.generate.text", "Generate text through capsem-ai."),
    explicit_tool("local.generate.image", "Generate image through capsem-ai."),
    deferred_tool(
        "local.generate.audio",
        "Generate audio when provider/config support is wired.",
    ),
    deferred_tool(
        "local.generate.video",
        "Generate video when provider/config support is wired.",
    ),
    explicit_tool(
        "local.generate.embedding",
        "Generate embeddings through capsem-ai.",
    ),
    explicit_tool(
        "local.data.sqlite",
        "Use the per-instance SQLite workbench.",
    ),
    explicit_tool(
        "local.data.spreadsheet",
        "Create or update spreadsheet artifacts.",
    ),
    explicit_tool("local.data.sheet", "Create or update sheet artifacts."),
    explicit_tool(
        "local.export.spreadsheet",
        "Export spreadsheet/sheet artifacts through the configured VM document toolchain.",
    ),
    explicit_tool(
        "local.export.slideDeck",
        "Export slide deck artifacts through the configured VM document toolchain.",
    ),
    deferred_tool(
        "local.export.chart",
        "Export charts through the authoritative Plotly browser renderer path.",
    ),
    deferred_tool(
        "local.export.diagram",
        "Export diagrams through the authoritative Mermaid browser renderer path.",
    ),
    deferred_tool(
        "local.export.pdf",
        "Export decks/reports to PDF after the office/PDF toolchain spike is selected.",
    ),
    deferred_tool("local.website.render", "Render website/page artifacts."),
    deferred_tool(
        "local.website.form",
        "Create or update forms and validation.",
    ),
    explicit_tool(
        "local.diagram.render",
        "Create and render diagram artifacts.",
    ),
    explicit_tool("local.ui.timeline", "Create structured timeline artifacts."),
    deferred_tool(
        "local.web.preview",
        "Preview full websites/pages in a browser surface.",
    ),
];

const CHART_TARGETS: &[&str] = &[
    "title",
    "legend",
    "xAxis",
    "yAxis",
    "secondaryYAxis",
    "series",
    "dataPoint",
];
const CHART_MUTATIONS: &[&str] = &[
    "title",
    "series",
    "axis",
    "legend",
    "direction",
    "stack",
    "fit",
];
const CHART_REQUIRED_FIELDS: &[&str] = &[
    "id", "title", "data", "x", "series", "xLabel", "yLabel", "yUnit",
];
const CHART_EXPORT_FORMATS: &[&str] = &["png", "svg"];

const fn shipped(
    component: &'static str,
    helper: Option<&'static str>,
    renderer: Coverage,
    mutation: MutationCoverage,
) -> A2uiPrimitiveCoverage {
    A2uiPrimitiveCoverage {
        component,
        rust_type: Coverage::Explicit,
        helper,
        renderer,
        topology: Coverage::Explicit,
        mutation,
        pack: ContractPack::Pack01,
    }
}

const fn promoted_template(component: &'static str) -> A2uiPrimitiveCoverage {
    A2uiPrimitiveCoverage {
        component,
        rust_type: Coverage::Explicit,
        helper: None,
        renderer: Coverage::Explicit,
        topology: Coverage::Explicit,
        mutation: MutationCoverage::Deferred,
        pack: ContractPack::Pack02,
    }
}

const fn deferred(component: &'static str) -> A2uiPrimitiveCoverage {
    A2uiPrimitiveCoverage {
        component,
        rust_type: Coverage::Explicit,
        helper: None,
        renderer: Coverage::Deferred,
        topology: Coverage::Deferred,
        mutation: MutationCoverage::Deferred,
        pack: ContractPack::Pack02,
    }
}

const fn explicit_tool(tool: &'static str, purpose: &'static str) -> LocalToolCoverage {
    LocalToolCoverage {
        tool,
        purpose,
        status: Coverage::Explicit,
    }
}

const fn deferred_tool(tool: &'static str, purpose: &'static str) -> LocalToolCoverage {
    LocalToolCoverage {
        tool,
        purpose,
        status: Coverage::Deferred,
    }
}
