//! Centralized tracing bootstrap for every capsem binary.
//!
//! Each binary calls `init(TelemetryConfig { ... })` exactly once at startup
//! and holds the returned `TelemetryGuard` for the lifetime of `main()`. The
//! shape of the JSON tracing layer, the env-filter default, and the file/
//! stderr sink lives here -- not in eight copies across eight `main.rs`
//! files.
//!
//! OpenTelemetry layer is intentionally NOT wired this sprint. The function
//! captures `TRACEPARENT` from env and stashes it in a process-global
//! [`OnceLock`] so [`current_parent_traceparent`] and
//! [`ambient_capsem_trace_id`] can return it for in-band propagation (W4/W5)
//! without requiring an OTel runtime dependency. Adding the OTLP exporter
//! later is a layer addition; the API stays stable.

use std::path::PathBuf;
use std::sync::OnceLock;

use tracing::Subscriber;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::registry::LookupSpan;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{fmt, EnvFilter, Layer};

/// Where the binary's tracing output goes. The choice is per-binary, not
/// per-build-time: capsem-service writes to `~/.capsem/run/service.log`,
/// capsem-process writes to stderr (its parent reaps stderr), capsem-app
/// writes to both a file and stderr (the file feeds the support bundle,
/// stderr feeds the dev's terminal).
pub enum LogSink {
    /// Write JSON-per-line to stderr. Used by short-lived companion
    /// subprocesses whose parent reaps stderr (capsem-process,
    /// capsem-mcp-aggregator, capsem-mcp-builtin).
    Stderr,
    /// Write JSON-per-line to a daily-rotated stream derived from `path`.
    /// Used by long-lived daemons whose log is consumed from disk (service,
    /// mcp, gateway, tray). `path` names the stream, not a single file:
    /// `<run>/service.log` produces `service.<date>.log`, bounded to
    /// [`LOG_FILES_RETAINED`]. Read one back with [`log_stream_files`].
    File { path: PathBuf },
    /// File (json) + stderr (pretty). Used by capsem-app so the file
    /// feeds the support bundle and stderr feeds `pnpm tauri dev` output.
    FileAndPretty { path: PathBuf },
}

/// Static per-binary telemetry config. `service` is the binary name (also
/// used as the OTel resource service.name when the OTel layer ships).
/// `default_filter` is the [`RUST_LOG`] fallback (e.g. `"capsem_service=info"`).
pub struct TelemetryConfig {
    pub service: &'static str,
    pub sink: LogSink,
    pub default_filter: &'static str,
}

/// Subsystem-target directives that every capsem binary should accept at
/// `info` level by default. We use `target: "suspend"` / `"fs"` / `"ipc"`
/// / `"host"` / `"handshake"` as semantic categories on info!() calls so
/// individual subsystems can be filtered or grepped (e.g. `RUST_LOG=ipc=debug`
/// turns up only IPC-layer noise). Without these directives in the
/// effective `EnvFilter`, the default `capsem=info` filter silently
/// discards them all because the targets don't start with `capsem`.
///
/// This constant is the canonical list. Both the per-binary `default_filter`
/// in [`TelemetryConfig`] and the `RUST_LOG` env var that capsem-service
/// passes to spawned children should be built using
/// [`with_subsys_targets`] to keep the list in one place.
pub const SUBSYS_TARGETS: &str =
    "suspend=info,fs=info,ipc=info,host=info,handshake=info,vsock=info";

/// Enables local debug spans/metrics for benchmark and release triage.
///
/// Accepted true values: `1`, `true`, `yes`, `on`, `local`, `debug`.
/// This switch widens local tracing filters only; it does not create an OTLP
/// exporter.
pub const DEBUG_TELEMETRY_ENV: &str = "CAPSEM_DEBUG_TELEMETRY";

/// Explicit escape hatch for future lab-only upstream OTEL exporter work.
///
/// This is intentionally not a normal user-facing knob. Without it, OTLP
/// endpoint/exporter env vars are reported as blocked and ignored by Capsem's
/// telemetry bootstrap.
pub const ALLOW_UPSTREAM_OTEL_ENV: &str = "CAPSEM_ALLOW_UPSTREAM_OTEL";

/// Local debug tracing directives used when [`DEBUG_TELEMETRY_ENV`] is enabled.
pub const DEBUG_TELEMETRY_TARGETS: &str = concat!(
    "capsem.mitm=debug,",
    "capsem.security_event=debug,",
    "capsem.db=debug,",
    "capsem.launch=debug,",
    "mitm.hook=debug,",
    "mitm.hook.chunk=debug"
);

pub const LAUNCH_SERVICE_SPAN: &str = "capsem.launch.service";
pub const LAUNCH_GATEWAY_SPAN: &str = "capsem.launch.gateway";
pub const LAUNCH_PROCESS_SPAWN_SPAN: &str = "capsem.launch.process_spawn";
pub const LAUNCH_VM_BOOT_SPAN: &str = "capsem.launch.vm_boot";
pub const LAUNCH_VSOCK_READY_SPAN: &str = "capsem.launch.vsock_ready";
pub const LAUNCH_FIRST_NETWORK_READY_SPAN: &str = "capsem.launch.first_network_ready";

const UPSTREAM_OTEL_ENV_VARS: &[&str] = &[
    "OTEL_EXPORTER_OTLP_ENDPOINT",
    "OTEL_EXPORTER_OTLP_TRACES_ENDPOINT",
    "OTEL_EXPORTER_OTLP_METRICS_ENDPOINT",
    "OTEL_TRACES_EXPORTER",
    "OTEL_METRICS_EXPORTER",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DebugTelemetryPolicy {
    pub local_debug_enabled: bool,
    pub upstream_export_allowed: bool,
    pub blocked_upstream_env: Vec<String>,
}

/// Compose a filter string by appending [`SUBSYS_TARGETS`] to a base.
/// Use for `TelemetryConfig::default_filter` and for `RUST_LOG=...` env
/// vars passed to spawned children.
///
/// Example: `with_subsys_targets("capsem=info")` ->
/// `"capsem=info,suspend=info,fs=info,ipc=info,host=info,handshake=info,vsock=info"`.
pub fn with_subsys_targets(base: &str) -> String {
    if base.is_empty() {
        SUBSYS_TARGETS.to_string()
    } else {
        format!("{base},{SUBSYS_TARGETS}")
    }
}

/// Files retained per log stream, i.e. days of history at daily rotation.
///
/// A week covers a weekend plus the round-trip of a user noticing a problem
/// and running `capsem support-bundle`. It is also the reason these files are
/// bounded at all: an append-forever sink reached 314 MB of gateway request
/// tracing in under a month, which cost far more disk than it ever bought
/// back in answers.
pub const LOG_FILES_RETAINED: usize = 7;

/// Split a logical log path into the `(directory, prefix, suffix)` a rolling
/// appender needs.
///
/// `<run>/service.log` becomes `<run>`, `service`, `log`, so a rotated file is
/// `service.2026-07-30.log` rather than `service.log.2026-07-30`. Keeping the
/// extension last matters: the asset gate's failure-evidence copy and every
/// operator reflex filter on `*.log`, and a rotated file that stops matching
/// is a file nobody collects.
fn rolling_parts(path: &std::path::Path) -> (PathBuf, String, String) {
    let dir = match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent.to_path_buf(),
        _ => PathBuf::from("."),
    };
    let prefix = path
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_else(|| "capsem".to_string());
    let suffix = path
        .extension()
        .map(|ext| ext.to_string_lossy().into_owned())
        .unwrap_or_else(|| "log".to_string());
    (dir, prefix, suffix)
}

/// Build the rotating appender behind a [`LogSink::File`] sink.
fn rolling_appender(
    path: &std::path::Path,
) -> std::io::Result<tracing_appender::rolling::RollingFileAppender> {
    let (dir, prefix, suffix) = rolling_parts(path);
    std::fs::create_dir_all(&dir)?;
    tracing_appender::rolling::Builder::new()
        .rotation(tracing_appender::rolling::Rotation::DAILY)
        .filename_prefix(prefix)
        .filename_suffix(suffix)
        .max_log_files(LOG_FILES_RETAINED)
        .build(&dir)
        .map_err(std::io::Error::other)
}

/// Every file belonging to one log stream, newest first.
///
/// The single way to answer "where are this binary's logs": rotation means
/// there is no longer one filename, an install that predates rotation still
/// has its unrotated `service.log`, and raw stderr (panics, death before
/// [`init`] returns) lands in that same unrotated file. Callers that read
/// logs -- the support bundle above all -- must go through here rather than
/// reconstruct a filename, or they silently ship part of the history.
pub fn log_stream_files(path: &std::path::Path) -> Vec<PathBuf> {
    let (dir, prefix, suffix) = rolling_parts(path);
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut files: Vec<(std::time::SystemTime, PathBuf)> = entries
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            let name = path.file_name()?.to_str()?;
            // `service.log` and `service.<date>.log`, never `services.log`.
            let in_stream = name == format!("{prefix}.{suffix}")
                || (name.starts_with(&format!("{prefix}."))
                    && name.ends_with(&format!(".{suffix}")));
            if !in_stream || !entry.file_type().ok()?.is_file() {
                return None;
            }
            Some((entry.metadata().ok()?.modified().ok()?, path))
        })
        .collect();
    files.sort_by(|a, b| b.0.cmp(&a.0));
    files.into_iter().map(|(_, path)| path).collect()
}

/// Read the last `max_bytes` of a rotated log stream, oldest line first.
///
/// `path` names the stream (`<run>/service.log`), not a file on disk. Every
/// caller wanting "the recent log" needs the same three steps -- resolve the
/// stream, walk back through rotated files until the tail is full, trim to a
/// line boundary -- so they live here once. Open the stream name directly and
/// you get nothing: that is how `/service-logs` came to report an empty log
/// for a service that was writing normally.
///
/// `None` when the stream has no files at all, which is a different answer
/// from an empty log and should be reported differently.
pub fn read_log_tail(path: &std::path::Path, max_bytes: usize) -> Option<String> {
    use std::io::{Read, Seek, SeekFrom};

    let files = log_stream_files(path);
    if files.is_empty() {
        return None;
    }

    // Seek to each file's tail rather than reading it and slicing. Guest
    // console output grows on the guest's terms, so reading whole files to
    // return the last few MiB would let a chatty VM choose how much memory the
    // caller allocates.
    let mut newest_first: Vec<Vec<u8>> = Vec::new();
    let mut budget = max_bytes;
    for file in files {
        if budget == 0 {
            break;
        }
        let Ok(mut handle) = std::fs::File::open(&file) else {
            continue;
        };
        let Ok(meta) = handle.metadata() else {
            continue;
        };
        if !meta.is_file() {
            continue;
        }
        let len = meta.len();
        let take = budget.min(len as usize);
        if (len as usize) > take && handle.seek(SeekFrom::Start(len - take as u64)).is_err() {
            continue;
        }
        let mut buf = Vec::with_capacity(take);
        if handle.take(take as u64).read_to_end(&mut buf).is_err() {
            continue;
        }
        budget -= buf.len().min(budget);
        newest_first.push(buf);
    }
    if newest_first.is_empty() {
        return None;
    }
    newest_first.reverse();
    let joined = newest_first.concat();

    // Lossy once, at the boundary: a seek can land mid-character, and a
    // partial first line is noise rather than data.
    let mut text = String::from_utf8_lossy(&joined).into_owned();
    if joined.len() >= max_bytes {
        if let Some(newline) = text.find('\n') {
            text = text[newline + 1..].to_string();
        }
    }
    Some(text)
}

/// Route panics into the tracing sink instead of stderr.
///
/// A daemon's stderr belongs to whoever spawned it -- launchd, systemd, or a
/// parent that redirected it -- so an unhooked panic lands somewhere the
/// binary's own log is not, or nowhere at all. This is the one failure whose
/// message a user cannot get any other way: by the time the process is gone,
/// tracing has stopped and the panic text is all there was.
///
/// Call once, after [`init`], so the hook has a subscriber to write to.
pub fn install_panic_logger(service: &'static str) {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        tracing::error!(
            target: "service",
            service,
            panic = %info,
            location = info.location().map(|l| l.to_string()).unwrap_or_default(),
            "process panicked"
        );
        previous(info);
    }));
}

/// Hold this guard for the lifetime of `main`. Drop flushes any
/// non-blocking file writer and (in a future sprint) the OTLP exporter.
pub struct TelemetryGuard {
    #[allow(dead_code)]
    file_guard: Option<tracing_appender::non_blocking::WorkerGuard>,
}

/// Process-global parent traceparent captured from the `TRACEPARENT` env
/// var at startup. W4/W5 read this for in-band propagation. Empty when
/// unset (CLI invocations and top-level binaries).
static PARENT_TRACEPARENT: OnceLock<String> = OnceLock::new();

/// Initialize tracing. Call exactly once per binary, in `main()`, before
/// any `tracing::info!` macro fires.
///
/// This consumes the [`TRACEPARENT`] env var (if set) and stashes it for
/// in-band propagation. Children spawned by this binary read it back via
/// [`current_parent_traceparent`].
pub fn init(cfg: TelemetryConfig) -> std::io::Result<TelemetryGuard> {
    if let Ok(tp) = std::env::var("TRACEPARENT") {
        if !tp.is_empty() {
            let _ = PARENT_TRACEPARENT.set(tp);
        }
    }

    let debug_policy = current_debug_telemetry_policy();
    let default_filter = default_filter_with_debug_telemetry(cfg.default_filter, &debug_policy);

    // Prepend `service=info` so the synthetic `service.start` line below
    // always reaches the sink, even when callers pass a narrow default
    // filter like `"capsem_gateway=info,tower_http=debug,hyper=info"`. A
    // user override via the `RUST_LOG` env var keeps full control.
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(format!("service=info,{default_filter}")));

    let registry = tracing_subscriber::registry().with(filter);
    let mut file_guard: Option<tracing_appender::non_blocking::WorkerGuard> = None;

    match cfg.sink {
        LogSink::Stderr => {
            registry
                .with(fmt::layer().json().with_writer(std::io::stderr).boxed())
                .init();
        }
        LogSink::File { path } => {
            let (nb, guard) = tracing_appender::non_blocking(rolling_appender(&path)?);
            file_guard = Some(guard);
            registry
                .with(fmt::layer().json().with_writer(nb).boxed())
                .init();
        }
        LogSink::FileAndPretty { path } => {
            let (nb, guard) = tracing_appender::non_blocking(rolling_appender(&path)?);
            file_guard = Some(guard);
            registry
                .with(fmt::layer().json().with_writer(nb).boxed())
                .with(stderr_pretty_layer())
                .init();
        }
    }

    // Once the subscriber is wired, emit a "service started" line that
    // includes the protocol version + (in W3) the schema_hash so a support
    // bundle parser can detect cross-version mixes immediately.
    tracing::info!(
        target: "service",
        service = cfg.service,
        protocol_version = capsem_proto::PROTOCOL_VERSION,
        schema_hash = format!("{:016x}", capsem_proto::SCHEMA_HASH),
        parent_traceparent = current_parent_traceparent(),
        debug_telemetry_local = debug_policy.local_debug_enabled,
        "service.start",
    );
    if !debug_policy.blocked_upstream_env.is_empty() {
        tracing::warn!(
            target: "service",
            service = cfg.service,
            blocked_env = ?debug_policy.blocked_upstream_env,
            allow_env = ALLOW_UPSTREAM_OTEL_ENV,
            "upstream OTEL exporter env ignored; Capsem debug telemetry is local-only by default",
        );
    }

    Ok(TelemetryGuard { file_guard })
}

fn stderr_pretty_layer<S>() -> Box<dyn Layer<S> + Send + Sync + 'static>
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    fmt::layer().with_writer(std::io::stderr).boxed()
}

/// W3C traceparent inherited from the parent process via the `TRACEPARENT`
/// env var, or `""` if this binary is the top of the trace tree.
pub fn current_parent_traceparent() -> &'static str {
    PARENT_TRACEPARENT.get().map(String::as_str).unwrap_or("")
}

pub fn current_debug_telemetry_policy() -> DebugTelemetryPolicy {
    debug_telemetry_policy_from_pairs(std::env::vars())
}

pub fn debug_telemetry_policy_from_pairs<I, K, V>(vars: I) -> DebugTelemetryPolicy
where
    I: IntoIterator<Item = (K, V)>,
    K: AsRef<str>,
    V: AsRef<str>,
{
    let vars: std::collections::HashMap<String, String> = vars
        .into_iter()
        .map(|(key, value)| (key.as_ref().to_string(), value.as_ref().to_string()))
        .collect();
    let local_debug_enabled = vars
        .get(DEBUG_TELEMETRY_ENV)
        .is_some_and(|value| env_truthy(value));
    let upstream_export_allowed = vars
        .get(ALLOW_UPSTREAM_OTEL_ENV)
        .is_some_and(|value| env_truthy(value));
    let blocked_upstream_env = if upstream_export_allowed {
        Vec::new()
    } else {
        UPSTREAM_OTEL_ENV_VARS
            .iter()
            .filter(|key| vars.get(**key).is_some_and(|value| !value.is_empty()))
            .map(|key| (*key).to_string())
            .collect()
    };
    DebugTelemetryPolicy {
        local_debug_enabled,
        upstream_export_allowed,
        blocked_upstream_env,
    }
}

pub fn default_filter_with_debug_telemetry(base: &str, policy: &DebugTelemetryPolicy) -> String {
    if policy.local_debug_enabled {
        format!("{base},{DEBUG_TELEMETRY_TARGETS}")
    } else {
        base.to_string()
    }
}

fn env_truthy(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "1" | "true" | "yes" | "on" | "local" | "debug"
    )
}

/// Extract just the trace-id (16 hex chars, the lower half of the W3C
/// trace-id) from the parent traceparent. Returns `None` if no parent.
///
/// Format: `00-<32-hex>-<16-hex>-<2-hex>` per W3C Trace Context.
/// We return the LOWER 16 hex chars of the 32-hex trace-id so it lines up
/// with the existing `CAPSEM_TRACE_ID` 16-hex convention -- one fewer
/// representation to remember when grepping.
pub fn ambient_capsem_trace_id() -> Option<String> {
    let env = std::env::var("CAPSEM_TRACE_ID").ok();
    resolve_ambient_capsem_trace_id(env.as_deref(), PARENT_TRACEPARENT.get().map(String::as_str))
}

fn resolve_ambient_capsem_trace_id(
    capsem_trace_id: Option<&str>,
    parent_traceparent: Option<&str>,
) -> Option<String> {
    if let Some(env) = capsem_trace_id {
        if !env.is_empty() {
            return Some(env.to_string());
        }
    }
    let tp = parent_traceparent?;
    let mut parts = tp.split('-');
    let _version = parts.next()?;
    let trace_id = parts.next()?;
    if trace_id.len() < 16 {
        return None;
    }
    Some(trace_id[trace_id.len() - 16..].to_string())
}

/// Build the env-var pairs that propagate the current trace context to
/// a child process. Caller does `cmd.envs(child_trace_env(vm_id))`.
///
/// Sets four pairs:
///   - `CAPSEM_VM_ID`     -- our existing convention
///   - `CAPSEM_TRACE_ID`  -- 16-hex grep-friendly id
///   - `TRACEPARENT`      -- W3C Trace Context: `00-<32hex>-<16hex>-01`
///   - `TRACESTATE`       -- W3C tracestate (always empty for now)
///
/// If we already have a parent traceparent (we're a child of another
/// capsem-* binary), we propagate it unchanged so the whole tree shares
/// one trace_id. If we don't, we synthesize a fresh one from a random
/// 16-hex span_id and a 32-hex trace_id derived from `vm_id` + a random
/// suffix so each VM gets a deterministic-looking trace anchor.
pub fn child_trace_env(vm_id: &str) -> Vec<(String, String)> {
    let mut out = vec![("CAPSEM_VM_ID".to_string(), vm_id.to_string())];

    if let Some(parent_tp) = PARENT_TRACEPARENT.get() {
        // Parent already provided a traceparent -- propagate verbatim.
        if let Some(trace_id) = ambient_capsem_trace_id() {
            out.push(("CAPSEM_TRACE_ID".to_string(), trace_id));
        }
        out.push(("TRACEPARENT".to_string(), parent_tp.clone()));
        out.push(("TRACESTATE".to_string(), String::new()));
        return out;
    }

    // Top-of-tree: synthesize a fresh trace context. The 16-hex
    // CAPSEM_TRACE_ID stays the lower half of the 32-hex W3C trace_id
    // so a future OTel layer doesn't need a separate id space.
    let lower16 = synthesize_16hex_id(vm_id);
    let upper16 = synthesize_16hex_id(&format!("{vm_id}-upper"));
    let span_id = synthesize_16hex_id(&format!("{vm_id}-span"));
    let trace_id_32 = format!("{upper16}{lower16}");
    let traceparent = format!("00-{trace_id_32}-{span_id}-01");

    out.push(("CAPSEM_TRACE_ID".to_string(), lower16));
    out.push(("TRACEPARENT".to_string(), traceparent));
    out.push(("TRACESTATE".to_string(), String::new()));
    out
}

/// Cheap 16-hex-char id derived from a seed. Uses blake3 for a stable,
/// well-distributed mapping; deterministic so tests can exercise it.
fn synthesize_16hex_id(seed: &str) -> String {
    // Mix in process-startup nanos so two independent capsem-service
    // launches don't collide on the same vm_id.
    let salt = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let mut hasher = blake3::Hasher::new();
    hasher.update(seed.as_bytes());
    hasher.update(&salt.to_le_bytes());
    let hash = hasher.finalize();
    hash.to_hex().chars().take(16).collect()
}

#[cfg(test)]
mod tests;
