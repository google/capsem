import http from "node:http";

import { compileRun, installPlugin, runPlugin } from "./component-runner.js";
import { LIMITS } from "./limits.js";
import type {
  CompileRunRequest,
  CompileRunResponse,
  InstallPluginRequest,
  InstallPluginResponse,
  RunPluginRequest,
  RunPluginResponse,
} from "./types.js";

const DEFAULT_SOURCE = `export function invoke(objectJson, contextJson) {
  const object = JSON.parse(objectJson);
  const context = JSON.parse(contextJson);
  object.labels = context.trace?.labels ?? [];
  object.decision = object.decision ?? { verdict: "review", reasons: [] };
  object.called = "invoke";
  return JSON.stringify(object);
}

export function inspect(objectJson, contextJson) {
  const object = JSON.parse(objectJson);
  const context = JSON.parse(contextJson);
  return JSON.stringify({
    kind: "Inspection",
    object_kind: object.kind,
    object_keys: Object.keys(object),
    context_keys: Object.keys(context),
    labels: context.trace?.labels ?? [],
    called: "inspect"
  });
}`;

const DEFAULT_OBJECT = {
  kind: "ModelOutput",
  content: { text: "hi" },
};

const DEFAULT_CONTEXT = {
  trace: { labels: ["manual"] },
};

const DEFAULT_MANIFEST = {
  id: "demo.security",
  name: "Demo Security Plugin",
  version: "0.1.0",
  callbacks: ["invoke", "inspect"],
  capabilities: [],
  contributes: {
    rules: ["rules/pii.json"],
    tools: ["tools/redact.json"],
    skills: ["skills/pii-review/SKILL.md"],
    ui: ["ui/panel.json"],
  },
};

function escapeHtml(value: string): string {
  return value
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
    .replaceAll('"', "&quot;");
}

function pageHtml(): string {
  return `<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>Capsem JS to WASM Component Spike</title>
  <style>
    :root {
      color-scheme: dark;
      --bg: #111318;
      --panel: #181b22;
      --panel-2: #20242d;
      --border: #353b47;
      --text: #f2f5f8;
      --muted: #9aa4b2;
      --accent: #5fb3a8;
      --bad: #ff7878;
      --good: #72d391;
      --warn: #f4c36a;
      font-family: Inter, ui-sans-serif, system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif;
    }

    * { box-sizing: border-box; }

    body {
      margin: 0;
      min-height: 100vh;
      background: var(--bg);
      color: var(--text);
    }

    header {
      height: 56px;
      display: flex;
      align-items: center;
      justify-content: space-between;
      padding: 0 18px;
      border-bottom: 1px solid var(--border);
      background: #0f1116;
    }

    h1 {
      margin: 0;
      font-size: 16px;
      font-weight: 700;
      letter-spacing: 0;
    }

    .status {
      display: flex;
      align-items: center;
      gap: 8px;
      color: var(--muted);
      font-size: 13px;
    }

    .dot {
      width: 8px;
      height: 8px;
      border-radius: 999px;
      background: var(--accent);
    }

    main {
      display: grid;
      grid-template-columns: minmax(360px, 1.05fr) minmax(360px, 0.95fr);
      gap: 1px;
      min-height: calc(100vh - 56px);
      background: var(--border);
    }

    section {
      background: var(--bg);
      min-width: 0;
      padding: 16px;
      display: flex;
      flex-direction: column;
      gap: 12px;
    }

    form {
      display: flex;
      flex-direction: column;
      gap: 12px;
      min-height: 0;
    }

    .row {
      display: flex;
      align-items: center;
      justify-content: space-between;
      gap: 12px;
    }

    h2 {
      margin: 0;
      color: var(--muted);
      font-size: 12px;
      font-weight: 700;
      text-transform: uppercase;
      letter-spacing: 0.08em;
    }

    label {
      display: flex;
      flex-direction: column;
      gap: 7px;
      min-height: 0;
      color: var(--muted);
      font-size: 12px;
      font-weight: 700;
      text-transform: uppercase;
      letter-spacing: 0.06em;
    }

    .field {
      display: flex;
      flex-direction: column;
      gap: 7px;
      min-height: 0;
    }

    .field-label {
      color: var(--muted);
      font-size: 12px;
      font-weight: 700;
      text-transform: uppercase;
      letter-spacing: 0.06em;
    }

    textarea, pre, input {
      width: 100%;
      margin: 0;
      border: 1px solid var(--border);
      border-radius: 6px;
      background: var(--panel);
      color: var(--text);
      font: 13px/1.45 ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
      letter-spacing: 0;
      resize: vertical;
      outline: none;
    }

    textarea, input {
      padding: 12px;
    }

    input {
      min-height: 40px;
      resize: none;
    }

    textarea:focus, input:focus {
      border-color: var(--accent);
    }

    #source { min-height: 250px; }
    #object, #context, #manifest { min-height: 130px; }

    button {
      min-height: 36px;
      border: 1px solid var(--accent);
      border-radius: 6px;
      padding: 0 14px;
      background: var(--accent);
      color: #08110f;
      font-weight: 800;
      cursor: pointer;
    }

    button:disabled {
      opacity: 0.6;
      cursor: wait;
    }

    .secondary {
      border-color: var(--border);
      background: var(--panel-2);
      color: var(--text);
    }

    .controls {
      display: grid;
      grid-template-columns: minmax(120px, 160px) minmax(0, 1fr);
      gap: 10px;
      align-items: end;
    }

    .checkbox-row {
      min-height: 40px;
      display: flex;
      align-items: center;
      gap: 12px;
      padding: 8px 10px;
      border: 1px solid var(--border);
      border-radius: 6px;
      background: var(--panel);
    }

    .checkbox-row label {
      flex-direction: row;
      align-items: center;
      gap: 7px;
      min-height: auto;
      text-transform: none;
      letter-spacing: 0;
      font-size: 13px;
      font-weight: 700;
      color: var(--text);
    }

    input[type="checkbox"] {
      width: 16px;
      height: 16px;
      min-height: 16px;
      padding: 0;
      accent-color: var(--accent);
    }

    .metrics {
      display: grid;
      grid-template-columns: repeat(4, minmax(0, 1fr));
      gap: 8px;
    }

    .metric {
      border: 1px solid var(--border);
      border-radius: 6px;
      padding: 10px;
      background: var(--panel);
    }

    .metric span {
      display: block;
      color: var(--muted);
      font-size: 11px;
      font-weight: 700;
      text-transform: uppercase;
      letter-spacing: 0.04em;
    }

    .metric strong {
      display: block;
      margin-top: 6px;
      font-size: 14px;
      overflow-wrap: anywhere;
    }

    pre {
      min-height: 180px;
      overflow: auto;
      padding: 12px;
      white-space: pre-wrap;
    }

    .result-stack {
      display: grid;
      grid-template-rows: minmax(110px, auto) minmax(150px, auto) minmax(180px, 1fr) minmax(180px, 1fr);
      gap: 12px;
      flex: 1;
      min-height: 0;
    }

    .result-stack label {
      min-height: 0;
    }

    .result-stack pre {
      height: 100%;
    }

    .chart {
      min-height: 150px;
      border: 1px solid var(--border);
      border-radius: 6px;
      background: var(--panel);
      padding: 10px;
      display: flex;
      flex-direction: column;
      gap: 8px;
      overflow: auto;
    }

    .chart-empty {
      min-height: 128px;
      display: grid;
      place-items: center;
      color: var(--muted);
      font-size: 13px;
    }

    .bar-row {
      display: grid;
      grid-template-columns: 92px minmax(0, 1fr) 76px;
      gap: 8px;
      align-items: center;
      min-height: 24px;
      font: 12px/1.2 ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
    }

    .bar-label {
      color: var(--muted);
      overflow: hidden;
      text-overflow: ellipsis;
      white-space: nowrap;
    }

    .bar-track {
      height: 14px;
      border-radius: 4px;
      background: var(--panel-2);
      overflow: hidden;
      border: 1px solid var(--border);
    }

    .bar {
      height: 100%;
      min-width: 2px;
      background: var(--accent);
    }

    .bar.inspect {
      background: var(--warn);
    }

    .bar-value {
      color: var(--text);
      text-align: right;
    }

    .lifecycle {
      min-height: 110px;
      border: 1px solid var(--border);
      border-radius: 6px;
      background: var(--panel);
      padding: 10px;
      display: flex;
      flex-direction: column;
      gap: 7px;
      font: 12px/1.25 ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
      overflow: auto;
    }

    .step {
      display: grid;
      grid-template-columns: 86px minmax(0, 1fr);
      gap: 8px;
      color: var(--text);
    }

    .step span:first-child {
      color: var(--muted);
    }

    .ok { color: var(--good); }
    .fail { color: var(--bad); }
    .pending { color: var(--warn); }

    @media (max-width: 900px) {
      main {
        grid-template-columns: 1fr;
      }

      .metrics {
        grid-template-columns: repeat(2, minmax(0, 1fr));
      }

      .controls {
        grid-template-columns: 1fr;
      }
    }
  </style>
</head>
<body>
  <header>
    <h1>Capsem JS to WASM Component Spike</h1>
    <div class="status"><span class="dot"></span><span id="state">ready</span></div>
  </header>

  <main>
    <section>
      <div class="row">
        <h2>Compiler Input</h2>
        <button id="reset" class="secondary" type="button">Reset</button>
      </div>

      <form id="compile-form">
      <label>
        JavaScript module
        <textarea id="source" spellcheck="false">${escapeHtml(DEFAULT_SOURCE)}</textarea>
      </label>

      <label>
        Plugin manifest
        <textarea id="manifest" spellcheck="false">${escapeHtml(JSON.stringify(DEFAULT_MANIFEST, null, 2))}</textarea>
      </label>

      <label>
        Object JSON
        <textarea id="object" spellcheck="false">${escapeHtml(JSON.stringify(DEFAULT_OBJECT, null, 2))}</textarea>
      </label>

      <label>
        Context JSON
        <textarea id="context" spellcheck="false">${escapeHtml(JSON.stringify(DEFAULT_CONTEXT, null, 2))}</textarea>
      </label>

      <div class="controls">
        <label>
          Runs
          <input id="runs" type="number" min="1" max="${LIMITS.maxRuns}" step="1" value="3">
        </label>
        <div class="field">
          <span class="field-label">Functions</span>
          <div class="checkbox-row">
            <label><input id="function-invoke" type="checkbox" checked> invoke</label>
            <label><input id="function-inspect" type="checkbox" checked> inspect</label>
          </div>
        </div>
      </div>

      <div class="row">
        <span id="message" class="pending"></span>
        <button id="run" type="submit">Run</button>
      </div>
      </form>
    </section>

    <section>
      <div class="row">
        <h2>Result</h2>
        <span id="result-status" class="pending">not run</span>
      </div>

      <div class="metrics">
        <div class="metric"><span>Compile</span><strong id="compile-ms">-</strong></div>
        <div class="metric"><span>Transpile</span><strong id="transpile-ms">-</strong></div>
        <div class="metric"><span>Instantiate</span><strong id="instantiate-ms">-</strong></div>
        <div class="metric"><span>Run</span><strong id="run-ms">-</strong></div>
        <div class="metric"><span>WASM</span><strong id="wasm-bytes">-</strong></div>
        <div class="metric"><span>Traces</span><strong id="trace-count">-</strong></div>
        <div class="metric"><span>Plugin</span><strong id="plugin-id">-</strong></div>
        <div class="metric"><span>Artifact</span><strong id="artifact-id">-</strong></div>
      </div>

      <div class="result-stack">
        <label>
          Lifecycle
          <div id="lifecycle-output" class="lifecycle">
            <div class="step"><span>ready</span><span>click Run to install, save, register, and execute</span></div>
          </div>
        </label>
        <label>
          Timing Bars
          <div id="trace-chart" class="chart">
            <div class="chart-empty">no traces</div>
          </div>
        </label>
        <label>
          Output
          <pre id="output">{}</pre>
        </label>
        <label>
          Trace
          <pre id="trace-output">[]</pre>
        </label>
      </div>
    </section>
  </main>

  <script>
    const defaults = {
      source: ${JSON.stringify(DEFAULT_SOURCE)},
      manifest: ${JSON.stringify(JSON.stringify(DEFAULT_MANIFEST, null, 2))},
      object: ${JSON.stringify(JSON.stringify(DEFAULT_OBJECT, null, 2))},
      context: ${JSON.stringify(JSON.stringify(DEFAULT_CONTEXT, null, 2))},
      runs: 3,
      functions: ["invoke", "inspect"]
    };

    const el = (id) => document.getElementById(id);
    let installedPluginId = "";
    let lastInstallPayload = null;
    const setMetric = (id, value, suffix = " ms") => {
      el(id).textContent = typeof value === "number" ? value.toFixed(3) + suffix : "-";
    };

    function resetMetrics() {
      setMetric("compile-ms");
      setMetric("transpile-ms");
      setMetric("instantiate-ms");
      setMetric("run-ms");
      el("wasm-bytes").textContent = "-";
      el("trace-count").textContent = "-";
      el("plugin-id").textContent = installedPluginId || "-";
      el("artifact-id").textContent = "-";
      renderChart([]);
    }

    function setLifecycle(steps) {
      const container = el("lifecycle-output");
      container.replaceChildren(
        ...steps.map((step) => {
          const row = document.createElement("div");
          row.className = "step";
          const phase = document.createElement("span");
          phase.textContent = step.phase;
          const detail = document.createElement("span");
          detail.textContent = step.detail;
          row.append(phase, detail);
          return row;
        })
      );
    }

    function renderChart(traces) {
      const chart = el("trace-chart");
      if (!Array.isArray(traces) || traces.length === 0) {
        chart.innerHTML = '<div class="chart-empty">no traces</div>';
        return;
      }

      const max = Math.max(...traces.map((trace) => Number(trace.run_ms) || 0), 0.001);
      chart.replaceChildren(
        ...traces.map((trace) => {
          const row = document.createElement("div");
          row.className = "bar-row";

          const label = document.createElement("div");
          label.className = "bar-label";
          label.textContent = trace.iteration + ":" + trace.function;
          label.title = label.textContent;

          const track = document.createElement("div");
          track.className = "bar-track";
          const bar = document.createElement("div");
          bar.className = "bar " + trace.function;
          bar.style.width = Math.max(2, ((Number(trace.run_ms) || 0) / max) * 100).toFixed(2) + "%";
          track.appendChild(bar);

          const value = document.createElement("div");
          value.className = "bar-value";
          value.textContent = (Number(trace.run_ms) || 0).toFixed(3) + " ms";

          row.append(label, track, value);
          return row;
        })
      );
    }

    function showPayload(payload) {
      if (typeof payload.compile_ms === "number") setMetric("compile-ms", payload.compile_ms);
      if (typeof payload.transpile_ms === "number") setMetric("transpile-ms", payload.transpile_ms);
      setMetric("instantiate-ms", payload.instantiate_ms);
      setMetric("run-ms", payload.run_ms);
      el("wasm-bytes").textContent =
        typeof payload.wasm_bytes === "number" ? (payload.wasm_bytes / 1024 / 1024).toFixed(2) + " MB" : "-";
      el("trace-count").textContent = Array.isArray(payload.traces) ? String(payload.traces.length) : "-";
      if (payload.plugin_id) {
        installedPluginId = payload.plugin_id;
        el("plugin-id").textContent = payload.plugin_id;
      }
      el("artifact-id").textContent = payload.artifact_blake3 ? payload.artifact_blake3.slice(7, 19) : "-";
      el("output").textContent = JSON.stringify(payload.ok ? payload.result : payload, null, 2);
      el("trace-output").textContent = JSON.stringify(payload.ok ? payload.traces : [], null, 2);
      renderChart(payload.ok ? payload.traces : []);
      el("result-status").textContent = payload.ok ? "ok" : "failed: " + payload.phase;
      el("result-status").className = payload.ok ? "ok" : "fail";
    }

    function showInstallPayload(payload) {
      lastInstallPayload = payload.ok ? payload : null;
      setMetric("compile-ms", payload.compile_ms);
      setMetric("transpile-ms", payload.transpile_ms);
      setMetric("instantiate-ms");
      setMetric("run-ms");
      el("wasm-bytes").textContent =
        typeof payload.wasm_bytes === "number" ? (payload.wasm_bytes / 1024 / 1024).toFixed(2) + " MB" : "-";
      el("trace-count").textContent = "-";
      if (payload.ok) {
        installedPluginId = payload.plugin_id;
        el("plugin-id").textContent = payload.plugin_id;
        el("artifact-id").textContent = payload.artifact_blake3.slice(7, 19);
      }
      el("output").textContent = JSON.stringify(payload, null, 2);
      el("trace-output").textContent = "[]";
      renderChart([]);
      el("result-status").textContent = payload.ok ? "installed" : "failed: " + payload.phase;
      el("result-status").className = payload.ok ? "ok" : "fail";
    }

    async function install() {
      el("message").textContent = "";
      el("state").textContent = "installing";
      el("run").disabled = true;
      resetMetrics();
      setLifecycle([{ phase: "install", detail: "validating manifest and compiling source" }]);

      let manifest;
      try {
        manifest = JSON.parse(el("manifest").value);
      } catch (error) {
        el("message").textContent = String(error.message || error);
        el("state").textContent = "input error";
        el("run").disabled = false;
        return null;
      }

      const started = performance.now();
      try {
        const response = await fetch("/plugins/install", {
          method: "POST",
          headers: { "content-type": "application/json" },
          body: JSON.stringify({ source: el("source").value, manifest })
        });
        const payload = await response.json();
        showInstallPayload(payload);
        el("message").textContent = "install wall time " + (performance.now() - started).toFixed(1) + " ms";
        el("state").textContent = payload.ok ? "ready" : "failed";
        if (payload.ok) {
          setLifecycle([
            { phase: "install", detail: "compiled source through fixed WIT" },
            { phase: "artifact", detail: payload.artifact_blake3 + " saved to disk" },
            { phase: "manifest", detail: payload.plugin_id + " registered in memory" },
          ]);
          return payload.plugin_id;
        }
        return null;
      } catch (error) {
        el("output").textContent = String(error.stack || error);
        el("result-status").textContent = "request failed";
        el("result-status").className = "fail";
        el("state").textContent = "failed";
        el("trace-output").textContent = "[]";
        renderChart([]);
        return null;
      } finally {
        el("run").disabled = false;
      }
    }

    async function run() {
      el("message").textContent = "";
      el("state").textContent = "installing";
      el("run").disabled = true;

      const fullStarted = performance.now();
      installedPluginId = "";
      const pluginId = await install();
      if (!pluginId) {
        el("run").disabled = false;
        return;
      }

      let object;
      let context;
      let runs;
      const functions = [];
      try {
        object = JSON.parse(el("object").value);
        context = JSON.parse(el("context").value);
        runs = Number(el("runs").value);
        if (!Number.isInteger(runs) || runs < 1 || runs > ${LIMITS.maxRuns}) {
          throw new Error("runs must be an integer from 1 to ${LIMITS.maxRuns}");
        }
        if (el("function-invoke").checked) functions.push("invoke");
        if (el("function-inspect").checked) functions.push("inspect");
        if (functions.length === 0) {
          throw new Error("select at least one function");
        }
      } catch (error) {
        el("message").textContent = String(error.message || error);
        el("state").textContent = "input error";
        el("run").disabled = false;
        return;
      }

      const started = performance.now();
      try {
        el("state").textContent = "running";
        setLifecycle([
          { phase: "install", detail: "compiled source through fixed WIT" },
          { phase: "artifact", detail: el("artifact-id").textContent + " saved to disk" },
          { phase: "manifest", detail: installedPluginId + " registered in memory" },
          { phase: "run", detail: "calling selected callbacks against object/context copies" },
        ]);
        setMetric("instantiate-ms");
        setMetric("run-ms");
        el("trace-count").textContent = "-";
        renderChart([]);
        const response = await fetch("/plugins/run", {
          method: "POST",
          headers: { "content-type": "application/json" },
          body: JSON.stringify({ plugin_id: installedPluginId, object, context, functions, runs })
        });
        const payload = await response.json();
        showPayload(payload);
        if (payload.ok) {
          setLifecycle([
            {
              phase: "install",
              detail:
                "compile " + (lastInstallPayload?.compile_ms ?? "-") +
                " ms, transpile " + (lastInstallPayload?.transpile_ms ?? "-") + " ms",
            },
            { phase: "artifact", detail: payload.artifact_blake3 + " saved as .wasm" },
            { phase: "manifest", detail: payload.plugin_id + " internal manifest loaded" },
            { phase: "run", detail: payload.traces.length + " callback calls in " + payload.run_ms + " ms" },
          ]);
        }
        el("message").textContent =
          "run wall time " + (performance.now() - started).toFixed(1) +
          " ms, full wall time " + (performance.now() - fullStarted).toFixed(1) + " ms";
        el("state").textContent = payload.ok ? "ready" : "failed";
      } catch (error) {
        el("output").textContent = String(error.stack || error);
        el("result-status").textContent = "request failed";
        el("result-status").className = "fail";
        el("state").textContent = "failed";
        el("trace-output").textContent = "[]";
        renderChart([]);
      } finally {
        el("run").disabled = false;
      }
    }

    el("compile-form").addEventListener("submit", (event) => {
      event.preventDefault();
      run();
    });
    el("reset").addEventListener("click", () => {
      installedPluginId = "";
      el("source").value = defaults.source;
      el("manifest").value = defaults.manifest;
      el("object").value = defaults.object;
      el("context").value = defaults.context;
      el("runs").value = String(defaults.runs);
      el("function-invoke").checked = true;
      el("function-inspect").checked = true;
      el("message").textContent = "";
      el("result-status").textContent = "not run";
      el("result-status").className = "pending";
      el("output").textContent = "{}";
      el("trace-output").textContent = "[]";
      setLifecycle([{ phase: "ready", detail: "click Run to install, save, register, and execute" }]);
      resetMetrics();
    });
  </script>
</body>
</html>`;
}

async function readBody(req: http.IncomingMessage): Promise<string> {
  let total = 0;
  const chunks: Buffer[] = [];
  for await (const chunk of req) {
    const buffer = Buffer.isBuffer(chunk) ? chunk : Buffer.from(chunk);
    total += buffer.length;
    if (total > LIMITS.requestBytes) {
      throw new Error(`request exceeds ${LIMITS.requestBytes} bytes`);
    }
    chunks.push(buffer);
  }
  return Buffer.concat(chunks).toString("utf8");
}

function sendJson(
  res: http.ServerResponse,
  status: number,
  payload: CompileRunResponse | InstallPluginResponse | RunPluginResponse | { ok: false; error: string },
) {
  const body = JSON.stringify(payload);
  res.writeHead(status, {
    "content-type": "application/json",
    "content-length": Buffer.byteLength(body),
  });
  res.end(body);
}

function sendHtml(res: http.ServerResponse, html: string) {
  res.writeHead(200, {
    "content-type": "text/html; charset=utf-8",
    "content-length": Buffer.byteLength(html),
  });
  res.end(html);
}

function redirect(res: http.ServerResponse, target: string) {
  res.writeHead(308, { location: target });
  res.end();
}

export function createServer(): http.Server {
  return http.createServer(async (req, res) => {
    if (req.method === "GET" && req.url === "/compile-run") {
      redirect(res, "/");
      return;
    }

    if (req.method === "GET" && req.url === "/") {
      sendHtml(res, pageHtml());
      return;
    }

    if (req.method === "POST" && req.url === "/plugins/install") {
      let parsed: InstallPluginRequest;
      try {
        parsed = JSON.parse(await readBody(req)) as InstallPluginRequest;
      } catch (error) {
        sendJson(res, 400, { ok: false, error: String((error as Error).message || error) });
        return;
      }
      const result = await installPlugin(parsed);
      sendJson(res, result.ok ? 200 : 400, result);
      return;
    }

    if (req.method === "POST" && req.url === "/plugins/run") {
      let parsed: RunPluginRequest;
      try {
        parsed = JSON.parse(await readBody(req)) as RunPluginRequest;
      } catch (error) {
        sendJson(res, 400, { ok: false, error: String((error as Error).message || error) });
        return;
      }
      const result = await runPlugin(parsed);
      sendJson(res, result.ok ? 200 : 400, result);
      return;
    }

    if (req.method !== "POST" || req.url !== "/") {
      sendJson(res, 404, { ok: false, error: "not found" });
      return;
    }

    let parsed: CompileRunRequest;
    try {
      parsed = JSON.parse(await readBody(req)) as CompileRunRequest;
    } catch (error) {
      sendJson(res, 400, { ok: false, error: String((error as Error).message || error) });
      return;
    }

    const result = await compileRun(parsed);
    sendJson(res, result.ok ? 200 : 400, result);
  });
}

if (import.meta.url === `file://${process.argv[1]}`) {
  const port = Number(process.env.PORT || 8787);
  createServer().listen(port, () => {
    console.log(`compile-run service listening on http://127.0.0.1:${port}`);
  });
}
