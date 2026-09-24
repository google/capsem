import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

import {
  ARTIFACT_TOPOLOGY_REGISTRY,
  implementedArtifactTopology,
  type ArtifactTopologyEntry,
} from "../ui-preview/src/elements/topologyRegistry.js";

const topologySnapshot = JSON.parse(
  readFileSync("schemas/capsem-ui/artifacts/topology-targets.v1.json", "utf8"),
) as {
  artifactTopologyTargets: Record<string, string[]>;
};

const rendererSource = readFileSync("ui-preview/src/elements/capsem-artifacts.ts", "utf8");

describe("artifact topology registry", () => {
  it("covers the same artifact families as the Rust topology snapshot", () => {
    expect(Object.keys(ARTIFACT_TOPOLOGY_REGISTRY).sort()).toEqual(
      Object.keys(topologySnapshot.artifactTopologyTargets).sort(),
    );
  });

  it("matches Rust-declared topology targets for implemented renderer families", () => {
    for (const [artifact, roles] of Object.entries(implementedArtifactTopology()) as Array<
      [string, readonly string[]]
    >) {
      expect([...roles].sort(), artifact).toEqual(
        [...topologySnapshot.artifactTopologyTargets[artifact]].sort(),
      );
    }
  });

  it("keeps deferred renderer families visible without pretending topology support", () => {
    const entries = Object.entries(ARTIFACT_TOPOLOGY_REGISTRY) as Array<[string, ArtifactTopologyEntry]>;
    const deferred = entries.filter(([, entry]) => entry.status === "deferred");
    expect(deferred.map(([artifact]) => artifact).sort()).toEqual([
      "audio",
      "form",
      "page",
      "spreadsheet",
      "video",
      "website",
    ]);
    for (const [, entry] of deferred) expect(entry.roles).toEqual([]);
  });

  it("stamps every implemented topology role in the artifact renderer source", () => {
    for (const [artifact, roles] of Object.entries(implementedArtifactTopology()) as Array<
      [string, readonly string[]]
    >) {
      for (const role of roles) {
        expect(rendererSource, `${artifact} must stamp topology role ${role}`).toContain(`"${role}"`);
      }
    }
    expect(rendererSource).toContain("annotateMermaidTopology");
    expect(rendererSource).toContain('markTopologyElement(node, spec, "node"');
    expect(rendererSource).toContain('markTopologyElement(node, spec, "edge"');
  });
});
