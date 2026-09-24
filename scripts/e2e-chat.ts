import { spawn, type ChildProcess } from "node:child_process";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import { setTimeout as delay } from "node:timers/promises";

import { chromium, type Browser, type Page } from "playwright";

const port = Number(process.env.CAPSEM_E2E_PORT ?? 8898);
const baseUrl = `http://127.0.0.1:${port}`;

type JsonRecord = Record<string, unknown>;

let server: ChildProcess | undefined;
let browser: Browser | undefined;
let tempDir = "";

try {
  tempDir = await mkdtemp(path.join(tmpdir(), "capsem-chat-e2e-"));
  const dbPath = path.join(tempDir, "workspace.sqlite");
  server = startServer(dbPath);
  await waitForServer();
  await post("/native/workspace/reset", {});
  await post("/native/data/sheet", {
    id: "e2e-house-sheet",
    title: "House Sheet",
    columns: ["house", "motto"],
    rows: [
      { house: "Stark", motto: "Winter Is Coming" },
      { house: "Lannister", motto: "Hear Me Roar" },
      { house: "Targaryen", motto: "Fire and Blood" },
    ],
    source: { kind: "fixture" },
  });
  await post("/native/ui/table", {
    id: "e2e-house-table",
    title: "House Ledger",
    sourceArtifact: "e2e",
    columns: ["house", "motto"],
    rows: [
      { house: "Stark", motto: "Winter Is Coming" },
      { house: "Lannister", motto: "Hear Me Roar" },
      { house: "Targaryen", motto: "Fire and Blood" },
    ],
    searchable: true,
    filterable: true,
    pageSize: 10,
  });
  await post("/native/ui/diagram", {
    id: "e2e-broken-diagram",
    title: "Broken Workflow",
    kind: "mermaid",
    source: "flowchart TD\n  A -->",
  });
  await post("/native/ui/chart", {
    id: "e2e-house-chart",
    title: "House Scores",
    chart: "barChart",
    sourceArtifact: "e2e-house-table",
    data: [
      { house: "Stark", score: 82 },
      { house: "Lannister", score: 91 },
      { house: "Targaryen", score: 88 },
    ],
    x: "house",
    series: [{ name: "score", field: "score" }],
    xLabel: "House",
    yLabel: "Score",
    yUnit: "points",
    direction: "vertical",
  });
  const chartRows = [
    { house: "Stark", score: 82, risk: 22 },
    { house: "Lannister", score: 91, risk: 38 },
    { house: "Targaryen", score: 88, risk: 31 },
  ];
  await post("/native/ui/chart", {
    id: "e2e-house-heatmap",
    title: "House Heatmap",
    chart: "heatmapChart",
    sourceArtifact: "e2e-house-table",
    data: chartRows,
    x: "house",
    series: [
      { name: "score", field: "score" },
      { name: "risk", field: "risk" },
    ],
    xLabel: "House",
    yLabel: "Metric",
    yUnit: "score",
    direction: "vertical",
  });
  await post("/native/ui/chart", {
    id: "e2e-house-box",
    title: "House Box Plot",
    chart: "boxPlot",
    sourceArtifact: "e2e-house-table",
    data: chartRows,
    x: "house",
    series: [{ name: "score", field: "score" }],
    xLabel: "House",
    yLabel: "Score",
    yUnit: "points",
    direction: "vertical",
  });
  await post("/native/ui/chart", {
    id: "e2e-house-scatter",
    title: "House Scatter",
    chart: "scatterPlot",
    sourceArtifact: "e2e-house-table",
    data: chartRows,
    x: "house",
    series: [{ name: "risk", field: "risk" }],
    xLabel: "House",
    yLabel: "Risk",
    yUnit: "points",
    direction: "vertical",
    fit: { method: "linear", display: true },
  });
  await post("/native/ui/chart", {
    id: "e2e-house-dual-axis",
    title: "House Dual Axis",
    chart: "lineChart",
    sourceArtifact: "e2e-house-table",
    data: chartRows,
    x: "house",
    series: [
      { name: "score", field: "score", axis: "left" },
      { name: "risk", field: "risk", axis: "right" },
    ],
    xLabel: "House",
    yLabel: "Score",
    yUnit: "points",
    direction: "vertical",
    legend: "bottom",
    secondAxis: { label: "Risk", unit: "points" },
  });
  await post("/native/ui/diagram", {
    id: "e2e-valid-diagram",
    title: "Valid Workflow",
    kind: "mermaid",
    source: "flowchart TD\n  A[Plan] --> B[Render]",
  });
  await post("/native/ui/timeline", {
    id: "e2e-runtime-timeline",
    title: "Runtime Timeline",
    lanes: [
      { id: "compiler", title: "Compiler" },
      { id: "renderer", title: "Renderer" },
    ],
    events: [
      {
        id: "schema",
        title: "Schema accepted",
        lane: "compiler",
        start: "2026-06-06",
        description: "Rust validated the native artifact contract.",
      },
      {
        id: "paint",
        title: "Timeline painted",
        lane: "renderer",
        start: "2026-06-06",
        description: "Svelte rendered capsem-timeline in chat.",
      },
    ],
  });
  await post("/native/generate/image", {
    id: "e2e-generated-image",
    title: "Generated Image Fixture",
    prompt: "A deterministic topology fixture for generated image rendering.",
    caption: "Generated image caption.",
    provider: "gemini",
    model: "gemini-3.5-flash",
  });
  await post("/native/ui/slide", {
    id: "e2e-runtime-slide",
    title: "Runtime Slide",
    blocks: [
      { kind: "text", title: "Compiler", body: "Rust owns validation and projection." },
      { kind: "chart", artifactId: "e2e-house-dual-axis" },
      { kind: "diagram", artifactId: "e2e-valid-diagram" },
      { kind: "image", artifactId: "e2e-generated-image" },
    ],
  });
  await post("/native/ui/slide-deck", {
    id: "e2e-runtime-deck",
    title: "Runtime Deck",
    slides: [{ artifactId: "e2e-runtime-slide", title: "Runtime Slide" }],
  });

  browser = await chromium.launch();
  const page = await browser.newPage({ viewport: { width: 1280, height: 900 } });
  await page.goto(`${baseUrl}/chat`, { waitUntil: "networkidle" });
  await expectVisibleText(page, "House Sheet");
  await expectVisibleText(page, "House Ledger");
  await expectVisibleText(page, "Winter Is Coming");
  await expectVisibleText(page, "House Scores");
  await expectVisibleText(page, "House Heatmap");
  await expectVisibleText(page, "House Box Plot");
  await expectVisibleText(page, "House Scatter");
  await expectVisibleText(page, "House Dual Axis");
  await expectVisibleText(page, "Valid Workflow");
  await expectVisibleText(page, "Runtime Timeline");
  await expectVisibleText(page, "Timeline painted");
  await expectVisibleText(page, "Generated Image Fixture");
  await expectVisibleText(page, "Generated image caption.");
  await expectVisibleText(page, "Runtime Slide");
  await expectVisibleText(page, "Runtime Deck");
  await expectArtifactShadowSelector(page, "e2e-house-chart", '[part~="plot"] .main-svg');
  await expectArtifactShadowSelector(page, "e2e-house-heatmap", '[part~="plot"] .main-svg');
  await expectArtifactShadowSelector(page, "e2e-house-box", '[part~="plot"] .main-svg');
  await expectArtifactShadowSelector(page, "e2e-house-scatter", '[part~="plot"] .main-svg');
  await expectArtifactShadowSelector(page, "e2e-house-dual-axis", '[part~="plot"] .main-svg');
  await expectArtifactShadowSelector(page, "e2e-runtime-timeline", '[part~="timelineEvent"]');
  const scatterPlot = await getPlotlyState(page, "e2e-house-scatter");
  assert(
    scatterPlot.traces.some((trace) => trace.name === "risk linear fit" && trace.mode === "lines"),
    "expected scatter chart to include a generated linear fit trace",
  );
  const dualAxisPlot = await getPlotlyState(page, "e2e-house-dual-axis");
  assert(
    dualAxisPlot.traces.some((trace) => trace.name === "risk" && trace.yaxis === "y2"),
    "expected dual-axis chart to send right-axis series to y2",
  );
  assert(dualAxisPlot.layout.yaxis2?.side === "right", "expected dual-axis chart to render a right axis");
  await expectArtifactShadowSelector(page, "e2e-valid-diagram", '[part~="diagramPreview"] svg');
  await expectVisibleText(page, "Render errors");
  await expectVisibleText(page, "e2e-broken-diagram");
  await expectVisibleText(page, "mermaid");
  await waitForTelemetryOperation("render.error", (event) => event.artifactId === "e2e-broken-diagram");
  await expectArtifactTopologyRoles(page, "e2e-house-sheet", [
    "title",
    "sheet",
    "column",
    "range",
    "row",
    "cell",
  ]);
  await expectArtifactTopologyRoles(page, "e2e-house-dual-axis", [
    "title",
    "legend",
    "xAxis",
    "yAxis",
    "secondaryYAxis",
    "series",
    "dataPoint",
  ]);
  await expectArtifactTopologyRoles(page, "e2e-valid-diagram", ["title", "source", "node", "edge"]);
  await expectArtifactTopologyRoles(page, "e2e-runtime-timeline", [
    "title",
    "lane",
    "event",
    "date",
    "annotation",
  ]);
  await expectArtifactTopologyRoles(page, "e2e-generated-image", [
    "title",
    "media",
    "caption",
    "controls",
    "source",
  ]);
  await expectArtifactTopologyRoles(page, "e2e-runtime-slide", [
    "slideTitle",
    "region",
    "textBlock",
    "image",
    "chart",
    "diagram",
  ]);
  await expectArtifactTopologyRoles(page, "e2e-runtime-deck", ["deckTitle", "slide"]);
  await page.getByLabel("Theme").selectOption("theme-ocean");
  await page.getByLabel("Dark mode").check();
  await expectThemeState(page, "theme-ocean", true);
  await expectShadowTokenColor(page, "e2e-house-table", '[part~="title"]');

  await page.locator("button[aria-pressed]").click();
  await page.locator('[data-capsem-role="card-title"]', { hasText: "House Ledger" }).click();
  await page.getByPlaceholder("What should change?").fill("Rename this card to House Index");
  await page.locator('aside[aria-label="Comment panel"]').getByRole("button", { name: "Comment" }).click();
  await expectVisibleText(page, "Rename this card to House Index");
  await expectVisibleText(page, "pending");

  let info = await getWorkspaceInfo();
  assert(info.changeRequests.length === 1, "expected one workspace change request");
  const titleTaskId = info.changeRequests[0]?.id;
  assert(typeof titleTaskId === "string" && titleTaskId.length > 0, "expected durable task id");

  await clickArtifactShadowText(page, "e2e-house-table", "tbody tr", "Lannister");
  await page.getByPlaceholder("What should change?").fill("Highlight the Lannister row");
  await page.locator('aside[aria-label="Comment panel"]').getByRole("button", { name: "Comment" }).click();
  await expectVisibleText(page, "Highlight the Lannister row");
  info = await getWorkspaceInfo();
  const rowTask = changeRequestByInstruction(info, "Highlight the Lannister row");
  assert(rowTask.annotation?.kind === "domElement", "expected row task to target a DOM element");
  assert(
    rowTask.annotation.shadowSelector === '[data-capsem-node="e2e-house-table::row::1"]',
    `expected stable row topology selector, got ${rowTask.annotation.shadowSelector}`,
  );
  assert(rowTask.annotation.selector?.includes(">>>"), "expected composed host/shadow selector");
  assert(rowTask.annotation.topologyId === "e2e-house-table::row::1", "expected row topology id");
  assert(rowTask.annotation.topologyRole === "row", "expected row topology role");
  assert(rowTask.annotation.metadata?.text?.includes("Lannister"), "expected annotation metadata to capture row text");

  await clickArtifactShadowText(
    page,
    "e2e-generated-image",
    '[data-capsem-topology-role="caption"]',
    "Generated image caption.",
  );
  await page.getByPlaceholder("What should change?").fill("Tighten the generated image caption");
  await page.locator('aside[aria-label="Comment panel"]').getByRole("button", { name: "Comment" }).click();
  await expectVisibleText(page, "Tighten the generated image caption");
  info = await getWorkspaceInfo();
  const imageCaptionTask = changeRequestByInstruction(info, "Tighten the generated image caption");
  assert(imageCaptionTask.annotation?.kind === "domElement", "expected image caption task to target a DOM element");
  assert(
    imageCaptionTask.annotation.shadowSelector === '[data-capsem-node="e2e-generated-image::caption"]',
    `expected stable image caption selector, got ${imageCaptionTask.annotation.shadowSelector}`,
  );
  assert(imageCaptionTask.annotation.topologyId === "e2e-generated-image::caption", "expected caption topology id");
  assert(imageCaptionTask.annotation.topologyRole === "caption", "expected caption topology role");

  await clickArtifactShadowText(
    page,
    "e2e-runtime-slide",
    '[data-capsem-topology-role="textBlock"]',
    "Rust owns validation",
  );
  await page.getByPlaceholder("What should change?").fill("Make the slide text more decisive");
  await page.locator('aside[aria-label="Comment panel"]').getByRole("button", { name: "Comment" }).click();
  await expectVisibleText(page, "Make the slide text more decisive");
  info = await getWorkspaceInfo();
  const slideTextTask = changeRequestByInstruction(info, "Make the slide text more decisive");
  assert(slideTextTask.annotation?.kind === "domElement", "expected slide text task to target a DOM element");
  assert(
    slideTextTask.annotation.shadowSelector === '[data-capsem-node="e2e-runtime-slide::block::0::text"]',
    `expected stable slide text selector, got ${slideTextTask.annotation.shadowSelector}`,
  );
  assert(slideTextTask.annotation.topologyId === "e2e-runtime-slide::block::0::text", "expected slide text topology id");
  assert(slideTextTask.annotation.topologyRole === "textBlock", "expected slide text topology role");

  await post("/native/workspace/mutate", {
    type: "text",
    target: "e2e-generated-image",
    selector: imageCaptionTask.annotation.selector,
    hostSelector: imageCaptionTask.annotation.hostSelector,
    shadowSelector: imageCaptionTask.annotation.shadowSelector,
    text: "Caption tightened by topology patch.",
    sourceRequestSeq: imageCaptionTask.seq,
  });
  await expectArtifactShadowText(
    page,
    "e2e-generated-image",
    imageCaptionTask.annotation.shadowSelector,
    "Caption tightened by topology patch.",
  );

  await post("/native/workspace/mutate", {
    type: "text",
    target: "e2e-runtime-slide",
    selector: slideTextTask.annotation.selector,
    hostSelector: slideTextTask.annotation.hostSelector,
    shadowSelector: slideTextTask.annotation.shadowSelector,
    text: "Rust owns validation. No bypass.",
    sourceRequestSeq: slideTextTask.seq,
  });
  await expectArtifactShadowText(
    page,
    "e2e-runtime-slide",
    slideTextTask.annotation.shadowSelector,
    "Rust owns validation. No bypass.",
  );

  await post("/native/workspace/mutate", {
    type: "style",
    target: "e2e-house-table",
    selector: rowTask.annotation.selector,
    hostSelector: rowTask.annotation.hostSelector,
    shadowSelector: rowTask.annotation.shadowSelector,
    styles: { fontWeight: "700" },
    sourceRequestSeq: rowTask.seq,
  });
  await expectShadowStyle(page, "e2e-house-table", rowTask.annotation.shadowSelector, "fontWeight", "700");

  await post("/native/workspace/mutate", {
    type: "title",
    target: "e2e-house-table",
    title: "House Index",
  });
  await expectVisibleText(page, "House Index");
  assertOperations(await get("/native/telemetry"), [
    "workspace.create",
    "workspace.request",
    "workspace.patch",
  ]);

  await stopServer(server);
  server = undefined;
  server = startServer(dbPath);
  await waitForServer();
  await post("/native/workspace/mutate", {
    type: "title",
    target: "e2e-house-table",
    title: "House Directory",
  });
  await expectVisibleText(page, "House Directory");

  await page.reload({ waitUntil: "networkidle" });
  await expectThemeState(page, "theme-ocean", true);
  await expectVisibleText(page, "House Directory");
  await expectVisibleText(page, "Rename this card to House Index");
  await expectVisibleText(page, "Highlight the Lannister row");
  await expectVisibleText(page, "Tighten the generated image caption");
  await expectVisibleText(page, "Make the slide text more decisive");
  await expectVisibleText(page, "Caption tightened by topology patch.");
  await expectVisibleText(page, "Rust owns validation. No bypass.");
  await expectVisibleText(page, "pending");
  await expectShadowTokenColor(page, "e2e-house-table", '[part~="title"]');
  await expectShadowStyle(page, "e2e-house-table", rowTask.annotation.shadowSelector, "fontWeight", "700");

  await page.getByRole("button", { name: "Resolve" }).first().click();
  await expectVisibleText(page, "resolved");
  info = await getWorkspaceInfo();
  assert(
    info.changeRequests.some((request) => request.status === "complete"),
    "expected at least one server task to be complete",
  );

  assertOperations(await get("/native/telemetry"), [
    "workspace.patch",
    "workspace.respond",
  ]);

  console.log("chat E2E passed");
} finally {
  await browser?.close().catch(() => undefined);
  if (server) await stopServer(server);
  if (tempDir) await rm(tempDir, { recursive: true, force: true });
}

function startServer(dbPath: string): ChildProcess {
  const child = spawn("cargo", ["run", "-p", "capsem-plugin-server"], {
    cwd: process.cwd(),
    env: {
      ...process.env,
      CAPSEM_PLUGIN_BIND: `127.0.0.1:${port}`,
      CAPSEM_NATIVE_WORKSPACE_DB: dbPath,
    },
    stdio: ["ignore", "pipe", "pipe"],
  });
  child.stdout.on("data", (chunk) => process.stdout.write(`[server] ${chunk}`));
  child.stderr.on("data", (chunk) => process.stderr.write(`[server] ${chunk}`));
  return child;
}

async function waitForServer(): Promise<void> {
  const deadline = Date.now() + 30_000;
  while (Date.now() < deadline) {
    try {
      const response = await fetch(`${baseUrl}/health`);
      if (response.ok) return;
    } catch {
      // Server is still starting.
    }
    await delay(250);
  }
  throw new Error("server did not become healthy");
}

async function stopServer(child: ChildProcess): Promise<void> {
  if (child.exitCode !== null) return;
  child.kill("SIGTERM");
  await Promise.race([
    new Promise<void>((resolve) => child.once("exit", () => resolve())),
    delay(5_000).then(() => {
      child.kill("SIGKILL");
    }),
  ]);
}

async function post(pathname: string, body: JsonRecord): Promise<JsonRecord> {
  const response = await fetch(`${baseUrl}${pathname}`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(body),
  });
  const text = await response.text();
  if (!response.ok) throw new Error(`${pathname} failed ${response.status}: ${text}`);
  return text ? (JSON.parse(text) as JsonRecord) : {};
}

async function get(pathname: string): Promise<JsonRecord[]> {
  const response = await fetch(`${baseUrl}${pathname}`);
  const text = await response.text();
  if (!response.ok) throw new Error(`${pathname} failed ${response.status}: ${text}`);
  return JSON.parse(text) as JsonRecord[];
}

type WorkspaceInfo = {
  changeRequests: Array<{
    seq: number;
    id: string;
    status: string;
    instruction: string;
    annotation?: {
      kind: string;
      topologyId?: string;
      topologyRole?: string;
      selector?: string;
      hostSelector?: string;
      shadowSelector?: string;
      metadata?: {
        text?: string;
      };
    };
  }>;
};

async function getWorkspaceInfo(): Promise<WorkspaceInfo> {
  const response = await fetch(`${baseUrl}/native/workspace/info`);
  const text = await response.text();
  if (!response.ok) throw new Error(`/native/workspace/info failed ${response.status}: ${text}`);
  return JSON.parse(text) as WorkspaceInfo;
}

async function expectVisibleText(page: Page, text: string): Promise<void> {
  await page.getByText(text, { exact: false }).first().waitFor({ state: "visible", timeout: 7_500 });
}

async function expectThemeState(page: Page, theme: string, dark: boolean): Promise<void> {
  await page.waitForFunction(
    ({ theme, dark }) => {
      const root = document.documentElement;
      return (
        root.dataset.theme === theme &&
        root.classList.contains("dark") === dark &&
        window.localStorage.getItem("capsem-chat-theme") === theme &&
        window.localStorage.getItem("capsem-chat-dark") === String(dark)
      );
    },
    { theme, dark },
    { timeout: 7_500 },
  );
}

async function expectShadowTokenColor(page: Page, artifactId: string, shadowSelector: string): Promise<void> {
  const colors = await page.evaluate(
    ({ artifactId, shadowSelector }) => {
      const host = document.querySelector<HTMLElement>(`[data-capsem-artifact-id="${artifactId}"]`);
      const target = host?.shadowRoot?.querySelector<HTMLElement>(shadowSelector);
      return {
        hostFound: Boolean(host),
        targetFound: Boolean(target),
        hostColor: host ? getComputedStyle(host).color : "",
        targetColor: target ? getComputedStyle(target).color : "",
        foregroundToken: getComputedStyle(document.documentElement).getPropertyValue("--foreground").trim(),
      };
    },
    { artifactId, shadowSelector },
  );
  assert(colors.hostFound, `missing artifact host ${artifactId}`);
  assert(colors.targetFound, `missing shadow token target ${shadowSelector}`);
  assert(colors.foregroundToken.length > 0, "expected foreground CSS token to resolve");
  assert(
    colors.targetColor === colors.hostColor,
    `expected shadow target to inherit semantic foreground token, got ${JSON.stringify(colors)}`,
  );
}

async function expectArtifactShadowSelector(page: Page, artifactId: string, shadowSelector: string): Promise<void> {
  await page.waitForFunction(
    ({ artifactId, shadowSelector }) => {
      const host = document.querySelector<HTMLElement>(`[data-capsem-artifact-id="${artifactId}"]`);
      return Boolean(host?.shadowRoot?.querySelector(shadowSelector));
    },
    { artifactId, shadowSelector },
    { timeout: 10_000 },
  );
}

async function expectArtifactTopologyRoles(page: Page, artifactId: string, roles: string[]): Promise<void> {
  try {
    await page.waitForFunction(
      ({ artifactId, roles }) => {
        const host = document.querySelector<HTMLElement>(`[data-capsem-artifact-id="${artifactId}"]`);
        if (!host?.shadowRoot) return false;
        return roles.every((role) =>
          Boolean(host.shadowRoot?.querySelector(`[data-capsem-topology-role="${role}"]`)),
        );
      },
      { artifactId, roles },
      { timeout: 10_000 },
    );
  } catch (cause) {
    const diagnostic = await page.evaluate(
      ({ artifactId, roles }) => {
        const host = document.querySelector<HTMLElement>(`[data-capsem-artifact-id="${artifactId}"]`);
        const present = host?.shadowRoot
          ? Array.from(host.shadowRoot.querySelectorAll("[data-capsem-topology-role]")).map((node) =>
              node.getAttribute("data-capsem-topology-role"),
            )
          : [];
        return {
          artifactId,
          hostFound: Boolean(host),
          missing: roles.filter((role) => !present.includes(role)),
          present,
        };
      },
      { artifactId, roles },
    );
    throw new Error(`missing artifact topology roles: ${JSON.stringify(diagnostic)}`, { cause });
  }
}

async function getPlotlyState(
  page: Page,
  artifactId: string,
): Promise<{ traces: Array<Record<string, unknown>>; layout: Record<string, JsonRecord> }> {
  return page.evaluate(
    ({ artifactId }) => {
      const host = document.querySelector<HTMLElement>(`[data-capsem-artifact-id="${artifactId}"]`);
      const plot = host?.shadowRoot?.querySelector<HTMLElement>('[part~="plot"]') as
        | (HTMLElement & { data?: Array<Record<string, unknown>>; layout?: Record<string, unknown> })
        | null;
      return {
        traces: plot?.data ?? [],
        layout: (plot?.layout ?? {}) as Record<string, JsonRecord>,
      };
    },
    { artifactId },
  );
}

async function clickArtifactShadowText(
  page: Page,
  artifactId: string,
  selector: string,
  text: string,
): Promise<void> {
  const clicked = await page.evaluate(
    ({ artifactId, selector, text }) => {
      const host = document.querySelector<HTMLElement>(`[data-capsem-artifact-id="${artifactId}"]`);
      const root = host?.shadowRoot;
      if (!root) return false;
      const target = Array.from(root.querySelectorAll<HTMLElement>(selector)).find((node) =>
        (node.textContent ?? "").includes(text),
      );
      target?.click();
      return Boolean(target);
    },
    { artifactId, selector, text },
  );
  assert(clicked, `missing shadow target ${selector} containing ${text}`);
}

async function expectArtifactShadowText(
  page: Page,
  artifactId: string,
  shadowSelector: string | undefined,
  expectedText: string,
): Promise<void> {
  assert(shadowSelector, "expected shadow selector");
  await page.waitForFunction(
    ({ artifactId, shadowSelector, expectedText }) => {
      const host = document.querySelector<HTMLElement>(`[data-capsem-artifact-id="${artifactId}"]`);
      const target = host?.shadowRoot?.querySelector<HTMLElement>(shadowSelector);
      return (target?.textContent ?? "").includes(expectedText);
    },
    { artifactId, shadowSelector, expectedText },
    { timeout: 7_500 },
  );
}

async function expectShadowStyle(
  page: Page,
  artifactId: string,
  shadowSelector: string | undefined,
  property: keyof CSSStyleDeclaration,
  expected: string,
): Promise<void> {
  assert(shadowSelector, "expected shadow selector");
  try {
    await page.waitForFunction(
      ({ artifactId, selector, propertyName, expectedValue }) => {
        const host = document.querySelector<HTMLElement>(`[data-capsem-artifact-id="${artifactId}"]`);
        const target = host?.shadowRoot?.querySelector<HTMLElement>(selector);
        if (!target) return false;
        return getComputedStyle(target)[propertyName as keyof CSSStyleDeclaration] === expectedValue;
      },
      { artifactId, selector: shadowSelector, propertyName: property, expectedValue: expected },
      { timeout: 7_500 },
    );
  } catch (cause) {
    const diagnostic = await page.evaluate(
      ({ artifactId, selector, propertyName }) => {
      const host = document.querySelector<HTMLElement>(`[data-capsem-artifact-id="${artifactId}"]`);
      const target = host?.shadowRoot?.querySelector<HTMLElement>(selector);
        return {
          hostFound: Boolean(host),
          selector,
          targetFound: Boolean(target),
          value: target ? getComputedStyle(target)[propertyName as keyof CSSStyleDeclaration] : null,
          inlineStyle: target?.getAttribute("style") ?? null,
          specStylePatches:
            (host as unknown as { spec?: { spec: { stylePatches?: unknown } } } | null)?.spec?.spec
              .stylePatches ?? null,
        };
      },
      { artifactId, selector: shadowSelector, propertyName: property },
    );
    throw new Error(`expected ${String(property)}=${expected}; diagnostic=${JSON.stringify(diagnostic)}`, {
      cause,
    });
  }
}

function assert(condition: unknown, message: string): asserts condition {
  if (!condition) throw new Error(message);
}

function assertOperations(events: JsonRecord[], expected: string[]): void {
  const operations = events.map((event) => String(event.operation));
  for (const operation of expected) {
    assert(operations.includes(operation), `missing ${operation} telemetry`);
  }
}

async function waitForTelemetryOperation(
  operation: string,
  predicate: (event: JsonRecord) => boolean = () => true,
): Promise<void> {
  const deadline = Date.now() + 7_500;
  while (Date.now() < deadline) {
    const match = (await get("/native/telemetry")).some(
      (event) => event.operation === operation && predicate(event),
    );
    if (match) return;
    await delay(100);
  }
  throw new Error(`missing telemetry operation ${operation}`);
}

function changeRequestByInstruction(info: WorkspaceInfo, instruction: string): WorkspaceInfo["changeRequests"][number] {
  const request = info.changeRequests.find((item) => item.instruction === instruction);
  assert(request, `missing change request: ${instruction}`);
  return request;
}
