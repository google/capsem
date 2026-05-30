import { describe, expect, it } from "vitest";

import { parseA2uiRenderResponse, renderA2ui } from "../ui-preview/src/a2uiClient.js";

const conformantResponse = {
  ok: true,
  conformance: { ok: true, errors: [] },
  observations: [],
  surfaces: [
    {
      surfaceId: "agent-draft",
      catalogId: "https://a2ui.org/specification/v0_9/catalogs/basic/catalog.json",
      componentCount: 2,
      recipe: {
        component: "card",
        variant: "simple",
        docsUrl: "https://preline.co/docs/components/card.html#simple-card",
        link: {
          label: "visit homepage",
          href: "https://gemini.google.com/",
        },
      },
      messages: [
        {
          version: "v0.9",
          createSurface: {
            surfaceId: "agent-draft",
            catalogId: "https://a2ui.org/specification/v0_9/catalogs/basic/catalog.json",
          },
        },
        {
          version: "v0.9",
          updateComponents: {
            surfaceId: "agent-draft",
            components: [],
          },
        },
      ],
    },
  ],
};

describe("a2uiClient", () => {
  it("parses the Hyper render response and preserves recipe link fields", () => {
    const parsed = parseA2uiRenderResponse(conformantResponse);

    expect(parsed.ok).toBe(true);
    expect(parsed.surfaces[0].recipe?.component).toBe("card");
    expect(parsed.surfaces[0].recipe?.link).toEqual({
      label: "visit homepage",
      href: "https://gemini.google.com/",
    });
  });

  it("parses warning alert tone fields", () => {
    const parsed = parseA2uiRenderResponse({
      ...conformantResponse,
      surfaces: [
        {
          ...conformantResponse.surfaces[0],
          recipe: {
            component: "alert",
            variant: "soft",
            docsUrl: "https://preline.co/docs/components/alerts.html#discovery",
            tone: "warning",
          },
        },
      ],
    });

    expect(parsed.surfaces[0].recipe?.tone).toBe("warning");
  });

  it("rejects malformed response shapes", () => {
    expect(() => parseA2uiRenderResponse({ ok: true })).toThrow(/conformance/);
    expect(() =>
      parseA2uiRenderResponse({
        ...conformantResponse,
        surfaces: [{ ...conformantResponse.surfaces[0], recipe: { component: "card" } }],
      }),
    ).toThrow(/variant/);
  });

  it("posts tool programs and rejects non-ok HTTP responses", async () => {
    const fetcher = async () =>
      new Response(JSON.stringify(conformantResponse), {
        status: 200,
        headers: { "content-type": "application/json" },
      });

    await expect(renderA2ui("http://127.0.0.1/a2ui/render", { calls: [] }, fetcher)).resolves
      .toMatchObject({
        ok: true,
        surfaces: [{ surfaceId: "agent-draft" }],
      });
  });
});
