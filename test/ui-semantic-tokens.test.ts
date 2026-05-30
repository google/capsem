import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

const rendererFiles = [
  "ui-preview/src/A2Node.svelte",
  "ui-preview/src/ChatPage.svelte",
  "ui-preview/src/a2ui.ts",
];

const rawPaletteUtility =
  /\b(?:bg|text|border|ring|divide|placeholder|from|via|to)-(?:slate|gray|zinc|neutral|stone|red|orange|amber|yellow|green|emerald|teal|cyan|sky|blue|indigo|violet|purple|fuchsia|pink|rose|white|black)(?:-\d{2,3})?(?:\/\d+)?\b/;

describe("chat renderer semantic tokens", () => {
  it("keeps generated UI renderer classes on Preline semantic tokens", () => {
    for (const file of rendererFiles) {
      const source = readFileSync(file, "utf8");
      expect(source, `${file} contains raw Tailwind palette utilities`).not.toMatch(
        rawPaletteUtility,
      );
    }
  });
});
