import { CapsemElement, type CapsemElementRenderContext } from "./capsem-elt";
import type { NativeArtifact } from "../nativeDeck";

export class CapsemArtifactElement extends CapsemElement<NativeArtifact> {
  protected render({ spec, emit }: CapsemElementRenderContext<NativeArtifact>): void {
    this.setAttribute("data-capsem-artifact-id", spec.id);
    this.setAttribute("data-capsem-artifact-kind", spec.kind);
    const style = document.createElement("style");
    style.textContent = artifactStyle;
    const shell = el("section", "artifact");
    shell.dataset.capsemArtifactId = spec.id;
    markTopology(shell, spec, "root", "root");
    shell.append(header(spec));
    let hydrate: (() => void) | undefined;

    switch (spec.kind) {
      case "sheet":
      case "table":
        shell.append(sheetPreview(spec));
        break;
      case "chart":
        {
          const preview = chartPreview(spec);
          shell.append(preview);
          hydrate = () => renderPlotlyChart(preview, spec, emit);
        }
        break;
      case "diagram":
        {
          const preview = diagramPreview(spec);
          shell.append(preview);
          hydrate = () => renderMermaidDiagram(preview, spec, emit);
        }
        break;
      case "timeline":
        shell.append(timelinePreview(spec));
        break;
      case "generatedText":
        shell.append(textPreview(spec));
        break;
      case "generatedImage":
        shell.append(mediaPreview(spec));
        break;
      case "slide":
        shell.append(slidePreview(spec));
        break;
      case "slideDeck":
        shell.append(deckPreview(spec));
        break;
    }

    installGenericAnnotator(shell, spec, emit);
    applyStylePatches(shell, spec);
    applyTextPatches(shell, spec);
    this.replaceRootChildren(style, shell);
    if (hydrate) queueMicrotask(hydrate);
    emit("ready", { kind: spec.kind, artifactId: spec.id });
  }
}

type ArtifactEmit = CapsemElementRenderContext<NativeArtifact>["emit"];

function header(spec: NativeArtifact): HTMLElement {
  const box = el("div", "header");
  markTopology(box, spec, "header", "header");
  const title = el("h3", "title", spec.title);
  title.tabIndex = 0;
  title.setAttribute("role", "button");
  title.setAttribute("aria-label", `Annotate title: ${spec.title}`);
  title.part.add("annotatable");
  markTopology(title, spec, titleTopologyRole(spec), "title");
  const meta = el("p", "meta", `${spec.kind} · ${spec.id}`);
  markTopology(meta, spec, "metadata", "metadata");
  box.append(title, meta);
  return box;
}

function sheetPreview(spec: NativeArtifact): HTMLElement {
  const columns = arrayOfStrings(spec.spec.columns);
  const rows = Array.isArray(spec.spec.rows) ? spec.spec.rows.slice(0, 6) : [];
  const table = document.createElement("table");
  table.part.add("table");
  markTopology(table, spec, spec.kind === "table" ? "table" : "sheet", spec.kind);
  const thead = document.createElement("thead");
  const headRow = document.createElement("tr");
  columns.forEach((column, columnIndex) => {
    const cell = document.createElement("th");
    cell.textContent = column;
    markTopology(cell, spec, "column", `column::${columnKey(column, columnIndex)}`);
    headRow.append(cell);
  });
  thead.append(headRow);
  const tbody = document.createElement("tbody");
  markTopology(tbody, spec, "range", "range::visible");
  rows.forEach((row, rowIndex) => {
    const tr = document.createElement("tr");
    tr.tabIndex = 0;
    tr.setAttribute("role", "button");
    tr.setAttribute("aria-label", `Annotate row ${rowIndex + 1}`);
    tr.part.add("annotatable");
    tr.part.add("row");
    markTopology(tr, spec, "row", `row::${rowIndex}`);
    columns.forEach((column, columnIndex) => {
      const cell = document.createElement("td");
      cell.textContent = valueText(isRecord(row) ? row[column] : undefined);
      markTopology(cell, spec, "cell", `cell::${rowIndex}::${columnKey(column, columnIndex)}`);
      tr.append(cell);
    });
    tbody.append(tr);
  });
  table.append(thead, tbody);
  return table;
}

function chartPreview(spec: NativeArtifact): HTMLElement {
  const box = el("div", "chart");
  markTopology(box, spec, "chart", "chart");
  const chart = valueText(spec.spec.chart);
  const series = Array.isArray(spec.spec.series) ? spec.spec.series : [];
  const data = Array.isArray(spec.spec.data) ? spec.spec.data.filter(isRecord) : [];
  const xField = valueText(spec.spec.x);
  const firstSeries = series.find(isRecord);
  const yField = firstSeries ? valueText(firstSeries.field) : "";
  const chartKind = el("p", "chartKind", chart || "chart");
  markTopology(chartKind, spec, "chart.kind", "chart-kind");
  box.append(chartKind);
  box.append(exportButton("export-svg", "Export SVG"));
  box.append(exportButton("export-png", "Export PNG"));
  const plot = el("div", "plot");
  markTopology(plot, spec, "plot", "plot");
  box.append(plot);
  const bars = el("div", "bars");
  markTopology(bars, spec, "dataPoints", "data-points");
  const axes = el("div", "axisMeta");
  const xAxis = el("span", "axisLabel", valueText(spec.spec.xLabel));
  markTopology(xAxis, spec, "xAxis", "x-axis");
  const yAxis = el("span", "axisLabel", axisTitle(spec.spec.yLabel, spec.spec.yUnit));
  markTopology(yAxis, spec, "yAxis", "y-axis");
  axes.append(xAxis, yAxis);
  if (isRecord(spec.spec.secondAxis)) {
    const secondary = el("span", "axisLabel", axisTitle(spec.spec.secondAxis.label, spec.spec.secondAxis.unit));
    markTopology(secondary, spec, "secondaryYAxis", "secondary-y-axis");
    axes.append(secondary);
  }
  const values = data
    .map((row) => Number(row[yField]))
    .filter((value) => Number.isFinite(value) && value > 0);
  const max = Math.max(1, ...values);

  if (data.length > 0 && yField) {
    data.forEach((row, rowIndex) => {
      const label = valueText(row[xField]);
      const value = Number(row[yField]);
      const bar = el("div", "bar");
      bar.style.width = `${Math.max(14, Math.round((value / max) * 100))}%`;
      bar.textContent = `${label} · ${Number.isFinite(value) ? value : "n/a"}`;
      markTopology(bar, spec, "dataPoint", `data-point::${rowIndex}`);
      bars.append(bar);
    });
  } else {
    series.forEach((item, index) => {
      const name = isRecord(item) ? valueText(item.name) : `series ${index + 1}`;
      const bar = el("div", "bar");
      bar.style.width = `${Math.max(32, 88 - index * 18)}%`;
      bar.textContent = name;
      markTopology(bar, spec, "series", `series::${index}`);
      bars.append(bar);
    });
  }
  if (series.length > 1) {
    const legend = el("div", "legend");
    markTopology(legend, spec, "legend", "legend");
    series.forEach((item, index) => {
      if (!isRecord(item)) return;
      const legendItem = el("span", "legendItem", valueText(item.name));
      markTopology(legendItem, spec, "series", `series::${index}`);
      legend.append(legendItem);
    });
    box.append(legend);
  }
  box.append(axes);
  box.append(bars);
  return box;
}

function diagramPreview(spec: NativeArtifact): HTMLElement {
  const source = valueText(spec.spec.source);
  const box = el("div", "diagram");
  markTopology(box, spec, "diagram", "diagram");
  box.append(exportButton("export-svg", "Export SVG"));
  const preview = el("div", "diagramPreview");
  markTopology(preview, spec, "diagram.preview", "diagram-preview");
  const pre = document.createElement("pre");
  pre.part.add("code");
  pre.textContent = source;
  markTopology(pre, spec, "source", "source");
  preview.append(pre);
  box.append(preview);
  return box;
}

function timelinePreview(spec: NativeArtifact): HTMLElement {
  const lanes = Array.isArray(spec.spec.lanes) ? spec.spec.lanes.filter(isRecord) : [];
  const events = Array.isArray(spec.spec.events) ? spec.spec.events.filter(isRecord) : [];
  const byLane = new Map<string, Record<string, unknown>[]>();
  for (const event of events) {
    const lane = valueText(event.lane);
    byLane.set(lane, [...(byLane.get(lane) ?? []), event]);
  }
  const box = el("div", "timeline");
  markTopology(box, spec, "timeline", "timeline");
  for (const lane of lanes) {
    const laneId = valueText(lane.id);
    const section = el("section", "timelineLane");
    markTopology(section, spec, "lane", `lane::${stableSegment(laneId)}`);
    const laneTitle = el("h4", "timelineLaneTitle", valueText(lane.title));
    markTopology(laneTitle, spec, "lane.title", `lane::${stableSegment(laneId)}::title`);
    section.append(laneTitle);
    for (const event of byLane.get(laneId) ?? []) {
      const item = el("article", "timelineEvent");
      const eventId = stableSegment(valueText(event.id) || valueText(event.title));
      item.tabIndex = 0;
      item.setAttribute("role", "button");
      item.setAttribute("aria-label", `Annotate timeline event: ${valueText(event.title)}`);
      item.part.add("annotatable");
      markTopology(item, spec, "event", `event::${eventId}`);
      const date = el("span", "timelineDate", timelineDate(event));
      markTopology(date, spec, "date", `event::${eventId}::date`);
      const title = el("strong", "timelineTitle", valueText(event.title));
      markTopology(title, spec, "event.title", `event::${eventId}::title`);
      item.append(date, title);
      const description = valueText(event.description);
      if (description) {
        const body = el("p", "body", description);
        markTopology(body, spec, "annotation", `event::${eventId}::description`);
        item.append(body);
      }
      section.append(item);
    }
    box.append(section);
  }
  return box;
}

function timelineDate(event: Record<string, unknown>): string {
  const start = valueText(event.start);
  const end = valueText(event.end);
  return end ? `${start} - ${end}` : start;
}

function textPreview(spec: NativeArtifact): HTMLElement {
  const box = el("div", "textArtifact");
  markTopology(box, spec, "text", "text");
  const text = valueText(spec.spec.text);
  const prompt = valueText(spec.spec.prompt);
  const status = valueText(spec.spec.status);
  if (status) {
    const statusNode = el("p", "meta", status);
    markTopology(statusNode, spec, "status", "status");
    box.append(statusNode);
  }
  const body = el("p", "body", text || prompt);
  markTopology(body, spec, "body", "body");
  box.append(body);
  const error = valueText(spec.spec.error);
  if (error) {
    const errorNode = el("p", "errorText", error);
    markTopology(errorNode, spec, "error", "error");
    box.append(errorNode);
  }
  return box;
}

function mediaPreview(spec: NativeArtifact): HTMLElement {
  const box = el("div", "media");
  markTopology(box, spec, "media", "media");
  const dataUrl = valueText(spec.spec.dataUrl);
  if (dataUrl.startsWith("data:image/")) {
    const image = document.createElement("img");
    image.part.add("mediaImage");
    image.alt = spec.title;
    image.src = dataUrl;
    markTopology(image, spec, "media", "media-image");
    box.append(image);
  } else {
    const frame = el("div", "mediaFrame", spec.title);
    markTopology(frame, spec, "media", "media-frame");
    box.append(frame);
  }
  const caption = valueText(spec.spec.caption);
  if (caption) {
    const captionNode = el("p", "body", caption);
    markTopology(captionNode, spec, "caption", "caption");
    box.append(captionNode);
  }
  const controls = el("div", "mediaControls");
  markTopology(controls, spec, "controls", "controls");
  controls.append(exportButton("export-png", "Export PNG"));
  box.append(controls);
  const status = valueText(spec.spec.status);
  if (status) {
    const statusNode = el("p", "meta", status);
    markTopology(statusNode, spec, "status", "status");
    box.append(statusNode);
  }
  const error = valueText(spec.spec.error);
  if (error) {
    const errorNode = el("p", "errorText", error);
    markTopology(errorNode, spec, "error", "error");
    box.append(errorNode);
  }
  const prompt = el("p", "body", valueText(spec.spec.prompt));
  markTopology(prompt, spec, "source", "source");
  box.append(prompt);
  return box;
}

function slidePreview(spec: NativeArtifact): HTMLElement {
  const blocks = Array.isArray(spec.spec.blocks) ? spec.spec.blocks : [];
  const box = el("div", "blocks");
  markTopology(box, spec, "region", "region");
  blocks.forEach((block, blockIndex) => {
    if (!isRecord(block)) {
      box.append(el("div", "block", valueText(block)));
      return;
    }
    const kind = valueText(block.kind);
    const item = el("div", kind === "image" ? "imageBlock" : "block");
    markTopology(item, spec, slideBlockTopologyRole(kind), `block::${blockIndex}::${stableSegment(kind || "unknown")}`);
    if (kind === "text") {
      const title = el("strong", "blockTitle", valueText(block.title));
      markTopology(title, spec, "block.title", `block::${blockIndex}::title`);
      const body = el("p", "body", valueText(block.body));
      markTopology(body, spec, "block.body", `block::${blockIndex}::body`);
      item.append(title, body);
    } else if (kind === "image") {
      const label = el("span", "slideNumber", "Generated image");
      markTopology(label, spec, "block.label", `block::${blockIndex}::label`);
      const title = el("strong", "blockTitle", valueText(block.artifactId));
      markTopology(title, spec, "block.artifactRef", `block::${blockIndex}::artifact`);
      item.append(label, title);
    } else {
      item.textContent = `${kind} · ${valueText(block.title ?? block.artifactId)}`;
    }
    box.append(item);
  });
  return box;
}

function deckPreview(spec: NativeArtifact): HTMLElement {
  const slides = Array.isArray(spec.spec.slides) ? spec.spec.slides : [];
  const grid = el("div", "deck");
  markTopology(grid, spec, "deck", "deck");
  slides.forEach((slide, index) => {
    const card = el("div", "slideCard");
    markTopology(card, spec, "slide", `slide::${index}`);
    const title = isRecord(slide) ? valueText(slide.title) : `Slide ${index + 1}`;
    const number = el("span", "slideNumber", String(index + 1));
    markTopology(number, spec, "slide.index", `slide::${index}::index`);
    const titleNode = el("strong", "slideTitle", title);
    markTopology(titleNode, spec, "slide.title", `slide::${index}::title`);
    card.append(number, titleNode);
    grid.append(card);
  });
  return grid;
}

function el(tag: string, part: string, text?: string): HTMLElement {
  const node = document.createElement(tag);
  node.part.add(part);
  if (text !== undefined) node.textContent = text;
  return node;
}

function exportButton(part: string, text: string): HTMLButtonElement {
  const button = document.createElement("button");
  button.type = "button";
  button.part.add("exportButton");
  button.part.add(part);
  button.textContent = text;
  return button;
}

function markTopology(node: HTMLElement, spec: NativeArtifact, role: string, localId: string): HTMLElement {
  markTopologyElement(node, spec, role, localId);
  return node;
}

function markTopologyElement(node: Element, spec: NativeArtifact, role: string, localId: string): Element {
  const topologyId = `${spec.id}::${localId}`;
  node.setAttribute("data-capsem-topology-role", role);
  node.setAttribute("data-capsem-topology-id", topologyId);
  node.setAttribute("data-capsem-node", topologyId);
  return node;
}

function titleTopologyRole(spec: NativeArtifact): string {
  if (spec.kind === "slide") return "slideTitle";
  if (spec.kind === "slideDeck") return "deckTitle";
  return "title";
}

function slideBlockTopologyRole(kind: string): string {
  if (kind === "text") return "textBlock";
  if (kind === "image") return "image";
  if (kind === "chart") return "chart";
  if (kind === "diagram") return "diagram";
  return "region";
}

function columnKey(column: string, index: number): string {
  return `${index}::${stableSegment(column)}`;
}

function stableSegment(value: string): string {
  const normalized = value
    .trim()
    .toLowerCase()
    .replace(/[^a-z0-9_-]+/g, "-")
    .replace(/^-+|-+$/g, "");
  return normalized || "item";
}

function installGenericAnnotator(root: HTMLElement, spec: NativeArtifact, emit: ArtifactEmit): void {
  numberDom(root);
  let previewNode = "";
  const annotateTarget = (event: Event): Element | null =>
    event
      .composedPath()
      .find((node): node is Element => node instanceof Element && root.contains(node)) ?? null;

  const annotatePreview = (event: Event) => {
    const host = root.getRootNode() instanceof ShadowRoot ? root.getRootNode().host : null;
    if (!(host instanceof HTMLElement) || host.dataset.capsemAnnotateMode !== "true") return;
    const target = annotateTarget(event);
    if (!target) return;
    const node = capsemNode(target);
    if (node === previewNode) return;
    previewNode = node;
    emit("annotate-preview", annotationPayload(root, target, spec));
  };

  const clearPreview = () => {
    if (!previewNode) return;
    previewNode = "";
    emit("annotate-preview", null);
  };

  const annotate = (event: Event) => {
    const host = root.getRootNode() instanceof ShadowRoot ? root.getRootNode().host : null;
    if (!(host instanceof HTMLElement) || host.dataset.capsemAnnotateMode !== "true") return;
    if (event instanceof KeyboardEvent && event.key !== "Enter" && event.key !== " ") return;
    const target = annotateTarget(event);
    if (!target) return;
    event.preventDefault();
    event.stopPropagation();
    emit("annotate", annotationPayload(root, target, spec));
  };
  root.addEventListener("pointermove", annotatePreview, { capture: true });
  root.addEventListener("pointerleave", clearPreview, { capture: true });
  root.addEventListener("click", annotate, { capture: true });
  root.addEventListener("keydown", annotate, { capture: true });
}

function numberDom(root: HTMLElement): void {
  [root, ...Array.from(root.querySelectorAll<HTMLElement>("*"))].forEach((node, index) => {
    node.dataset.capsemNode ||= `${root.dataset.capsemArtifactId ?? "artifact"}::dom::${index}`;
  });
}

function annotationPayload(
  root: HTMLElement,
  target: Element,
  spec: NativeArtifact,
): Record<string, unknown> {
  const shadowSelector = numberedSelector(target);
  return {
    kind: "domElement",
    label: labelForElement(target),
    topologyId: topologyId(target) || null,
    topologyRole: topologyRole(target) || null,
    path: domPath(root, target).map((segment) => segment.selector),
    shadowSelector,
    selectorVerified: root.querySelector(shadowSelector) === target,
    rect: rectPayload(target),
    metadata: {
      artifactId: spec.id,
      artifactKind: spec.kind,
      topologyId: topologyId(target),
      topologyRole: topologyRole(target),
      tag: target.localName,
      capsemNode: capsemNode(target),
      role: target.getAttribute("role") ?? "",
      ariaLabel: target.getAttribute("aria-label") ?? "",
      part: target.getAttribute("part") ?? "",
      classes: Array.from(target.classList),
      text: compactText(target.textContent),
      attributes: attributesFor(target),
      computedStyle: computedStyleFor(target),
      traversal: domPath(root, target),
    },
  };
}

function numberedSelector(el: Element): string {
  return `[data-capsem-node="${cssString(capsemNode(el))}"]`;
}

function labelForElement(el: Element): string {
  const aria = el.getAttribute("aria-label");
  if (aria) return aria;
  const part = el.getAttribute("part");
  const text = compactText(el.textContent);
  if (text) return `${el.localName}${part ? `[${part}]` : ""}: ${text.slice(0, 60)}`;
  return `${el.localName}${part ? `[${part}]` : ""}`;
}

function domPath(root: HTMLElement, target: Element): Array<Record<string, unknown>> {
  const path: Array<Record<string, unknown>> = [];
  let current: Element | null = target;
  while (current && root.contains(current)) {
    path.unshift({
      tag: current.localName,
      selector: numberedSelector(current),
      capsemNode: capsemNode(current),
      topologyId: topologyId(current),
      topologyRole: topologyRole(current),
      part: current.getAttribute("part") ?? "",
      role: current.getAttribute("role") ?? "",
      index: current.parentElement ? Array.from(current.parentElement.children).indexOf(current) : 0,
    });
    if (current === root) break;
    current = current.parentElement;
  }
  return path;
}

function attributesFor(el: Element): Record<string, string> {
  const attrs: Record<string, string> = {};
  for (const attr of Array.from(el.attributes)) {
    if (attr.name === "class" || attr.name === "style") continue;
    attrs[attr.name] = attr.value;
  }
  return attrs;
}

function computedStyleFor(el: Element): Record<string, string> {
  const style = getComputedStyle(el);
  return {
    display: style.display,
    position: style.position,
    boxSizing: style.boxSizing,
    width: style.width,
    height: style.height,
    margin: style.margin,
    padding: style.padding,
    gap: style.gap,
    alignItems: style.alignItems,
    justifyContent: style.justifyContent,
    fontSize: style.fontSize,
    lineHeight: style.lineHeight,
    color: style.color,
    backgroundColor: style.backgroundColor,
  };
}

function compactText(value: string | null): string {
  return (value ?? "").replace(/\s+/g, " ").trim();
}

function rectPayload(el: Element): Record<string, number> {
  const rect = el.getBoundingClientRect();
  return {
    top: rect.top,
    left: rect.left,
    width: rect.width,
    height: rect.height,
  };
}

function capsemNode(el: Element): string {
  return el.getAttribute("data-capsem-node") ?? "";
}

function topologyId(el: Element): string {
  return el.getAttribute("data-capsem-topology-id") ?? "";
}

function topologyRole(el: Element): string {
  return el.getAttribute("data-capsem-topology-role") ?? "";
}

function cssString(value: string): string {
  return value.replace(/\\/g, "\\\\").replace(/"/g, '\\"');
}

function applyStylePatches(root: HTMLElement, spec: NativeArtifact): void {
  const patches = Array.isArray(spec.spec.stylePatches) ? spec.spec.stylePatches : [];
  for (const patch of patches) {
    if (!isRecord(patch)) continue;
    const shadowSelector = valueText(patch.shadowSelector);
    if (!shadowSelector) continue;
    const target = root.querySelector<HTMLElement>(shadowSelector);
    if (!target || !isRecord(patch.styles)) continue;
    for (const [name, value] of Object.entries(patch.styles)) {
      const property = styleProperty(name);
      if (!property || typeof value !== "string") continue;
      target.style.setProperty(property, value);
    }
  }
}

function applyTextPatches(root: HTMLElement, spec: NativeArtifact): void {
  const patches = Array.isArray(spec.spec.textPatches) ? spec.spec.textPatches : [];
  for (const patch of patches) {
    if (!isRecord(patch)) continue;
    const shadowSelector = valueText(patch.shadowSelector);
    const text = valueText(patch.text);
    if (!shadowSelector || !text) continue;
    const target = root.querySelector<HTMLElement>(shadowSelector);
    if (!target) continue;
    target.textContent = text;
  }
}

function styleProperty(name: string): string | null {
  const allowed: Record<string, string> = {
    backgroundColor: "background-color",
    color: "color",
    fontStyle: "font-style",
    fontWeight: "font-weight",
    opacity: "opacity",
    textDecoration: "text-decoration",
  };
  return allowed[name] ?? null;
}

async function renderPlotlyChart(
  container: HTMLElement,
  spec: NativeArtifact,
  emit: <TDetail>(type: string, payload: TDetail) => void,
): Promise<void> {
  const plot = container.querySelector<HTMLElement>('[part~="plot"]');
  if (!plot) return;
  try {
    const mod = await import("plotly.js-dist-min");
    const Plotly = mod.default ?? mod;
    const data = Array.isArray(spec.spec.data) ? spec.spec.data.filter(isRecord) : [];
    const series = Array.isArray(spec.spec.series) ? spec.spec.series.filter(isRecord) : [];
    const xField = valueText(spec.spec.x);
    const chart = valueText(spec.spec.chart);
    const traces = series.map((item) => {
      const field = valueText(item.field);
      const trace: Record<string, unknown> = {
        name: valueText(item.name),
        x: data.map((row) => row[xField]),
        y: data.map((row) => row[field]),
        type: plotlyTraceType(chart),
      };
      if (valueText(item.axis) === "right") trace.yaxis = "y2";
      if (chart === "lineChart") trace.mode = "lines+markers";
      if (chart === "scatterPlot") trace.mode = "markers";
      if (chart === "barChart" && spec.spec.direction === "horizontal") {
        trace.orientation = "h";
        trace.x = data.map((row) => row[field]);
        trace.y = data.map((row) => row[xField]);
      }
      return trace;
    });
    const fit = isRecord(spec.spec.fit) ? spec.spec.fit : null;
    if (
      (chart === "lineChart" || chart === "scatterPlot") &&
      fit &&
      fit.display !== false &&
      valueText(fit.method) === "linear"
    ) {
      const firstSeries = series.find(isRecord);
      const fitTrace = firstSeries
        ? linearFitTrace(data, xField, valueText(firstSeries.field), valueText(firstSeries.name))
        : null;
      if (fitTrace) traces.push(fitTrace);
    }
    const stackMode = valueText(spec.spec.stack);
    const secondAxis = isRecord(spec.spec.secondAxis) ? spec.spec.secondAxis : null;
    const layout: Record<string, unknown> = {
      title: { text: spec.title },
      margin: { t: 36, r: 16, b: 44, l: 48 },
      paper_bgcolor: "transparent",
      plot_bgcolor: "transparent",
      font: { color: getComputedStyle(container).getPropertyValue("--foreground") || "currentColor" },
      showlegend: traces.length > 1,
      xaxis: { title: { text: valueText(spec.spec.xLabel) } },
      yaxis: { title: { text: axisTitle(spec.spec.yLabel, spec.spec.yUnit) } },
      barmode: stackMode === "stacked" ? "stack" : stackMode === "grouped" ? "group" : undefined,
    };
    if (secondAxis) {
      layout.yaxis2 = {
        title: { text: axisTitle(secondAxis.label, secondAxis.unit) },
        overlaying: "y",
        side: "right",
      };
    }
    await Plotly.newPlot(plot, traces, layout, {
      displayModeBar: false,
      responsive: true,
    });
    container.querySelector('[part~="export-svg"]')?.addEventListener("click", async () => {
      const url = await Plotly.toImage(plot, { format: "svg", height: 640, width: 960 });
      downloadDataUrl(`${spec.id}.svg`, url);
      emit("export", { artifactId: spec.id, format: "svg" });
    });
    container.querySelector('[part~="export-png"]')?.addEventListener("click", async () => {
      const url = await Plotly.toImage(plot, { format: "png", height: 640, width: 960 });
      downloadDataUrl(`${spec.id}.png`, url);
      emit("export", { artifactId: spec.id, format: "png" });
    });
    emit("rendered", { artifactId: spec.id, renderer: "plotly" });
  } catch (cause) {
    const message = cause instanceof Error ? cause.message : String(cause);
    plot.textContent = message;
    emit("error", { artifactId: spec.id, renderer: "plotly", message });
  }
}

function plotlyTraceType(chart: string): string {
  if (chart === "lineChart" || chart === "scatterPlot") return "scatter";
  if (chart === "heatmapChart") return "heatmap";
  if (chart === "boxPlot") return "box";
  return "bar";
}

function axisTitle(label: unknown, unit: unknown): string {
  const labelText = valueText(label);
  const unitText = valueText(unit);
  return unitText ? `${labelText} (${unitText})` : labelText;
}

function linearFitTrace(
  data: Record<string, unknown>[],
  xField: string,
  yField: string,
  name: string,
): Record<string, unknown> | null {
  const points = data
    .map((row, index) => ({ index, x: row[xField], y: Number(row[yField]) }))
    .filter((point) => Number.isFinite(point.y));
  if (points.length < 2) return null;
  const n = points.length;
  const sumX = points.reduce((sum, point) => sum + point.index, 0);
  const sumY = points.reduce((sum, point) => sum + point.y, 0);
  const sumXX = points.reduce((sum, point) => sum + point.index * point.index, 0);
  const sumXY = points.reduce((sum, point) => sum + point.index * point.y, 0);
  const denominator = n * sumXX - sumX * sumX;
  if (denominator === 0) return null;
  const slope = (n * sumXY - sumX * sumY) / denominator;
  const intercept = (sumY - slope * sumX) / n;
  return {
    name: `${name} linear fit`,
    x: points.map((point) => point.x),
    y: points.map((point) => intercept + slope * point.index),
    type: "scatter",
    mode: "lines",
    line: { dash: "dot" },
  };
}

async function renderMermaidDiagram(
  container: HTMLElement,
  spec: NativeArtifact,
  emit: <TDetail>(type: string, payload: TDetail) => void,
): Promise<void> {
  const preview = container.querySelector<HTMLElement>('[part~="diagramPreview"]');
  if (!preview) return;
  try {
    const mermaid = (await import("mermaid")).default;
    mermaid.initialize({ startOnLoad: false, securityLevel: "strict" });
    const id = `capsem-${spec.id.replace(/[^a-zA-Z0-9_-]/g, "-")}`;
    const rendered = await mermaid.render(id, valueText(spec.spec.source));
    const parsed = new DOMParser().parseFromString(rendered.svg, "image/svg+xml");
    parsed.querySelectorAll("script").forEach((script) => script.remove());
    const svg = parsed.documentElement;
    const source = document.createElement("pre");
    source.part.add("code");
    source.hidden = true;
    source.textContent = valueText(spec.spec.source);
    markTopology(source, spec, "source", "source");
    preview.replaceChildren(document.importNode(svg, true), source);
    annotateMermaidTopology(preview, spec);
    container.querySelector('[part~="export-svg"]')?.addEventListener("click", () => {
      downloadDataUrl(`${spec.id}.svg`, `data:image/svg+xml;charset=utf-8,${encodeURIComponent(rendered.svg)}`);
      emit("export", { artifactId: spec.id, format: "svg" });
    });
    emit("rendered", { artifactId: spec.id, renderer: "mermaid" });
  } catch (cause) {
    const message = cause instanceof Error ? cause.message : String(cause);
    preview.textContent = message;
    emit("error", { artifactId: spec.id, renderer: "mermaid", message });
  }
}

function annotateMermaidTopology(preview: HTMLElement, spec: NativeArtifact): void {
  preview.querySelectorAll(".node").forEach((node, index) => {
    markTopologyElement(node, spec, "node", `node::${index}`);
  });
  preview.querySelectorAll(".edgePath, .edgeLabel").forEach((node, index) => {
    markTopologyElement(node, spec, "edge", `edge::${index}`);
  });
}

function downloadDataUrl(filename: string, url: string): void {
  const link = document.createElement("a");
  link.href = url;
  link.download = filename;
  link.click();
}

function arrayOfStrings(value: unknown): string[] {
  return Array.isArray(value) ? value.filter((item): item is string => typeof item === "string") : [];
}

function valueText(value: unknown): string {
  if (typeof value === "string") return value;
  if (typeof value === "number" || typeof value === "boolean") return String(value);
  if (value === null || value === undefined) return "";
  return JSON.stringify(value);
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

const artifactStyle = `
  [part="artifact"] {
    border: 1px solid var(--card-line);
    border-radius: 0.75rem;
    background: var(--card);
    color: var(--foreground);
    overflow: hidden;
  }

  [part="header"] {
    border-bottom: 1px solid var(--card-line);
    padding: 0.875rem 1rem;
  }

  [part="title"] {
    margin: 0;
    font-size: 0.95rem;
    font-weight: 650;
    cursor: default;
    border-radius: 0.375rem;
    outline: none;
    width: fit-content;
    max-width: 100%;
  }

  :host([data-capsem-annotate-mode="true"]) [data-capsem-node] {
    cursor: url("data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='24' height='24' viewBox='0 0 24 24' fill='%232563eb'%3E%3Cpath d='M6.75 4A4.75 4.75 0 0 0 2 8.75v5.5A4.75 4.75 0 0 0 6.75 19H8v2.25a.75.75 0 0 0 1.18.61L13.25 19h4A4.75 4.75 0 0 0 22 14.25v-5.5A4.75 4.75 0 0 0 17.25 4H6.75Z'/%3E%3C/svg%3E") 4 4, cell;
  }

  [part="meta"], [part="body"] {
    margin: 0.25rem 0 0;
    color: var(--muted-foreground-1);
    font-size: 0.8125rem;
    line-height: 1.3rem;
  }

  [part="table"] {
    width: 100%;
    border-collapse: collapse;
    font-size: 0.8125rem;
  }

  th, td {
    border-bottom: 1px solid var(--card-line);
    padding: 0.625rem 0.75rem;
    text-align: left;
  }

  tbody tr {
    cursor: default;
    outline: none;
  }

  th {
    color: var(--muted-foreground-1);
    font-weight: 650;
  }

  [part="chart"], [part="diagram"], [part="timeline"], [part="media"], [part="blocks"], [part="deck"], [part="code"] {
    margin: 0;
    padding: 1rem;
  }

  [part="plot"], [part="diagramPreview"] {
    min-height: 18rem;
    margin-top: 0.75rem;
  }

  [part="diagramPreview"] svg {
    max-width: 100%;
    height: auto;
  }

  [part="exportButton"] {
    display: inline-flex;
    align-items: center;
    border: 1px solid var(--card-line);
    border-radius: 0.5rem;
    background: var(--surface);
    color: var(--foreground);
    padding: 0.375rem 0.625rem;
    margin-right: 0.5rem;
    font-size: 0.75rem;
    font-weight: 650;
    cursor: pointer;
  }

  [part="chartKind"] {
    margin: 0 0 0.75rem;
    color: var(--muted-foreground-1);
    font-size: 0.8125rem;
  }

  [part="bars"] {
    display: grid;
    gap: 0.5rem;
  }

  [part="legend"] {
    display: flex;
    flex-wrap: wrap;
    gap: 0.375rem;
    margin-bottom: 0.75rem;
  }

  [part="legendItem"] {
    border: 1px solid var(--card-line);
    border-radius: 999px;
    color: var(--muted-foreground-1);
    padding: 0.25rem 0.5rem;
    font-size: 0.75rem;
  }

  [part="bar"] {
    min-width: 4rem;
    border-radius: 999px;
    background: var(--primary);
    color: var(--primary-foreground);
    padding: 0.45rem 0.75rem;
    font-size: 0.75rem;
    font-weight: 650;
  }

  [part="timeline"] {
    display: grid;
    gap: 0.875rem;
  }

  [part="timelineLane"] {
    display: grid;
    gap: 0.5rem;
    border-left: 2px solid var(--primary);
    padding-left: 0.875rem;
  }

  [part="timelineLaneTitle"] {
    margin: 0;
    color: var(--foreground);
    font-size: 0.875rem;
    font-weight: 700;
  }

  [part="timelineEvent"] {
    border: 1px solid var(--card-line);
    border-radius: 0.625rem;
    background: var(--surface);
    padding: 0.75rem;
    outline: none;
  }

  [part="timelineDate"] {
    display: block;
    color: var(--muted-foreground-1);
    font-size: 0.75rem;
    font-weight: 650;
    margin-bottom: 0.25rem;
  }

  [part="timelineTitle"] {
    display: block;
    color: var(--foreground);
    font-size: 0.875rem;
  }

  [part="code"] {
    overflow: auto;
    background: var(--surface);
    color: var(--foreground);
    font: 0.8125rem/1.45 ui-monospace, SFMono-Regular, Menlo, monospace;
  }

  [part="mediaFrame"] {
    aspect-ratio: 16 / 9;
    display: grid;
    place-items: center;
    border-radius: 0.625rem;
    background: color-mix(in oklab, var(--primary) 14%, var(--surface));
    color: var(--primary);
    font-weight: 700;
  }

  [part="mediaImage"] {
    display: block;
    width: 100%;
    border-radius: 0.625rem;
    object-fit: cover;
  }

  [part="errorText"] {
    color: var(--destructive);
    font-size: 0.8125rem;
    line-height: 1.3rem;
    margin: 0.5rem 0 0;
  }

  [part="blocks"] {
    display: grid;
    gap: 0.625rem;
  }

  [part="block"], [part="imageBlock"], [part="slideCard"] {
    border: 1px solid var(--card-line);
    border-radius: 0.625rem;
    background: var(--surface);
    padding: 0.75rem;
    font-size: 0.8125rem;
  }

  [part="imageBlock"] {
    min-height: 7rem;
    display: flex;
    flex-direction: column;
    justify-content: space-between;
    background: color-mix(in oklab, var(--primary) 10%, var(--surface));
  }

  [part="blockTitle"] {
    display: block;
    color: var(--foreground);
    font-weight: 700;
  }

  [part="deck"] {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(9rem, 1fr));
    gap: 0.75rem;
  }

  [part="slideCard"] {
    min-height: 6rem;
    display: flex;
    flex-direction: column;
    justify-content: space-between;
  }

  [part="slideNumber"] {
    color: var(--muted-foreground-1);
    font-size: 0.75rem;
  }
`;
