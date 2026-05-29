export type CompileRunPhase =
  | "request"
  | "compile"
  | "transpile"
  | "instantiate"
  | "run"
  | "result"
  | "cleanup";

export type ComponentFunctionName = "invoke" | "inspect";

export interface CompileRunRequest {
  source: string;
  object: unknown;
  context: unknown;
  functions?: ComponentFunctionName[];
  runs?: number;
}

export interface PublicPluginManifest {
  id: string;
  name: string;
  version: string;
  callbacks: ComponentFunctionName[];
  capabilities?: string[];
  contributes?: {
    rules?: string[];
    tools?: string[];
    skills?: string[];
    ui?: string[];
  };
}

export interface InstallPluginRequest {
  source: string;
  manifest: PublicPluginManifest;
}

export interface InternalPluginManifest {
  id: string;
  name: string;
  version: string;
  artifact_path: string;
  artifact_blake3: string;
  source_blake3: string;
  manifest_blake3: string;
  abi: {
    wit_version: string;
    exports: ComponentFunctionName[];
  };
  compiler: {
    name: string;
    version: string;
    flags: string[];
  };
  capabilities: string[];
  contributes?: {
    rules?: string[];
    tools?: string[];
    skills?: string[];
    ui?: string[];
  };
  trace: {
    callbacks: ComponentFunctionName[];
    timings: boolean;
    result_bytes: boolean;
  };
  installed_at: string;
}

export interface InstallPluginSuccess {
  ok: true;
  plugin_id: string;
  artifact_blake3: string;
  artifact_path: string;
  compile_ms: number;
  transpile_ms: number;
  wasm_bytes: number;
  manifest: InternalPluginManifest;
}

export interface InstallPluginFailure {
  ok: false;
  phase: "request" | "compile" | "transpile";
  error: string;
  compile_ms?: number;
  wasm_bytes?: number;
}

export type InstallPluginResponse = InstallPluginSuccess | InstallPluginFailure;

export interface RunPluginRequest {
  plugin_id: string;
  object: unknown;
  context: unknown;
  functions?: ComponentFunctionName[];
  runs?: number;
}

export interface FunctionRunTrace {
  function: ComponentFunctionName;
  iteration: number;
  run_ms: number;
  result: unknown;
  result_bytes: number;
}

export interface CompileRunSuccess {
  ok: true;
  plugin_id?: string;
  artifact_blake3?: string;
  compile_ms: number;
  transpile_ms: number;
  instantiate_ms: number;
  run_ms: number;
  wasm_bytes: number;
  result: unknown;
  traces: FunctionRunTrace[];
  functions: ComponentFunctionName[];
  runs: number;
}

export interface CompileRunFailure {
  ok: false;
  phase: CompileRunPhase;
  error: string;
  compile_ms?: number;
  transpile_ms?: number;
  instantiate_ms?: number;
  run_ms?: number;
  wasm_bytes?: number;
}

export type CompileRunResponse = CompileRunSuccess | CompileRunFailure;

export type RunPluginResponse =
  | (Omit<CompileRunSuccess, "compile_ms" | "transpile_ms" | "wasm_bytes"> & {
      plugin_id: string;
      artifact_blake3: string;
      wasm_bytes: number;
    })
  | CompileRunFailure;

export interface InvocationLog {
  ok: boolean;
  phase?: CompileRunPhase;
  compile_ms?: number;
  transpile_ms?: number;
  instantiate_ms?: number;
  run_ms?: number;
  wasm_bytes?: number;
  source_bytes: number;
  object_bytes: number;
  context_bytes: number;
  result_bytes?: number;
  functions?: ComponentFunctionName[];
  runs?: number;
  trace_count?: number;
}
