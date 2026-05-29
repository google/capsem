use std::{
    env,
    path::PathBuf,
    time::{Instant, SystemTime, UNIX_EPOCH},
};

use capsem_plugin_engine::{
    CallbackName, ExtensionContributions, InstallPluginRequest, PluginRegistry,
    PublicPluginManifest, RunPluginRequest,
};
use serde::Serialize;
use serde_json::json;

const SOURCE: &str = r#"
    (module
      (import "capsem" "emit_output" (func $emit_output (param i32 i32) (result i32)))
      (memory (export "memory") 1)
      (data (i32.const 4096) "{\"kind\":\"ModelOutput\",\"decision\":{\"verdict\":\"allow\",\"reasons\":[]},\"patches\":[],\"findings\":[]}")
      (func (export "onModelOutput") (param i32 i32 i32 i32) (result i32)
        i32.const 4096
        i32.const 93
        call $emit_output
        drop
        i32.const 17))
"#;

#[derive(Debug, Serialize)]
struct BenchmarkReport {
    mode: String,
    repeats: u32,
    batch_runs: u32,
    callbacks: u32,
    install_compile_ms: f64,
    install_load_ms: f64,
    artifact_bytes: u64,
    warm_wall_ms: f64,
    callbacks_per_second: f64,
    run_request_avg_ms: f64,
    callback_avg_ms: f64,
    callback_p50_ms: f64,
    callback_p95_ms: f64,
    callback_max_ms: f64,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let repeats = env_u32("CAPSEM_BENCH_REPEATS", 200);
    let batch_runs = env_u32("CAPSEM_BENCH_BATCH_RUNS", 25);
    let mode = env::var("CAPSEM_BENCH_ENGINE").unwrap_or_else(|_| "wasmtime-wat".to_owned());
    let artifact_dir = artifact_dir();
    let registry = match mode.as_str() {
        "wasmtime-wat-fuel" => PluginRegistry::wasmtime_wat_fuel(artifact_dir),
        "wasmtime-wat" => PluginRegistry::wasmtime_wat(artifact_dir),
        other => return Err(format!("unknown CAPSEM_BENCH_ENGINE: {other}").into()),
    };

    let install = registry.install(InstallPluginRequest {
        source: SOURCE.to_owned(),
        manifest: PublicPluginManifest {
            id: "bench.wasmtime_wat".to_owned(),
            name: "Benchmark WASM".to_owned(),
            version: "0.1.0".to_owned(),
            callbacks: vec![CallbackName::OnModelOutput],
            capabilities: Vec::new(),
            contributes: ExtensionContributions::default(),
        },
    })?;

    let object = json!({
        "kind": "ModelOutput",
        "decision": { "verdict": "allow", "reasons": [] },
        "patches": [],
        "findings": [],
        "subject": { "content": { "text": "benchmark" } }
    });
    let context = json!({
        "trace": {
            "labels": ["benchmark"],
            "snapshot": { "mcp_tools": ["fetch", "shell"], "skills": ["audit"] }
        }
    });

    registry.run(RunPluginRequest {
        plugin_id: install.plugin_id.clone(),
        object: object.clone(),
        context: context.clone(),
        functions: vec![CallbackName::OnModelOutput],
        runs: 1,
    })?;

    let wall_start = Instant::now();
    let mut request_ms = Vec::new();
    let mut callback_ms = Vec::new();
    for _ in 0..repeats {
        let run = registry.run(RunPluginRequest {
            plugin_id: install.plugin_id.clone(),
            object: object.clone(),
            context: context.clone(),
            functions: vec![CallbackName::OnModelOutput],
            runs: batch_runs,
        })?;
        request_ms.push(run.run_ms);
        callback_ms.extend(run.traces.into_iter().map(|trace| trace.run_ms));
    }
    let warm_wall_ms = wall_start.elapsed().as_secs_f64() * 1000.0;
    callback_ms.sort_by(f64::total_cmp);
    request_ms.sort_by(f64::total_cmp);

    let callbacks = repeats * batch_runs;
    let report = BenchmarkReport {
        mode,
        repeats,
        batch_runs,
        callbacks,
        install_compile_ms: install.compile_ms,
        install_load_ms: install.load_ms,
        artifact_bytes: install.runtime_bytes,
        warm_wall_ms,
        callbacks_per_second: callbacks as f64 / (warm_wall_ms / 1000.0),
        run_request_avg_ms: avg(&request_ms),
        callback_avg_ms: avg(&callback_ms),
        callback_p50_ms: percentile(&callback_ms, 0.50),
        callback_p95_ms: percentile(&callback_ms, 0.95),
        callback_max_ms: callback_ms.last().copied().unwrap_or_default(),
    };

    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}

fn env_u32(name: &str, default: u32) -> u32 {
    env::var(name)
        .ok()
        .and_then(|value| value.parse().ok())
        .filter(|value| (1..=10_000).contains(value))
        .unwrap_or(default)
}

fn artifact_dir() -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or_default();
    env::temp_dir().join(format!("capsem-plugin-bench-{nonce}"))
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
