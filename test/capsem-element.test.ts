import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

const source = readFileSync("ui-preview/src/elements/capsem-elt.ts", "utf8");
const artifactSource = readFileSync("ui-preview/src/elements/capsem-artifacts.ts", "utf8");
const chatSource = readFileSync("ui-preview/src/ChatPage.svelte", "utf8");
const entrypoint = readFileSync("ui-preview/src/main.ts", "utf8");
const elementIndex = readFileSync("ui-preview/src/elements/index.ts", "utf8");

describe("capsem element foundation", () => {
  it("uses Shadow DOM as the default UI island boundary", () => {
    expect(source).toContain('attachShadow({ mode: "open" })');
    expect(source).toContain('customElements.define(tagName, elementClass)');
    expect(entrypoint).toContain("defineCapsemElements()");
  });

  it("emits composed Capsem events across the shadow boundary", () => {
    expect(source).toContain("new CustomEvent(`capsem:${type}`");
    expect(source).toContain("bubbles: true");
    expect(source).toContain("composed: true");
    expect(source).toContain("specId: specId(this.currentSpec)");
  });

  it("centralizes lifecycle cleanup and render error handling", () => {
    expect(source).toContain("cleanupCallbacks");
    expect(source).toContain("disconnectedCallback()");
    expect(source).toContain("this.runCleanup()");
    expect(source).toContain('this.emit("error"');
  });

  it("registers native artifact components on the same element foundation", () => {
    expect(artifactSource).toContain("extends CapsemElement<NativeArtifact>");
    expect(elementIndex).toContain('defineCapsemElement("capsem-sheet"');
    expect(elementIndex).toContain('defineCapsemElement("capsem-chart"');
    expect(elementIndex).toContain('defineCapsemElement("capsem-diagram"');
    expect(elementIndex).toContain('defineCapsemElement("capsem-timeline"');
    expect(elementIndex).toContain('"capsem-slide-deck"');
    expect(elementIndex).toContain("class CapsemSlideDeckElement extends CapsemArtifactElement");
  });

  it("emits stable topology-backed DOM annotation targets from artifact surfaces", () => {
    expect(artifactSource).toContain('emit("annotate"');
    expect(artifactSource).toContain('emit("annotate-preview"');
    expect(artifactSource).toContain("installGenericAnnotator");
    expect(artifactSource).toContain('root.addEventListener("pointermove"');
    expect(artifactSource).toContain("data-capsem-artifact-id");
    expect(artifactSource).toContain("markTopology");
    expect(artifactSource).toContain('setAttribute("data-capsem-topology-id"');
    expect(artifactSource).toContain('setAttribute("data-capsem-topology-role"');
    expect(artifactSource).toContain("dataset.capsemNode");
    expect(artifactSource).toContain('kind: "domElement"');
    expect(artifactSource).toContain("topologyId: topologyId(target)");
    expect(artifactSource).toContain("topologyRole: topologyRole(target)");
    expect(artifactSource).toContain("metadata: {");
    expect(artifactSource).toContain("traversal: domPath");
  });

  it("applies selector-scoped style patches from artifact state", () => {
    expect(artifactSource).toContain("applyStylePatches(shell, spec)");
    expect(artifactSource).toContain("spec.spec.stylePatches");
    expect(artifactSource).toContain("root.querySelector<HTMLElement>(shadowSelector)");
    expect(artifactSource).toContain("fontWeight");
    expect(artifactSource).not.toContain('[part="media"] > [part="meta"]');
  });

  it("maps the frozen Plotly chart catalogue to concrete renderer traces", () => {
    expect(artifactSource).toContain("function plotlyTraceType");
    expect(artifactSource).toContain('chart === "lineChart" || chart === "scatterPlot"');
    expect(artifactSource).toContain('chart === "heatmapChart"');
    expect(artifactSource).toContain('return "heatmap"');
    expect(artifactSource).toContain('chart === "boxPlot"');
    expect(artifactSource).toContain('return "box"');
    expect(artifactSource).toContain('chart === "barChart" && spec.spec.direction === "horizontal"');
    expect(artifactSource).toContain('barmode: stackMode === "stacked" ? "stack"');
    expect(artifactSource).toContain('trace.yaxis = "y2"');
    expect(artifactSource).toContain("layout.yaxis2");
    expect(artifactSource).toContain("linearFitTrace");
    expect(artifactSource).toContain('name: `${name} linear fit`');
    expect(artifactSource).toContain('emit("export", { artifactId: spec.id, format: "svg" })');
    expect(artifactSource).toContain('emit("export", { artifactId: spec.id, format: "png" })');
  });

  it("renders timeline artifacts as structured annotatable lanes", () => {
    expect(artifactSource).toContain('case "timeline"');
    expect(artifactSource).toContain("function timelinePreview");
    expect(artifactSource).toContain('el("section", "timelineLane")');
    expect(artifactSource).toContain('el("article", "timelineEvent")');
    expect(artifactSource).toContain('item.part.add("annotatable")');
    expect(artifactSource).toContain('markTopology(item, spec, "event"');
  });

  it("applies selector-scoped style patches to chat shell chrome", () => {
    expect(chatSource).toContain("applyElementStylePatches");
    expect(chatSource).toContain("applyLightDomStylePatches");
    expect(chatSource).toContain('data-capsem-role="card-title"');
    expect(chatSource).toContain('selector.includes(">>>")');
    expect(chatSource).toContain("target.style.setProperty(property, value)");
    expect(chatSource).toContain("topologyId: typeof payload.topologyId");
    expect(chatSource).toContain("item.annotation?.topologyId");
  });

  it("gates annotation behind the chat comment tool", () => {
    expect(artifactSource).toContain('dataset.capsemAnnotateMode !== "true"');
    expect(chatSource).toContain("let commentMode = $state(false)");
    expect(chatSource).toContain("let previewAnnotationAnchor = $state");
    expect(chatSource).toContain("data-capsem-annotate-mode={commentMode");
    expect(chatSource).toContain("capsem:annotate-preview");
    expect(chatSource).toContain("previewLightElement");
    expect(chatSource).toContain("commentMarkers");
    expect(chatSource).toContain("reopenFeedbackMarker");
    expect(chatSource).toContain("toggleCommentMode");
  });
});
