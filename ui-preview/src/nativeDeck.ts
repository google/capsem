export type NativeDeckProof = {
  ok: boolean;
  title: string;
  summary: {
    sqliteRows: number;
    chartCount: number;
    individualArtifactCount: number;
    requiredArtifactsPresent: boolean;
  };
  artifacts: NativeArtifact[];
  deck: {
    id: string;
    title: string;
    slides: Array<{
      artifactId: string;
      title: string;
    }>;
  };
};

import type {
  ChartDirection,
  ChartSeriesSpec,
  ChartStackMode,
  NativeArtifact,
  NativeArtifactKind,
  NativeArtifactSpec,
  PlotlyChartKind,
  SlideBlock,
  TimelineEvent,
  TimelineLane,
} from "./generated/nativeArtifact.js";

export type {
  ArtifactSpecBase,
  ChartDirection,
  ChartSeriesSpec,
  ChartSpec,
  ChartStackMode,
  DiagramSpec,
  GeneratedEmbeddingSpec,
  GeneratedImageSpec,
  GeneratedTextSpec,
  JsonRow,
  LegendPosition,
  NativeArtifact,
  NativeArtifactBase,
  NativeArtifactKind,
  NativeArtifactSpec,
  PlotlyChartKind,
  SheetSpec,
  SlideBlock,
  SlideDeckSpec,
  SlideSpec,
  TableSpec,
  TimelineEvent,
  TimelineLane,
  TimelineSpec,
} from "./generated/nativeArtifact.js";

export type RenderErrorReportInput = {
  message: string;
  source?: string;
  status?: string;
  artifactId?: string;
  component?: string;
  renderer?: string;
  phase?: string;
};

export async function loadNativeDeckProof(fetcher: typeof fetch = fetch): Promise<NativeDeckProof> {
  const response = await fetcher("/native/deck-proof");
  const payload = await response.json();
  return parseNativeDeckProof(payload);
}

export type WorkspaceRole = "user" | "assistant" | "tool" | "plugin" | "system";
export type WorkspaceContentType =
  | "artifact"
  | "text"
  | "ui"
  | "toolCall"
  | "action"
  | "elementPatch"
  | "status"
  | "error";
export type WorkspaceVerb =
  | "create"
  | "append"
  | "replace"
  | "patch"
  | "delete"
  | "select"
  | "request"
  | "respond";
export type WorkspaceStatus = "pending" | "running" | "complete" | "failed" | "cancelled";

export type RenderElement = {
  id: string;
  title: string;
  contentType: WorkspaceContentType;
  status: WorkspaceStatus;
  component: string;
  provenance: ElementProvenance;
  artifact?: NativeArtifact;
  content?: Record<string, unknown>;
};

export type ElementProvenance = {
  createdSeq: number;
  createdAt: string;
  createdBy: string;
  updatedSeq: number;
  updatedAt: string;
  updatedBy: string;
  lastRecordId: string;
  lastVerb: WorkspaceVerb;
};

export type RenderProjection = {
  seq: number;
  elements: Record<string, RenderElement>;
  topology: RenderTopology;
  tasks?: Record<string, WorkspaceTask>;
  selected?: string;
};

export type RenderTopology = {
  roots: string[];
  nodes: Record<string, TopologyNode>;
};

export type TopologyNode = {
  id: string;
  parent?: string;
  slot: string;
  index: number;
};

export type RenderDelta =
  | { type: "upsertElement"; id: string; element: RenderElement }
  | { type: "deleteElement"; id: string }
  | { type: "upsertTopologyNode"; node: TopologyNode }
  | { type: "deleteTopologyNode"; id: string }
  | { type: "upsertTask"; id: string; task: WorkspaceTask }
  | { type: "select"; id?: string | null };

export type WorkspaceTaskStatus = "open" | "resolved";

export type WorkspaceTask = {
  id: string;
  target: string;
  instruction: string;
  annotation?: AnnotationTarget;
  status: WorkspaceTaskStatus;
  createdSeq: number;
  createdAt: string;
  createdBy: string;
  updatedSeq: number;
  updatedAt: string;
  updatedBy: string;
  sourceRecordId: string;
  resolvedSeq?: number;
  resolvedAt?: string;
  resolvedBy?: string;
};

export type WorkspaceRecord = {
  seq: number;
  id: string;
  timestamp: string;
  role: WorkspaceRole;
  principal: string;
  title: string;
  contentType: WorkspaceContentType;
  verb: WorkspaceVerb;
  status: WorkspaceStatus;
  target?: string;
  content: Record<string, unknown>;
};

export type AnnotationTarget = {
  kind: string;
  label: string;
  path: string[];
  topologyId?: string;
  topologyRole?: string;
  selector?: string;
  hostSelector?: string;
  shadowSelector?: string;
  selectorVerified?: boolean;
  metadata?: Record<string, unknown>;
};

export type WorkspaceFrame = {
  record: WorkspaceRecord;
  deltas: RenderDelta[];
};

export type WorkspaceCheckpoint = {
  checkpointSeq: number;
  createdAt: string;
  workspaceId: string;
  projectionVersion: number;
  recordsCompacted?: { start: number; end: number } | null;
  projection: RenderProjection;
};

export type WorkspaceSnapshot = {
  workspaceId: string;
  checkpoint: WorkspaceCheckpoint;
  projection: RenderProjection;
  tail: WorkspaceFrame[];
};

export type WorkspaceStreamMessage =
  | { type: "snapshot"; snapshot: WorkspaceSnapshot }
  | { type: "record"; frame: WorkspaceFrame };

export async function loadNativeWorkspaceSnapshot(
  fetcher: typeof fetch = fetch,
): Promise<WorkspaceSnapshot> {
  const response = await fetcher("/native/workspace/snapshot");
  const payload = await response.json();
  return parseWorkspaceSnapshot(payload);
}

export async function selectWorkspaceElement(
  target: string | null,
  fetcher: typeof fetch = fetch,
): Promise<WorkspaceFrame> {
  const response = await fetcher("/native/workspace/select", {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ target }),
  });
  if (!response.ok) throw new Error(await response.text());
  return parseWorkspaceFrame(await response.json());
}

export async function requestWorkspaceChange(
  target: string,
  instruction: string,
  annotationOrFetcher?: AnnotationTarget | null | typeof fetch,
  maybeFetcher: typeof fetch = fetch,
): Promise<WorkspaceFrame> {
  const annotation =
    typeof annotationOrFetcher === "function" ? null : (annotationOrFetcher ?? null);
  const fetcher = typeof annotationOrFetcher === "function" ? annotationOrFetcher : maybeFetcher;
  const body: Record<string, unknown> = { target, instruction };
  if (annotation) body.annotation = annotation;
  const response = await fetcher("/native/workspace/change-request", {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(body),
  });
  if (!response.ok) throw new Error(await response.text());
  return parseWorkspaceFrame(await response.json());
}

export async function resolveWorkspaceTask(
  taskId: string,
  fetcher: typeof fetch = fetch,
): Promise<WorkspaceFrame> {
  const response = await fetcher("/native/workspace/resolve", {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ taskId }),
  });
  if (!response.ok) throw new Error(await response.text());
  return parseWorkspaceFrame(await response.json());
}

export async function mutateWorkspaceTitle(
  target: string,
  title: string,
  fetcher: typeof fetch = fetch,
): Promise<WorkspaceFrame> {
  const response = await fetcher("/native/workspace/mutate", {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ type: "title", target, title }),
  });
  if (!response.ok) throw new Error(await response.text());
  return parseWorkspaceFrame(await response.json());
}

export async function mutateWorkspaceText(
  target: string,
  text: string,
  optionsOrFetcher:
    | {
        selector?: string;
        hostSelector?: string;
        shadowSelector?: string;
        sourceRequestSeq?: number;
      }
    | typeof fetch = {},
  maybeFetcher?: typeof fetch,
): Promise<WorkspaceFrame> {
  const options =
    typeof optionsOrFetcher === "function"
      ? {}
      : optionsOrFetcher;
  const fetcher =
    typeof optionsOrFetcher === "function"
      ? optionsOrFetcher
      : maybeFetcher ?? fetch;
  const response = await fetcher("/native/workspace/mutate", {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ type: "text", target, text, ...options }),
  });
  if (!response.ok) throw new Error(await response.text());
  return parseWorkspaceFrame(await response.json());
}

export async function mutateWorkspaceTextPatch(
  input: {
    target: string;
    text: string;
    selector?: string;
    hostSelector?: string;
    shadowSelector?: string;
    sourceRequestSeq?: number;
  },
  fetcher: typeof fetch = fetch,
): Promise<WorkspaceFrame> {
  const response = await fetcher("/native/workspace/mutate", {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ type: "text", ...input }),
  });
  if (!response.ok) throw new Error(await response.text());
  return parseWorkspaceFrame(await response.json());
}

export async function mutateWorkspaceStyle(
  input: {
    target: string;
    styles: Record<string, string>;
    selector?: string;
    hostSelector?: string;
    shadowSelector?: string;
    sourceRequestSeq?: number;
  },
  fetcher: typeof fetch = fetch,
): Promise<WorkspaceFrame> {
  const response = await fetcher("/native/workspace/mutate", {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ type: "style", ...input }),
  });
  if (!response.ok) throw new Error(await response.text());
  return parseWorkspaceFrame(await response.json());
}

export async function reportRenderError(
  input: RenderErrorReportInput,
  fetcher: typeof fetch = fetch,
): Promise<void> {
  const response = await fetcher("/native/ui/render-error", {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(input),
  });
  if (!response.ok) throw new Error(await response.text());
}

export async function checkpointWorkspace(fetcher: typeof fetch = fetch): Promise<WorkspaceCheckpoint> {
  const response = await fetcher("/native/workspace/checkpoint", {
    method: "POST",
    headers: { "content-type": "application/json" },
  });
  if (!response.ok) throw new Error(await response.text());
  return parseWorkspaceCheckpoint(await response.json());
}

export function parseWorkspaceStreamMessage(value: unknown): WorkspaceStreamMessage {
  assertRecord(value, "workspace stream message");
  if (value.type === "snapshot") {
    return { type: "snapshot", snapshot: parseWorkspaceSnapshot(value.snapshot) };
  }
  if (value.type === "record") {
    return { type: "record", frame: parseWorkspaceFrame(value.frame) };
  }
  throw new Error("workspace stream message type is not supported");
}

export function parseWorkspaceSnapshot(value: unknown): WorkspaceSnapshot {
  assertRecord(value, "workspace snapshot");
  const projection = parseRenderProjection(value.projection);
  return {
    workspaceId: requireString(value.workspaceId, "workspaceId"),
    checkpoint: parseWorkspaceCheckpoint(value.checkpoint),
    projection,
    tail: requireArray(value.tail, "snapshot.tail").map(parseWorkspaceFrame),
  };
}

export function parseWorkspaceCheckpoint(value: unknown): WorkspaceCheckpoint {
  assertRecord(value, "workspace checkpoint");
  return {
    checkpointSeq: requireNumber(value.checkpointSeq, "checkpoint.checkpointSeq"),
    createdAt: requireString(value.createdAt, "checkpoint.createdAt"),
    workspaceId: requireString(value.workspaceId, "checkpoint.workspaceId"),
    projectionVersion: requireNumber(value.projectionVersion, "checkpoint.projectionVersion"),
    recordsCompacted:
      value.recordsCompacted === null || value.recordsCompacted === undefined
        ? null
        : parseCompactedRange(value.recordsCompacted),
    projection: parseRenderProjection(value.projection),
  };
}

export function parseRenderProjection(value: unknown): RenderProjection {
  assertRecord(value, "render projection");
  const elementsInput = value.elements;
  assertRecord(elementsInput, "projection.elements");
  const elements: Record<string, RenderElement> = {};
  for (const [id, element] of Object.entries(elementsInput)) {
    elements[id] = parseRenderElement(element);
  }
  const tasksInput = value.tasks;
  const tasks: Record<string, WorkspaceTask> = {};
  if (tasksInput !== undefined && tasksInput !== null) {
    assertRecord(tasksInput, "projection.tasks");
    for (const [id, task] of Object.entries(tasksInput)) {
      tasks[id] = parseWorkspaceTask(task);
    }
  }
  return {
    seq: requireNumber(value.seq, "projection.seq"),
    elements,
    topology: parseRenderTopology(value.topology),
    tasks: Object.keys(tasks).length > 0 ? tasks : undefined,
    selected:
      value.selected === undefined || value.selected === null
        ? undefined
        : requireString(value.selected, "projection.selected"),
  };
}

export function projectionToMaps(projection: RenderProjection): {
  elementsById: Map<string, RenderElement>;
  topology: RenderTopology;
  tasksById: Map<string, WorkspaceTask>;
  selectedId: string;
} {
  return {
    elementsById: new Map(Object.entries(projection.elements)),
    topology: cloneTopology(projection.topology),
    tasksById: new Map(Object.entries(projection.tasks ?? {})),
    selectedId: projection.selected ?? "",
  };
}

export function applyRenderDeltas(
  currentElements: Map<string, RenderElement>,
  currentTopology: RenderTopology,
  currentSelectedId: string,
  deltas: RenderDelta[],
  currentTasks: Map<string, WorkspaceTask> = new Map(),
): {
  elementsById: Map<string, RenderElement>;
  topology: RenderTopology;
  tasksById: Map<string, WorkspaceTask>;
  selectedId: string;
} {
  const elementsById = new Map(currentElements);
  const topology = cloneTopology(currentTopology);
  const tasksById = new Map(currentTasks);
  let selectedId = currentSelectedId;

  for (const delta of deltas) {
    if (delta.type === "upsertElement") {
      elementsById.set(delta.id, delta.element);
    } else if (delta.type === "deleteElement") {
      elementsById.delete(delta.id);
      if (selectedId === delta.id) selectedId = "";
    } else if (delta.type === "upsertTopologyNode") {
      topology.nodes[delta.node.id] = { ...delta.node };
      if (!delta.node.parent && !topology.roots.includes(delta.node.id)) {
        topology.roots.push(delta.node.id);
      }
      topology.roots.sort((left, right) => {
        const leftNode = topology.nodes[left];
        const rightNode = topology.nodes[right];
        return (leftNode?.index ?? 0) - (rightNode?.index ?? 0);
      });
    } else if (delta.type === "deleteTopologyNode") {
      delete topology.nodes[delta.id];
      topology.roots = topology.roots.filter((id) => id !== delta.id);
    } else if (delta.type === "upsertTask") {
      tasksById.set(delta.id, delta.task);
    } else if (delta.type === "select") {
      selectedId = delta.id ?? "";
    }
  }

  return { elementsById, topology, tasksById, selectedId };
}

export function storeWorkspaceCheckpoint(snapshot: WorkspaceSnapshot): void {
  try {
    window.localStorage.setItem(
      "capsem-workspace-pointer",
      JSON.stringify({
        workspaceId: snapshot.workspaceId,
        checkpointSeq: snapshot.checkpoint.checkpointSeq,
        seq: snapshot.projection.seq,
      }),
    );
  } catch {
    // Browser storage is an optimization. The server snapshot remains canonical.
  }
}

export function openWorkspaceStream(lastSeq: number): WebSocket {
  const protocol = window.location.protocol === "https:" ? "wss:" : "ws:";
  return new WebSocket(`${protocol}//${window.location.host}/native/workspace/stream?lastSeq=${lastSeq}`);
}

export function parseNativeDeckProof(value: unknown): NativeDeckProof {
  assertRecord(value, "native deck proof");
  if (value.ok !== true) throw new Error("native deck proof must be ok");
  if (typeof value.title !== "string") throw new Error("native deck proof title must be string");
  assertRecord(value.summary, "native deck proof summary");
  if (!Array.isArray(value.artifacts)) throw new Error("native deck proof artifacts must be array");
  assertRecord(value.deck, "native deck proof deck");

  return {
    ok: value.ok,
    title: value.title,
    summary: {
      sqliteRows: requireNumber(value.summary.sqliteRows, "summary.sqliteRows"),
      chartCount: requireNumber(value.summary.chartCount, "summary.chartCount"),
      individualArtifactCount: requireNumber(
        value.summary.individualArtifactCount,
        "summary.individualArtifactCount",
      ),
      requiredArtifactsPresent: requireBoolean(
        value.summary.requiredArtifactsPresent,
        "summary.requiredArtifactsPresent",
      ),
    },
    artifacts: value.artifacts.map(parseNativeArtifact),
    deck: {
      id: requireString(value.deck.id, "deck.id"),
      title: requireString(value.deck.title, "deck.title"),
      slides: requireArray(value.deck.slides, "deck.slides").map((slide) => {
        assertRecord(slide, "deck slide");
        return {
          artifactId: requireString(slide.artifactId, "deck slide artifactId"),
          title: requireString(slide.title, "deck slide title"),
        };
      }),
    },
  };
}

export function tagNameForArtifact(artifact: NativeArtifact): string {
  switch (artifact.kind) {
    case "sheet":
    case "table":
      return "capsem-sheet";
    case "chart":
      return "capsem-chart";
    case "diagram":
      return "capsem-diagram";
    case "timeline":
      return "capsem-timeline";
    case "generatedText":
      return "capsem-text";
    case "generatedImage":
      return "capsem-media";
    case "generatedEmbedding":
      return "capsem-embedding";
    case "slide":
      return "capsem-slide";
    case "slideDeck":
      return "capsem-slide-deck";
  }
}

export function parseNativeArtifact(value: unknown): NativeArtifact {
  assertRecord(value, "native artifact");
  const spec = value.spec;
  assertRecord(spec, "native artifact spec");
  const kind = requireKind(value.kind);
  return {
    id: requireString(value.id, "artifact.id"),
    kind,
    title: requireString(value.title, "artifact.title"),
    handle: requireString(value.handle, "artifact.handle"),
    spec: parseNativeArtifactSpec(kind, spec),
  } as NativeArtifact;
}

function requireKind(value: unknown): NativeArtifactKind {
  const allowed = new Set([
    "generatedImage",
    "generatedText",
    "generatedEmbedding",
    "sheet",
    "table",
    "chart",
    "diagram",
    "timeline",
    "slide",
    "slideDeck",
  ]);
  if (typeof value !== "string" || !allowed.has(value)) {
    throw new Error("artifact.kind is not a supported native artifact kind");
  }
  return value as NativeArtifactKind;
}

function parseNativeArtifactSpec(
  kind: NativeArtifactKind,
  spec: Record<string, unknown>,
): NativeArtifactSpec {
  switch (kind) {
    case "generatedText":
      requireComponent(spec, "capsem-text");
      requireMedia(spec, "text");
      return {
        ...spec,
        component: "capsem-text",
        media: "text",
        provider: requireString(spec.provider, "generatedText.provider"),
        model: optionalString(spec.model, "generatedText.model"),
        system: optionalString(spec.system, "generatedText.system"),
        prompt: requireString(spec.prompt, "generatedText.prompt"),
        text: optionalString(spec.text, "generatedText.text"),
        durationMs: optionalNumber(spec.durationMs, "generatedText.durationMs"),
        status: requireString(spec.status, "generatedText.status"),
        error: optionalString(spec.error, "generatedText.error"),
      };
    case "generatedImage":
      requireComponent(spec, "capsem-media");
      requireMedia(spec, "image");
      return {
        ...spec,
        component: "capsem-media",
        media: "image",
        provider: requireString(spec.provider, "generatedImage.provider"),
        model: optionalString(spec.model, "generatedImage.model"),
        prompt: requireString(spec.prompt, "generatedImage.prompt"),
        revisedPrompt: optionalString(spec.revisedPrompt, "generatedImage.revisedPrompt"),
        dataUrl: optionalString(spec.dataUrl, "generatedImage.dataUrl"),
        durationMs: optionalNumber(spec.durationMs, "generatedImage.durationMs"),
        status: requireString(spec.status, "generatedImage.status"),
        error: optionalString(spec.error, "generatedImage.error"),
      };
    case "generatedEmbedding":
      requireComponent(spec, "capsem-embedding");
      requireMedia(spec, "embedding");
      return {
        ...spec,
        component: "capsem-embedding",
        media: "embedding",
        provider: requireString(spec.provider, "generatedEmbedding.provider"),
        model: optionalString(spec.model, "generatedEmbedding.model"),
        input: requireStringArray(spec.input, "generatedEmbedding.input"),
        durationMs: optionalNumber(spec.durationMs, "generatedEmbedding.durationMs"),
        status: requireString(spec.status, "generatedEmbedding.status"),
        error: optionalString(spec.error, "generatedEmbedding.error"),
      };
    case "sheet":
      requireComponent(spec, "capsem-sheet");
      return {
        ...spec,
        component: "capsem-sheet",
        columns: requireStringArray(spec.columns, "sheet.columns"),
        rows: requireRecordArray(spec.rows, "sheet.rows"),
      };
    case "table":
      requireComponent(spec, "capsem-table");
      return {
        ...spec,
        component: "capsem-table",
        sourceArtifact: requireString(spec.sourceArtifact, "table.sourceArtifact"),
        columns: requireStringArray(spec.columns, "table.columns"),
        rows: requireRecordArray(spec.rows, "table.rows"),
        searchable:
          spec.searchable === undefined || spec.searchable === null
            ? undefined
            : requireBoolean(spec.searchable, "table.searchable"),
        filterable:
          spec.filterable === undefined || spec.filterable === null
            ? undefined
            : requireBoolean(spec.filterable, "table.filterable"),
        pageSize: requireNumber(spec.pageSize, "table.pageSize"),
      };
    case "chart":
      requireComponent(spec, "capsem-chart");
      {
        const chart = requireEnum(spec.chart, "chart.chart", [
          "barChart",
          "lineChart",
          "heatmapChart",
          "boxPlot",
          "scatterPlot",
        ]);
        const stack =
          spec.stack === undefined || spec.stack === null
            ? undefined
            : requireEnum(spec.stack, "chart.stack", ["none", "stacked", "grouped"]);
        const direction =
          spec.direction === undefined || spec.direction === null
            ? undefined
            : requireEnum(spec.direction, "chart.direction", ["vertical", "horizontal"]);
        const legend =
          spec.legend === undefined || spec.legend === null
            ? null
            : requireEnum(spec.legend, "chart.legend", [
                "top",
                "right",
                "bottom",
                "left",
                "none",
              ]);
        const secondAxis =
          spec.secondAxis === undefined || spec.secondAxis === null
            ? null
            : requireRecord(spec.secondAxis, "chart.secondAxis");
        const fit =
          spec.fit === undefined || spec.fit === null ? null : requireRecord(spec.fit, "chart.fit");
        validateChartOptionCompatibility(chart, stack, direction, secondAxis, fit);
        return {
          ...spec,
          component: "capsem-chart",
          chart,
          sourceArtifact: requireString(spec.sourceArtifact, "chart.sourceArtifact"),
          data: requireRecordArray(spec.data, "chart.data"),
          x: requireString(spec.x, "chart.x"),
          series: requireArray(spec.series, "chart.series").map(parseChartSeries),
          xLabel: requireString(spec.xLabel, "chart.xLabel"),
          xUnit: optionalString(spec.xUnit, "chart.xUnit"),
          yLabel: requireString(spec.yLabel, "chart.yLabel"),
          yUnit: requireString(spec.yUnit, "chart.yUnit"),
          stack,
          direction,
          legend,
          secondAxis,
          fit,
          export:
            spec.export === undefined || spec.export === null
              ? undefined
              : requireArray(spec.export, "chart.export").map((format) =>
                  requireEnum(format, "chart.export[]", ["png", "svg"]),
                ),
        };
      }
    case "diagram":
      requireComponent(spec, "capsem-diagram");
      return {
        ...spec,
        component: "capsem-diagram",
        kind: requireEnum(spec.kind, "diagram.kind", ["mermaid"]),
        source: requireString(spec.source, "diagram.source"),
        export:
          spec.export === undefined || spec.export === null
            ? undefined
            : requireArray(spec.export, "diagram.export").map((format) =>
                requireEnum(format, "diagram.export[]", ["svg", "png"]),
              ),
      };
    case "timeline":
      requireComponent(spec, "capsem-timeline");
      return {
        ...spec,
        component: "capsem-timeline",
        lanes: requireArray(spec.lanes, "timeline.lanes").map(parseTimelineLane),
        events: requireArray(spec.events, "timeline.events").map(parseTimelineEvent),
        export:
          spec.export === undefined || spec.export === null
            ? undefined
            : requireArray(spec.export, "timeline.export").map((format) =>
                requireEnum(format, "timeline.export[]", ["html", "png", "svg"]),
              ),
      };
    case "slide":
      requireComponent(spec, "capsem-slide");
      return {
        ...spec,
        component: "capsem-slide",
        blocks: requireArray(spec.blocks, "slide.blocks").map(parseSlideBlock),
      };
    case "slideDeck":
      requireComponent(spec, "capsem-slide-deck");
      return {
        ...spec,
        component: "capsem-slide-deck",
        slides: requireArray(spec.slides, "slideDeck.slides").map(parseSlideRef),
        export:
          spec.export === undefined || spec.export === null
            ? undefined
            : requireArray(spec.export, "slideDeck.export").map((format) =>
                requireEnum(format, "slideDeck.export[]", ["html", "pdf"]),
            ),
      };
  }
  throw new Error(`unsupported native artifact kind ${kind}`);
}

function parseChartSeries(value: unknown): ChartSeriesSpec {
  assertRecord(value, "chart.series[]");
  return {
    name: requireString(value.name, "chart.series[].name"),
    field: requireString(value.field, "chart.series[].field"),
    axis:
      value.axis === undefined || value.axis === null
        ? undefined
        : requireEnum(value.axis, "chart.series[].axis", ["left", "right"]),
  };
}

function validateChartOptionCompatibility(
  chart: PlotlyChartKind,
  stack: ChartStackMode | undefined,
  direction: ChartDirection | undefined,
  secondAxis: Record<string, unknown> | null,
  fit: Record<string, unknown> | null,
): void {
  if (stack && stack !== "none" && chart !== "barChart") {
    throw new Error(`${chart} does not support stack mode ${stack}`);
  }
  if (direction === "horizontal" && chart !== "barChart") {
    throw new Error(`${chart} does not support horizontal direction`);
  }
  if (secondAxis && chart !== "barChart" && chart !== "lineChart" && chart !== "scatterPlot") {
    throw new Error(`${chart} does not support secondAxis`);
  }
  if (fit && chart !== "lineChart" && chart !== "scatterPlot") {
    throw new Error(`${chart} does not support fit metadata`);
  }
}

function parseTimelineLane(value: unknown): TimelineLane {
  assertRecord(value, "timeline.lanes[]");
  return {
    id: requireString(value.id, "timeline.lanes[].id"),
    title: requireString(value.title, "timeline.lanes[].title"),
  };
}

function parseTimelineEvent(value: unknown): TimelineEvent {
  assertRecord(value, "timeline.events[]");
  return {
    id: requireString(value.id, "timeline.events[].id"),
    title: requireString(value.title, "timeline.events[].title"),
    lane: requireString(value.lane, "timeline.events[].lane"),
    start: requireString(value.start, "timeline.events[].start"),
    end: optionalString(value.end, "timeline.events[].end"),
    description: optionalString(value.description, "timeline.events[].description"),
  };
}

function parseSlideBlock(value: unknown): SlideBlock {
  assertRecord(value, "slide.blocks[]");
  const kind = requireEnum(value.kind, "slide.blocks[].kind", [
    "text",
    "image",
    "diagram",
    "table",
    "sheet",
    "chart",
  ]);
  if (kind === "text") {
    return {
      kind,
      title: requireString(value.title, "slide.blocks[].title"),
      body: requireString(value.body, "slide.blocks[].body"),
    };
  }
  return {
    kind,
    artifactId: requireString(value.artifactId, "slide.blocks[].artifactId"),
  };
}

function parseSlideRef(value: unknown): { artifactId: string; title: string } {
  assertRecord(value, "slideDeck.slides[]");
  return {
    artifactId: requireString(value.artifactId, "slideDeck.slides[].artifactId"),
    title: requireString(value.title, "slideDeck.slides[].title"),
  };
}

function requireComponent(spec: Record<string, unknown>, component: NativeArtifactSpec["component"]) {
  if (spec.component !== component) {
    throw new Error(`artifact spec component must be ${component}`);
  }
}

function requireMedia(spec: Record<string, unknown>, media: "text" | "image" | "embedding") {
  if (spec.media !== media) {
    throw new Error(`artifact spec media must be ${media}`);
  }
}

function parseWorkspaceFrame(value: unknown): WorkspaceFrame {
  assertRecord(value, "workspace frame");
  return {
    record: parseWorkspaceRecord(value.record),
    deltas: requireArray(value.deltas, "workspace frame deltas").map(parseRenderDelta),
  };
}

function parseRenderTopology(value: unknown): RenderTopology {
  assertRecord(value, "projection.topology");
  const nodesInput = value.nodes;
  assertRecord(nodesInput, "projection.topology.nodes");
  const nodes: Record<string, TopologyNode> = {};
  for (const [id, node] of Object.entries(nodesInput)) {
    nodes[id] = parseTopologyNode(node);
  }
  return {
    roots: requireArray(value.roots, "projection.topology.roots").map((id) =>
      requireString(id, "projection.topology.roots[]"),
    ),
    nodes,
  };
}

function parseTopologyNode(value: unknown): TopologyNode {
  assertRecord(value, "topology node");
  return {
    id: requireString(value.id, "topologyNode.id"),
    parent:
      value.parent === undefined || value.parent === null
        ? undefined
        : requireString(value.parent, "topologyNode.parent"),
    slot: requireString(value.slot, "topologyNode.slot"),
    index: requireNumber(value.index, "topologyNode.index"),
  };
}

function parseWorkspaceRecord(value: unknown): WorkspaceRecord {
  assertRecord(value, "workspace record");
  return {
    seq: requireNumber(value.seq, "record.seq"),
    id: requireString(value.id, "record.id"),
    timestamp: requireString(value.timestamp, "record.timestamp"),
    role: requireEnum(value.role, "record.role", ["user", "assistant", "tool", "plugin", "system"]),
    principal: requireString(value.principal, "record.principal"),
    title: requireString(value.title, "record.title"),
    contentType: requireEnum(value.contentType, "record.contentType", [
      "artifact",
      "text",
      "ui",
      "toolCall",
      "action",
      "elementPatch",
      "status",
      "error",
    ]),
    verb: requireEnum(value.verb, "record.verb", [
      "create",
      "append",
      "replace",
      "patch",
      "delete",
      "select",
      "request",
      "respond",
    ]),
    status: requireEnum(value.status, "record.status", [
      "pending",
      "running",
      "complete",
      "failed",
      "cancelled",
    ]),
    target:
      value.target === undefined || value.target === null
        ? undefined
        : requireString(value.target, "record.target"),
    content: requireRecord(value.content, "record.content"),
  };
}

function parseRenderDelta(value: unknown): RenderDelta {
  assertRecord(value, "render delta");
  if (value.type === "upsertElement") {
    return {
      type: "upsertElement",
      id: requireString(value.id, "delta.id"),
      element: parseRenderElement(value.element),
    };
  }
  if (value.type === "deleteElement") {
    return { type: "deleteElement", id: requireString(value.id, "delta.id") };
  }
  if (value.type === "upsertTopologyNode") {
    return {
      type: "upsertTopologyNode",
      node: parseTopologyNode(value.node),
    };
  }
  if (value.type === "deleteTopologyNode") {
    return { type: "deleteTopologyNode", id: requireString(value.id, "delta.id") };
  }
  if (value.type === "upsertTask") {
    return {
      type: "upsertTask",
      id: requireString(value.id, "delta.id"),
      task: parseWorkspaceTask(value.task),
    };
  }
  if (value.type === "select") {
    return {
      type: "select",
      id:
        value.id === undefined || value.id === null
          ? null
          : requireString(value.id, "delta.id"),
    };
  }
  throw new Error("render delta type is not supported");
}

function parseWorkspaceTask(value: unknown): WorkspaceTask {
  assertRecord(value, "workspace task");
  const annotation =
    value.annotation === undefined || value.annotation === null
      ? undefined
      : parseAnnotationTarget(value.annotation);
  return {
    id: requireString(value.id, "task.id"),
    target: requireString(value.target, "task.target"),
    instruction: requireString(value.instruction, "task.instruction"),
    annotation,
    status: requireEnum(value.status, "task.status", ["open", "resolved"]),
    createdSeq: requireNumber(value.createdSeq, "task.createdSeq"),
    createdAt: requireString(value.createdAt, "task.createdAt"),
    createdBy: requireString(value.createdBy, "task.createdBy"),
    updatedSeq: requireNumber(value.updatedSeq, "task.updatedSeq"),
    updatedAt: requireString(value.updatedAt, "task.updatedAt"),
    updatedBy: requireString(value.updatedBy, "task.updatedBy"),
    sourceRecordId: requireString(value.sourceRecordId, "task.sourceRecordId"),
    resolvedSeq:
      value.resolvedSeq === undefined || value.resolvedSeq === null
        ? undefined
        : requireNumber(value.resolvedSeq, "task.resolvedSeq"),
    resolvedAt:
      value.resolvedAt === undefined || value.resolvedAt === null
        ? undefined
        : requireString(value.resolvedAt, "task.resolvedAt"),
    resolvedBy:
      value.resolvedBy === undefined || value.resolvedBy === null
        ? undefined
        : requireString(value.resolvedBy, "task.resolvedBy"),
  };
}

function parseAnnotationTarget(value: unknown): AnnotationTarget {
  assertRecord(value, "annotation");
  return {
    kind: requireString(value.kind, "annotation.kind"),
    label: requireString(value.label, "annotation.label"),
    path: requireArray(value.path, "annotation.path").map((part, index) =>
      requireString(part, `annotation.path.${index}`),
    ),
    topologyId:
      value.topologyId === undefined || value.topologyId === null
        ? undefined
        : requireString(value.topologyId, "annotation.topologyId"),
    topologyRole:
      value.topologyRole === undefined || value.topologyRole === null
        ? undefined
        : requireString(value.topologyRole, "annotation.topologyRole"),
    selector:
      value.selector === undefined || value.selector === null
        ? undefined
        : requireString(value.selector, "annotation.selector"),
    hostSelector:
      value.hostSelector === undefined || value.hostSelector === null
        ? undefined
        : requireString(value.hostSelector, "annotation.hostSelector"),
    shadowSelector:
      value.shadowSelector === undefined || value.shadowSelector === null
        ? undefined
        : requireString(value.shadowSelector, "annotation.shadowSelector"),
    selectorVerified:
      value.selectorVerified === undefined || value.selectorVerified === null
        ? undefined
        : requireBoolean(value.selectorVerified, "annotation.selectorVerified"),
    metadata:
      value.metadata === undefined || value.metadata === null
        ? undefined
        : requireRecord(value.metadata, "annotation.metadata"),
  };
}

function cloneTopology(topology: RenderTopology): RenderTopology {
  return {
    roots: [...topology.roots],
    nodes: Object.fromEntries(Object.entries(topology.nodes).map(([id, node]) => [id, { ...node }])),
  };
}

function parseRenderElement(value: unknown): RenderElement {
  assertRecord(value, "render element");
  const artifact =
    value.artifact === undefined || value.artifact === null
      ? undefined
      : parseNativeArtifact(value.artifact);
  const content =
    value.content === undefined || value.content === null
      ? undefined
      : requireRecord(value.content, "render element content");
  return {
    id: requireString(value.id, "element.id"),
    title: requireString(value.title, "element.title"),
    contentType: requireEnum(value.contentType, "element.contentType", [
      "artifact",
      "text",
      "ui",
      "toolCall",
      "action",
      "elementPatch",
      "status",
      "error",
    ]),
    status: requireEnum(value.status, "element.status", [
      "pending",
      "running",
      "complete",
      "failed",
      "cancelled",
    ]),
    component: requireString(value.component, "element.component"),
    provenance: parseElementProvenance(value.provenance),
    artifact,
    content,
  };
}

function parseElementProvenance(value: unknown): ElementProvenance {
  assertRecord(value, "element.provenance");
  return {
    createdSeq: requireNumber(value.createdSeq, "element.provenance.createdSeq"),
    createdAt: requireString(value.createdAt, "element.provenance.createdAt"),
    createdBy: requireString(value.createdBy, "element.provenance.createdBy"),
    updatedSeq: requireNumber(value.updatedSeq, "element.provenance.updatedSeq"),
    updatedAt: requireString(value.updatedAt, "element.provenance.updatedAt"),
    updatedBy: requireString(value.updatedBy, "element.provenance.updatedBy"),
    lastRecordId: requireString(value.lastRecordId, "element.provenance.lastRecordId"),
    lastVerb: requireEnum(value.lastVerb, "element.provenance.lastVerb", [
      "create",
      "append",
      "replace",
      "patch",
      "delete",
      "select",
      "request",
      "respond",
    ]),
  };
}

function parseCompactedRange(value: unknown): { start: number; end: number } {
  assertRecord(value, "compacted range");
  return {
    start: requireNumber(value.start, "recordsCompacted.start"),
    end: requireNumber(value.end, "recordsCompacted.end"),
  };
}

function requireString(value: unknown, name: string): string {
  if (typeof value !== "string") throw new Error(`${name} must be string`);
  return value;
}

function requireNumber(value: unknown, name: string): number {
  if (typeof value !== "number") throw new Error(`${name} must be number`);
  return value;
}

function optionalNumber(value: unknown, name: string): number | null | undefined {
  if (value === undefined) return undefined;
  if (value === null) return null;
  return requireNumber(value, name);
}

function requireBoolean(value: unknown, name: string): boolean {
  if (typeof value !== "boolean") throw new Error(`${name} must be boolean`);
  return value;
}

function requireArray(value: unknown, name: string): unknown[] {
  if (!Array.isArray(value)) throw new Error(`${name} must be array`);
  return value;
}

function requireRecord(value: unknown, name: string): Record<string, unknown> {
  assertRecord(value, name);
  return value;
}

function requireStringArray(value: unknown, name: string): string[] {
  return requireArray(value, name).map((entry, index) => requireString(entry, `${name}.${index}`));
}

function requireRecordArray(value: unknown, name: string): Record<string, unknown>[] {
  return requireArray(value, name).map((entry, index) => requireRecord(entry, `${name}.${index}`));
}

function optionalString(value: unknown, name: string): string | null | undefined {
  if (value === undefined) return undefined;
  if (value === null) return null;
  return requireString(value, name);
}

function requireEnum<const T extends string>(value: unknown, name: string, allowed: readonly T[]): T {
  if (typeof value !== "string" || !allowed.includes(value as T)) {
    throw new Error(`${name} must be one of ${allowed.join(", ")}`);
  }
  return value as T;
}

function assertRecord(value: unknown, name: string): asserts value is Record<string, unknown> {
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    throw new Error(`${name} must be object`);
  }
}
