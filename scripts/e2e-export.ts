import { spawn, spawnSync, type ChildProcess } from "node:child_process";
import { existsSync } from "node:fs";
import { mkdtemp, rm, writeFile } from "node:fs/promises";
import { homedir, tmpdir } from "node:os";
import path from "node:path";
import { setTimeout as delay } from "node:timers/promises";

const port = Number(process.env.CAPSEM_EXPORT_E2E_PORT ?? 8904);
const baseUrl = `http://127.0.0.1:${port}`;

type JsonRecord = Record<string, unknown>;

type ExportResponse = {
  ok: boolean;
  artifactId: string;
  format: string;
  adapter: string;
  status: string;
  filePath?: string;
  fileUrl?: string;
  mimeType?: string;
  bytes?: number;
  message?: string;
};

let server: ChildProcess | undefined;
let tempDir = "";

try {
  tempDir = await mkdtemp(path.join(tmpdir(), "capsem-export-e2e-"));
  const dbPath = path.join(tempDir, "workspace.sqlite");
  const exportDir = path.join(tempDir, "exports");
  const python = exportPython();
  server = startServer(dbPath, exportDir, python);
  await waitForServer();

  await post("/native/workspace/reset", {});
  await post("/native/data/sheet", {
    id: "export-house-sheet",
    title: "House Export Sheet",
    columns: ["house", "motto", "score"],
    rows: [
      { house: "Stark", motto: "Winter Is Coming", score: 82 },
      { house: "Lannister", motto: "Hear Me Roar", score: 91 },
      { house: "Targaryen", motto: "Fire and Blood", score: 88 },
    ],
    source: { kind: "mcp-export-e2e" },
  });
  await post("/native/ui/chart", {
    id: "export-house-chart",
    title: "House Scores",
    chart: "barChart",
    sourceArtifact: "export-house-sheet",
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
  await post("/native/ui/table", {
    id: "export-house-table",
    title: "House Table",
    sourceArtifact: "export-house-sheet",
    columns: ["house", "motto", "score"],
    rows: [
      { house: "Stark", motto: "Winter Is Coming", score: 82 },
      { house: "Lannister", motto: "Hear Me Roar", score: 91 },
      { house: "Targaryen", motto: "Fire and Blood", score: 88 },
    ],
    searchable: true,
    filterable: true,
    pageSize: 10,
  });
  await post("/native/ui/slide", {
    id: "export-title-slide",
    title: "Realm Export",
    blocks: [
      {
        kind: "text",
        title: "The Realms Of Code",
        body: "A deck assembled through the local tool surface.",
      },
      { kind: "chart", artifactId: "export-house-chart" },
    ],
  });
  await post("/native/ui/slide", {
    id: "export-table-slide",
    title: "House Ledger",
    blocks: [
      { kind: "text", title: "Overview", body: "The workbook data appears in the deck preview." },
      { kind: "table", artifactId: "export-house-table" },
    ],
  });
  await post("/native/ui/slide-deck", {
    id: "export-house-deck",
    title: "The Realms Of Code",
    slides: [
      { artifactId: "export-title-slide", title: "Realm Export" },
      { artifactId: "export-table-slide", title: "House Ledger" },
    ],
    export: ["html", "pdf"],
  });

  const exportResult = (await post("/native/export/spreadsheet", {
    artifactId: "export-house-sheet",
    format: "xlsx",
  })) as ExportResponse;
  assert(exportResult.ok, `expected XLSX export to complete: ${JSON.stringify(exportResult)}`);
  assert(exportResult.status === "complete", `unexpected export status ${exportResult.status}`);
  assert(exportResult.adapter === "python.export_artifact", "export should use the Python VM adapter");
  assert(exportResult.filePath && exportResult.filePath.endsWith(".xlsx"), "expected XLSX file path");
  assert(exportResult.fileUrl?.startsWith("/native/export/files/"), "expected downloadable export URL");
  assert((exportResult.bytes ?? 0) > 1000, "expected non-empty workbook");

  const downloaded = await getBytes(exportResult.fileUrl);
  assert(downloaded.byteLength === exportResult.bytes, "downloaded export should match recorded byte size");
  const downloadedPath = path.join(tempDir, "downloaded.xlsx");
  await writeFile(downloadedPath, Buffer.from(downloaded));
  const inspection = inspectWorkbook(python, downloadedPath);
  assert(inspection.rows === 4, `expected header plus 3 rows, got ${inspection.rows}`);
  assert(inspection.columns === 3, `expected 3 columns, got ${inspection.columns}`);
  assert(inspection.hasChartSheet, "expected chart sheet from chart artifact");
  assert(inspection.houses.includes("Lannister"), "expected workbook data to include Lannister row");

  const deckHtmlExport = (await post("/native/export/slide-deck", {
    artifactId: "export-house-deck",
    format: "html",
  })) as ExportResponse;
  assert(deckHtmlExport.ok, `expected HTML deck export to complete: ${JSON.stringify(deckHtmlExport)}`);
  assert(deckHtmlExport.status === "complete", `unexpected deck HTML status ${deckHtmlExport.status}`);
  assert(deckHtmlExport.filePath?.endsWith(".html"), "expected HTML deck file path");
  assert(deckHtmlExport.fileUrl?.startsWith("/native/export/files/"), "expected downloadable deck URL");
  const deckHtml = await getText(deckHtmlExport.fileUrl);
  assert(deckHtml.includes("The Realms Of Code"), "deck HTML should include the deck title");
  assert(deckHtml.includes("House Ledger"), "deck HTML should include the second slide");
  assert(deckHtml.includes("Lannister"), "deck HTML should include table data");

  const deckPptxExport = (await post("/native/export/slide-deck", {
    artifactId: "export-house-deck",
    format: "pptx",
  })) as ExportResponse;
  assert(!deckPptxExport.ok, "PPTX should remain unavailable until office tooling is configured");
  assert(deckPptxExport.status === "toolUnavailable", "PPTX should fail as a typed unavailable export");
  assert(
    deckPptxExport.message?.includes("office toolchain"),
    "PPTX unavailable response should name the missing office toolchain",
  );

  const telemetry = (await get("/native/telemetry")) as JsonRecord[];
  const exportTelemetry = telemetry.find(
    (event) => event.operation === "export.spreadsheet" && event.artifactId === "export-house-sheet",
  );
  const deckHtmlTelemetry = telemetry.find(
    (event) => event.operation === "export.slideDeck" && event.artifactId === "export-house-deck" && event.format === "html",
  );
  const deckPptxTelemetry = telemetry.find(
    (event) => event.operation === "export.slideDeck" && event.artifactId === "export-house-deck" && event.format === "pptx",
  );
  assert(Boolean(exportTelemetry), "expected export telemetry");
  assert(exportTelemetry?.status === "complete", "expected complete export telemetry");
  assert(exportTelemetry?.format === "xlsx", "expected xlsx export telemetry");
  assert(Number(exportTelemetry?.bytes ?? 0) === exportResult.bytes, "telemetry should record output bytes");
  assert(deckHtmlTelemetry?.status === "complete", "expected complete deck HTML export telemetry");
  assert(deckPptxTelemetry?.status === "toolUnavailable", "expected typed PPTX unavailable telemetry");

  console.log(
    JSON.stringify(
      {
        ok: true,
        export: {
          artifactId: exportResult.artifactId,
          format: exportResult.format,
          bytes: exportResult.bytes,
          fileUrl: exportResult.fileUrl,
          adapter: exportResult.adapter,
        },
        workbook: inspection,
        deck: {
          htmlBytes: deckHtmlExport.bytes,
          htmlUrl: deckHtmlExport.fileUrl,
          pptxStatus: deckPptxExport.status,
        },
        telemetry: {
          count: telemetry.length,
          exportStatus: exportTelemetry?.status,
          deckHtmlStatus: deckHtmlTelemetry?.status,
          deckPptxStatus: deckPptxTelemetry?.status,
        },
      },
      null,
      2,
    ),
  );
} finally {
  if (server) await stopServer(server);
  if (tempDir) await rm(tempDir, { recursive: true, force: true });
}

function exportPython(): string {
  if (process.env.CAPSEM_EXPORT_PYTHON) return process.env.CAPSEM_EXPORT_PYTHON;
  const bundled = path.join(
    homedir(),
    ".cache/codex-runtimes/codex-primary-runtime/dependencies/python/bin/python3",
  );
  return existsSync(bundled) ? bundled : "python3";
}

function startServer(dbPath: string, exportDir: string, python: string): ChildProcess {
  const child = spawn("cargo", ["run", "-p", "capsem-plugin-server"], {
    cwd: process.cwd(),
    env: {
      ...process.env,
      CAPSEM_PLUGIN_BIND: `127.0.0.1:${port}`,
      CAPSEM_NATIVE_WORKSPACE_DB: dbPath,
      CAPSEM_NATIVE_EXPORT_DIR: exportDir,
      CAPSEM_EXPORT_PYTHON: python,
      CAPSEM_EXPORT_SCRIPT: path.join(process.cwd(), "scripts/export_artifact.py"),
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
    await delay(100);
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
  return JSON.parse(text) as JsonRecord;
}

async function get(pathname: string): Promise<unknown> {
  const response = await fetch(`${baseUrl}${pathname}`);
  const text = await response.text();
  if (!response.ok) throw new Error(`${pathname} failed ${response.status}: ${text}`);
  return JSON.parse(text);
}

async function getBytes(pathname: string | undefined): Promise<ArrayBuffer> {
  assert(Boolean(pathname), "download path must be present");
  const response = await fetch(`${baseUrl}${pathname}`);
  if (!response.ok) {
    throw new Error(`${pathname} failed ${response.status}: ${await response.text()}`);
  }
  return response.arrayBuffer();
}

async function getText(pathname: string | undefined): Promise<string> {
  assert(Boolean(pathname), "download path must be present");
  const response = await fetch(`${baseUrl}${pathname}`);
  const text = await response.text();
  if (!response.ok) {
    throw new Error(`${pathname} failed ${response.status}: ${text}`);
  }
  return text;
}

function inspectWorkbook(python: string, workbookPath: string): {
  rows: number;
  columns: number;
  houses: string[];
  hasChartSheet: boolean;
} {
  const code = `
import json
import sys
from openpyxl import load_workbook
wb = load_workbook(sys.argv[1])
ws = wb["House Export Sheet"]
houses = [ws.cell(row=i, column=1).value for i in range(2, ws.max_row + 1)]
print(json.dumps({
    "rows": ws.max_row,
    "columns": ws.max_column,
    "houses": houses,
    "hasChartSheet": "Charts" in wb.sheetnames,
}))
`;
  const result = spawnSync(python, ["-c", code, workbookPath], {
    encoding: "utf8",
  });
  if (result.status !== 0) {
    throw new Error(`workbook inspection failed: ${result.stderr || result.stdout}`);
  }
  return JSON.parse(result.stdout) as {
    rows: number;
    columns: number;
    houses: string[];
    hasChartSheet: boolean;
  };
}

function assert(condition: unknown, message: string): asserts condition {
  if (!condition) throw new Error(message);
}
