import { describe, expect, it } from "vitest";
import { LoroDoc, LoroMap, LoroMovableList } from "loro-crdt";

const schema = "capsem.ui.editable.v0";

function createFixture() {
  const doc = new LoroDoc();
  doc.setPeerId("7");

  const workspace = doc.getMap("workspace");
  workspace.set("schema", schema);
  workspace.set("engine", "loro");

  const components = workspace.setContainer("components", new LoroMovableList());
  const card = components.insertContainer(0, new LoroMap());
  card.set("id", "card:model");
  card.set("kind", "card");
  card.set("title", "Model");
  card.set("body", "Gemini model card");
  card.set("href", "https://gemini.google.com/");

  const alert = components.insertContainer(1, new LoroMap());
  alert.set("id", "alert:security-warning");
  alert.set("kind", "alert");
  alert.set("tone", "warning");
  alert.set("message", "All your base belong to us");

  const deck = workspace.setContainer("slideDeck", new LoroMap());
  deck.set("id", "deck:realms-of-code");
  deck.set("title", "Realms Of Code");
  const slides = deck.setContainer("slides", new LoroMovableList());
  const overview = slides.insertContainer(0, new LoroMap());
  overview.set("id", "slide:overview");
  overview.set("title", "Houses Overview");
  const lannister = slides.insertContainer(1, new LoroMap());
  lannister.set("id", "slide:lannister");
  lannister.set("title", "House Lannister");

  const spreadsheet = workspace.setContainer("spreadsheet", new LoroMap());
  spreadsheet.set("id", "spreadsheet:finance");
  spreadsheet.set("title", "Finance Sheet");
  const sheets = spreadsheet.setContainer("sheets", new LoroMovableList());
  const sheet = sheets.insertContainer(0, new LoroMap());
  sheet.set("id", "sheet:houses");
  sheet.set("name", "Houses");
  const cells = sheet.setContainer("cells", new LoroMap());
  cells.set("A1", "House");
  cells.set("B1", "Motto");
  cells.set("A2", "Lannister");
  cells.set("B2", "Hear Me Roar");

  const website = workspace.setContainer("website", new LoroMap());
  website.set("id", "website:onboarding");
  website.set("title", "Onboarding");
  const pages = website.setContainer("pages", new LoroMovableList());
  const page = pages.insertContainer(0, new LoroMap());
  page.set("id", "page:welcome");
  page.set("title", "Welcome");
  const form = page.setContainer("form", new LoroMap());
  form.set("id", "form:signup");
  const fields = form.setContainer("fields", new LoroMovableList());
  const email = fields.insertContainer(0, new LoroMap());
  email.set("id", "field:email");
  email.set("kind", "textField");
  email.set("label", "Email");

  doc.commit();
  const baseSnapshot = doc.export({ mode: "snapshot" });
  const baseVersion = doc.oplogVersion();

  card.set("title", "Model, live edited");
  cells.set("B2", "A Lannister always pays his debts");
  email.set("label", "Work email");
  components.move(1, 0);
  slides.move(1, 0);
  doc.commit();

  return {
    doc,
    baseSnapshot,
    update: doc.export({ mode: "update", from: baseVersion }),
  };
}

function collectIds(value: unknown, ids = new Set<string>()): string[] {
  if (Array.isArray(value)) {
    for (const item of value) collectIds(item, ids);
  } else if (value && typeof value === "object") {
    const record = value as Record<string, unknown>;
    if (typeof record.id === "string") ids.add(record.id);
    for (const child of Object.values(record)) collectIds(child, ids);
  }
  return [...ids].sort();
}

describe("Loro editable state audit", () => {
  it("round-trips card, slide deck, sheet, and website form edits in browser JS", () => {
    const { doc, baseSnapshot, update } = createFixture();
    const restored = LoroDoc.fromSnapshot(baseSnapshot);
    restored.import(update);

    expect(restored.toJSON()).toEqual(doc.toJSON());
    expect(doc.toJSON()).toEqual({
      workspace: {
        schema,
        engine: "loro",
        components: [
          {
            id: "alert:security-warning",
            kind: "alert",
            tone: "warning",
            message: "All your base belong to us",
          },
          {
            id: "card:model",
            kind: "card",
            title: "Model, live edited",
            body: "Gemini model card",
            href: "https://gemini.google.com/",
          },
        ],
        slideDeck: {
          id: "deck:realms-of-code",
          title: "Realms Of Code",
          slides: [
            { id: "slide:lannister", title: "House Lannister" },
            { id: "slide:overview", title: "Houses Overview" },
          ],
        },
        spreadsheet: {
          id: "spreadsheet:finance",
          title: "Finance Sheet",
          sheets: [
            {
              id: "sheet:houses",
              name: "Houses",
              cells: {
                A1: "House",
                B1: "Motto",
                A2: "Lannister",
                B2: "A Lannister always pays his debts",
              },
            },
          ],
        },
        website: {
          id: "website:onboarding",
          title: "Onboarding",
          pages: [
            {
              id: "page:welcome",
              title: "Welcome",
              form: {
                id: "form:signup",
                fields: [{ id: "field:email", kind: "textField", label: "Work email" }],
              },
            },
          ],
        },
      },
    });
    expect(update.length).toBeGreaterThan(0);
    expect(collectIds(doc.toJSON())).toEqual([
      "alert:security-warning",
      "card:model",
      "deck:realms-of-code",
      "field:email",
      "form:signup",
      "page:welcome",
      "sheet:houses",
      "slide:lannister",
      "slide:overview",
      "spreadsheet:finance",
      "website:onboarding",
    ]);
  });
});
