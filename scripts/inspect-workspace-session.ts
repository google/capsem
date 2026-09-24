import { spawn, spawnSync, type ChildProcess } from "node:child_process";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import { setTimeout as delay } from "node:timers/promises";

const port = Number(process.env.CAPSEM_WORKSPACE_INSPECT_PORT ?? 8902);
const baseUrl = `http://127.0.0.1:${port}`;

type JsonRecord = Record<string, unknown>;

type TelemetryEvent = JsonRecord & {
  operation?: string;
  seq?: number;
  principal?: string;
  contentType?: string;
  verb?: string;
  status?: string;
  target?: string;
  taskId?: string;
  mutation?: JsonRecord;
  artifactId?: string;
};

type StoredRecord = {
  seq: number;
  id: string;
  principal: string;
  verb: string;
  contentType: string;
  target?: string;
  content: JsonRecord;
};

type SessionInspection = {
  dbPath: string;
  telemetry: {
    count: number;
    operations: string[];
    workspaceOperations: string[];
    renderErrors: string[];
  };
  store: {
    recordCount: number;
    checkpointCount: number;
    maxSeq: number;
    latestCheckpointSeq: number;
    principals: string[];
    verbs: string[];
    contentTypes: string[];
    hasTaskRecord: boolean;
    hasTitlePatch: boolean;
    hasStylePatch: boolean;
    hasResolveRecord: boolean;
  };
};

let server: ChildProcess | undefined;
let tempDir = "";

try {
  tempDir = await mkdtemp(path.join(tmpdir(), "capsem-workspace-inspect-"));
  const dbPath = path.join(tempDir, "workspace.sqlite");
  server = startServer(dbPath);
  await waitForServer();

  await post("/native/workspace/reset", {});
  await post("/native/ui/table", {
    id: "inspect-house-table",
    title: "Inspection Ledger",
    sourceArtifact: "inspection",
    columns: ["house", "motto", "score"],
    rows: [
      { house: "Stark", motto: "Winter Is Coming", score: 82 },
      { house: "Lannister", motto: "Hear Me Roar", score: 91 },
    ],
    searchable: true,
    filterable: true,
    pageSize: 10,
  });
  const taskFrame = await post("/native/workspace/change-request", {
    target: "inspect-house-table",
    instruction: "Make the Lannister row easier to scan",
    annotation: {
      kind: "domElement",
      label: "Lannister row",
      path: ["artifact", "shadow", "tbody", "tr:nth-child(2)"],
      selector:
        '[data-capsem-artifact-id="inspect-house-table"] >>> [data-capsem-node="inspect-house-table::row::1"]',
      hostSelector: '[data-capsem-artifact-id="inspect-house-table"]',
      shadowSelector: '[data-capsem-node="inspect-house-table::row::1"]',
      selectorVerified: true,
      metadata: {
        text: "Lannister Hear Me Roar 91",
        tagName: "tr",
      },
    },
  });
  const taskId = taskIdFromFrame(taskFrame);
  const sourceRequestSeq = Number((taskFrame.record as JsonRecord | undefined)?.seq ?? 0);
  assert(taskId.length > 0, "expected task id from change request frame");
  assert(sourceRequestSeq > 0, "expected source request seq");

  await post("/native/workspace/mutate", {
    type: "style",
    target: "inspect-house-table",
    selector:
      '[data-capsem-artifact-id="inspect-house-table"] >>> [data-capsem-node="inspect-house-table::row::1"]',
    hostSelector: '[data-capsem-artifact-id="inspect-house-table"]',
    shadowSelector: '[data-capsem-node="inspect-house-table::row::1"]',
    styles: { fontWeight: "700", backgroundColor: "var(--color-warning-soft)" },
    sourceRequestSeq,
  });
  await post("/native/workspace/mutate", {
    type: "title",
    target: "inspect-house-table",
    title: "Inspected House Ledger",
  });
  await post("/native/ui/render-error", {
    artifactId: "inspect-house-table",
    component: "capsem-sheet",
    renderer: "preline-table",
    phase: "inspection",
    message: "synthetic inspection render warning",
  });
  await post("/native/workspace/resolve", { taskId });
  await post("/native/workspace/checkpoint", {});

  const telemetry = (await get("/native/telemetry")) as TelemetryEvent[];
  assertTelemetry(telemetry, taskId);

  await stopServer(server);
  server = undefined;

  const stored = inspectSqlite(dbPath);
  assertStore(stored, taskId);

  const report: SessionInspection = {
    dbPath,
    telemetry: {
      count: telemetry.length,
      operations: unique(telemetry.map((event) => String(event.operation ?? ""))).filter(Boolean),
      workspaceOperations: unique(
        telemetry
          .filter((event) => String(event.operation ?? "").startsWith("workspace."))
          .map((event) => String(event.operation)),
      ),
      renderErrors: telemetry
        .filter((event) => event.operation === "render.error")
        .map((event) => `${event.artifactId ?? "unknown"}:${event.renderer ?? "unknown"}`),
    },
    store: {
      recordCount: stored.records.length,
      checkpointCount: stored.checkpoints.length,
      maxSeq: Math.max(...stored.records.map((record) => record.seq)),
      latestCheckpointSeq: Math.max(...stored.checkpoints.map((checkpoint) => checkpoint.checkpointSeq)),
      principals: unique(stored.records.map((record) => record.principal)),
      verbs: unique(stored.records.map((record) => record.verb)),
      contentTypes: unique(stored.records.map((record) => record.contentType)),
      hasTaskRecord: stored.records.some((record) => record.verb === "request" && record.target === "inspect-house-table"),
      hasTitlePatch: stored.records.some((record) => hasElementPatch(record, "title")),
      hasStylePatch: stored.records.some((record) => hasElementPatch(record, "stylePatches")),
      hasResolveRecord: stored.records.some((record) => record.verb === "respond" && String(record.target ?? "") === taskId),
    },
  };

  printReport(report);
} finally {
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
  return text ? (JSON.parse(text) as JsonRecord) : {};
}

async function get(pathname: string): Promise<unknown> {
  const response = await fetch(`${baseUrl}${pathname}`);
  const text = await response.text();
  if (!response.ok) throw new Error(`${pathname} failed ${response.status}: ${text}`);
  return text ? JSON.parse(text) : undefined;
}

function taskIdFromFrame(frame: JsonRecord): string {
  const deltas = frame.deltas;
  if (!Array.isArray(deltas)) return "";
  for (const delta of deltas) {
    const candidate = delta as JsonRecord;
    if (candidate.type === "upsertTask" && typeof candidate.id === "string") return candidate.id;
  }
  return "";
}

function assertTelemetry(events: TelemetryEvent[], taskId: string): void {
  assert(events.length >= 7, `expected at least 7 telemetry events, got ${events.length}`);
  requireEvent(events, "workspace.create", (event) =>
    event.principal === "local.ui.table" &&
    event.contentType === "artifact" &&
    event.verb === "create" &&
    event.status === "complete" &&
    event.target === "inspect-house-table" &&
    event.mutation?.artifactId === "inspect-house-table",
  );
  requireEvent(events, "workspace.request", (event) =>
    event.principal === "chat.ui" &&
    event.target === "inspect-house-table" &&
    event.taskId === taskId &&
    event.contentType === "action",
  );
  requireEvent(events, "workspace.patch", (event) =>
    event.principal === "local.ui.mutate" &&
    event.target === "inspect-house-table" &&
    event.mutation?.stylePatchCount === 1,
  );
  requireEvent(events, "workspace.patch", (event) =>
    event.principal === "local.ui.mutate" &&
    event.target === "inspect-house-table" &&
    event.mutation?.title === true,
  );
  requireEvent(events, "workspace.respond", (event) =>
    event.principal === "assistant.ui" &&
    event.target === taskId &&
    event.taskId === taskId,
  );
  requireEvent(events, "render.error", (event) =>
    event.artifactId === "inspect-house-table" &&
    event.renderer === "preline-table" &&
    event.status === "failed",
  );
}

function requireEvent(
  events: TelemetryEvent[],
  operation: string,
  predicate: (event: TelemetryEvent) => boolean,
): void {
  const found = events.some((event) => event.operation === operation && predicate(event));
  assert(found, `missing telemetry contract for ${operation}`);
}

function inspectSqlite(dbPath: string): { records: StoredRecord[]; checkpoints: Array<{ checkpointSeq: number }> } {
  const code = String.raw`
import json
import sqlite3
import sys

db_path = sys.argv[1]
conn = sqlite3.connect(db_path)
conn.row_factory = sqlite3.Row
records = []
for row in conn.execute("SELECT seq, record_json FROM workspace_records ORDER BY seq ASC"):
    record = json.loads(row["record_json"])
    records.append({
        "seq": row["seq"],
        "id": record["id"],
        "principal": record["principal"],
        "verb": record["verb"],
        "contentType": record["contentType"],
        "target": record.get("target"),
        "content": record["content"],
    })
checkpoints = []
for row in conn.execute("SELECT checkpoint_seq FROM workspace_checkpoints ORDER BY checkpoint_seq ASC"):
    checkpoints.append({"checkpointSeq": row["checkpoint_seq"]})
print(json.dumps({"records": records, "checkpoints": checkpoints}, separators=(",", ":")))
`;
  const result = spawnSync("python3", ["-c", code, dbPath], {
    cwd: process.cwd(),
    encoding: "utf8",
  });
  if (result.status !== 0) {
    throw new Error(`sqlite inspection failed: ${result.stderr || result.stdout}`);
  }
  return JSON.parse(result.stdout) as { records: StoredRecord[]; checkpoints: Array<{ checkpointSeq: number }> };
}

function assertStore(stored: { records: StoredRecord[]; checkpoints: Array<{ checkpointSeq: number }> }, taskId: string): void {
  assert(stored.records.length === 5, `expected 5 durable workspace records, got ${stored.records.length}`);
  assert(stored.checkpoints.length === 1, `expected one durable checkpoint, got ${stored.checkpoints.length}`);
  assert(stored.records.every((record, index) => record.seq === index + 1), "records must be append-only seq 1..n");
  assert(stored.checkpoints[0]?.checkpointSeq === 5, "checkpoint should compact through the resolve record");
  assert(stored.records.some((record) => record.principal === "chat.ui" && record.verb === "request"), "missing durable chat request");
  assert(stored.records.some((record) => record.principal === "local.ui.mutate" && hasElementPatch(record, "stylePatches")), "missing durable style patch");
  assert(stored.records.some((record) => record.principal === "local.ui.mutate" && hasElementPatch(record, "title")), "missing durable title patch");
  assert(stored.records.some((record) => record.principal === "assistant.ui" && record.target === taskId), "missing durable resolve record");
}

function hasElementPatch(record: StoredRecord, key: "stylePatches" | "title"): boolean {
  if (record.contentType !== "elementPatch") return false;
  if (key === "title") return record.content.title === "Inspected House Ledger";
  const patches = record.content.stylePatches;
  return Array.isArray(patches) && patches.length === 1;
}

function unique(values: string[]): string[] {
  return [...new Set(values)].sort();
}

function printReport(report: SessionInspection): void {
  console.log("# Workspace Session Inspection");
  console.log("");
  console.log("| Lane | Proof |");
  console.log("| --- | --- |");
  console.log(`| live telemetry | ${report.telemetry.count} events: ${report.telemetry.operations.join(", ")} |`);
  console.log(`| workspace audit | ${report.telemetry.workspaceOperations.join(", ")} |`);
  console.log(`| render audit | ${report.telemetry.renderErrors.join(", ")} |`);
  console.log(`| durable records | ${report.store.recordCount} records, max seq ${report.store.maxSeq}, verbs ${report.store.verbs.join(", ")} |`);
  console.log(`| durable checkpoint | ${report.store.checkpointCount} checkpoint at seq ${report.store.latestCheckpointSeq} |`);
  console.log(`| durable principals | ${report.store.principals.join(", ")} |`);
  console.log(`| durable content | ${report.store.contentTypes.join(", ")} |`);
  console.log(`| mutation/task coverage | task=${report.store.hasTaskRecord}, title=${report.store.hasTitlePatch}, style=${report.store.hasStylePatch}, resolve=${report.store.hasResolveRecord} |`);
}

function assert(condition: unknown, message: string): asserts condition {
  if (!condition) throw new Error(message);
}
