import { CapsemElement, type CapsemElementRenderContext } from "./capsem-elt";
import type { NativeArtifact } from "../nativeDeck";

export class CapsemArtifactElement extends CapsemElement<NativeArtifact> {
  protected render({ spec, emit }: CapsemElementRenderContext<NativeArtifact>): void {
    const style = document.createElement("style");
    style.textContent = artifactStyle;
    const shell = el("section", "artifact");
    shell.append(header(spec));

    switch (spec.kind) {
      case "sheet":
      case "table":
        shell.append(sheetPreview(spec));
        break;
      case "chart":
        shell.append(chartPreview(spec));
        break;
      case "diagram":
        shell.append(diagramPreview(spec));
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
  const pre = document.createElement("pre");
  pre.part.add("code");
  pre.textContent = source;
  return pre;
}

function mediaPreview(spec: NativeArtifact): HTMLElement {
  const box = el("div", "media");
  box.append(el("div", "mediaFrame", spec.title));
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

  [part="chart"], [part="media"], [part="blocks"], [part="deck"], [part="code"] {
    margin: 0;
    padding: 1rem;
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
