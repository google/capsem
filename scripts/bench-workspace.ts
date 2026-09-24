import { spawn, type ChildProcess } from "node:child_process";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import { performance } from "node:perf_hooks";
import { setTimeout as delay } from "node:timers/promises";

import { chromium, type Browser, type Page } from "playwright";

const port = Number(process.env.CAPSEM_WORKSPACE_BENCH_PORT ?? 8901);
const baseUrl = `http://127.0.0.1:${port}`;
const artifactCount = Number(process.env.CAPSEM_WORKSPACE_BENCH_ARTIFACTS ?? 24);
const samples = Number(process.env.CAPSEM_WORKSPACE_BENCH_SAMPLES ?? 30);
const restoreSamples = Number(process.env.CAPSEM_WORKSPACE_BENCH_RESTORE_SAMPLES ?? 3);
const renderSamples = Number(process.env.CAPSEM_WORKSPACE_BENCH_RENDER_SAMPLES ?? 5);
const streamClients = Number(process.env.CAPSEM_WORKSPACE_BENCH_STREAM_CLIENTS ?? 5);

type JsonRecord = Record<string, unknown>;

type Metric = {
  name: string;
  samples: number[];
  opsPerSample?: number;
};

type BenchRow = {
  name: string;
  samples: number;
  ops: number;
  meanMs: number;
  p50Ms: number;
  p95Ms: number;
  qps: number;
};

type StreamClient = {
  close: () => void;
  waitNextRecord: () => Promise<void>;
};

let server: ChildProcess | undefined;
let browser: Browser | undefined;
let tempDir = "";

try {
  tempDir = await mkdtemp(path.join(tmpdir(), "capsem-workspace-bench-"));
  const dbPath = path.join(tempDir, "workspace.sqlite");
  server = startServer(dbPath);
  await waitForServer();
  await seedWorkspace();

  const rows: BenchRow[] = [];
  rows.push(toRow(await sample("projection GET", samples, () => time(() => get("/native/workspace/projection")))));
  rows.push(toRow(await sample("snapshot GET", samples, () => time(() => get("/native/workspace/snapshot")))));
  rows.push(toRow(await sample("title mutation POST", samples, (index) =>
    time(() =>
      post("/native/workspace/mutate", {
        type: "title",
        target: "bench-table-0",
        title: `Bench Table ${index}`,
      }),
    ),
  )));
  rows.push(toRow(await sample("checkpoint POST", Math.max(5, Math.floor(samples / 3)), () =>
    time(() => post("/native/workspace/checkpoint", {})),
  )));
  rows.push(toRow(await sample(`stream fanout ${streamClients} clients`, Math.max(5, Math.floor(samples / 3)), (index) =>
    measureStreamFanout(index),
  )));
  rows.push(toRow(await sample("restore + server start", restoreSamples, () => restartServer(dbPath))));

  browser = await chromium.launch();
  rows.push(toRow(await sample("browser render table/chart/diagram", renderSamples, () =>
    measureBrowserRender(browser!),
  )));

  printReport(rows);
} finally {
  await browser?.close().catch(() => undefined);
  if (server) await stopServer(server);
  if (tempDir) await rm(tempDir, { recursive: true, force: true });
}

async function seedWorkspace(): Promise<void> {
  await post("/native/workspace/reset", {});
  for (let index = 0; index < artifactCount; index += 1) {
    await post("/native/ui/table", {
      id: `bench-table-${index}`,
      title: `Bench Table ${index}`,
      sourceArtifact: "bench",
      columns: ["house", "motto", "score"],
      rows: [
        { house: "Stark", motto: "Winter Is Coming", score: 82 + index },
        { house: "Lannister", motto: "Hear Me Roar", score: 91 + index },
        { house: "Targaryen", motto: "Fire and Blood", score: 88 + index },
        { house: "Tyrell", motto: "Growing Strong", score: 76 + index },
      ],
      searchable: true,
      filterable: true,
      pageSize: 10,
    });
  }
  await post("/native/ui/chart", {
    id: "bench-chart",
    title: "Bench Scores",
    chart: "barChart",
    sourceArtifact: "bench-table-0",
    data: [
      { house: "Stark", score: 82 },
      { house: "Lannister", score: 91 },
      { house: "Targaryen", score: 88 },
      { house: "Tyrell", score: 76 },
    ],
    x: "house",
    series: [{ name: "score", field: "score" }],
    xLabel: "House",
    yLabel: "Score",
    yUnit: "points",
    direction: "vertical",
  });
  await post("/native/ui/diagram", {
    id: "bench-diagram",
    title: "Bench Flow",
    kind: "mermaid",
    source: "flowchart TD\n  A[Records] --> B[Projection]\n  B --> C[Renderer]",
  });
}

async function measureStreamFanout(index: number): Promise<number> {
  const clients = await Promise.all(
    Array.from({ length: streamClients }, () => openStreamClient()),
  );
  try {
    const waits = clients.map((client) => client.waitNextRecord());
    const started = performance.now();
    await post("/native/workspace/mutate", {
      type: "title",
      target: "bench-table-0",
      title: `Stream Fanout ${index}`,
    });
    await Promise.all(waits);
    return performance.now() - started;
  } finally {
    clients.forEach((client) => client.close());
  }
}

async function measureBrowserRender(activeBrowser: Browser): Promise<number> {
  const page = await activeBrowser.newPage({ viewport: { width: 1280, height: 900 } });
  try {
    const started = performance.now();
    await page.goto(`${baseUrl}/chat`, { waitUntil: "networkidle" });
    await expectVisibleText(page, "Bench Scores");
    await expectVisibleText(page, "Bench Flow");
    await expectArtifactShadowSelector(page, "bench-chart", '[part~="plot"] .main-svg');
    await expectArtifactShadowSelector(page, "bench-diagram", '[part~="diagramPreview"] svg');
    return performance.now() - started;
  } finally {
    await page.close();
  }
}

async function restartServer(dbPath: string): Promise<number> {
  const started = performance.now();
  if (server) {
    await stopServer(server);
    server = undefined;
  }
  server = startServer(dbPath);
  await waitForServer();
  await get("/native/workspace/projection");
  return performance.now() - started;
}

async function sample(
  name: string,
  count: number,
  run: (index: number) => Promise<number>,
): Promise<Metric> {
  await run(-1);
  const values: number[] = [];
  for (let index = 0; index < count; index += 1) {
    values.push(await run(index));
  }
  return { name, samples: values };
}

function toRow(metric: Metric): BenchRow {
  const sorted = [...metric.samples].sort((left, right) => left - right);
  const opsPerSample = metric.opsPerSample ?? 1;
  const total = metric.samples.reduce((sum, value) => sum + value, 0);
  const meanMs = total / metric.samples.length;
  return {
    name: metric.name,
    samples: metric.samples.length,
    ops: metric.samples.length * opsPerSample,
    meanMs,
    p50Ms: percentile(sorted, 0.5),
    p95Ms: percentile(sorted, 0.95),
    qps: (opsPerSample * 1000) / meanMs,
  };
}

function percentile(sorted: number[], quantile: number): number {
  if (sorted.length === 0) return 0;
  const index = Math.min(sorted.length - 1, Math.ceil(sorted.length * quantile) - 1);
  return sorted[index] ?? 0;
}

function printReport(rows: BenchRow[]): void {
  console.log("# Workspace Runtime Benchmark");
  console.log("");
  console.log(`Artifacts: ${artifactCount} tables + 1 chart + 1 diagram`);
  console.log(`Samples: ${samples}; restore samples: ${restoreSamples}; render samples: ${renderSamples}; stream clients: ${streamClients}`);
  console.log("");
  console.log("| Operation | Samples | Ops | Mean ms | p50 ms | p95 ms | QPS |");
  console.log("| --- | ---: | ---: | ---: | ---: | ---: | ---: |");
  for (const row of rows) {
    console.log(
      `| ${row.name} | ${row.samples} | ${row.ops} | ${format(row.meanMs)} | ${format(row.p50Ms)} | ${format(row.p95Ms)} | ${Math.round(row.qps).toLocaleString("en-US")} |`,
    );
  }
}

function format(value: number): string {
  return value.toFixed(value >= 100 ? 1 : value >= 10 ? 2 : 3);
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

async function time(action: () => Promise<unknown>): Promise<number> {
  const started = performance.now();
  await action();
  return performance.now() - started;
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

async function get(pathname: string): Promise<JsonRecord> {
  const response = await fetch(`${baseUrl}${pathname}`);
  const text = await response.text();
  if (!response.ok) throw new Error(`${pathname} failed ${response.status}: ${text}`);
  return text ? (JSON.parse(text) as JsonRecord) : {};
}

async function openStreamClient(): Promise<StreamClient> {
  const socket = new WebSocket(`ws://127.0.0.1:${port}/native/workspace/stream?lastSeq=0`);
  const pending: Array<() => void> = [];
  let recordCount = 0;
  socket.addEventListener("message", (event) => {
    const message = JSON.parse(String(event.data)) as { type?: string };
    if (message.type !== "record") return;
    recordCount += 1;
    pending.splice(0).forEach((resolve) => resolve());
  });
  await new Promise<void>((resolve, reject) => {
    const timer = setTimeout(() => reject(new Error("workspace stream open timed out")), 5_000);
    socket.addEventListener("open", () => {
      clearTimeout(timer);
      resolve();
    }, { once: true });
    socket.addEventListener("error", () => {
      clearTimeout(timer);
      reject(new Error("workspace stream failed"));
    }, { once: true });
  });
  await delay(50);
  recordCount = 0;
  return {
    close: () => socket.close(),
    waitNextRecord: () =>
      new Promise<void>((resolve) => {
        if (recordCount > 0) {
          recordCount -= 1;
          resolve();
          return;
        }
        pending.push(resolve);
      }),
  };
}

async function expectVisibleText(page: Page, text: string): Promise<void> {
  await page.getByText(text, { exact: false }).first().waitFor({ state: "visible", timeout: 10_000 });
}

async function expectArtifactShadowSelector(page: Page, artifactId: string, shadowSelector: string): Promise<void> {
  await page.waitForFunction(
    ({ artifactId, shadowSelector }) => {
      const host = document.querySelector<HTMLElement>(`[data-capsem-artifact-id="${artifactId}"]`);
      return Boolean(host?.shadowRoot?.querySelector(shadowSelector));
    },
    { artifactId, shadowSelector },
    { timeout: 15_000 },
  );
}
