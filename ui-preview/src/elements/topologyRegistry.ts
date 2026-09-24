export type ArtifactTopologyStatus = "implemented" | "deferred";

export type ArtifactTopologyEntry = {
  status: ArtifactTopologyStatus;
  roles: readonly string[];
};

export const ARTIFACT_TOPOLOGY_REGISTRY = {
  audio: {
    status: "deferred",
    roles: [],
  },
  chart: {
    status: "implemented",
    roles: ["title", "legend", "xAxis", "yAxis", "secondaryYAxis", "series", "dataPoint"],
  },
  diagram: {
    status: "implemented",
    roles: ["title", "source", "node", "edge"],
  },
  form: {
    status: "deferred",
    roles: [],
  },
  image: {
    status: "implemented",
    roles: ["title", "media", "caption", "source"],
  },
  page: {
    status: "deferred",
    roles: [],
  },
  sheet: {
    status: "implemented",
    roles: ["sheet", "cell", "range", "row", "column"],
  },
  slide: {
    status: "implemented",
    roles: ["slideTitle", "region", "textBlock", "image", "chart", "diagram"],
  },
  slideDeck: {
    status: "implemented",
    roles: ["deckTitle", "slide"],
  },
  spreadsheet: {
    status: "deferred",
    roles: [],
  },
  timeline: {
    status: "implemented",
    roles: ["title", "lane", "event", "date", "annotation"],
  },
  video: {
    status: "deferred",
    roles: [],
  },
  website: {
    status: "deferred",
    roles: [],
  },
} as const satisfies Record<string, ArtifactTopologyEntry>;

export function implementedArtifactTopology(): Record<string, readonly string[]> {
  return Object.fromEntries(
    Object.entries(ARTIFACT_TOPOLOGY_REGISTRY)
      .filter(([, entry]) => entry.status === "implemented")
      .map(([artifact, entry]) => [artifact, entry.roles]),
  );
}
