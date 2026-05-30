import type { A2uiMessage, PreviewRecipe } from "./a2ui.js";

export type A2uiRenderResponse = {
  ok: boolean;
  conformance: {
    ok: boolean;
    errors: string[];
  };
  observations: unknown[];
  surfaces: A2uiRenderSurface[];
};

export type A2uiRenderSurface = {
  surfaceId: string;
  catalogId: string;
  componentCount: number;
  recipe?: PreviewRecipe;
  messages: A2uiMessage[];
};

export async function renderA2ui(
  endpoint: string,
  program: unknown,
  fetcher: typeof fetch = fetch,
): Promise<A2uiRenderResponse> {
  const response = await fetcher(endpoint, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(program),
  });
  const payload = await response.json();
  const parsed = parseA2uiRenderResponse(payload);
  if (!response.ok || !parsed.ok) {
    throw new Error(`A2UI render failed: ${response.status}`);
  }
  return parsed;
}

export function parseA2uiRenderResponse(value: unknown): A2uiRenderResponse {
  assertRecord(value, "response");
  if (typeof value.ok !== "boolean") throw new Error("response.ok must be boolean");
  assertRecord(value.conformance, "response.conformance");
  if (typeof value.conformance.ok !== "boolean") {
    throw new Error("response.conformance.ok must be boolean");
  }
  if (!Array.isArray(value.conformance.errors)) {
    throw new Error("response.conformance.errors must be array");
  }
  if (!Array.isArray(value.observations)) {
    throw new Error("response.observations must be array");
  }
  if (!Array.isArray(value.surfaces)) {
    throw new Error("response.surfaces must be array");
  }

  return {
    ok: value.ok,
    conformance: {
      ok: value.conformance.ok,
      errors: value.conformance.errors.map((error) => {
        if (typeof error !== "string") throw new Error("conformance error must be string");
        return error;
      }),
    },
    observations: value.observations,
    surfaces: value.surfaces.map(parseSurface),
  };
}

function parseSurface(value: unknown): A2uiRenderSurface {
  assertRecord(value, "surface");
  if (typeof value.surfaceId !== "string") throw new Error("surface.surfaceId must be string");
  if (typeof value.catalogId !== "string") throw new Error("surface.catalogId must be string");
  if (typeof value.componentCount !== "number") {
    throw new Error("surface.componentCount must be number");
  }
  if (!Array.isArray(value.messages)) throw new Error("surface.messages must be array");
  return {
    surfaceId: value.surfaceId,
    catalogId: value.catalogId,
    componentCount: value.componentCount,
    recipe: value.recipe === undefined ? undefined : parseRecipe(value.recipe),
    messages: value.messages as A2uiMessage[],
  };
}

function parseRecipe(value: unknown): PreviewRecipe {
  assertRecord(value, "recipe");
  if (typeof value.component !== "string") throw new Error("recipe.component must be string");
  if (typeof value.variant !== "string") throw new Error("recipe.variant must be string");
  if (typeof value.docsUrl !== "string") throw new Error("recipe.docsUrl must be string");
  return {
    component: value.component,
    variant: value.variant,
    docsUrl: value.docsUrl,
    tone: value.tone === undefined ? undefined : requireString(value.tone, "recipe.tone"),
    link: value.link === undefined ? undefined : parseLink(value.link),
  };
}

function parseLink(value: unknown): { label: string; href: string } {
  assertRecord(value, "recipe.link");
  return {
    label: requireString(value.label, "recipe.link.label"),
    href: requireString(value.href, "recipe.link.href"),
  };
}

function requireString(value: unknown, name: string): string {
  if (typeof value !== "string") throw new Error(`${name} must be string`);
  return value;
}

function assertRecord(value: unknown, name: string): asserts value is Record<string, unknown> {
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    throw new Error(`${name} must be object`);
  }
}
