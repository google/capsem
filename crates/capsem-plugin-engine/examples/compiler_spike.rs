use std::{
    env, fs,
    path::{Path, PathBuf},
    process::Command,
    sync::{Arc, Mutex},
    time::{Instant, SystemTime, UNIX_EPOCH},
};

use capsem_plugin_engine::{
    CallbackName, ExtensionContributions, InstallPluginRequest, PluginError, PluginRegistry,
    PublicPluginManifest, RunPluginRequest, RuntimeCompiler, RuntimeCompilerInput,
    RuntimeCompilerOutput, WasmtimePluginExecutor,
};
use serde::Serialize;
use serde_json::json;

const MINIMAL_ASSEMBLYSCRIPT_SOURCE: &str = r#"@external("capsem", "emit_output")
declare function emitOutput(ptr: usize, len: i32): i32;

const OUTPUT = String.UTF8.encode('{"kind":"ModelOutput","decision":{"verdict":"allow","reasons":[]},"patches":[],"findings":[]}', false);

export function onModelOutput(objectPtr: i32, objectLen: i32, contextPtr: i32, contextLen: i32): i32 {
  if (objectPtr < 0 || objectLen < 0 || contextPtr < 0 || contextLen < 0) {
    return -1;
  }
  emitOutput(changetype<usize>(OUTPUT), OUTPUT.byteLength);
  return 41;
}
"#;

const TWO_CALLBACK_ASSEMBLYSCRIPT_SOURCE: &str = r#"@external("capsem", "emit_output")
declare function emitOutput(ptr: usize, len: i32): i32;

const MODEL_OUTPUT = String.UTF8.encode('{"kind":"ModelOutput","decision":{"verdict":"allow","reasons":[]},"patches":[],"findings":[]}', false);
const HTTP_OUTPUT = String.UTF8.encode('{"kind":"HttpRequest","decision":{"verdict":"allow","reasons":[]},"patches":[],"findings":[]}', false);

function emitStatic(payload: ArrayBuffer, status: i32): i32 {
  emitOutput(changetype<usize>(payload), payload.byteLength);
  return status;
}

export function onModelOutput(objectPtr: i32, objectLen: i32, contextPtr: i32, contextLen: i32): i32 {
  if (objectPtr < 0 || objectLen < 0 || contextPtr < 0 || contextLen < 0) {
    return -1;
  }
  return emitStatic(MODEL_OUTPUT, 41);
}

export function onHttpRequest(objectPtr: i32, objectLen: i32, contextPtr: i32, contextLen: i32): i32 {
  if (objectPtr < 0 || objectLen < 0 || contextPtr < 0 || contextLen < 0) {
    return -1;
  }
  return emitStatic(HTTP_OUTPUT, 42);
}
"#;

const SDKISH_ASSEMBLYSCRIPT_SOURCE: &str = r#"@external("capsem", "emit_output")
declare function emitOutput(ptr: usize, len: i32): i32;

const MODEL_OUTPUT = String.UTF8.encode('{"kind":"ModelOutput","decision":{"verdict":"review","reasons":["sdk.redaction"]},"patches":[{"op":"replace","path":"/subject/content/text","value":"[REDACTED]"}],"findings":[{"kind":"secret","severity":"high","message":"credential-like text found"}]}', false);
const HTTP_BLOCK = String.UTF8.encode('{"kind":"HttpRequest","decision":{"verdict":"block","reasons":["sdk.egress"]},"patches":[],"findings":[{"kind":"egress","severity":"high","message":"blocked external request after sensitive context"}]}', false);
const TOOL_REVIEW = String.UTF8.encode('{"kind":"McpToolCall","decision":{"verdict":"review","reasons":["sdk.tool_budget"]},"patches":[],"findings":[{"kind":"tool","severity":"medium","message":"tool call requires review"}]}', false);
const VM_READY = String.UTF8.encode('{"kind":"VmReady","decision":{"verdict":"allow","reasons":[]},"patches":[],"findings":[]}', false);

function isInvalid(objectPtr: i32, objectLen: i32, contextPtr: i32, contextLen: i32): bool {
  return objectPtr < 0 || objectLen < 0 || contextPtr < 0 || contextLen < 0;
}

function emitStatic(payload: ArrayBuffer, status: i32): i32 {
  emitOutput(changetype<usize>(payload), payload.byteLength);
  return status;
}

export function onModelOutput(objectPtr: i32, objectLen: i32, contextPtr: i32, contextLen: i32): i32 {
  if (isInvalid(objectPtr, objectLen, contextPtr, contextLen)) {
    return -1;
  }
  return emitStatic(MODEL_OUTPUT, 51);
}

export function onHttpRequest(objectPtr: i32, objectLen: i32, contextPtr: i32, contextLen: i32): i32 {
  if (isInvalid(objectPtr, objectLen, contextPtr, contextLen)) {
    return -1;
  }
  return emitStatic(HTTP_BLOCK, 52);
}

export function onMcpToolCall(objectPtr: i32, objectLen: i32, contextPtr: i32, contextLen: i32): i32 {
  if (isInvalid(objectPtr, objectLen, contextPtr, contextLen)) {
    return -1;
  }
  return emitStatic(TOOL_REVIEW, 53);
}

export function onVmReady(objectPtr: i32, objectLen: i32, contextPtr: i32, contextLen: i32): i32 {
  if (isInvalid(objectPtr, objectLen, contextPtr, contextLen)) {
    return -1;
  }
  return emitStatic(VM_READY, 54);
}
"#;

#[derive(Clone, Debug, Default, Serialize)]
struct PhaseTimings {
    asc_ms: f64,
    total_ms: f64,
}

#[derive(Debug, Serialize)]
struct CompilerSpikeReport {
    repeats: u32,
    cases: Vec<CompilerCaseReport>,
}

#[derive(Debug, Serialize)]
struct CompilerCaseReport {
    case: &'static str,
    callbacks: Vec<CallbackName>,
    source_bytes: usize,
    artifact_bytes: usize,
    compile_ms_avg: f64,
    compile_ms_p50: f64,
    compile_ms_min: f64,
    compile_ms_max: f64,
    install_load_ms_avg: f64,
    smoke_run_ms_avg: f64,
    wasm_statuses: Vec<Option<i64>>,
    note: &'static str,
}

struct AssemblyScriptCompiler {
    root: PathBuf,
    timings: Arc<Mutex<Option<PhaseTimings>>>,
}

impl AssemblyScriptCompiler {
    fn new(root: PathBuf) -> Self {
        Self {
            root,
            timings: Arc::new(Mutex::new(None)),
        }
    }

    fn timings(&self) -> PhaseTimings {
        self.timings
            .lock()
            .expect("timings lock")
            .clone()
            .unwrap_or_default()
    }
}

impl RuntimeCompiler for AssemblyScriptCompiler {
    fn compile(
        &self,
        input: RuntimeCompilerInput<'_>,
    ) -> Result<RuntimeCompilerOutput, PluginError> {
        let total = Instant::now();
        let work_dir = temp_dir("capsem-assemblyscript");
        fs::create_dir_all(&work_dir)?;

        let result = (|| {
            let source_path = work_dir.join("plugin.ts");
            let wasm_path = work_dir.join("plugin.wasm");
            fs::write(&source_path, input.source)?;

            let asc_started = Instant::now();
            run_command(
                self.root.join("node_modules/.bin/asc"),
                [
                    source_path.as_os_str(),
                    "-o".as_ref(),
                    wasm_path.as_os_str(),
                    "-O3".as_ref(),
                    "--runtime".as_ref(),
                    "stub".as_ref(),
                    "--initialMemory".as_ref(),
                    "1".as_ref(),
                    "--maximumMemory".as_ref(),
                    "32".as_ref(),
                    "--noAssert".as_ref(),
                ],
                &work_dir,
            )?;
            let asc_ms = elapsed_ms(asc_started);
            let bytes = fs::read(wasm_path)?;
            let timings = PhaseTimings {
                asc_ms,
                total_ms: elapsed_ms(total),
                ..PhaseTimings::default()
            };
            *self.timings.lock().expect("timings lock") = Some(timings);

            Ok(RuntimeCompilerOutput {
                kind: "wasmtime-core-module".to_owned(),
                bytes,
            })
        })();

        let cleanup = fs::remove_dir_all(&work_dir);
        if result.is_ok() {
            cleanup?;
        }
        result
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let root = env::current_dir()?;
    let repeats = env_u32("CAPSEM_COMPILER_BENCH_REPEATS", 5);
    let cases = [
        CompilerCase {
            name: "minimal_model_output",
            source: MINIMAL_ASSEMBLYSCRIPT_SOURCE,
            callbacks: vec![CallbackName::OnModelOutput],
            note: "Smallest production-shaped core-module plugin.",
        },
        CompilerCase {
            name: "two_callbacks_shared_helpers",
            source: TWO_CALLBACK_ASSEMBLYSCRIPT_SOURCE,
            callbacks: vec![CallbackName::OnModelOutput, CallbackName::OnHttpRequest],
            note: "Two exported callbacks sharing helper code and static outputs.",
        },
        CompilerCase {
            name: "sdkish_security_plugin",
            source: SDKISH_ASSEMBLYSCRIPT_SOURCE,
            callbacks: vec![
                CallbackName::OnModelOutput,
                CallbackName::OnHttpRequest,
                CallbackName::OnMcpToolCall,
                CallbackName::OnVmReady,
            ],
            note: "Representative generated-SDK shape with several callbacks, decisions, findings, and patches.",
        },
    ];

    let report = CompilerSpikeReport {
        repeats,
        cases: cases
            .into_iter()
            .map(|case| measure_case(&root, repeats, case))
            .collect::<Result<Vec<_>, _>>()?,
    };

    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}

struct CompilerCase {
    name: &'static str,
    source: &'static str,
    callbacks: Vec<CallbackName>,
    note: &'static str,
}

fn measure_case(
    root: &Path,
    repeats: u32,
    case: CompilerCase,
) -> Result<CompilerCaseReport, Box<dyn std::error::Error>> {
    let mut compile_ms = Vec::new();
    let mut load_ms = Vec::new();
    let mut run_ms = Vec::new();
    let mut wasm_statuses = Vec::new();
    let mut artifact_bytes = 0;

    for iteration in 0..repeats {
        let compiler = Arc::new(AssemblyScriptCompiler::new(root.to_path_buf()));
        let registry = PluginRegistry::with_engine(
            temp_dir("capsem-assemblyscript-artifacts"),
            compiler.clone(),
            Arc::new(WasmtimePluginExecutor::with_budget(
                10_000_000,
                2 * 1024 * 1024,
            )),
        );
        let install = registry.install(InstallPluginRequest {
            source: case.source.to_owned(),
            manifest: PublicPluginManifest {
                id: format!("compiler.{}.{}", case.name, iteration),
                name: format!("Compiler {}", case.name),
                version: "0.1.0".to_owned(),
                callbacks: case.callbacks.clone(),
                capabilities: Vec::new(),
                contributes: ExtensionContributions::default(),
            },
        })?;
        artifact_bytes = install.runtime_bytes as usize;
        compile_ms.push(compiler.timings().total_ms);
        load_ms.push(install.load_ms);

        let run = registry.run(RunPluginRequest {
            plugin_id: install.plugin_id.clone(),
            object: json!({
                "kind": "ModelOutput",
                "decision": { "verdict": "allow", "reasons": [] },
                "patches": [],
                "findings": [],
                "subject": { "content": { "text": "compiler benchmark" } }
            }),
            context: json!({
                "trace": {
                    "labels": ["compiler-spike", case.name],
                    "snapshot": {
                        "mcp_tools": ["fetch", "shell", "browser"],
                        "skills": ["audit", "review"]
                    }
                }
            }),
            functions: case.callbacks.clone(),
            runs: 1,
        })?;
        run_ms.push(run.run_ms);
        wasm_statuses.push(run.result["wasm_status"].as_i64());
    }

    compile_ms.sort_by(f64::total_cmp);
    load_ms.sort_by(f64::total_cmp);
    run_ms.sort_by(f64::total_cmp);

    Ok(CompilerCaseReport {
        case: case.name,
        callbacks: case.callbacks,
        source_bytes: case.source.len(),
        artifact_bytes,
        compile_ms_avg: avg(&compile_ms),
        compile_ms_p50: percentile(&compile_ms, 0.50),
        compile_ms_min: compile_ms.first().copied().unwrap_or_default(),
        compile_ms_max: compile_ms.last().copied().unwrap_or_default(),
        install_load_ms_avg: avg(&load_ms),
        smoke_run_ms_avg: avg(&run_ms),
        wasm_statuses,
        note: case.note,
    })
}

fn run_command<I, S>(program: PathBuf, args: I, cwd: &Path) -> Result<(), PluginError>
where
    I: IntoIterator<Item = S>,
    S: AsRef<std::ffi::OsStr>,
{
    let output = Command::new(&program)
        .args(args)
        .current_dir(cwd)
        .output()
        .map_err(|error| {
            PluginError::Runtime(format!("failed to run {}: {error}", program.display()))
        })?;

    if output.status.success() {
        return Ok(());
    }

    Err(PluginError::Runtime(format!(
        "{} failed with status {}\nstdout:\n{}\nstderr:\n{}",
        program.display(),
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )))
}

fn temp_dir(prefix: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or_default();
    env::temp_dir().join(format!("{prefix}-{nonce}-{}", std::process::id()))
}

fn elapsed_ms(started: Instant) -> f64 {
    started.elapsed().as_secs_f64() * 1000.0
}

fn env_u32(name: &str, default: u32) -> u32 {
    env::var(name)
        .ok()
        .and_then(|value| value.parse().ok())
        .filter(|value| (1..=20).contains(value))
        .unwrap_or(default)
}

fn avg(values: &[f64]) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    values.iter().sum::<f64>() / values.len() as f64
}

fn percentile(values: &[f64], p: f64) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    let index = ((values.len() - 1) as f64 * p).round() as usize;
    values[index]
}
