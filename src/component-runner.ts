import { execFile } from "node:child_process";
import { copyFile, mkdir, mkdtemp, readFile, readdir, rm, stat, symlink, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { promisify } from "node:util";
import { Blake3Hasher, blake3 } from "@napi-rs/blake-hash";

import { LIMITS } from "./limits.js";
import { measure, roundMs } from "./timing.js";
import type {
  CompileRunFailure,
  CompileRunRequest,
  CompileRunResponse,
  ComponentFunctionName,
  FunctionRunTrace,
  InstallPluginFailure,
  InstallPluginRequest,
  InstallPluginResponse,
  InternalPluginManifest,
  InvocationLog,
  PublicPluginManifest,
  RunPluginRequest,
  RunPluginResponse,
} from "./types.js";

const execFileAsync = promisify(execFile);
const ROOT_DIR = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const JCO_BIN = path.join(ROOT_DIR, "node_modules", ".bin", "jco");
const ARTIFACTS_DIR = path.join(ROOT_DIR, "artifacts");
const TRANSPILED_DIR = path.join(ARTIFACTS_DIR, "_transpiled");
const RUNTIME_KIND = "node-transpiled-component";
const WIT_VERSION = "capsem-plugin-spike-v1";
const JCO_VERSION = "1.19.0";

const WIT = `package capsem:plugin-spike;

world plugin {
  export invoke: func(object-json: string, context-json: string) -> string;
  export inspect: func(object-json: string, context-json: string) -> string;
}
`;

const COMPONENT_FUNCTIONS: readonly ComponentFunctionName[] = ["invoke", "inspect"];
const registry = new Map<string, InstalledPlugin>();

interface ValidatedRequest {
  objectJson: string;
  contextJson: string;
  functions: ComponentFunctionName[];
  runs: number;
}

interface InstalledPlugin {
  manifest: InternalPluginManifest;
  modulePath: string;
  wasmBytes: number;
}

function byteLength(value: string): number {
  return Buffer.byteLength(value, "utf8");
}

function blake3Hex(payload: string | Buffer): string {
  return `blake3:${blake3(payload).toString("hex")}`;
}

async function hashDirectory(dir: string): Promise<{ hash: string; bytes: number }> {
  const hasher = new Blake3Hasher();
  let bytes = 0;

  async function walk(current: string, relativePrefix = ""): Promise<void> {
    const entries = await readdir(current, { withFileTypes: true });
    entries.sort((a, b) => a.name.localeCompare(b.name));
    for (const entry of entries) {
      if (entry.name === "node_modules") continue;
      const absolute = path.join(current, entry.name);
      const relative = path.join(relativePrefix, entry.name);
      if (entry.isDirectory()) {
        await walk(absolute, relative);
      } else if (entry.isFile()) {
        const payload = await readFile(absolute);
        hasher.update(Buffer.from(`${relative}\0${payload.byteLength}\0`, "utf8"));
        hasher.update(payload);
        bytes += payload.byteLength;
      }
    }
  }

  await walk(dir);
  return { hash: `blake3:${hasher.digest("hex")}`, bytes };
}

function fail(
  phase: CompileRunFailure["phase"],
  error: string,
  partial: Omit<CompileRunFailure, "ok" | "phase" | "error"> = {},
): CompileRunFailure {
  return { ok: false, phase, error, ...partial };
}

function installFail(
  phase: InstallPluginFailure["phase"],
  error: string,
  partial: Omit<InstallPluginFailure, "ok" | "phase" | "error"> = {},
): InstallPluginFailure {
  return { ok: false, phase, error, ...partial };
}

function validateManifest(manifest: PublicPluginManifest): ComponentFunctionName[] | InstallPluginFailure {
  if (!manifest || typeof manifest !== "object") {
    return installFail("request", "manifest must be a JSON object");
  }
  if (!/^[a-z0-9][a-z0-9._-]{1,63}$/i.test(manifest.id)) {
    return installFail("request", "manifest.id must be 2-64 URL-safe characters");
  }
  if (typeof manifest.name !== "string" || manifest.name.trim().length === 0) {
    return installFail("request", "manifest.name must be a non-empty string");
  }
  if (typeof manifest.version !== "string" || manifest.version.trim().length === 0) {
    return installFail("request", "manifest.version must be a non-empty string");
  }
  if (!Array.isArray(manifest.callbacks) || manifest.callbacks.length === 0) {
    return installFail("request", "manifest.callbacks must be a non-empty array");
  }
  const callbacks: ComponentFunctionName[] = [];
  for (const callback of manifest.callbacks) {
    if (!COMPONENT_FUNCTIONS.includes(callback as ComponentFunctionName)) {
      return installFail("request", `unsupported callback: ${String(callback)}`);
    }
    if (!callbacks.includes(callback as ComponentFunctionName)) {
      callbacks.push(callback as ComponentFunctionName);
    }
  }
  if (manifest.capabilities !== undefined) {
    if (!Array.isArray(manifest.capabilities) || !manifest.capabilities.every((capability) => typeof capability === "string")) {
      return installFail("request", "manifest.capabilities must be an array of strings");
    }
  }
  return callbacks;
}

function validateInstallRequest(request: InstallPluginRequest): { callbacks: ComponentFunctionName[] } | InstallPluginFailure {
  if (!request || typeof request !== "object") {
    return installFail("request", "request must be a JSON object");
  }
  if (typeof request.source !== "string") {
    return installFail("request", "source must be a string");
  }
  if (byteLength(request.source) > LIMITS.sourceBytes) {
    return installFail("request", `source exceeds ${LIMITS.sourceBytes} bytes`);
  }
  const callbacks = validateManifest(request.manifest);
  if (!Array.isArray(callbacks)) {
    return callbacks;
  }
  return { callbacks };
}

function validateRequest(request: CompileRunRequest): ValidatedRequest | CompileRunFailure {
  if (!request || typeof request !== "object") {
    return fail("request", "request must be a JSON object");
  }
  if (typeof request.source !== "string") {
    return fail("request", "source must be a string");
  }
  if (byteLength(request.source) > LIMITS.sourceBytes) {
    return fail("request", `source exceeds ${LIMITS.sourceBytes} bytes`);
  }

  let objectJson: string;
  let contextJson: string;
  try {
    objectJson = JSON.stringify(request.object);
    contextJson = JSON.stringify(request.context);
  } catch (error) {
    return fail("request", `object/context must be JSON-serializable: ${String(error)}`);
  }
  if (objectJson === undefined || contextJson === undefined) {
    return fail("request", "object and context must be present");
  }
  if (byteLength(objectJson) > LIMITS.objectBytes) {
    return fail("request", `object exceeds ${LIMITS.objectBytes} bytes`);
  }
  if (byteLength(contextJson) > LIMITS.contextBytes) {
    return fail("request", `context exceeds ${LIMITS.contextBytes} bytes`);
  }

  const requestedFunctions = request.functions ?? ["invoke"];
  if (!Array.isArray(requestedFunctions) || requestedFunctions.length === 0) {
    return fail("request", "functions must be a non-empty array");
  }

  const functions: ComponentFunctionName[] = [];
  for (const name of requestedFunctions) {
    if (!COMPONENT_FUNCTIONS.includes(name as ComponentFunctionName)) {
      return fail("request", `unsupported function: ${String(name)}`);
    }
    if (!functions.includes(name as ComponentFunctionName)) {
      functions.push(name as ComponentFunctionName);
    }
  }

  const runs = request.runs ?? 1;
  if (!Number.isInteger(runs) || runs < 1 || runs > LIMITS.maxRuns) {
    return fail("request", `runs must be an integer from 1 to ${LIMITS.maxRuns}`);
  }

  return { objectJson, contextJson, functions, runs };
}

function validateRunRequest(request: RunPluginRequest, installed?: InstalledPlugin): ValidatedRequest | CompileRunFailure {
  if (!request || typeof request !== "object") {
    return fail("request", "request must be a JSON object");
  }
  if (typeof request.plugin_id !== "string" || request.plugin_id.length === 0) {
    return fail("request", "plugin_id must be a non-empty string");
  }
  if (!installed) {
    return fail("request", `plugin is not installed: ${request.plugin_id}`);
  }

  let objectJson: string;
  let contextJson: string;
  try {
    objectJson = JSON.stringify(request.object);
    contextJson = JSON.stringify(request.context);
  } catch (error) {
    return fail("request", `object/context must be JSON-serializable: ${String(error)}`);
  }
  if (objectJson === undefined || contextJson === undefined) {
    return fail("request", "object and context must be present");
  }
  if (byteLength(objectJson) > LIMITS.objectBytes) {
    return fail("request", `object exceeds ${LIMITS.objectBytes} bytes`);
  }
  if (byteLength(contextJson) > LIMITS.contextBytes) {
    return fail("request", `context exceeds ${LIMITS.contextBytes} bytes`);
  }

  const requestedFunctions = request.functions ?? installed.manifest.trace.callbacks;
  if (!Array.isArray(requestedFunctions) || requestedFunctions.length === 0) {
    return fail("request", "functions must be a non-empty array");
  }
  const functions: ComponentFunctionName[] = [];
  for (const name of requestedFunctions) {
    if (!installed.manifest.trace.callbacks.includes(name as ComponentFunctionName)) {
      return fail("request", `callback is not declared by plugin: ${String(name)}`);
    }
    if (!functions.includes(name as ComponentFunctionName)) {
      functions.push(name as ComponentFunctionName);
    }
  }

  const runs = request.runs ?? 1;
  if (!Number.isInteger(runs) || runs < 1 || runs > LIMITS.maxRuns) {
    return fail("request", `runs must be an integer from 1 to ${LIMITS.maxRuns}`);
  }

  return { objectJson, contextJson, functions, runs };
}

async function runCommand(
  command: string,
  args: string[],
  cwd: string,
  timeout: number,
): Promise<{ stdout: string; stderr: string }> {
  try {
    return await execFileAsync(command, args, {
      cwd,
      timeout,
      maxBuffer: 10 * 1024 * 1024,
    });
  } catch (error) {
    const err = error as NodeJS.ErrnoException & { stderr?: string; stdout?: string; killed?: boolean };
    const detail = err.killed
      ? `timed out after ${timeout}ms`
      : err.stderr || err.stdout || err.message || String(error);
    throw new Error(detail.trim());
  }
}

async function runTranspiledComponent(
  modulePath: string,
  objectJson: string,
  contextJson: string,
  functions: ComponentFunctionName[],
  runs: number,
  workDir: string,
): Promise<{
  instantiate_ms: number;
  run_ms: number;
  traces: Array<{ function: ComponentFunctionName; iteration: number; run_ms: number; resultString: string }>;
}> {
  const objectPath = path.join(workDir, "object.json");
  const contextPath = path.join(workDir, "context.json");
  const configPath = path.join(workDir, "runner-config.json");
  const runnerPath = path.join(workDir, "invoke-runner.mjs");
  await writeFile(objectPath, objectJson);
  await writeFile(contextPath, contextJson);
  await writeFile(configPath, JSON.stringify({ functions, runs }));
  await writeFile(
    runnerPath,
    `
import { readFile } from "node:fs/promises";
import { pathToFileURL } from "node:url";

const [modulePath, objectPath, contextPath, configPath] = process.argv.slice(2);
const objectJson = await readFile(objectPath, "utf8");
const contextJson = await readFile(contextPath, "utf8");
const config = JSON.parse(await readFile(configPath, "utf8"));

const startedImport = performance.now();
let mod;
try {
  mod = await import(pathToFileURL(modulePath).href + "?v=" + Date.now());
} catch (error) {
  console.log(JSON.stringify({ ok: false, phase: "instantiate", error: String(error?.stack || error) }));
  process.exit(0);
}
const instantiate_ms = performance.now() - startedImport;

const startedRun = performance.now();
try {
  const traces = [];
  for (let iteration = 1; iteration <= config.runs; iteration += 1) {
    for (const fnName of config.functions) {
      if (typeof mod[fnName] !== "function") {
        throw new Error("missing exported function: " + fnName);
      }
      const startedCall = performance.now();
      const resultString = mod[fnName](objectJson, contextJson);
      traces.push({
        function: fnName,
        iteration,
        run_ms: performance.now() - startedCall,
        resultString,
      });
    }
  }
  const run_ms = performance.now() - startedRun;
  console.log(JSON.stringify({ ok: true, instantiate_ms, run_ms, traces }));
} catch (error) {
  const run_ms = performance.now() - startedRun;
  console.log(JSON.stringify({ ok: false, phase: "run", run_ms, error: String(error?.stack || error) }));
}
`,
  );

  const { stdout } = await runCommand(
    process.execPath,
    [runnerPath, modulePath, objectPath, contextPath, configPath],
    workDir,
    LIMITS.runTimeoutMs,
  );
  const line = stdout.trim().split("\n").at(-1);
  if (!line) {
    throw new Error("component runner produced no output");
  }
  const payload = JSON.parse(line) as
    | {
        ok: true;
        instantiate_ms: number;
        run_ms: number;
        traces: Array<{ function: ComponentFunctionName; iteration: number; run_ms: number; resultString: string }>;
      }
    | { ok: false; phase: "instantiate" | "run"; run_ms?: number; error: string };
  if (!payload.ok) {
    const error = new Error(payload.error) as Error & { phase?: "instantiate" | "run"; run_ms?: number };
    error.phase = payload.phase;
    error.run_ms = payload.run_ms;
    throw error;
  }
  return payload;
}

function requireOptimizedBlake3(): InstallPluginFailure | undefined {
  if (process.env.NAPI_RS_FORCE_WASI) {
    return installFail("request", "BLAKE3 must use the native optimized binding; NAPI_RS_FORCE_WASI is set");
  }
  return undefined;
}

async function parseRunPayload(
  runPayload: {
    instantiate_ms: number;
    run_ms: number;
    traces: Array<{ function: ComponentFunctionName; iteration: number; run_ms: number; resultString: string }>;
  },
  timings: { compile_ms?: number; transpile_ms?: number; wasm_bytes?: number },
): Promise<FunctionRunTrace[] | CompileRunFailure> {
  const traces: FunctionRunTrace[] = [];
  for (const trace of runPayload.traces) {
    const resultBytes = byteLength(trace.resultString);
    if (resultBytes > LIMITS.resultBytes) {
      return fail("result", `result exceeds ${LIMITS.resultBytes} bytes`, {
        ...timings,
        instantiate_ms: roundMs(runPayload.instantiate_ms),
        run_ms: roundMs(runPayload.run_ms),
      });
    }

    let result: unknown;
    try {
      result = JSON.parse(trace.resultString);
    } catch (error) {
      return fail("result", `returned value from ${trace.function} run ${trace.iteration} is not JSON: ${String(error)}`, {
        ...timings,
        instantiate_ms: roundMs(runPayload.instantiate_ms),
        run_ms: roundMs(runPayload.run_ms),
      });
    }

    traces.push({
      function: trace.function,
      iteration: trace.iteration,
      run_ms: roundMs(trace.run_ms),
      result,
      result_bytes: resultBytes,
    });
  }
  return traces;
}

export async function installPlugin(request: InstallPluginRequest): Promise<InstallPluginResponse> {
  const blake3Check = requireOptimizedBlake3();
  if (blake3Check) {
    return blake3Check;
  }

  const checked = validateInstallRequest(request);
  if ("ok" in checked) {
    return checked;
  }

  const sourceBytes = byteLength(request.source);
  const manifestJson = JSON.stringify(request.manifest);
  const source_blake3 = blake3Hex(request.source);
  const manifest_blake3 = blake3Hex(manifestJson);
  const workDir = await mkdtemp(path.join(tmpdir(), "capsem-plugin-install-"));
  let compile_ms: number | undefined;
  let wasm_bytes: number | undefined;

  try {
    const sourcePath = path.join(workDir, "plugin.js");
    const witPath = path.join(workDir, "plugin.wit");
    const tempWasmPath = path.join(workDir, "plugin.wasm");
    await writeFile(sourcePath, request.source);
    await writeFile(witPath, WIT);

    try {
      const measured = await measure(() =>
        runCommand(JCO_BIN, ["componentize", sourcePath, "--wit", witPath, "-o", tempWasmPath], workDir, LIMITS.compileTimeoutMs),
      );
      compile_ms = roundMs(measured.ms);
    } catch (error) {
      return installFail("compile", String((error as Error).message || error), { compile_ms });
    }

    const wasmPayload = await readFile(tempWasmPath);
    const artifact_blake3 = blake3Hex(wasmPayload);
    wasm_bytes = wasmPayload.byteLength;
    const artifactName = `${artifact_blake3.slice("blake3:".length)}.wasm`;
    const artifactPath = path.join(ARTIFACTS_DIR, artifactName);
    const transpiledDir = path.join(TRANSPILED_DIR, artifact_blake3.slice("blake3:".length));

    await mkdir(ARTIFACTS_DIR, { recursive: true });
    await mkdir(TRANSPILED_DIR, { recursive: true });
    await copyFile(tempWasmPath, artifactPath);

    try {
      await rm(transpiledDir, { recursive: true, force: true });
      const measured = await measure(() =>
        runCommand(JCO_BIN, ["transpile", artifactPath, "--name", "plugin", "-o", transpiledDir], workDir, LIMITS.transpileTimeoutMs),
      );
      const transpile_ms = roundMs(measured.ms);
      await writeFile(path.join(transpiledDir, "package.json"), "{\"type\":\"module\"}\n");
      await symlink(path.join(ROOT_DIR, "node_modules"), path.join(transpiledDir, "node_modules"), "dir");

      const internalManifest: InternalPluginManifest = {
        id: request.manifest.id,
        name: request.manifest.name,
        version: request.manifest.version,
        artifact_path: artifactPath,
        artifact_blake3,
        source_blake3,
        manifest_blake3,
        abi: {
          wit_version: WIT_VERSION,
          exports: [...checked.callbacks],
        },
        compiler: {
          name: "jco/componentize-js",
          version: JCO_VERSION,
          flags: ["componentize", "--wit", WIT_VERSION],
        },
        capabilities: request.manifest.capabilities ?? [],
        trace: {
          callbacks: [...checked.callbacks],
          timings: true,
          result_bytes: true,
        },
        installed_at: new Date().toISOString(),
      };

      registry.set(internalManifest.id, {
        manifest: internalManifest,
        modulePath: path.join(transpiledDir, "plugin.js"),
        wasmBytes: wasm_bytes,
      });

      console.log(
        JSON.stringify({
          type: "plugin_install",
          ok: true,
          plugin_id: internalManifest.id,
          artifact_blake3,
          source_blake3,
          compile_ms,
          transpile_ms,
          wasm_bytes,
          source_bytes: sourceBytes,
        }),
      );

      return {
        ok: true,
        plugin_id: internalManifest.id,
        artifact_blake3,
        artifact_path: artifactPath,
        compile_ms,
        transpile_ms,
        wasm_bytes,
        manifest: internalManifest,
      };
    } catch (error) {
      return installFail("transpile", String((error as Error).message || error), {
        compile_ms,
        wasm_bytes,
      });
    }
  } finally {
    await rm(workDir, { recursive: true, force: true });
  }
}

export async function runPlugin(request: RunPluginRequest): Promise<RunPluginResponse> {
  const installed = registry.get(request.plugin_id);
  const checked = validateRunRequest(request, installed);
  if ("ok" in checked) {
    console.log(
      JSON.stringify({
        type: "plugin_run",
        ok: false,
        phase: checked.phase,
        plugin_id: request?.plugin_id,
        validation_status: "rejected",
      }),
    );
    return checked;
  }
  if (!installed) {
    return fail("request", `plugin is not installed: ${request.plugin_id}`);
  }

  const workDir = await mkdtemp(path.join(tmpdir(), "capsem-plugin-run-"));
  try {
    let runPayload: {
      instantiate_ms: number;
      run_ms: number;
      traces: Array<{ function: ComponentFunctionName; iteration: number; run_ms: number; resultString: string }>;
    };
    try {
      runPayload = await runTranspiledComponent(
        installed.modulePath,
        checked.objectJson,
        checked.contextJson,
        checked.functions,
        checked.runs,
        workDir,
      );
    } catch (error) {
      const err = error as Error & { phase?: "instantiate" | "run"; run_ms?: number };
      return fail(err.phase || "run", err.message, {
        wasm_bytes: installed.wasmBytes,
        run_ms: err.run_ms === undefined ? undefined : roundMs(err.run_ms),
      });
    }

    const traces = await parseRunPayload(runPayload, { wasm_bytes: installed.wasmBytes });
    if ("ok" in traces) {
      return traces;
    }
    const result = traces.at(-1)?.result;
    const response: RunPluginResponse = {
      ok: true,
      plugin_id: installed.manifest.id,
      artifact_blake3: installed.manifest.artifact_blake3,
      instantiate_ms: roundMs(runPayload.instantiate_ms),
      run_ms: roundMs(runPayload.run_ms),
      wasm_bytes: installed.wasmBytes,
      result,
      traces,
      functions: checked.functions,
      runs: checked.runs,
    };
    console.log(
      JSON.stringify({
        type: "plugin_run",
        ok: true,
        plugin_id: installed.manifest.id,
        artifact_blake3: installed.manifest.artifact_blake3,
        instantiate_ms: response.instantiate_ms,
        run_ms: response.run_ms,
        wasm_bytes: installed.wasmBytes,
        trace_count: traces.length,
      }),
    );
    return response;
  } finally {
    await rm(workDir, { recursive: true, force: true });
  }
}

export function installedPlugins(): InternalPluginManifest[] {
  return [...registry.values()].map((plugin) => plugin.manifest);
}

export async function compileRun(request: CompileRunRequest): Promise<CompileRunResponse> {
  const checked = validateRequest(request);
  if ("ok" in checked) {
    console.log(
      JSON.stringify({
        type: "compile_run",
        ok: false,
        phase: checked.phase,
        source_bytes: typeof request?.source === "string" ? byteLength(request.source) : 0,
        object_bytes: 0,
        context_bytes: 0,
        validation_status: "rejected",
      }),
    );
    return checked;
  }

  const sourceBytes = byteLength(request.source);
  const objectBytes = byteLength(checked.objectJson);
  const contextBytes = byteLength(checked.contextJson);
  const workDir = await mkdtemp(path.join(tmpdir(), "capsem-plugin-spike-"));
  let compile_ms: number | undefined;
  let transpile_ms: number | undefined;
  let wasm_bytes: number | undefined;
  let finalResponse: CompileRunResponse | undefined;
  const complete = <T extends CompileRunResponse>(response: T): T => {
    finalResponse = response;
    return response;
  };

  try {
    const sourcePath = path.join(workDir, "plugin.js");
    const witPath = path.join(workDir, "plugin.wit");
    const wasmPath = path.join(workDir, "plugin.wasm");
    const transpiledDir = path.join(workDir, "transpiled");

    await writeFile(sourcePath, request.source);
    await writeFile(witPath, WIT);

    try {
      const measured = await measure(() =>
        runCommand(
          JCO_BIN,
          ["componentize", sourcePath, "--wit", witPath, "-o", wasmPath],
          workDir,
          LIMITS.compileTimeoutMs,
        ),
      );
      compile_ms = roundMs(measured.ms);
    } catch (error) {
      return complete(fail("compile", String((error as Error).message || error), { compile_ms }));
    }

    wasm_bytes = (await stat(wasmPath)).size;

    try {
      const measured = await measure(() =>
        runCommand(JCO_BIN, ["transpile", wasmPath, "-o", transpiledDir], workDir, LIMITS.transpileTimeoutMs),
      );
      transpile_ms = roundMs(measured.ms);
    } catch (error) {
      return complete(
        fail("transpile", String((error as Error).message || error), {
          compile_ms,
          wasm_bytes,
        }),
      );
    }

    await writeFile(path.join(transpiledDir, "package.json"), "{\"type\":\"module\"}\n");
    await symlink(path.join(ROOT_DIR, "node_modules"), path.join(transpiledDir, "node_modules"), "dir");

    let runPayload: {
      instantiate_ms: number;
      run_ms: number;
      traces: Array<{ function: ComponentFunctionName; iteration: number; run_ms: number; resultString: string }>;
    };
    try {
      runPayload = await runTranspiledComponent(
        path.join(transpiledDir, "plugin.js"),
        checked.objectJson,
        checked.contextJson,
        checked.functions,
        checked.runs,
        workDir,
      );
    } catch (error) {
      const err = error as Error & { phase?: "instantiate" | "run"; run_ms?: number };
      return complete(
        fail(err.phase || "run", err.message, {
          compile_ms,
          transpile_ms,
          wasm_bytes,
          run_ms: err.run_ms === undefined ? undefined : roundMs(err.run_ms),
        }),
      );
    }

    const traces: FunctionRunTrace[] = [];
    for (const trace of runPayload.traces) {
      const resultBytes = byteLength(trace.resultString);
      if (resultBytes > LIMITS.resultBytes) {
        return complete(
          fail("result", `result exceeds ${LIMITS.resultBytes} bytes`, {
            compile_ms,
            transpile_ms,
            instantiate_ms: roundMs(runPayload.instantiate_ms),
            run_ms: roundMs(runPayload.run_ms),
            wasm_bytes,
          }),
        );
      }

      let result: unknown;
      try {
        result = JSON.parse(trace.resultString);
      } catch (error) {
        return complete(
          fail("result", `returned value from ${trace.function} run ${trace.iteration} is not JSON: ${String(error)}`, {
            compile_ms,
            transpile_ms,
            instantiate_ms: roundMs(runPayload.instantiate_ms),
            run_ms: roundMs(runPayload.run_ms),
            wasm_bytes,
          }),
        );
      }

      traces.push({
        function: trace.function,
        iteration: trace.iteration,
        run_ms: roundMs(trace.run_ms),
        result,
        result_bytes: resultBytes,
      });
    }

    const result = traces.at(-1)?.result;

    return complete({
      ok: true,
      compile_ms,
      transpile_ms,
      instantiate_ms: roundMs(runPayload.instantiate_ms),
      run_ms: roundMs(runPayload.run_ms),
      wasm_bytes,
      result,
      traces,
      functions: checked.functions,
      runs: checked.runs,
    });
  } finally {
    const responseLog: InvocationLog = {
      ok: finalResponse?.ok ?? false,
      phase: finalResponse?.ok ? undefined : finalResponse?.phase,
      compile_ms,
      transpile_ms,
      instantiate_ms: finalResponse?.instantiate_ms,
      run_ms: finalResponse?.run_ms,
      wasm_bytes,
      source_bytes: sourceBytes,
      object_bytes: objectBytes,
      context_bytes: contextBytes,
      result_bytes:
        finalResponse?.ok
          ? finalResponse.traces.reduce((total, trace) => total + trace.result_bytes, 0)
          : undefined,
      functions: checked.functions,
      runs: checked.runs,
      trace_count: finalResponse?.ok ? finalResponse.traces.length : undefined,
    };
    console.log(JSON.stringify({ type: "compile_run", ...responseLog }));
    await rm(workDir, { recursive: true, force: true });
  }
}
