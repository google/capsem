import { CapsemElement, type CapsemElementRenderContext } from "./capsem-elt";
import type { NativeArtifact } from "../nativeDeck";

export class CapsemArtifactElement extends CapsemElement<NativeArtifact> {
  protected render({ spec, emit }: CapsemElementRenderContext<NativeArtifact>): void {
    const style = document.createElement("style");
    style.textContent = artifactStyle;
    const shell = el("section", "artifact");
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

    this.replaceRootChildren(style, shell);
    if (hydrate) queueMicrotask(hydrate);
    emit("ready", { kind: spec.kind, artifactId: spec.id });
  }
}

function header(spec: NativeArtifact): HTMLElement {
  const box = el("div", "header");
  const title = el("h3", "title", spec.title);
  const meta = el("p", "meta", `${spec.kind} · ${spec.id}`);
  box.append(title, meta);
  return box;
}

function sheetPreview(spec: NativeArtifact): HTMLElement {
  const columns = arrayOfStrings(spec.spec.columns);
  const rows = Array.isArray(spec.spec.rows) ? spec.spec.rows.slice(0, 6) : [];
  const table = document.createElement("table");
  table.part.add("table");
  const thead = document.createElement("thead");
  const headRow = document.createElement("tr");
  for (const column of columns) {
    const cell = document.createElement("th");
    cell.textContent = column;
    headRow.append(cell);
  }
  thead.append(headRow);
  const tbody = document.createElement("tbody");
  for (const row of rows) {
    const tr = document.createElement("tr");
    for (const column of columns) {
      const cell = document.createElement("td");
      cell.textContent = valueText(isRecord(row) ? row[column] : undefined);
      tr.append(cell);
    }
    tbody.append(tr);
  }
  table.append(thead, tbody);
  return table;
}

function chartPreview(spec: NativeArtifact): HTMLElement {
  const box = el("div", "chart");
  const chart = valueText(spec.spec.chart);
  const series = Array.isArray(spec.spec.series) ? spec.spec.series : [];
  const data = Array.isArray(spec.spec.data) ? spec.spec.data.filter(isRecord) : [];
  const xField = valueText(spec.spec.x);
  const firstSeries = series.find(isRecord);
  const yField = firstSeries ? valueText(firstSeries.field) : "";
  box.append(el("p", "chartKind", chart || "chart"));
  box.append(exportButton("export-svg", "Export SVG"));
  box.append(exportButton("export-png", "Export PNG"));
  const plot = el("div", "plot");
  box.append(plot);
  const bars = el("div", "bars");
  const values = data
    .map((row) => Number(row[yField]))
    .filter((value) => Number.isFinite(value) && value > 0);
  const max = Math.max(1, ...values);

  if (data.length > 0 && yField) {
    for (const row of data) {
      const label = valueText(row[xField]);
      const value = Number(row[yField]);
      const bar = el("div", "bar");
      bar.style.width = `${Math.max(14, Math.round((value / max) * 100))}%`;
      bar.textContent = `${label} · ${Number.isFinite(value) ? value : "n/a"}`;
      bars.append(bar);
    }
  } else {
    series.forEach((item, index) => {
      const name = isRecord(item) ? valueText(item.name) : `series ${index + 1}`;
      const bar = el("div", "bar");
      bar.style.width = `${Math.max(32, 88 - index * 18)}%`;
      bar.textContent = name;
      bars.append(bar);
    });
  }
  if (series.length > 1) {
    const legend = el("div", "legend");
    for (const item of series) {
      if (!isRecord(item)) continue;
      legend.append(el("span", "legendItem", valueText(item.name)));
    }
    box.append(legend);
  }
  box.append(bars);
  return box;
}

function diagramPreview(spec: NativeArtifact): HTMLElement {
  const source = valueText(spec.spec.source);
  const box = el("div", "diagram");
  box.append(exportButton("export-svg", "Export SVG"));
  const preview = el("div", "diagramPreview");
  const pre = document.createElement("pre");
  pre.part.add("code");
  pre.textContent = source;
  preview.append(pre);
  box.append(preview);
  return box;
}

function textPreview(spec: NativeArtifact): HTMLElement {
  const box = el("div", "textArtifact");
  const text = valueText(spec.spec.text);
  const prompt = valueText(spec.spec.prompt);
  const status = valueText(spec.spec.status);
  if (status) box.append(el("p", "meta", status));
  box.append(el("p", "body", text || prompt));
  const error = valueText(spec.spec.error);
  if (error) box.append(el("p", "errorText", error));
  return box;
}

function mediaPreview(spec: NativeArtifact): HTMLElement {
  const box = el("div", "media");
  const dataUrl = valueText(spec.spec.dataUrl);
  if (dataUrl.startsWith("data:image/")) {
    const image = document.createElement("img");
    image.part.add("mediaImage");
    image.alt = spec.title;
    image.src = dataUrl;
    box.append(image);
  } else {
    box.append(el("div", "mediaFrame", spec.title));
  }
  const status = valueText(spec.spec.status);
  if (status) box.append(el("p", "meta", status));
  const error = valueText(spec.spec.error);
  if (error) box.append(el("p", "errorText", error));
  box.append(el("p", "body", valueText(spec.spec.prompt)));
  return box;
}

function slidePreview(spec: NativeArtifact): HTMLElement {
  const blocks = Array.isArray(spec.spec.blocks) ? spec.spec.blocks : [];
  const box = el("div", "blocks");
  for (const block of blocks) {
    if (!isRecord(block)) {
      box.append(el("div", "block", valueText(block)));
      continue;
    }
    const kind = valueText(block.kind);
    const item = el("div", kind === "image" ? "imageBlock" : "block");
    if (kind === "text") {
      item.append(el("strong", "blockTitle", valueText(block.title)));
      item.append(el("p", "body", valueText(block.body)));
    } else if (kind === "image") {
      item.append(el("span", "slideNumber", "Generated image"));
      item.append(el("strong", "blockTitle", valueText(block.artifactId)));
    } else {
      item.textContent = `${kind} · ${valueText(block.title ?? block.artifactId)}`;
    }
    box.append(item);
  }
  return box;
}

function deckPreview(spec: NativeArtifact): HTMLElement {
  const slides = Array.isArray(spec.spec.slides) ? spec.spec.slides : [];
  const grid = el("div", "deck");
  slides.forEach((slide, index) => {
    const card = el("div", "slideCard");
    const title = isRecord(slide) ? valueText(slide.title) : `Slide ${index + 1}`;
    card.append(el("span", "slideNumber", String(index + 1)), el("strong", "slideTitle", title));
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
    const traces = series.map((item) => {
      const field = valueText(item.field);
      const trace: Record<string, unknown> = {
        name: valueText(item.name),
        x: data.map((row) => row[xField]),
        y: data.map((row) => row[field]),
        type: spec.spec.chart === "lineChart" ? "scatter" : "bar",
      };
      if (spec.spec.chart === "lineChart") trace.mode = "lines+markers";
      if (spec.spec.direction === "horizontal") {
        trace.orientation = "h";
        trace.x = data.map((row) => row[field]);
        trace.y = data.map((row) => row[xField]);
      }
      return trace;
    });
    await Plotly.newPlot(plot, traces, {
      title: { text: spec.title },
      margin: { t: 36, r: 16, b: 44, l: 48 },
      paper_bgcolor: "transparent",
      plot_bgcolor: "transparent",
      font: { color: getComputedStyle(container).getPropertyValue("--foreground") || "currentColor" },
      showlegend: series.length > 1,
    }, {
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
    plot.textContent = cause instanceof Error ? cause.message : String(cause);
    emit("error", { artifactId: spec.id, renderer: "plotly" });
  }
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
    preview.replaceChildren(document.importNode(svg, true));
    container.querySelector('[part~="export-svg"]')?.addEventListener("click", () => {
      downloadDataUrl(`${spec.id}.svg`, `data:image/svg+xml;charset=utf-8,${encodeURIComponent(rendered.svg)}`);
      emit("export", { artifactId: spec.id, format: "svg" });
    });
    emit("rendered", { artifactId: spec.id, renderer: "mermaid" });
  } catch (cause) {
    preview.textContent = cause instanceof Error ? cause.message : String(cause);
    emit("error", { artifactId: spec.id, renderer: "mermaid" });
  }
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

  th {
    color: var(--muted-foreground-1);
    font-weight: 650;
  }

  [part="chart"], [part="diagram"], [part="media"], [part="blocks"], [part="deck"], [part="code"] {
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
