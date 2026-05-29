use std::{
    collections::HashMap,
    fmt, fs,
    path::{Path, PathBuf},
    sync::{Arc, RwLock},
    time::{Instant, SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

pub mod ui;

const ABI_VERSION: &str = "capsem-plugin-v1";
const PLACEHOLDER_RUNTIME_KIND: &str = "wasmtime-precompiled-component-placeholder";
const WASMTIME_CORE_RUNTIME_KIND: &str = "wasmtime-core-module";
const MAX_ABI_JSON_BYTES: usize = 1024 * 1024;
const MAX_ABI_PATH_BYTES: usize = 8 * 1024;
const MAX_ABI_HTTP_RESPONSE_BYTES: usize = 1024 * 1024;
const DEFAULT_WASM_FUEL: u64 = 10_000_000;
const DEFAULT_WASM_MEMORY_BYTES: usize = 2 * 1024 * 1024;
const CAPABILITY_FS_READ: &str = "fs.read";
const CAPABILITY_FETCH: &str = "fetch";
const CAPABILITY_UI_EMIT: &str = "ui.emit";

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExtensionUnitKind {
    Plugin,
    Rule,
    Tool,
    Skill,
    Ui,
}

#[derive(Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub enum CallbackName {
    #[serde(rename = "onHttpRequest")]
    OnHttpRequest,
    #[serde(rename = "onHttpResponse")]
    OnHttpResponse,
    #[serde(rename = "onModelCall")]
    OnModelCall,
    #[serde(rename = "onModelOutput")]
    OnModelOutput,
    #[serde(rename = "onMcpToolCall")]
    OnMcpToolCall,
    #[serde(rename = "onMcpToolResult")]
    OnMcpToolResult,
    #[serde(rename = "onFileActivity")]
    OnFileActivity,
    #[serde(rename = "on_file_create")]
    OnFileCreate,
    #[serde(rename = "onVmStart")]
    OnVmStart,
    #[serde(rename = "onVmReady")]
    OnVmReady,
    #[serde(rename = "onPanelRender")]
    OnPanelRender,
    #[serde(rename = "onCommandInvocation")]
    OnCommandInvocation,
    #[serde(rename = "onToolInvocation")]
    OnToolInvocation,
    #[serde(rename = "onSkillActivation")]
    OnSkillActivation,
    #[serde(rename = "onRuleMatch")]
    OnRuleMatch,
}

impl fmt::Display for CallbackName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::OnHttpRequest => f.write_str("onHttpRequest"),
            Self::OnHttpResponse => f.write_str("onHttpResponse"),
            Self::OnModelCall => f.write_str("onModelCall"),
            Self::OnModelOutput => f.write_str("onModelOutput"),
            Self::OnMcpToolCall => f.write_str("onMcpToolCall"),
            Self::OnMcpToolResult => f.write_str("onMcpToolResult"),
            Self::OnFileActivity => f.write_str("onFileActivity"),
            Self::OnFileCreate => f.write_str("on_file_create"),
            Self::OnVmStart => f.write_str("onVmStart"),
            Self::OnVmReady => f.write_str("onVmReady"),
            Self::OnPanelRender => f.write_str("onPanelRender"),
            Self::OnCommandInvocation => f.write_str("onCommandInvocation"),
            Self::OnToolInvocation => f.write_str("onToolInvocation"),
            Self::OnSkillActivation => f.write_str("onSkillActivation"),
            Self::OnRuleMatch => f.write_str("onRuleMatch"),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct ExtensionContributions {
    pub rules: Vec<String>,
    pub tools: Vec<String>,
    pub skills: Vec<String>,
    pub ui: Vec<String>,
}

impl Default for ExtensionContributions {
    fn default() -> Self {
        Self {
            rules: Vec::new(),
            tools: Vec::new(),
            skills: Vec::new(),
            ui: Vec::new(),
        }
    }
}

impl ExtensionContributions {
    pub fn declared_unit_kinds(&self) -> Vec<ExtensionUnitKind> {
        let mut kinds = vec![ExtensionUnitKind::Plugin];
        if !self.rules.is_empty() {
            kinds.push(ExtensionUnitKind::Rule);
        }
        if !self.tools.is_empty() {
            kinds.push(ExtensionUnitKind::Tool);
        }
        if !self.skills.is_empty() {
            kinds.push(ExtensionUnitKind::Skill);
        }
        if !self.ui.is_empty() {
            kinds.push(ExtensionUnitKind::Ui);
        }
        kinds
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PublicPluginManifest {
    pub id: String,
    pub name: String,
    pub version: String,
    pub callbacks: Vec<CallbackName>,
    #[serde(default)]
    pub capabilities: Vec<String>,
    #[serde(default)]
    pub contributes: ExtensionContributions,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct InstallPluginRequest {
    pub source: String,
    pub manifest: PublicPluginManifest,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RuntimeArtifact {
    pub kind: String,
    pub path: PathBuf,
    pub blake3: String,
    pub bytes: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct InternalPluginManifest {
    pub id: String,
    pub name: String,
    pub version: String,
    pub source_blake3: String,
    pub manifest_blake3: String,
    pub runtime: RuntimeArtifact,
    pub abi_version: String,
    pub callbacks: Vec<CallbackName>,
    pub capabilities: Vec<String>,
    pub contributes: ExtensionContributions,
    pub unit_kinds: Vec<ExtensionUnitKind>,
    pub install_load_ms: f64,
    pub installed_at_unix_ms: u128,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct InstallPluginResponse {
    pub ok: bool,
    pub plugin_id: String,
    pub compile_ms: f64,
    pub load_ms: f64,
    pub runtime_blake3: String,
    pub runtime_artifact_path: PathBuf,
    pub runtime_kind: String,
    pub runtime_bytes: u64,
    pub manifest: InternalPluginManifest,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RunPluginRequest {
    pub plugin_id: String,
    pub object: Value,
    pub context: Value,
    #[serde(default)]
    pub functions: Vec<CallbackName>,
    #[serde(default = "default_runs")]
    pub runs: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FunctionRunTrace {
    pub function: CallbackName,
    pub iteration: u32,
    pub run_ms: f64,
    pub input_object_blake3: String,
    pub context_blake3: String,
    pub output_object_blake3: String,
    pub decision: Value,
    pub patch_count: usize,
    pub finding_count: usize,
    pub result: Value,
    pub result_bytes: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RunPluginResponse {
    pub ok: bool,
    pub plugin_id: String,
    pub runtime_blake3: String,
    pub load_ms: f64,
    pub run_ms: f64,
    pub runtime_bytes: u64,
    pub input_object_blake3: String,
    pub context_blake3: String,
    pub output_object_blake3: String,
    pub result: Value,
    pub traces: Vec<FunctionRunTrace>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct InstallRunRequest {
    pub source: String,
    pub manifest: PublicPluginManifest,
    pub object: Value,
    pub context: Value,
    #[serde(default)]
    pub functions: Vec<CallbackName>,
    #[serde(default = "default_runs")]
    pub runs: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct InstallRunResponse {
    pub ok: bool,
    pub install: InstallPluginResponse,
    pub run: RunPluginResponse,
}

pub struct RuntimeCompilerInput<'a> {
    pub source: &'a str,
    pub manifest: &'a PublicPluginManifest,
    pub source_blake3: &'a str,
    pub manifest_blake3: &'a str,
}

pub struct RuntimeCompilerOutput {
    pub kind: String,
    pub bytes: Vec<u8>,
}

pub trait RuntimeCompiler: Send + Sync {
    fn compile(
        &self,
        input: RuntimeCompilerInput<'_>,
    ) -> Result<RuntimeCompilerOutput, PluginError>;
}

#[derive(Default)]
pub struct PlaceholderRuntimeCompiler;

impl RuntimeCompiler for PlaceholderRuntimeCompiler {
    fn compile(
        &self,
        input: RuntimeCompilerInput<'_>,
    ) -> Result<RuntimeCompilerOutput, PluginError> {
        let payload = json!({
            "kind": PLACEHOLDER_RUNTIME_KIND,
            "note": "placeholder for Wasmtime precompiled component artifact",
            "abi_version": ABI_VERSION,
            "source_blake3": input.source_blake3,
            "manifest_blake3": input.manifest_blake3,
            "manifest": input.manifest,
            "source": input.source,
        });

        Ok(RuntimeCompilerOutput {
            kind: PLACEHOLDER_RUNTIME_KIND.to_owned(),
            bytes: serde_json::to_vec_pretty(&payload)?,
        })
    }
}

#[derive(Default)]
pub struct WatRuntimeCompiler;

impl RuntimeCompiler for WatRuntimeCompiler {
    fn compile(
        &self,
        input: RuntimeCompilerInput<'_>,
    ) -> Result<RuntimeCompilerOutput, PluginError> {
        let bytes = wat::parse_str(input.source)
            .map_err(|error| PluginError::Request(format!("wat compile failed: {error}")))?;

        Ok(RuntimeCompilerOutput {
            kind: WASMTIME_CORE_RUNTIME_KIND.to_owned(),
            bytes,
        })
    }
}

pub struct PluginCallInput<'a> {
    pub manifest: &'a InternalPluginManifest,
    pub function: &'a CallbackName,
    pub object: &'a Value,
    pub context: &'a Value,
}

pub struct PluginLoadInput<'a> {
    pub manifest: &'a InternalPluginManifest,
}

pub trait LoadedPluginRuntime: Send + Sync {
    fn call(&self, input: PluginCallInput<'_>) -> Result<Value, PluginError>;
}

pub trait PluginExecutor: Send + Sync {
    fn load(&self, input: PluginLoadInput<'_>)
        -> Result<Arc<dyn LoadedPluginRuntime>, PluginError>;
}

#[derive(Default)]
pub struct PlaceholderPluginExecutor;

impl PluginExecutor for PlaceholderPluginExecutor {
    fn load(
        &self,
        _input: PluginLoadInput<'_>,
    ) -> Result<Arc<dyn LoadedPluginRuntime>, PluginError> {
        Ok(Arc::new(PlaceholderLoadedPluginRuntime))
    }
}

struct PlaceholderLoadedPluginRuntime;

impl LoadedPluginRuntime for PlaceholderLoadedPluginRuntime {
    fn call(&self, input: PluginCallInput<'_>) -> Result<Value, PluginError> {
        let mut result = input.object.clone();

        if let Value::Object(ref mut map) = result {
            let labels = input
                .context
                .pointer("/trace/labels")
                .cloned()
                .unwrap_or_else(|| Value::Array(Vec::new()));
            map.insert("labels".to_owned(), labels);
            map.insert(
                "called".to_owned(),
                Value::String(input.function.to_string()),
            );
            map.insert(
                "plugin_id".to_owned(),
                Value::String(input.manifest.id.clone()),
            );
        }

        Ok(result)
    }
}

pub struct WasmtimePluginExecutor {
    engine: wasmtime::Engine,
    fuel_budget: Option<u64>,
    memory_limit_bytes: Option<usize>,
}

impl Default for WasmtimePluginExecutor {
    fn default() -> Self {
        Self {
            engine: wasmtime::Engine::default(),
            fuel_budget: None,
            memory_limit_bytes: None,
        }
    }
}

impl WasmtimePluginExecutor {
    pub fn with_fuel(fuel_budget: u64) -> Self {
        Self::with_budget(fuel_budget, DEFAULT_WASM_MEMORY_BYTES)
    }

    pub fn with_budget(fuel_budget: u64, memory_limit_bytes: usize) -> Self {
        let mut config = wasmtime::Config::new();
        config.consume_fuel(true);
        let engine = wasmtime::Engine::new(&config).expect("fuel-enabled Wasmtime engine");
        Self {
            engine,
            fuel_budget: Some(fuel_budget),
            memory_limit_bytes: Some(memory_limit_bytes),
        }
    }
}

impl PluginExecutor for WasmtimePluginExecutor {
    fn load(
        &self,
        input: PluginLoadInput<'_>,
    ) -> Result<Arc<dyn LoadedPluginRuntime>, PluginError> {
        if input.manifest.runtime.kind != WASMTIME_CORE_RUNTIME_KIND {
            return Err(PluginError::Request(format!(
                "wasmtime executor cannot run runtime kind: {}",
                input.manifest.runtime.kind
            )));
        }

        let module = wasmtime::Module::from_file(&self.engine, &input.manifest.runtime.path)
            .map_err(|error| PluginError::Runtime(format!("wasm module load failed: {error}")))?;
        Ok(Arc::new(WasmtimeLoadedPluginRuntime {
            engine: self.engine.clone(),
            module,
            fuel_budget: self.fuel_budget,
            memory_limit_bytes: self.memory_limit_bytes,
        }))
    }
}

struct WasmtimeLoadedPluginRuntime {
    engine: wasmtime::Engine,
    module: wasmtime::Module,
    fuel_budget: Option<u64>,
    memory_limit_bytes: Option<usize>,
}

impl LoadedPluginRuntime for WasmtimeLoadedPluginRuntime {
    fn call(&self, input: PluginCallInput<'_>) -> Result<Value, PluginError> {
        let mut linker = wasmtime::Linker::new(&self.engine);
        linker
            .func_wrap(
                "capsem",
                "emit_output",
                |mut caller: wasmtime::Caller<'_, WasmtimeCallState>, ptr: i32, len: i32| -> i32 {
                    if ptr < 0 || len < 0 {
                        caller.data_mut().error =
                            Some("wasm emitted negative output pointer or length".to_owned());
                        return -1;
                    }
                    if len as usize > MAX_ABI_JSON_BYTES {
                        caller.data_mut().error = Some(format!(
                            "wasm output exceeds {} byte ABI limit",
                            MAX_ABI_JSON_BYTES
                        ));
                        return -4;
                    }

                    let Some(memory) = caller
                        .get_export("memory")
                        .and_then(|export| export.into_memory())
                    else {
                        caller.data_mut().error =
                            Some("wasm output requires exported memory".to_owned());
                        return -2;
                    };

                    let mut bytes = vec![0; len as usize];
                    match memory.read(&caller, ptr as usize, &mut bytes) {
                        Ok(()) => {
                            caller.data_mut().output = Some(bytes);
                            0
                        }
                        Err(error) => {
                            caller.data_mut().error =
                                Some(format!("wasm output read failed: {error}"));
                            -3
                        }
                    }
                },
            )
            .map_err(|error| {
                PluginError::Runtime(format!("wasm host ABI setup failed: {error}"))
            })?;
        linker
            .func_wrap(
                "capsem",
                "fs_read",
                |mut caller: wasmtime::Caller<'_, WasmtimeCallState>, ptr: i32, len: i32| -> i32 {
                    if !caller.data().allow_fs_read {
                        caller.data_mut().error =
                            Some("fs.read capability is not declared".to_owned());
                        return -401;
                    }

                    let path_bytes = match read_guest_bytes(
                        &mut caller,
                        ptr,
                        len,
                        MAX_ABI_PATH_BYTES,
                        "fs path",
                    ) {
                        Ok(bytes) => bytes,
                        Err(code) => return code,
                    };
                    let path = match String::from_utf8(path_bytes) {
                        Ok(path) => path,
                        Err(error) => {
                            caller.data_mut().error =
                                Some(format!("fs path is not utf-8: {error}"));
                            return -400;
                        }
                    };
                    if !is_safe_fs_path(&path) {
                        caller.data_mut().error = Some(format!("fs path is not allowed: {path}"));
                        return -400;
                    }

                    let response = caller
                        .data()
                        .fs_files
                        .get(&path)
                        .map(|value| value.as_bytes().to_vec())
                        .unwrap_or_default();
                    let len = response.len();
                    caller.data_mut().host_response = Some(response);
                    len as i32
                },
            )
            .map_err(|error| PluginError::Runtime(format!("wasm fs ABI setup failed: {error}")))?;
        linker
            .func_wrap(
                "capsem",
                "read_host_response",
                |mut caller: wasmtime::Caller<'_, WasmtimeCallState>, ptr: i32, len: i32| -> i32 {
                    if ptr < 0 || len < 0 {
                        caller.data_mut().error =
                            Some("wasm host response target is negative".to_owned());
                        return -1;
                    }
                    let response = caller.data().host_response.clone().unwrap_or_default();
                    if response.len() > len as usize {
                        caller.data_mut().error = Some(format!(
                            "wasm host response buffer too small: need {}, got {}",
                            response.len(),
                            len
                        ));
                        return -2;
                    }
                    let Some(memory) = caller
                        .get_export("memory")
                        .and_then(|export| export.into_memory())
                    else {
                        caller.data_mut().error =
                            Some("wasm host response requires exported memory".to_owned());
                        return -3;
                    };
                    match memory.write(&mut caller, ptr as usize, &response) {
                        Ok(()) => response.len() as i32,
                        Err(error) => {
                            caller.data_mut().error =
                                Some(format!("wasm host response write failed: {error}"));
                            -4
                        }
                    }
                },
            )
            .map_err(|error| {
                PluginError::Runtime(format!("wasm host response ABI setup failed: {error}"))
            })?;
        linker
            .func_wrap(
                "capsem",
                "fetch",
                |mut caller: wasmtime::Caller<'_, WasmtimeCallState>, ptr: i32, len: i32| -> i32 {
                    if !caller.data().allow_fetch {
                        caller.data_mut().error =
                            Some("fetch capability is not declared".to_owned());
                        return -401;
                    }

                    let url_bytes = match read_guest_bytes(
                        &mut caller,
                        ptr,
                        len,
                        MAX_ABI_PATH_BYTES,
                        "fetch url",
                    ) {
                        Ok(bytes) => bytes,
                        Err(code) => return code,
                    };
                    let url = match String::from_utf8(url_bytes) {
                        Ok(url) => url,
                        Err(error) => {
                            caller.data_mut().error =
                                Some(format!("fetch url is not utf-8: {error}"));
                            return -400;
                        }
                    };
                    if !is_safe_fetch_url(&url) {
                        caller.data_mut().error = Some(format!("fetch url is not allowed: {url}"));
                        return -400;
                    }

                    let response = match caller.data().fetch_responses.get(&url) {
                        Some(response) => response.as_bytes().to_vec(),
                        None => match fetch_text(&url) {
                            Ok(response) => response.into_bytes(),
                            Err(error) => {
                                caller.data_mut().error = Some(format!("fetch failed: {error}"));
                                return -500;
                            }
                        },
                    };
                    if response.len() > MAX_ABI_HTTP_RESPONSE_BYTES {
                        caller.data_mut().error = Some(format!(
                            "fetch response exceeds {} byte ABI limit",
                            MAX_ABI_HTTP_RESPONSE_BYTES
                        ));
                        return -413;
                    }

                    let len = response.len();
                    caller.data_mut().host_response = Some(response);
                    len as i32
                },
            )
            .map_err(|error| {
                PluginError::Runtime(format!("wasm fetch ABI setup failed: {error}"))
            })?;
        linker
            .func_wrap(
                "capsem",
                "emit_ui_block",
                |mut caller: wasmtime::Caller<'_, WasmtimeCallState>, ptr: i32, len: i32| -> i32 {
                    if !caller.data().allow_ui_emit {
                        caller.data_mut().error =
                            Some("ui.emit capability is not declared".to_owned());
                        return -401;
                    }
                    let block_bytes = match read_guest_bytes(
                        &mut caller,
                        ptr,
                        len,
                        MAX_ABI_JSON_BYTES,
                        "ui block",
                    ) {
                        Ok(bytes) => bytes,
                        Err(code) => return code,
                    };
                    let block: Value = match serde_json::from_slice(&block_bytes) {
                        Ok(block) => block,
                        Err(error) => {
                            caller.data_mut().error =
                                Some(format!("wasm emitted invalid UI block JSON: {error}"));
                            return -5;
                        }
                    };
                    caller.data_mut().ui_blocks.push(block);
                    0
                },
            )
            .map_err(|error| PluginError::Runtime(format!("wasm UI ABI setup failed: {error}")))?;
        linker
            .func_wrap(
                "env",
                "abort",
                |mut caller: wasmtime::Caller<'_, WasmtimeCallState>,
                 _message: i32,
                 _file_name: i32,
                 line: i32,
                 column: i32| {
                    caller.data_mut().error = Some(format!("wasm guest abort at {line}:{column}"));
                },
            )
            .map_err(|error| {
                PluginError::Runtime(format!("wasm host compatibility setup failed: {error}"))
            })?;

        let mut store = wasmtime::Store::new(
            &self.engine,
            WasmtimeCallState::new(self.memory_limit_bytes, input.context, input.manifest),
        );
        if self.memory_limit_bytes.is_some() {
            store.limiter(|state| state.limits.as_mut().expect("limits configured"));
        }
        if let Some(fuel_budget) = self.fuel_budget {
            store.set_fuel(fuel_budget).map_err(|error| {
                PluginError::Runtime(format!("wasm fuel setup failed: {error}"))
            })?;
        }
        let instance = linker
            .instantiate(&mut store, &self.module)
            .map_err(|error| PluginError::Runtime(format!("wasm instance load failed: {error}")))?;
        let callback_name = input.function.to_string();
        let status = match instance
            .get_typed_func::<(i32, i32, i32, i32), i32>(&mut store, &callback_name)
        {
            Ok(callback) => {
                let object_bytes = serde_json::to_vec(input.object)?;
                let context_bytes = serde_json::to_vec(input.context)?;
                if object_bytes.len() > MAX_ABI_JSON_BYTES {
                    return Err(PluginError::Runtime(format!(
                        "object exceeds {} byte ABI limit",
                        MAX_ABI_JSON_BYTES
                    )));
                }
                if context_bytes.len() > MAX_ABI_JSON_BYTES {
                    return Err(PluginError::Runtime(format!(
                        "context exceeds {} byte ABI limit",
                        MAX_ABI_JSON_BYTES
                    )));
                }
                let memory = instance.get_memory(&mut store, "memory").ok_or_else(|| {
                    PluginError::Runtime("wasm ABI callback requires exported memory".to_owned())
                })?;
                let object_ptr = 0usize;
                let context_ptr = object_bytes.len() + 16;

                memory
                    .write(&mut store, object_ptr, &object_bytes)
                    .map_err(|error| {
                        PluginError::Runtime(format!("wasm object write failed: {error}"))
                    })?;
                memory
                    .write(&mut store, context_ptr, &context_bytes)
                    .map_err(|error| {
                        PluginError::Runtime(format!("wasm context write failed: {error}"))
                    })?;

                callback
                    .call(
                        &mut store,
                        (
                            object_ptr as i32,
                            object_bytes.len() as i32,
                            context_ptr as i32,
                            context_bytes.len() as i32,
                        ),
                    )
                    .map_err(|error| {
                        wasm_call_error(
                            error,
                            &mut store,
                            self.fuel_budget,
                            self.memory_limit_bytes,
                        )
                    })?
            }
            Err(_) => {
                let callback = instance
                    .get_typed_func::<(), i32>(&mut store, &callback_name)
                    .map_err(|error| {
                        PluginError::Runtime(format!("wasm callback lookup failed: {error}"))
                    })?;
                callback.call(&mut store, ()).map_err(|error| {
                    wasm_call_error(error, &mut store, self.fuel_budget, self.memory_limit_bytes)
                })?
            }
        };

        if let Some(error) = store.data().error.as_ref() {
            return Err(PluginError::Runtime(error.clone()));
        }

        if let Some(output) = store.data().output.as_ref() {
            let mut result: Value = serde_json::from_slice(output).map_err(|error| {
                PluginError::Runtime(format!("wasm emitted invalid output JSON: {error}"))
            })?;
            if let Value::Object(ref mut map) = result {
                map.insert("called".to_owned(), Value::String(callback_name));
                map.insert(
                    "plugin_id".to_owned(),
                    Value::String(input.manifest.id.clone()),
                );
                map.insert("wasm_status".to_owned(), Value::Number(status.into()));
                insert_ui_blocks(&store, map);
                insert_fuel_metadata(&mut store, self.fuel_budget, map)?;
            }
            return Ok(result);
        }

        let mut result = input.object.clone();
        if let Value::Object(ref mut map) = result {
            map.insert("called".to_owned(), Value::String(callback_name));
            map.insert(
                "plugin_id".to_owned(),
                Value::String(input.manifest.id.clone()),
            );
            map.insert("wasm_status".to_owned(), Value::Number(status.into()));
            insert_ui_blocks(&store, map);
            insert_fuel_metadata(&mut store, self.fuel_budget, map)?;
        }

        Ok(result)
    }
}

fn read_guest_bytes(
    caller: &mut wasmtime::Caller<'_, WasmtimeCallState>,
    ptr: i32,
    len: i32,
    max_len: usize,
    label: &str,
) -> Result<Vec<u8>, i32> {
    if ptr < 0 || len < 0 {
        caller.data_mut().error = Some(format!("wasm emitted negative {label} pointer or length"));
        return Err(-1);
    }
    if len as usize > max_len {
        caller.data_mut().error = Some(format!("{label} exceeds {max_len} byte ABI limit"));
        return Err(-4);
    }
    let Some(memory) = caller
        .get_export("memory")
        .and_then(|export| export.into_memory())
    else {
        caller.data_mut().error = Some(format!("{label} requires exported memory"));
        return Err(-2);
    };
    let mut bytes = vec![0; len as usize];
    match memory.read(&caller, ptr as usize, &mut bytes) {
        Ok(()) => Ok(bytes),
        Err(error) => {
            caller.data_mut().error = Some(format!("{label} read failed: {error}"));
            Err(-3)
        }
    }
}

fn wasm_call_error(
    error: wasmtime::Error,
    store: &mut wasmtime::Store<WasmtimeCallState>,
    fuel_budget: Option<u64>,
    memory_limit_bytes: Option<usize>,
) -> PluginError {
    if fuel_budget.is_some() && matches!(store.get_fuel(), Ok(0)) {
        return PluginError::Runtime(format!("wasm fuel exhausted: {error}"));
    }
    if memory_limit_bytes.is_some() {
        return PluginError::Runtime(format!("wasm budget trap: {error}"));
    }
    PluginError::Runtime(format!("wasm callback failed: {error}"))
}

fn insert_fuel_metadata(
    store: &mut wasmtime::Store<WasmtimeCallState>,
    fuel_budget: Option<u64>,
    map: &mut serde_json::Map<String, Value>,
) -> Result<(), PluginError> {
    let Some(fuel_budget) = fuel_budget else {
        return Ok(());
    };
    let fuel_remaining = store
        .get_fuel()
        .map_err(|error| PluginError::Runtime(format!("wasm fuel read failed: {error}")))?;
    map.insert(
        "wasm_fuel_budget".to_owned(),
        Value::Number(fuel_budget.into()),
    );
    map.insert(
        "wasm_fuel_remaining".to_owned(),
        Value::Number(fuel_remaining.into()),
    );
    map.insert(
        "wasm_fuel_consumed".to_owned(),
        Value::Number((fuel_budget.saturating_sub(fuel_remaining)).into()),
    );
    Ok(())
}

fn insert_ui_blocks(
    store: &wasmtime::Store<WasmtimeCallState>,
    map: &mut serde_json::Map<String, Value>,
) {
    if store.data().ui_blocks.is_empty() {
        return;
    }
    map.insert(
        "ui_mutations".to_owned(),
        Value::Array(store.data().ui_blocks.clone()),
    );
}

#[derive(Default)]
struct WasmtimeCallState {
    output: Option<Vec<u8>>,
    ui_blocks: Vec<Value>,
    host_response: Option<Vec<u8>>,
    fs_files: HashMap<String, String>,
    fetch_responses: HashMap<String, String>,
    allow_fs_read: bool,
    allow_fetch: bool,
    allow_ui_emit: bool,
    error: Option<String>,
    limits: Option<wasmtime::StoreLimits>,
}

impl WasmtimeCallState {
    fn new(
        memory_limit_bytes: Option<usize>,
        context: &Value,
        manifest: &InternalPluginManifest,
    ) -> Self {
        Self {
            output: None,
            ui_blocks: Vec::new(),
            host_response: None,
            fs_files: fs_files_from_context(context),
            fetch_responses: fetch_responses_from_context(context),
            allow_fs_read: manifest
                .capabilities
                .iter()
                .any(|capability| capability == CAPABILITY_FS_READ),
            allow_fetch: manifest
                .capabilities
                .iter()
                .any(|capability| capability == CAPABILITY_FETCH),
            allow_ui_emit: manifest
                .capabilities
                .iter()
                .any(|capability| capability == CAPABILITY_UI_EMIT),
            error: None,
            limits: memory_limit_bytes.map(|limit| {
                wasmtime::StoreLimitsBuilder::new()
                    .memory_size(limit)
                    .table_elements(1024)
                    .instances(1)
                    .tables(4)
                    .memories(1)
                    .trap_on_grow_failure(true)
                    .build()
            }),
        }
    }
}

fn fs_files_from_context(context: &Value) -> HashMap<String, String> {
    context
        .pointer("/fs/files")
        .and_then(Value::as_object)
        .map(|files| {
            files
                .iter()
                .filter_map(|(path, value)| {
                    if !is_safe_fs_path(path) {
                        return None;
                    }
                    value
                        .as_str()
                        .map(|content| (path.clone(), content.to_owned()))
                })
                .collect()
        })
        .unwrap_or_default()
}

fn fetch_responses_from_context(context: &Value) -> HashMap<String, String> {
    context
        .pointer("/fetch/responses")
        .and_then(Value::as_object)
        .map(|responses| {
            responses
                .iter()
                .filter_map(|(url, value)| {
                    if !is_safe_fetch_url(url) {
                        return None;
                    }
                    value.as_str().map(|body| (url.clone(), body.to_owned()))
                })
                .collect()
        })
        .unwrap_or_default()
}

fn is_safe_fs_path(path: &str) -> bool {
    !path.trim().is_empty()
        && path.len() <= MAX_ABI_PATH_BYTES
        && !Path::new(path).is_absolute()
        && !Path::new(path)
            .components()
            .any(|component| matches!(component, std::path::Component::ParentDir))
}

fn is_safe_fetch_url(url: &str) -> bool {
    if url.len() > MAX_ABI_PATH_BYTES || !url.starts_with("https://") {
        return false;
    }
    let Some(rest) = url.strip_prefix("https://") else {
        return false;
    };
    let host = rest.split('/').next().unwrap_or_default();
    matches!(host, "api.github.com" | "github.com")
}

fn fetch_text(url: &str) -> Result<String, PluginError> {
    let response = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .user_agent("capsem-plugin-prototype/0.1")
        .build()
        .map_err(|error| PluginError::Runtime(format!("fetch client setup failed: {error}")))?
        .get(url)
        .send()
        .map_err(|error| PluginError::Runtime(format!("fetch request failed: {error}")))?;
    let status = response.status();
    if !status.is_success() {
        return Err(PluginError::Runtime(format!(
            "fetch returned status {status}"
        )));
    }
    response
        .text()
        .map_err(|error| PluginError::Runtime(format!("fetch response read failed: {error}")))
}

#[derive(Debug)]
pub enum PluginError {
    Request(String),
    Runtime(String),
    Io(std::io::Error),
    Serde(serde_json::Error),
}

impl fmt::Display for PluginError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Request(message) => f.write_str(message),
            Self::Runtime(message) => f.write_str(message),
            Self::Io(error) => write!(f, "io error: {error}"),
            Self::Serde(error) => write!(f, "json error: {error}"),
        }
    }
}

impl std::error::Error for PluginError {}

impl From<std::io::Error> for PluginError {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value)
    }
}

impl From<serde_json::Error> for PluginError {
    fn from(value: serde_json::Error) -> Self {
        Self::Serde(value)
    }
}

#[derive(Clone)]
pub struct PluginRegistry {
    artifact_dir: PathBuf,
    compiler: Arc<dyn RuntimeCompiler>,
    executor: Arc<dyn PluginExecutor>,
    plugins: Arc<RwLock<HashMap<String, InstalledPlugin>>>,
}

#[derive(Clone)]
struct InstalledPlugin {
    manifest: InternalPluginManifest,
    runtime: Arc<dyn LoadedPluginRuntime>,
}

impl PluginRegistry {
    pub fn new(artifact_dir: impl Into<PathBuf>) -> Self {
        Self::with_engine(
            artifact_dir,
            Arc::new(PlaceholderRuntimeCompiler),
            Arc::new(PlaceholderPluginExecutor),
        )
    }

    pub fn wasmtime_wat(artifact_dir: impl Into<PathBuf>) -> Self {
        Self::with_engine(
            artifact_dir,
            Arc::new(WatRuntimeCompiler),
            Arc::new(WasmtimePluginExecutor::default()),
        )
    }

    pub fn wasmtime_wat_fuel(artifact_dir: impl Into<PathBuf>) -> Self {
        Self::wasmtime_wat_with_fuel(artifact_dir, DEFAULT_WASM_FUEL)
    }

    pub fn wasmtime_wat_with_fuel(artifact_dir: impl Into<PathBuf>, fuel_budget: u64) -> Self {
        Self::wasmtime_wat_with_budget(artifact_dir, fuel_budget, DEFAULT_WASM_MEMORY_BYTES)
    }

    pub fn wasmtime_wat_with_budget(
        artifact_dir: impl Into<PathBuf>,
        fuel_budget: u64,
        memory_limit_bytes: usize,
    ) -> Self {
        Self::with_engine(
            artifact_dir,
            Arc::new(WatRuntimeCompiler),
            Arc::new(WasmtimePluginExecutor::with_budget(
                fuel_budget,
                memory_limit_bytes,
            )),
        )
    }

    pub fn with_engine(
        artifact_dir: impl Into<PathBuf>,
        compiler: Arc<dyn RuntimeCompiler>,
        executor: Arc<dyn PluginExecutor>,
    ) -> Self {
        Self {
            artifact_dir: artifact_dir.into(),
            compiler,
            executor,
            plugins: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub fn install(
        &self,
        request: InstallPluginRequest,
    ) -> Result<InstallPluginResponse, PluginError> {
        validate_manifest(&request.manifest)?;

        let started = Instant::now();
        let source_blake3 = blake3_string(request.source.as_bytes());
        let manifest_payload = serde_json::to_vec(&request.manifest)?;
        let manifest_blake3 = blake3_string(&manifest_payload);
        let runtime = self.compiler.compile(RuntimeCompilerInput {
            source: &request.source,
            manifest: &request.manifest,
            source_blake3: &source_blake3,
            manifest_blake3: &manifest_blake3,
        })?;

        let runtime_blake3 = blake3_string(&runtime.bytes);
        let runtime_hex = runtime_blake3
            .strip_prefix("blake3:")
            .expect("blake3_string returns canonical prefix");
        let runtime_path = self.artifact_dir.join(format!("{runtime_hex}.cwasm"));

        fs::create_dir_all(&self.artifact_dir)?;
        fs::write(&runtime_path, &runtime.bytes)?;

        let unit_kinds = request.manifest.contributes.declared_unit_kinds();
        let runtime_artifact = RuntimeArtifact {
            kind: runtime.kind.clone(),
            path: runtime_path.clone(),
            blake3: runtime_blake3.clone(),
            bytes: runtime.bytes.len() as u64,
        };
        let mut internal = InternalPluginManifest {
            id: request.manifest.id.clone(),
            name: request.manifest.name,
            version: request.manifest.version,
            source_blake3,
            manifest_blake3,
            runtime: runtime_artifact,
            abi_version: ABI_VERSION.to_owned(),
            callbacks: request.manifest.callbacks,
            capabilities: request.manifest.capabilities,
            contributes: request.manifest.contributes,
            unit_kinds,
            install_load_ms: 0.0,
            installed_at_unix_ms: unix_ms(),
        };
        let load_started = Instant::now();
        let loaded_runtime = self.executor.load(PluginLoadInput {
            manifest: &internal,
        })?;
        internal.install_load_ms = elapsed_ms(load_started);

        self.plugins
            .write()
            .map_err(|_| PluginError::Request("plugin registry lock poisoned".to_owned()))?
            .insert(
                internal.id.clone(),
                InstalledPlugin {
                    manifest: internal.clone(),
                    runtime: loaded_runtime,
                },
            );

        Ok(InstallPluginResponse {
            ok: true,
            plugin_id: internal.id.clone(),
            compile_ms: elapsed_ms(started),
            load_ms: internal.install_load_ms,
            runtime_blake3,
            runtime_artifact_path: runtime_path,
            runtime_kind: runtime.kind,
            runtime_bytes: runtime.bytes.len() as u64,
            manifest: internal,
        })
    }

    pub fn run(&self, request: RunPluginRequest) -> Result<RunPluginResponse, PluginError> {
        if request.runs == 0 || request.runs > 25 {
            return Err(PluginError::Request("runs must be from 1 to 25".to_owned()));
        }

        let installed = self
            .plugins
            .read()
            .map_err(|_| PluginError::Request("plugin registry lock poisoned".to_owned()))?
            .get(&request.plugin_id)
            .cloned()
            .ok_or_else(|| {
                PluginError::Request(format!("plugin is not installed: {}", request.plugin_id))
            })?;

        let functions = if request.functions.is_empty() {
            installed.manifest.callbacks.clone()
        } else {
            for function in &request.functions {
                if !installed.manifest.callbacks.contains(function) {
                    return Err(PluginError::Request(format!(
                        "callback is not declared by plugin: {function}"
                    )));
                }
            }
            request.functions
        };

        let load_started = Instant::now();
        let runtime_bytes = installed.manifest.runtime.bytes;
        let load_ms = elapsed_ms(load_started);

        let input_object_blake3 = blake3_json(&request.object)?;
        let context_blake3 = blake3_json(&request.context)?;
        let run_started = Instant::now();
        let mut traces = Vec::new();
        for iteration in 1..=request.runs {
            for function in &functions {
                let call_started = Instant::now();
                let result = installed.runtime.call(PluginCallInput {
                    manifest: &installed.manifest,
                    function,
                    object: &request.object,
                    context: &request.context,
                })?;
                let result_bytes = serde_json::to_vec(&result)?.len();
                let output_object_blake3 = blake3_json(&result)?;
                traces.push(FunctionRunTrace {
                    function: function.clone(),
                    iteration,
                    run_ms: elapsed_ms(call_started),
                    input_object_blake3: input_object_blake3.clone(),
                    context_blake3: context_blake3.clone(),
                    output_object_blake3,
                    decision: result.get("decision").cloned().unwrap_or(Value::Null),
                    patch_count: result
                        .get("patches")
                        .and_then(Value::as_array)
                        .map_or(0, Vec::len),
                    finding_count: result
                        .get("findings")
                        .and_then(Value::as_array)
                        .map_or(0, Vec::len),
                    result,
                    result_bytes,
                });
            }
        }

        let result = traces
            .last()
            .map(|trace| trace.result.clone())
            .unwrap_or_else(|| request.object.clone());
        let output_object_blake3 = blake3_json(&result)?;

        Ok(RunPluginResponse {
            ok: true,
            plugin_id: installed.manifest.id,
            runtime_blake3: installed.manifest.runtime.blake3,
            load_ms,
            run_ms: elapsed_ms(run_started),
            runtime_bytes,
            input_object_blake3,
            context_blake3,
            output_object_blake3,
            result,
            traces,
        })
    }

    pub fn install_run(
        &self,
        request: InstallRunRequest,
    ) -> Result<InstallRunResponse, PluginError> {
        let plugin_id = request.manifest.id.clone();
        let install = self.install(InstallPluginRequest {
            source: request.source,
            manifest: request.manifest,
        })?;
        let run = self.run(RunPluginRequest {
            plugin_id,
            object: request.object,
            context: request.context,
            functions: request.functions,
            runs: request.runs,
        })?;

        Ok(InstallRunResponse {
            ok: true,
            install,
            run,
        })
    }

    pub fn artifact_dir(&self) -> &Path {
        &self.artifact_dir
    }
}

fn default_runs() -> u32 {
    1
}

fn validate_manifest(manifest: &PublicPluginManifest) -> Result<(), PluginError> {
    if manifest.id.len() < 2
        || manifest.id.len() > 64
        || !manifest
            .id
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-'))
    {
        return Err(PluginError::Request(
            "manifest.id must be 2-64 URL-safe characters".to_owned(),
        ));
    }
    if manifest.name.trim().is_empty() {
        return Err(PluginError::Request(
            "manifest.name must be non-empty".to_owned(),
        ));
    }
    if manifest.version.trim().is_empty() {
        return Err(PluginError::Request(
            "manifest.version must be non-empty".to_owned(),
        ));
    }
    if manifest.callbacks.is_empty() {
        return Err(PluginError::Request(
            "manifest.callbacks must be non-empty".to_owned(),
        ));
    }
    validate_contribution_paths("rules", &manifest.contributes.rules)?;
    validate_contribution_paths("tools", &manifest.contributes.tools)?;
    validate_contribution_paths("skills", &manifest.contributes.skills)?;
    validate_contribution_paths("ui", &manifest.contributes.ui)?;
    Ok(())
}

fn validate_contribution_paths(kind: &str, paths: &[String]) -> Result<(), PluginError> {
    for path in paths {
        if path.trim().is_empty() || path.starts_with('/') || path.contains("..") {
            return Err(PluginError::Request(format!(
                "contributes.{kind} paths must be non-empty relative paths without '..'"
            )));
        }
    }
    Ok(())
}

fn blake3_json<T: Serialize>(payload: &T) -> Result<String, PluginError> {
    Ok(blake3_string(&serde_json::to_vec(payload)?))
}

fn blake3_string(payload: &[u8]) -> String {
    format!("blake3:{}", blake3::hash(payload).to_hex())
}

fn unix_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or_default()
}

fn elapsed_ms(started: Instant) -> f64 {
    let duration = started.elapsed();
    duration.as_secs_f64() * 1000.0
}
