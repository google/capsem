import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

const source = readFileSync("ui-preview/src/elements/capsem-elt.ts", "utf8");
const artifactSource = readFileSync("ui-preview/src/elements/capsem-artifacts.ts", "utf8");
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
    expect(elementIndex).toContain('"capsem-slide-deck"');
    expect(elementIndex).toContain("class CapsemSlideDeckElement extends CapsemArtifactElement");
  });
});
