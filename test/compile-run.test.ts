import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { stat } from "node:fs/promises";
import path from "node:path";

import { compileRun, installPlugin, runPlugin } from "../src/component-runner.js";
import { LIMITS } from "../src/limits.js";
import { createServer } from "../src/server.js";

const successSource = `
export function invoke(objectJson, contextJson) {
  const object = JSON.parse(objectJson);
  const context = JSON.parse(contextJson);
  object.labels = context.trace?.labels ?? [];
  object.called = "invoke";
  return JSON.stringify(object);
}

export function inspect(objectJson, contextJson) {
  const object = JSON.parse(objectJson);
  const context = JSON.parse(contextJson);
  return JSON.stringify({
    kind: "Inspection",
    object_kind: object.kind,
    context_keys: Object.keys(context),
    labels: context.trace?.labels ?? [],
    called: "inspect"
  });
}
`;

describe("compileRun", () => {
  beforeEach(() => {
    vi.spyOn(console, "log").mockImplementation(() => {});
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  it(
    "installs a plugin as a BLAKE3-addressed WASM artifact and runs it from the registry",
    async () => {
      const install = await installPlugin({
        source: successSource,
        manifest: {
          id: "test.security",
          name: "Test Security Plugin",
          version: "0.1.0",
          callbacks: ["invoke", "inspect"],
          capabilities: [],
        },
      });

      expect(install.ok).toBe(true);
      if (!install.ok) throw new Error(install.error);
      expect(install.artifact_blake3).toMatch(/^blake3:[0-9a-f]{64}$/);
      expect(path.basename(install.artifact_path)).toBe(`${install.artifact_blake3.slice("blake3:".length)}.wasm`);
      expect((await stat(install.artifact_path)).size).toBe(install.wasm_bytes);
      expect(install.manifest.source_blake3).toMatch(/^blake3:[0-9a-f]{64}$/);
      expect(install.manifest.trace.callbacks).toEqual(["invoke", "inspect"]);

      const run = await runPlugin({
        plugin_id: "test.security",
        object: { kind: "ModelOutput", content: { text: "hi" } },
        context: { trace: { labels: ["registry"] } },
        functions: ["invoke", "inspect"],
        runs: 2,
      });

      expect(run.ok).toBe(true);
      if (!run.ok) throw new Error(run.error);
      expect(run.plugin_id).toBe("test.security");
      expect(run.artifact_blake3).toBe(install.artifact_blake3);
      expect(run.traces).toHaveLength(4);
      expect(run.result).toMatchObject({ kind: "Inspection", called: "inspect" });
      expect("compile_ms" in run).toBe(false);
      expect("transpile_ms" in run).toBe(false);
    },
    90_000,
  );

  it(
    "compiles two JavaScript callbacks into one WASM component and traces repeated runs",
    async () => {
      const result = await compileRun({
        source: successSource,
        object: { kind: "ModelOutput", content: { text: "hi" } },
        context: { trace: { labels: ["pii"] } },
        functions: ["invoke", "inspect"],
        runs: 2,
      });

      expect(result.ok).toBe(true);
      if (!result.ok) throw new Error(result.error);
      expect(result.result).toEqual({
        kind: "Inspection",
        object_kind: "ModelOutput",
        context_keys: ["trace"],
        labels: ["pii"],
        called: "inspect",
      });
      expect(result.functions).toEqual(["invoke", "inspect"]);
      expect(result.runs).toBe(2);
      expect(result.traces).toHaveLength(4);
      expect(result.traces.map((trace) => `${trace.iteration}:${trace.function}`)).toEqual([
        "1:invoke",
        "1:inspect",
        "2:invoke",
        "2:inspect",
      ]);
      expect(result.traces[0].result).toMatchObject({
        kind: "ModelOutput",
        content: { text: "hi" },
        labels: ["pii"],
        called: "invoke",
      });
      expect(result.traces.every((trace) => trace.run_ms >= 0 && trace.result_bytes > 0)).toBe(true);
      expect(result.compile_ms).toBeGreaterThan(0);
      expect(result.transpile_ms).toBeGreaterThan(0);
      expect(result.instantiate_ms).toBeGreaterThan(0);
      expect(result.run_ms).toBeGreaterThanOrEqual(0);
      expect(result.wasm_bytes).toBeGreaterThan(1_000_000);
    },
    90_000,
  );

  it("rejects oversized source before compiling", async () => {
    const result = await compileRun({
      source: "x".repeat(LIMITS.sourceBytes + 1),
      object: {},
      context: {},
    });

    expect(result).toMatchObject({ ok: false, phase: "request" });
  });

  it(
    "reports syntax errors as compile failures",
    async () => {
      const result = await compileRun({
        source: "export function invoke(objectJson, contextJson) {",
        object: {},
        context: {},
      });

      expect(result).toMatchObject({ ok: false, phase: "compile" });
    },
    90_000,
  );

  it(
    "reports callback throws as run failures",
    async () => {
      const result = await compileRun({
        source: `
export function invoke() {
  throw new Error("boom");
}
export function inspect() {
  return "{}";
}
`,
        object: {},
        context: {},
        functions: ["invoke"],
      });

      expect(result).toMatchObject({ ok: false, phase: "run" });
      if (!result.ok) expect(result.error).toContain("RuntimeError");
    },
    90_000,
  );

  it(
    "reports non-JSON return values as result failures",
    async () => {
      const result = await compileRun({
        source: `
export function invoke() {
  return "not-json";
}
export function inspect() {
  return "{}";
}
`,
        object: {},
        context: {},
        functions: ["invoke"],
      });

      expect(result).toMatchObject({ ok: false, phase: "result" });
    },
    90_000,
  );

  it(
    "kills long-running callbacks and reports run failure",
    async () => {
      const result = await compileRun({
        source: `
export function invoke() {
  while (true) {}
}
export function inspect() {
  return "{}";
}
`,
        object: {},
        context: {},
        functions: ["invoke"],
      });

      expect(result).toMatchObject({ ok: false, phase: "run" });
      if (!result.ok) expect(result.error).toContain("timed out");
    },
    120_000,
  );

  it(
    "exposes the same lane through POST /",
    async () => {
      const server = createServer();
      await new Promise<void>((resolve) => server.listen(0, "127.0.0.1", resolve));
      const address = server.address();
      if (!address || typeof address === "string") {
        throw new Error("expected TCP server address");
      }
      try {
        const response = await fetch(`http://127.0.0.1:${address.port}/`, {
          method: "POST",
          headers: { "content-type": "application/json" },
          body: JSON.stringify({
            source: successSource,
            object: { kind: "ModelOutput" },
            context: { trace: { labels: ["http"] } },
            functions: ["invoke", "inspect"],
            runs: 2,
          }),
        });
        expect(response.status).toBe(200);
        const payload = await response.json();
        expect(payload.ok).toBe(true);
        expect(payload.traces).toHaveLength(4);
        expect(payload.result.called).toBe("inspect");
        expect(payload.traces[0].result.labels).toEqual(["http"]);
      } finally {
        await new Promise<void>((resolve, reject) =>
          server.close((error) => (error ? reject(error) : resolve())),
        );
      }
    },
    90_000,
  );

  it("serves an HTML interface at GET /", async () => {
    const server = createServer();
    await new Promise<void>((resolve) => server.listen(0, "127.0.0.1", resolve));
    const address = server.address();
    if (!address || typeof address === "string") {
      throw new Error("expected TCP server address");
    }
    try {
      const response = await fetch(`http://127.0.0.1:${address.port}/`);
      expect(response.status).toBe(200);
      expect(response.headers.get("content-type")).toContain("text/html");
      const html = await response.text();
      expect(html).toContain("Capsem JS to WASM Component Spike");
      expect(html).toContain(">Run</button>");
      expect(html).not.toContain("Run Installed");
      expect(html).not.toContain(">Install</button>");
      expect(html).toContain("JavaScript module");
      expect(html).toContain("Plugin manifest");
      expect(html).toContain("Functions");
      expect(html).toContain("Trace");
      expect(html).toContain("Timing Bars");
      expect(html).toContain("Lifecycle");
      expect(html).toContain('id="trace-chart"');
      expect(html).toContain('<form id="compile-form">');
      expect(html).toContain('fetch("/plugins/install",');
      expect(html).toContain('fetch("/plugins/run",');
    } finally {
      await new Promise<void>((resolve, reject) =>
        server.close((error) => (error ? reject(error) : resolve())),
      );
    }
  });

  it("redirects old GET /compile-run to /", async () => {
    const server = createServer();
    await new Promise<void>((resolve) => server.listen(0, "127.0.0.1", resolve));
    const address = server.address();
    if (!address || typeof address === "string") {
      throw new Error("expected TCP server address");
    }
    try {
      const response = await fetch(`http://127.0.0.1:${address.port}/compile-run`, {
        redirect: "manual",
      });
      expect(response.status).toBe(308);
      expect(response.headers.get("location")).toBe("/");
    } finally {
      await new Promise<void>((resolve, reject) =>
        server.close((error) => (error ? reject(error) : resolve())),
      );
    }
  });
});
