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

export type NativeArtifact = {
  id: string;
  kind: NativeArtifactKind;
  title: string;
  handle: string;
  spec: Record<string, unknown>;
};

export type NativeArtifactKind =
  | "generatedImage"
  | "sheet"
  | "table"
  | "chart"
  | "diagram"
  | "slide"
  | "slideDeck";

export async function loadNativeDeckProof(fetcher: typeof fetch = fetch): Promise<NativeDeckProof> {
  const response = await fetcher("/native/deck-proof");
  const payload = await response.json();
  return parseNativeDeckProof(payload);
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
    case "generatedImage":
      return "capsem-media";
    case "slide":
      return "capsem-slide";
    case "slideDeck":
      return "capsem-slide-deck";
  }
}

function parseNativeArtifact(value: unknown): NativeArtifact {
  assertRecord(value, "native artifact");
  const spec = value.spec;
  assertRecord(spec, "native artifact spec");
  return {
    id: requireString(value.id, "artifact.id"),
    kind: requireKind(value.kind),
    title: requireString(value.title, "artifact.title"),
    handle: requireString(value.handle, "artifact.handle"),
    spec,
  };
}

function requireKind(value: unknown): NativeArtifactKind {
  const allowed = new Set([
    "generatedImage",
    "sheet",
    "table",
    "chart",
    "diagram",
    "slide",
    "slideDeck",
  ]);
  if (typeof value !== "string" || !allowed.has(value)) {
    throw new Error("artifact.kind is not a supported native artifact kind");
  }
  return value as NativeArtifactKind;
}

function requireString(value: unknown, name: string): string {
  if (typeof value !== "string") throw new Error(`${name} must be string`);
  return value;
}

function requireNumber(value: unknown, name: string): number {
  if (typeof value !== "number") throw new Error(`${name} must be number`);
  return value;
}

function requireBoolean(value: unknown, name: string): boolean {
  if (typeof value !== "boolean") throw new Error(`${name} must be boolean`);
  return value;
}

function requireArray(value: unknown, name: string): unknown[] {
  if (!Array.isArray(value)) throw new Error(`${name} must be array`);
  return value;
}

function assertRecord(value: unknown, name: string): asserts value is Record<string, unknown> {
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    throw new Error(`${name} must be object`);
  }
}
