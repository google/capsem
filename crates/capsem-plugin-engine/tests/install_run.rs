use capsem_plugin_engine::{
    CallbackName, ExtensionContributions, ExtensionUnitKind, InstallPluginRequest,
    InstallRunRequest, LoadedPluginRuntime, PluginCallInput, PluginError, PluginExecutor,
    PluginLoadInput, PluginRegistry, PublicPluginManifest, RunPluginRequest, RuntimeCompiler,
    RuntimeCompilerInput, RuntimeCompilerOutput,
};
use serde_json::json;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

#[test]
fn install_writes_runtime_artifact_and_run_loads_it() {
    let temp = tempfile::tempdir().expect("tempdir");
    let registry = PluginRegistry::new(temp.path().join("artifacts"));

    let install = registry
        .install(InstallPluginRequest {
            source: "export function invoke() {}".to_owned(),
            manifest: PublicPluginManifest {
                id: "demo.security".to_owned(),
                name: "Demo Security".to_owned(),
                version: "0.1.0".to_owned(),
                callbacks: vec![CallbackName::OnModelOutput, CallbackName::OnHttpRequest],
                capabilities: Vec::new(),
                contributes: ExtensionContributions {
                    rules: vec!["rules/pii.json".to_owned()],
                    tools: vec!["tools/redact.json".to_owned()],
                    skills: vec!["skills/pii-review/SKILL.md".to_owned()],
                    ui: vec!["ui/panel.json".to_owned()],
                },
            },
        })
        .expect("install succeeds");

    assert!(install.runtime_blake3.starts_with("blake3:"));
    assert_eq!(install.runtime_blake3.len(), "blake3:".len() + 64);
    assert_eq!(
        install
            .runtime_artifact_path
            .file_name()
            .expect("runtime filename")
            .to_string_lossy(),
        format!(
            "{}.cwasm",
            install.runtime_blake3.trim_start_matches("blake3:")
        )
    );
    assert!(install.runtime_artifact_path.exists());
    assert_eq!(
        install.runtime_kind,
        "wasmtime-precompiled-component-placeholder"
    );
    assert!(install.load_ms >= 0.0);
    assert!(install.manifest.install_load_ms >= 0.0);
    assert_eq!(install.manifest.contributes.rules, ["rules/pii.json"]);
    assert_eq!(install.manifest.contributes.tools, ["tools/redact.json"]);
    assert_eq!(
        install.manifest.contributes.skills,
        ["skills/pii-review/SKILL.md"]
    );
    assert_eq!(install.manifest.contributes.ui, ["ui/panel.json"]);
    assert_eq!(
        install.manifest.unit_kinds,
        [
            ExtensionUnitKind::Plugin,
            ExtensionUnitKind::Rule,
            ExtensionUnitKind::Tool,
            ExtensionUnitKind::Skill,
            ExtensionUnitKind::Ui,
        ]
    );

    let run = registry
        .run(RunPluginRequest {
            plugin_id: "demo.security".to_owned(),
            object: json!({
                "kind": "ModelOutput",
                "subject": { "content": { "text": "hi" } },
                "decision": { "verdict": "allow", "reasons": [] },
                "patches": [],
                "findings": []
            }),
            context: json!({ "trace": { "labels": ["manual"] } }),
            functions: vec![CallbackName::OnModelOutput, CallbackName::OnHttpRequest],
            runs: 2,
        })
        .expect("run succeeds");

    assert_eq!(run.plugin_id, "demo.security");
    assert_eq!(run.runtime_blake3, install.runtime_blake3);
    assert_eq!(run.traces.len(), 4);
    assert!(run.load_ms >= 0.0);
    assert!(run.run_ms >= 0.0);
    assert!(run.input_object_blake3.starts_with("blake3:"));
    assert!(run.context_blake3.starts_with("blake3:"));
    assert!(run.output_object_blake3.starts_with("blake3:"));
    assert_eq!(run.result["called"], "onHttpRequest");
    assert_eq!(run.traces[0].decision["verdict"], "allow");
}

#[test]
fn install_rejects_unsafe_contribution_paths() {
    let temp = tempfile::tempdir().expect("tempdir");
    let registry = PluginRegistry::new(temp.path().join("artifacts"));

    let error = registry
        .install(InstallPluginRequest {
            source: "export function invoke() {}".to_owned(),
            manifest: PublicPluginManifest {
                id: "demo.security".to_owned(),
                name: "Demo Security".to_owned(),
                version: "0.1.0".to_owned(),
                callbacks: vec![CallbackName::OnModelOutput],
                capabilities: Vec::new(),
                contributes: ExtensionContributions {
                    skills: vec!["../escape/SKILL.md".to_owned()],
                    ..ExtensionContributions::default()
                },
            },
        })
        .expect_err("unsafe contribution path should fail");

    assert!(error.to_string().contains("contributes.skills"));
}

#[test]
fn run_rejects_unknown_plugin() {
    let temp = tempfile::tempdir().expect("tempdir");
    let registry = PluginRegistry::new(temp.path().join("artifacts"));

    let error = registry
        .run(RunPluginRequest {
            plugin_id: "missing".to_owned(),
            object: json!({}),
            context: json!({}),
            functions: vec![CallbackName::OnModelOutput],
            runs: 1,
        })
        .expect_err("missing plugin should fail");

    assert!(error.to_string().contains("plugin is not installed"));
}

#[test]
fn registry_uses_swappable_compiler_and_executor_traits() {
    struct CountingCompiler {
        calls: Arc<AtomicUsize>,
    }

    impl RuntimeCompiler for CountingCompiler {
        fn compile(
            &self,
            input: RuntimeCompilerInput<'_>,
        ) -> Result<RuntimeCompilerOutput, PluginError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(RuntimeCompilerOutput {
                kind: "test-runtime".to_owned(),
                bytes: format!("{}:{}", input.source_blake3, input.manifest_blake3).into_bytes(),
            })
        }
    }

    struct CountingExecutor {
        loads: Arc<AtomicUsize>,
        calls: Arc<AtomicUsize>,
    }

    impl PluginExecutor for CountingExecutor {
        fn load(
            &self,
            _input: PluginLoadInput<'_>,
        ) -> Result<Arc<dyn LoadedPluginRuntime>, PluginError> {
            self.loads.fetch_add(1, Ordering::SeqCst);
            Ok(Arc::new(CountingLoadedRuntime {
                calls: Arc::clone(&self.calls),
            }))
        }
    }

    struct CountingLoadedRuntime {
        calls: Arc<AtomicUsize>,
    }

    impl LoadedPluginRuntime for CountingLoadedRuntime {
        fn call(&self, input: PluginCallInput<'_>) -> Result<serde_json::Value, PluginError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(json!({
                "kind": "ModelOutput",
                "called": input.function.to_string(),
                "plugin_id": input.manifest.id,
                "decision": { "verdict": "allow", "reasons": [] },
                "patches": [],
                "findings": []
            }))
        }
    }

    let temp = tempfile::tempdir().expect("tempdir");
    let compiler_calls = Arc::new(AtomicUsize::new(0));
    let executor_loads = Arc::new(AtomicUsize::new(0));
    let executor_calls = Arc::new(AtomicUsize::new(0));
    let registry = PluginRegistry::with_engine(
        temp.path().join("artifacts"),
        Arc::new(CountingCompiler {
            calls: Arc::clone(&compiler_calls),
        }),
        Arc::new(CountingExecutor {
            loads: Arc::clone(&executor_loads),
            calls: Arc::clone(&executor_calls),
        }),
    );

    let install = registry
        .install(InstallPluginRequest {
            source: "export default Plugin({})".to_owned(),
            manifest: PublicPluginManifest {
                id: "demo.traits".to_owned(),
                name: "Demo Traits".to_owned(),
                version: "0.1.0".to_owned(),
                callbacks: vec![CallbackName::OnModelOutput],
                capabilities: Vec::new(),
                contributes: ExtensionContributions::default(),
            },
        })
        .expect("install succeeds");

    assert_eq!(compiler_calls.load(Ordering::SeqCst), 1);
    assert_eq!(executor_loads.load(Ordering::SeqCst), 1);
    assert_eq!(install.runtime_kind, "test-runtime");

    let run = registry
        .run(RunPluginRequest {
            plugin_id: "demo.traits".to_owned(),
            object: json!({ "kind": "ModelOutput" }),
            context: json!({ "trace": { "labels": [] } }),
            functions: vec![CallbackName::OnModelOutput],
            runs: 3,
        })
        .expect("run succeeds");

    assert_eq!(executor_calls.load(Ordering::SeqCst), 3);
    assert_eq!(executor_loads.load(Ordering::SeqCst), 1);
    assert_eq!(run.traces.len(), 3);
    assert_eq!(run.result["plugin_id"], "demo.traits");
}

#[test]
fn wasmtime_wat_engine_loads_and_executes_wasm_callback() {
    let temp = tempfile::tempdir().expect("tempdir");
    let registry = PluginRegistry::wasmtime_wat(temp.path().join("artifacts"));

    let install = registry
        .install(InstallPluginRequest {
            source: r#"
                (module
                  (func (export "onModelOutput") (result i32)
                    i32.const 7))
            "#
            .to_owned(),
            manifest: PublicPluginManifest {
                id: "demo.wasm".to_owned(),
                name: "Demo WASM".to_owned(),
                version: "0.1.0".to_owned(),
                callbacks: vec![CallbackName::OnModelOutput],
                capabilities: Vec::new(),
                contributes: ExtensionContributions::default(),
            },
        })
        .expect("install succeeds");

    assert_eq!(install.runtime_kind, "wasmtime-core-module");

    let run = registry
        .run(RunPluginRequest {
            plugin_id: "demo.wasm".to_owned(),
            object: json!({ "kind": "ModelOutput" }),
            context: json!({ "trace": { "labels": [] } }),
            functions: vec![CallbackName::OnModelOutput],
            runs: 2,
        })
        .expect("run succeeds");

    assert_eq!(run.traces.len(), 2);
    assert!(run.load_ms >= 0.0);
    assert!(install.load_ms >= 0.0);
    assert_eq!(run.result["called"], "onModelOutput");
    assert_eq!(run.result["plugin_id"], "demo.wasm");
    assert_eq!(run.result["wasm_status"], 7);
}

#[test]
fn wasmtime_wat_abi_passes_object_context_and_emits_output() {
    let temp = tempfile::tempdir().expect("tempdir");
    let registry = PluginRegistry::wasmtime_wat(temp.path().join("artifacts"));

    registry
        .install(InstallPluginRequest {
            source: r#"
                (module
                  (import "capsem" "emit_output" (func $emit_output (param i32 i32) (result i32)))
                  (memory (export "memory") 1)
                  (data (i32.const 4096) "{\22kind\22:\22ModelOutput\22,\22decision\22:{\22verdict\22:\22block\22,\22reasons\22:[\22abi\22]},\22patches\22:[{}],\22findings\22:[{}]}")
                  (func (export "onModelOutput") (param i32 i32 i32 i32) (result i32)
                    i32.const 4096
                    i32.const 102
                    call $emit_output
                    drop
                    i32.const 23))
            "#
            .to_owned(),
            manifest: PublicPluginManifest {
                id: "demo.abi".to_owned(),
                name: "Demo ABI".to_owned(),
                version: "0.1.0".to_owned(),
                callbacks: vec![CallbackName::OnModelOutput],
                capabilities: Vec::new(),
                contributes: ExtensionContributions::default(),
            },
        })
        .expect("install succeeds");

    let run = registry
        .run(RunPluginRequest {
            plugin_id: "demo.abi".to_owned(),
            object: json!({
                "kind": "ModelOutput",
                "decision": { "verdict": "allow", "reasons": [] },
                "patches": [],
                "findings": []
            }),
            context: json!({ "trace": { "labels": ["abi"] } }),
            functions: vec![CallbackName::OnModelOutput],
            runs: 1,
        })
        .expect("run succeeds");

    assert_eq!(run.result["wasm_status"], 23);
    assert_eq!(run.result["called"], "onModelOutput");
    assert_eq!(run.result["plugin_id"], "demo.abi");
    assert_eq!(run.result["decision"]["verdict"], "block");
    assert_eq!(run.traces[0].decision["verdict"], "block");
    assert_eq!(run.traces[0].patch_count, 1);
    assert_eq!(run.traces[0].finding_count, 1);
}

#[test]
fn wasmtime_wat_abi_rejects_oversized_output() {
    let temp = tempfile::tempdir().expect("tempdir");
    let registry = PluginRegistry::wasmtime_wat(temp.path().join("artifacts"));

    registry
        .install(InstallPluginRequest {
            source: r#"
                (module
                  (import "capsem" "emit_output" (func $emit_output (param i32 i32) (result i32)))
                  (memory (export "memory") 1)
                  (func (export "onModelOutput") (param i32 i32 i32 i32) (result i32)
                    i32.const 0
                    i32.const 1048577
                    call $emit_output
                    drop
                    i32.const 0))
            "#
            .to_owned(),
            manifest: PublicPluginManifest {
                id: "demo.oversized".to_owned(),
                name: "Demo Oversized".to_owned(),
                version: "0.1.0".to_owned(),
                callbacks: vec![CallbackName::OnModelOutput],
                capabilities: Vec::new(),
                contributes: ExtensionContributions::default(),
            },
        })
        .expect("install succeeds");

    let error = registry
        .run(RunPluginRequest {
            plugin_id: "demo.oversized".to_owned(),
            object: json!({ "kind": "ModelOutput" }),
            context: json!({ "trace": { "labels": [] } }),
            functions: vec![CallbackName::OnModelOutput],
            runs: 1,
        })
        .expect_err("oversized output should fail");

    assert!(error.to_string().contains("wasm output exceeds"));
}

#[test]
fn wasmtime_wat_fuel_reports_consumed_fuel() {
    let temp = tempfile::tempdir().expect("tempdir");
    let registry = PluginRegistry::wasmtime_wat_with_fuel(temp.path().join("artifacts"), 10_000);

    registry
        .install(InstallPluginRequest {
            source: r#"
                (module
                  (import "capsem" "emit_output" (func $emit_output (param i32 i32) (result i32)))
                  (memory (export "memory") 1)
                  (data (i32.const 4096) "{\22kind\22:\22ModelOutput\22,\22decision\22:{\22verdict\22:\22allow\22,\22reasons\22:[]},\22patches\22:[],\22findings\22:[]}")
                  (func (export "onModelOutput") (param i32 i32 i32 i32) (result i32)
                    i32.const 4096
                    i32.const 93
                    call $emit_output
                    drop
                    i32.const 31))
            "#
            .to_owned(),
            manifest: PublicPluginManifest {
                id: "demo.fuel".to_owned(),
                name: "Demo Fuel".to_owned(),
                version: "0.1.0".to_owned(),
                callbacks: vec![CallbackName::OnModelOutput],
                capabilities: Vec::new(),
                contributes: ExtensionContributions::default(),
            },
        })
        .expect("install succeeds");

    let run = registry
        .run(RunPluginRequest {
            plugin_id: "demo.fuel".to_owned(),
            object: json!({ "kind": "ModelOutput" }),
            context: json!({ "trace": { "labels": [] } }),
            functions: vec![CallbackName::OnModelOutput],
            runs: 1,
        })
        .expect("run succeeds");

    assert_eq!(run.result["wasm_status"], 31);
    assert_eq!(run.result["wasm_fuel_budget"], 10_000);
    assert!(
        run.result["wasm_fuel_consumed"]
            .as_u64()
            .expect("fuel consumed")
            > 0
    );
    assert!(
        run.result["wasm_fuel_remaining"]
            .as_u64()
            .expect("fuel remaining")
            < 10_000
    );
}

#[test]
fn wasmtime_wat_fuel_traps_infinite_loop() {
    let temp = tempfile::tempdir().expect("tempdir");
    let registry = PluginRegistry::wasmtime_wat_with_fuel(temp.path().join("artifacts"), 1_000);

    registry
        .install(InstallPluginRequest {
            source: r#"
                (module
                  (memory (export "memory") 1)
                  (func (export "onModelOutput") (param i32 i32 i32 i32) (result i32)
                    loop
                      br 0
                    end
                    i32.const 0))
            "#
            .to_owned(),
            manifest: PublicPluginManifest {
                id: "demo.loop".to_owned(),
                name: "Demo Loop".to_owned(),
                version: "0.1.0".to_owned(),
                callbacks: vec![CallbackName::OnModelOutput],
                capabilities: Vec::new(),
                contributes: ExtensionContributions::default(),
            },
        })
        .expect("install succeeds");

    let error = registry
        .run(RunPluginRequest {
            plugin_id: "demo.loop".to_owned(),
            object: json!({ "kind": "ModelOutput" }),
            context: json!({ "trace": { "labels": [] } }),
            functions: vec![CallbackName::OnModelOutput],
            runs: 1,
        })
        .expect_err("fuel exhaustion should fail");

    assert!(error.to_string().contains("wasm fuel exhausted"));
}

#[test]
fn wasmtime_wat_budget_rejects_memory_growth() {
    let temp = tempfile::tempdir().expect("tempdir");
    let registry = PluginRegistry::wasmtime_wat_with_budget(
        temp.path().join("artifacts"),
        1_000_000,
        64 * 1024,
    );

    registry
        .install(InstallPluginRequest {
            source: r#"
                (module
                  (memory (export "memory") 1)
                  (func (export "onModelOutput") (param i32 i32 i32 i32) (result i32)
                    i32.const 1
                    memory.grow
                    drop
                    i32.const 0))
            "#
            .to_owned(),
            manifest: PublicPluginManifest {
                id: "demo.memory".to_owned(),
                name: "Demo Memory".to_owned(),
                version: "0.1.0".to_owned(),
                callbacks: vec![CallbackName::OnModelOutput],
                capabilities: Vec::new(),
                contributes: ExtensionContributions::default(),
            },
        })
        .expect("install succeeds");

    let error = registry
        .run(RunPluginRequest {
            plugin_id: "demo.memory".to_owned(),
            object: json!({ "kind": "ModelOutput" }),
            context: json!({ "trace": { "labels": [] } }),
            functions: vec![CallbackName::OnModelOutput],
            runs: 1,
        })
        .expect_err("memory growth should fail");

    assert!(error.to_string().contains("wasm budget trap"));
}

#[test]
fn install_run_runs_full_lifecycle_once() {
    let temp = tempfile::tempdir().expect("tempdir");
    let registry = PluginRegistry::wasmtime_wat(temp.path().join("artifacts"));

    let response = registry
        .install_run(InstallRunRequest {
            source: r#"
                (module
                  (func (export "onModelOutput") (result i32)
                    i32.const 11))
            "#
            .to_owned(),
            manifest: PublicPluginManifest {
                id: "demo.install_run".to_owned(),
                name: "Demo Install Run".to_owned(),
                version: "0.1.0".to_owned(),
                callbacks: vec![CallbackName::OnModelOutput],
                capabilities: Vec::new(),
                contributes: ExtensionContributions::default(),
            },
            object: json!({ "kind": "ModelOutput" }),
            context: json!({ "trace": { "labels": [] } }),
            functions: vec![CallbackName::OnModelOutput],
            runs: 4,
        })
        .expect("install-run succeeds");

    assert!(response.ok);
    assert_eq!(response.install.plugin_id, "demo.install_run");
    assert_eq!(response.install.runtime_kind, "wasmtime-core-module");
    assert_eq!(response.run.plugin_id, "demo.install_run");
    assert_eq!(response.run.traces.len(), 4);
    assert_eq!(response.run.result["wasm_status"], 11);
}
