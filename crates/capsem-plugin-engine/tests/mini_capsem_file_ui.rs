use std::{
    path::{Path, PathBuf},
    process::Command,
    sync::Arc,
};

use capsem_plugin_engine::{
    CallbackName, ExtensionContributions, InstallPluginRequest, PluginError, PluginRegistry,
    PublicPluginManifest, RunPluginRequest, RuntimeCompiler, RuntimeCompilerInput,
    RuntimeCompilerOutput, WasmtimePluginExecutor,
};
use serde_json::json;

const GIT_CONTEXT_PLUGIN: &str = include_str!("../../../examples/plugins/git_context_badge.ts");

struct AssemblyScriptCompiler {
    repo_root: PathBuf,
}

impl RuntimeCompiler for AssemblyScriptCompiler {
    fn compile(
        &self,
        input: RuntimeCompilerInput<'_>,
    ) -> Result<RuntimeCompilerOutput, PluginError> {
        let work_dir = tempfile::tempdir()?;
        let source_path = work_dir.path().join("plugin.ts");
        let wasm_path = work_dir.path().join("plugin.wasm");
        std::fs::write(&source_path, input.source)?;

        let asc = self.repo_root.join("node_modules/.bin/asc");
        let output = Command::new(&asc)
            .args([
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
            ])
            .current_dir(work_dir.path())
            .output()
            .map_err(|error| {
                PluginError::Runtime(format!("failed to run {}: {error}", asc.display()))
            })?;

        if !output.status.success() {
            return Err(PluginError::Runtime(format!(
                "{} failed with status {}\nstdout:\n{}\nstderr:\n{}",
                asc.display(),
                output.status,
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            )));
        }

        Ok(RuntimeCompilerOutput {
            kind: "wasmtime-core-module".to_owned(),
            bytes: std::fs::read(wasm_path)?,
        })
    }
}

#[test]
fn git_context_plugin_reads_fs_and_emits_ui_block() {
    let temp = tempfile::tempdir().expect("tempdir");
    let registry = PluginRegistry::with_engine(
        temp.path().join("artifacts"),
        Arc::new(AssemblyScriptCompiler {
            repo_root: repo_root(),
        }),
        Arc::new(WasmtimePluginExecutor::with_budget(
            10_000_000,
            2 * 1024 * 1024,
        )),
    );

    registry
        .install(InstallPluginRequest {
            source: GIT_CONTEXT_PLUGIN.to_owned(),
            manifest: git_context_manifest(vec!["fs.read", "fetch", "ui.emit"]),
        })
        .expect("install succeeds");

    let run = registry
        .run(RunPluginRequest {
            plugin_id: "capsem.git-context".to_owned(),
            object: json!({
                "kind": "FileCreate",
                "path": "workspace/.git",
                "action": "created",
                "decision": { "verdict": "allow", "reasons": [] },
                "patches": [],
                "findings": []
            }),
            context: json!({
                "fs": {
                    "files": {
                        "workspace/.git/HEAD": "ref: refs/heads/main\n",
                        "workspace/.git/config": "[remote \"origin\"]\n\turl = https://github.com/capsem-ai/capsem.git\n"
                    }
                },
                "fetch": {
                    "responses": {
                        "https://api.github.com/repos/capsem-ai/capsem": "{\"stargazers_count\":123,\"forks_count\":45,\"open_issues_count\":6}"
                    }
                },
                "trace": { "labels": ["file-activity"] }
            }),
            functions: vec![CallbackName::OnFileCreate],
            runs: 1,
        })
        .expect("run succeeds");

    assert_eq!(run.result["kind"], "FileCreate");
    assert_eq!(run.result["path"], "workspace/.git");
    assert_eq!(run.result["action"], "created");
    assert_eq!(run.result["wasm_status"], 70);

    let block = &run.result["ui_mutations"][0];
    assert_eq!(block["channel"], "workspace.context");
    assert_eq!(block["operation"], "upsert_block");
    assert_eq!(block["block"]["kind"], "git-context-card");
    assert_eq!(block["block"]["title"], "workspace");
    assert_eq!(block["block"]["subtitle"], "main");
    assert_eq!(
        block["block"]["body"],
        "workspace | stars 123 / forks 45 / issues 6"
    );
}

#[test]
fn fetch_abi_requires_declared_capability() {
    let temp = tempfile::tempdir().expect("tempdir");
    let registry = PluginRegistry::with_engine(
        temp.path().join("artifacts"),
        Arc::new(AssemblyScriptCompiler {
            repo_root: repo_root(),
        }),
        Arc::new(WasmtimePluginExecutor::with_budget(
            10_000_000,
            2 * 1024 * 1024,
        )),
    );

    registry
        .install(InstallPluginRequest {
            source: GIT_CONTEXT_PLUGIN.to_owned(),
            manifest: git_context_manifest(vec!["fs.read", "ui.emit"]),
        })
        .expect("install succeeds");

    let error = registry
        .run(RunPluginRequest {
            plugin_id: "capsem.git-context".to_owned(),
            object: json!({
                "kind": "FileCreate",
                "path": "workspace/.git",
                "action": "created",
                "decision": { "verdict": "allow", "reasons": [] },
                "patches": [],
                "findings": []
            }),
            context: json!({
                "fs": {
                    "files": {
                        "workspace/.git/HEAD": "ref: refs/heads/main\n",
                        "workspace/.git/config": "[remote \"origin\"]\n\turl = https://github.com/capsem-ai/capsem.git\n"
                    }
                },
                "fetch": {
                    "responses": {
                        "https://api.github.com/repos/capsem-ai/capsem": "{\"stargazers_count\":123,\"forks_count\":45,\"open_issues_count\":6}"
                    }
                }
            }),
            functions: vec![CallbackName::OnFileCreate],
            runs: 1,
        })
        .expect_err("fetch without capability should fail");

    assert!(error.to_string().contains("fetch capability"));
}

#[test]
#[ignore = "hits live GitHub API"]
fn git_context_plugin_can_fetch_live_github_stats() {
    let temp = tempfile::tempdir().expect("tempdir");
    let registry = PluginRegistry::with_engine(
        temp.path().join("artifacts"),
        Arc::new(AssemblyScriptCompiler {
            repo_root: repo_root(),
        }),
        Arc::new(WasmtimePluginExecutor::with_budget(
            10_000_000,
            2 * 1024 * 1024,
        )),
    );

    registry
        .install(InstallPluginRequest {
            source: GIT_CONTEXT_PLUGIN.to_owned(),
            manifest: git_context_manifest(vec!["fs.read", "fetch", "ui.emit"]),
        })
        .expect("install succeeds");

    let run = registry
        .run(RunPluginRequest {
            plugin_id: "capsem.git-context".to_owned(),
            object: json!({
                "kind": "FileCreate",
                "path": "workspace/.git",
                "action": "created",
                "decision": { "verdict": "allow", "reasons": [] },
                "patches": [],
                "findings": []
            }),
            context: json!({
                "fs": {
                    "files": {
                        "workspace/.git/HEAD": "ref: refs/heads/main\n",
                        "workspace/.git/config": "[remote \"origin\"]\n\turl = https://github.com/rust-lang/rust.git\n"
                    }
                }
            }),
            functions: vec![CallbackName::OnFileCreate],
            runs: 1,
        })
        .expect("run succeeds");

    let body = run.result["ui_mutations"][0]["block"]["body"]
        .as_str()
        .expect("body is string");
    assert!(body.contains("stars "));
    assert!(body.contains("forks "));
    assert!(body.contains("issues "));
}

#[test]
fn fs_abi_requires_declared_capability() {
    let temp = tempfile::tempdir().expect("tempdir");
    let registry = PluginRegistry::with_engine(
        temp.path().join("artifacts"),
        Arc::new(AssemblyScriptCompiler {
            repo_root: repo_root(),
        }),
        Arc::new(WasmtimePluginExecutor::with_budget(
            10_000_000,
            2 * 1024 * 1024,
        )),
    );

    registry
        .install(InstallPluginRequest {
            source: GIT_CONTEXT_PLUGIN.to_owned(),
            manifest: git_context_manifest(vec!["ui.emit"]),
        })
        .expect("install succeeds");

    let error = registry
        .run(RunPluginRequest {
            plugin_id: "capsem.git-context".to_owned(),
            object: json!({
                "kind": "FileCreate",
                "path": "workspace/.git",
                "action": "created",
                "decision": { "verdict": "allow", "reasons": [] },
                "patches": [],
                "findings": []
            }),
            context: json!({
                "fs": {
                    "files": {
                        "workspace/.git/HEAD": "ref: refs/heads/main\n"
                    }
                }
            }),
            functions: vec![CallbackName::OnFileCreate],
            runs: 1,
        })
        .expect_err("fs read without capability should fail");

    assert!(error.to_string().contains("fs.read capability"));
}

fn git_context_manifest(capabilities: Vec<&str>) -> PublicPluginManifest {
    PublicPluginManifest {
        id: "capsem.git-context".to_owned(),
        name: "Capsem Git Context".to_owned(),
        version: "0.1.0".to_owned(),
        callbacks: vec![CallbackName::OnFileCreate],
        capabilities: capabilities.into_iter().map(str::to_owned).collect(),
        contributes: ExtensionContributions {
            ui: vec!["ui/git-context-card.json".to_owned()],
            ..ExtensionContributions::default()
        },
    }
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("crate dir has crates parent")
        .parent()
        .expect("crates dir has repo parent")
        .to_path_buf()
}
