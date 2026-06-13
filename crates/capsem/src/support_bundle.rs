//! `capsem support-bundle` -- gather host logs, recent session
//! telemetry, configs, and version info into a single redacted tar.gz.
//!
//! Layout (rooted in the tar entries' top-level directory
//! `capsem-support-<ts>-<host>/`):
//!
//! ```text
//! manifest.json                          # entry point
//! host/{service,mcp,gateway,tray}.log    # last 5MB each
//! host/app/<latest 3>.jsonl
//! host/run-snapshot/{service.pid,gateway.pid,gateway.port}
//! sessions/<id>/{session.db,serial.log,process.log,metadata.json,...}
//! assets/manifest.json                   # ~/.capsem/assets/manifest.json
//! config/{settings.toml,corp.toml}       # secrets redacted
//! system/{version.json,os.txt,proxy.json,dmesg.log,mitm-ca-fingerprint.txt}
//! ```
//!
//! The MITM CA cert itself is NEVER bundled; we ship a SHA-256 of its
//! bytes instead so a maintainer can confirm "yes this user is on the
//! right CA" without seeing the cert.
//!
//! Manifest schema is v1; bump `SCHEMA_VERSION` for breaking changes.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use anyhow::{Context, Result};
use flate2::write::GzEncoder;
use flate2::Compression;
use serde::Serialize;
use tar::Builder as TarBuilder;

use crate::support::redact;

const SCHEMA_VERSION: u32 = 1;
const MAX_LOG_TAIL_BYTES: u64 = 5 * 1024 * 1024;
const MAX_SESSIONS: usize = 10;

/// Bundle options. Use `Default::default()` for the legacy three-flag
/// signature; `max_session_bytes = 0` disables the cap.
pub struct Opts {
    pub output: Option<PathBuf>,
    pub sessions: usize,
    pub include_rootfs: bool,
    pub no_redact: bool,
    pub max_session_bytes: u64,
}

impl Default for Opts {
    fn default() -> Self {
        Self {
            output: None,
            sessions: 3,
            include_rootfs: false,
            no_redact: false,
            max_session_bytes: 50 * 1024 * 1024,
        }
    }
}

/// Public entry: build a support bundle and return the path written.
/// Backwards-compat wrapper over [`run_with_opts`]. Test-only today;
/// the CLI dispatch goes through `run_with_opts` directly.
#[allow(dead_code)]
pub fn run(
    output: Option<PathBuf>,
    sessions: usize,
    include_rootfs: bool,
    no_redact: bool,
) -> Result<PathBuf> {
    run_with_opts(Opts {
        output,
        sessions,
        include_rootfs,
        no_redact,
        ..Default::default()
    })
}

pub fn run_with_opts(opts: Opts) -> Result<PathBuf> {
    let Opts {
        output,
        sessions,
        include_rootfs,
        no_redact,
        max_session_bytes,
    } = opts;
    let sessions = sessions.min(MAX_SESSIONS);
    // F8: drop oldest sessions when their session.db total exceeds the
    // cap. The pre-tar measurement is approximate (we read sizes via
    // metadata rather than from the DB itself) but adequate for the
    // user-facing "stays attachable to bug reports" goal.
    let _max_session_bytes = max_session_bytes; // consumed in the session-include loop below
    let home = capsem_core::paths::capsem_home();
    let support_dir = home.join("support");
    fs::create_dir_all(&support_dir)
        .with_context(|| format!("create {}", support_dir.display()))?;

    let timestamp = ts_filename();
    let host = host_label();
    let bundle_root = format!("capsem-support-{timestamp}-{host}");
    let output = output.unwrap_or_else(|| support_dir.join(format!("{bundle_root}.tar.gz")));

    let file = fs::File::create(&output).with_context(|| format!("create {}", output.display()))?;
    let gz = GzEncoder::new(file, Compression::default());
    let mut tar = TarBuilder::new(gz);

    let mut sections: Vec<Section> = Vec::new();
    let mut warnings: Vec<String> = Vec::new();

    // -- host logs --
    let run_dir = capsem_core::paths::capsem_run_dir();
    for name in ["service", "mcp", "gateway", "tray"] {
        let path = run_dir.join(format!("{name}.log"));
        let entry_path = format!("{bundle_root}/host/{name}.log");
        match read_tail(&path, MAX_LOG_TAIL_BYTES) {
            Some(bytes) => {
                let bytes = if no_redact {
                    bytes
                } else {
                    redact_log_bytes(&bytes)
                };
                let len = bytes.len() as u64;
                add_bytes(&mut tar, &entry_path, &bytes)?;
                sections.push(Section {
                    path: entry_path,
                    kind: "log",
                    bytes: Some(len),
                    missing: false,
                    reason: None,
                    truncated_to_last_bytes: if path.metadata().map(|m| m.len()).unwrap_or(0)
                        > MAX_LOG_TAIL_BYTES
                    {
                        Some(MAX_LOG_TAIL_BYTES)
                    } else {
                        None
                    },
                });
            }
            None => {
                sections.push(Section {
                    path: entry_path,
                    kind: "log",
                    bytes: None,
                    missing: true,
                    reason: Some(format!("file-not-found: {}", path.display())),
                    truncated_to_last_bytes: None,
                });
            }
        }
    }

    // -- app logs (newest 3 jsonl) --
    let app_log_dir = home.join("logs");
    if let Ok(read) = fs::read_dir(&app_log_dir) {
        let mut entries: Vec<_> = read
            .flatten()
            .filter(|e| e.path().extension().and_then(|s| s.to_str()) == Some("jsonl"))
            .collect();
        entries.sort_by_key(|e| {
            std::cmp::Reverse(
                e.metadata()
                    .and_then(|m| m.modified())
                    .unwrap_or(SystemTime::UNIX_EPOCH),
            )
        });
        for entry in entries.into_iter().take(3) {
            let p = entry.path();
            let name = p
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("unnamed.jsonl")
                .to_string();
            if let Some(bytes) = read_tail(&p, MAX_LOG_TAIL_BYTES) {
                let bytes = if no_redact {
                    bytes
                } else {
                    redact_log_bytes(&bytes)
                };
                let len = bytes.len() as u64;
                let entry_path = format!("{bundle_root}/host/app/{name}");
                add_bytes(&mut tar, &entry_path, &bytes)?;
                sections.push(Section {
                    path: entry_path,
                    kind: "log-jsonl",
                    bytes: Some(len),
                    missing: false,
                    reason: None,
                    truncated_to_last_bytes: None,
                });
            }
        }
    }

    // -- run-snapshot pids/port (NOT gateway.token) --
    for name in ["service.pid", "gateway.pid", "gateway.port"] {
        let path = run_dir.join(name);
        let entry_path = format!("{bundle_root}/host/run-snapshot/{name}");
        match fs::read(&path) {
            Ok(bytes) => {
                let len = bytes.len() as u64;
                add_bytes(&mut tar, &entry_path, &bytes)?;
                sections.push(Section {
                    path: entry_path,
                    kind: "metadata",
                    bytes: Some(len),
                    missing: false,
                    reason: None,
                    truncated_to_last_bytes: None,
                });
            }
            Err(_) => {
                sections.push(Section {
                    path: entry_path,
                    kind: "metadata",
                    bytes: None,
                    missing: true,
                    reason: Some("file-not-found".into()),
                    truncated_to_last_bytes: None,
                });
            }
        }
    }
    // Explicitly note that gateway.token is excluded.
    warnings.push("gateway.token is intentionally excluded from support bundles".into());

    // -- sessions --
    let mut included_session_ids: Vec<String> = Vec::new();
    let sessions_dir = home.join("sessions");
    if let Ok(read) = fs::read_dir(&sessions_dir) {
        let mut session_dirs: Vec<_> = read
            .flatten()
            .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
            .collect();
        session_dirs.sort_by_key(|e| {
            std::cmp::Reverse(
                e.metadata()
                    .and_then(|m| m.modified())
                    .unwrap_or(SystemTime::UNIX_EPOCH),
            )
        });
        let mut session_bytes_used: u64 = 0;
        for dir in session_dirs.into_iter().take(sessions) {
            // F8: skip this session if including its session.db would
            // exceed `max_session_bytes`. 0 disables the cap.
            if max_session_bytes > 0 {
                let db_size = std::fs::metadata(dir.path().join("session.db"))
                    .map(|m| m.len())
                    .unwrap_or(0);
                if session_bytes_used + db_size > max_session_bytes {
                    warnings.push(format!(
                        "session {} skipped: would exceed --max-session-bytes={max_session_bytes} cap",
                        dir.file_name().to_string_lossy()
                    ));
                    continue;
                }
                session_bytes_used += db_size;
            }
            let id = dir.file_name().to_string_lossy().to_string();
            included_session_ids.push(id.clone());
            for fname in ["session.db", "serial.log", "process.log", "metadata.json"] {
                let path = dir.path().join(fname);
                let entry_path = format!("{bundle_root}/sessions/{id}/{fname}");
                match fs::read(&path) {
                    Ok(bytes) => {
                        // session.db is binary; don't redact. Logs get redacted unless --no-redact.
                        let bytes = if !no_redact
                            && (fname.ends_with(".log") || fname.ends_with(".json"))
                        {
                            redact_log_bytes(&bytes)
                        } else {
                            bytes
                        };
                        let len = bytes.len() as u64;
                        add_bytes(&mut tar, &entry_path, &bytes)?;
                        sections.push(Section {
                            path: entry_path,
                            kind: if fname == "session.db" {
                                "sqlite"
                            } else {
                                "log"
                            },
                            bytes: Some(len),
                            missing: false,
                            reason: None,
                            truncated_to_last_bytes: None,
                        });
                    }
                    Err(_) => {
                        sections.push(Section {
                            path: entry_path,
                            kind: "log",
                            bytes: None,
                            missing: true,
                            reason: Some("file-not-found".into()),
                            truncated_to_last_bytes: None,
                        });
                    }
                }
            }
            if include_rootfs {
                let path = dir.path().join("guest").join("system").join("rootfs.img");
                if let Ok(bytes) = fs::read(&path) {
                    let len = bytes.len() as u64;
                    let entry_path = format!("{bundle_root}/sessions/{id}/rootfs.img");
                    add_bytes(&mut tar, &entry_path, &bytes)?;
                    sections.push(Section {
                        path: entry_path,
                        kind: "binary",
                        bytes: Some(len),
                        missing: false,
                        reason: None,
                        truncated_to_last_bytes: None,
                    });
                }
            } else {
                warnings.push(format!(
                    "rootfs.img for session {id} excluded (use --include-rootfs)"
                ));
            }
        }
    }

    // -- assets manifest --
    {
        let path = home.join("assets").join("manifest.json");
        let entry_path = format!("{bundle_root}/assets/manifest.json");
        if let Ok(bytes) = fs::read(&path) {
            let len = bytes.len() as u64;
            add_bytes(&mut tar, &entry_path, &bytes)?;
            sections.push(Section {
                path: entry_path,
                kind: "json",
                bytes: Some(len),
                missing: false,
                reason: None,
                truncated_to_last_bytes: None,
            });
        } else {
            sections.push(Section {
                path: entry_path,
                kind: "json",
                bytes: None,
                missing: true,
                reason: Some("file-not-found".into()),
                truncated_to_last_bytes: None,
            });
        }
    }
    {
        let path = home.join("assets").join("manifest-origin.json");
        let entry_path = format!("{bundle_root}/assets/manifest-origin.json");
        if let Ok(bytes) = fs::read(&path) {
            let len = bytes.len() as u64;
            add_bytes(&mut tar, &entry_path, &bytes)?;
            sections.push(Section {
                path: entry_path,
                kind: "json",
                bytes: Some(len),
                missing: false,
                reason: None,
                truncated_to_last_bytes: None,
            });
        } else {
            sections.push(Section {
                path: entry_path,
                kind: "json",
                bytes: None,
                missing: true,
                reason: Some("file-not-found".into()),
                truncated_to_last_bytes: None,
            });
        }
    }

    // -- configs (redacted) --
    for name in ["settings.toml", "corp.toml", "corp-source.json"] {
        let path = home.join(name);
        let entry_path = format!("{bundle_root}/config/{name}");
        if let Ok(text) = fs::read_to_string(&path) {
            let text = if no_redact {
                text
            } else {
                redact::redact_config_text(&text)
            };
            let bytes = text.into_bytes();
            let len = bytes.len() as u64;
            add_bytes(&mut tar, &entry_path, &bytes)?;
            sections.push(Section {
                path: entry_path,
                kind: "config",
                bytes: Some(len),
                missing: false,
                reason: None,
                truncated_to_last_bytes: None,
            });
        } else {
            sections.push(Section {
                path: entry_path,
                kind: "config",
                bytes: None,
                missing: true,
                reason: Some("file-not-found".into()),
                truncated_to_last_bytes: None,
            });
        }
    }

    // -- profile/corp diagnostics index --
    {
        let entry_path = format!("{bundle_root}/system/config-diagnostics.json");
        let diagnostics = config_diagnostics(&home);
        let bytes = serde_json::to_vec_pretty(&diagnostics)?;
        let len = bytes.len() as u64;
        add_bytes(&mut tar, &entry_path, &bytes)?;
        sections.push(Section {
            path: entry_path,
            kind: "json",
            bytes: Some(len),
            missing: false,
            reason: None,
            truncated_to_last_bytes: None,
        });
    }

    // -- runtime boundary/debug contract --
    {
        let entry_path = format!("{bundle_root}/system/runtime-boundary.json");
        let boundary = runtime_boundary_debug_contract();
        let bytes = serde_json::to_vec_pretty(&boundary)?;
        let len = bytes.len() as u64;
        add_bytes(&mut tar, &entry_path, &bytes)?;
        sections.push(Section {
            path: entry_path,
            kind: "json",
            bytes: Some(len),
            missing: false,
            reason: None,
            truncated_to_last_bytes: None,
        });
    }

    // -- system info --
    {
        let version_json = serde_json::json!({
            "binary": "capsem",
            "version": env!("CARGO_PKG_VERSION"),
            "build_ts": option_env!("CAPSEM_BUILD_TS").unwrap_or("dev"),
        });
        let entry_path = format!("{bundle_root}/system/version.json");
        let bytes = serde_json::to_vec_pretty(&version_json).unwrap();
        let len = bytes.len() as u64;
        add_bytes(&mut tar, &entry_path, &bytes)?;
        sections.push(Section {
            path: entry_path,
            kind: "json",
            bytes: Some(len),
            missing: false,
            reason: None,
            truncated_to_last_bytes: None,
        });
    }
    {
        let mut os = String::new();
        if let Ok(out) = std::process::Command::new("uname").arg("-a").output() {
            os.push_str("uname -a: ");
            os.push_str(&String::from_utf8_lossy(&out.stdout));
        }
        #[cfg(target_os = "macos")]
        if let Ok(out) = std::process::Command::new("sw_vers").output() {
            os.push_str("\n\n[sw_vers]\n");
            os.push_str(&String::from_utf8_lossy(&out.stdout));
        }
        #[cfg(target_os = "linux")]
        if let Ok(text) = std::fs::read_to_string("/etc/os-release") {
            os.push_str("\n\n[/etc/os-release]\n");
            os.push_str(&text);
        }
        let entry_path = format!("{bundle_root}/system/os.txt");
        let bytes = os.into_bytes();
        let len = bytes.len() as u64;
        add_bytes(&mut tar, &entry_path, &bytes)?;
        sections.push(Section {
            path: entry_path,
            kind: "text",
            bytes: Some(len),
            missing: false,
            reason: None,
            truncated_to_last_bytes: None,
        });
    }
    {
        let proxy = serde_json::json!({
            "HTTP_PROXY":  std::env::var("HTTP_PROXY").unwrap_or_default(),
            "HTTPS_PROXY": std::env::var("HTTPS_PROXY").unwrap_or_default(),
            "NO_PROXY":    std::env::var("NO_PROXY").unwrap_or_default(),
            "gateway_port_file": run_dir.join("gateway.port").display().to_string(),
        });
        let entry_path = format!("{bundle_root}/system/proxy.json");
        let bytes = serde_json::to_vec_pretty(&proxy).unwrap();
        let len = bytes.len() as u64;
        add_bytes(&mut tar, &entry_path, &bytes)?;
        sections.push(Section {
            path: entry_path,
            kind: "json",
            bytes: Some(len),
            missing: false,
            reason: None,
            truncated_to_last_bytes: None,
        });
    }
    #[cfg(target_os = "linux")]
    {
        let entry_path = format!("{bundle_root}/system/dmesg.log");
        if let Ok(out) = std::process::Command::new("dmesg").output() {
            let bytes: Vec<u8> = out
                .stdout
                .into_iter()
                .rev()
                .take(64 * 1024)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect();
            let len = bytes.len() as u64;
            add_bytes(&mut tar, &entry_path, &bytes)?;
            sections.push(Section {
                path: entry_path,
                kind: "log",
                bytes: Some(len),
                missing: false,
                reason: None,
                truncated_to_last_bytes: Some(64 * 1024),
            });
        }
    }
    #[cfg(not(target_os = "linux"))]
    {
        sections.push(Section {
            path: format!("{bundle_root}/system/dmesg.log"),
            kind: "log",
            bytes: None,
            missing: true,
            reason: Some("macos-not-applicable".into()),
            truncated_to_last_bytes: None,
        });
    }
    {
        let ca_path = home.join("config").join("capsem-ca.crt");
        let entry_path = format!("{bundle_root}/system/mitm-ca-fingerprint.txt");
        if let Ok(bytes) = fs::read(&ca_path) {
            let hash = blake3::hash(&bytes); // fast; not crypto -- this is just a fingerprint
            let s = format!("blake3: {}\n", hash.to_hex());
            let bytes = s.into_bytes();
            let len = bytes.len() as u64;
            add_bytes(&mut tar, &entry_path, &bytes)?;
            sections.push(Section {
                path: entry_path,
                kind: "fingerprint",
                bytes: Some(len),
                missing: false,
                reason: None,
                truncated_to_last_bytes: None,
            });
        }
    }

    // -- doctor output if present --
    {
        let path = run_dir.join("doctor-latest.log");
        let entry_path = format!("{bundle_root}/doctor/output.txt");
        if let Ok(bytes) = fs::read(&path) {
            let bytes = if no_redact {
                bytes
            } else {
                redact_log_bytes(&bytes)
            };
            let len = bytes.len() as u64;
            add_bytes(&mut tar, &entry_path, &bytes)?;
            sections.push(Section {
                path: entry_path,
                kind: "log",
                bytes: Some(len),
                missing: false,
                reason: None,
                truncated_to_last_bytes: None,
            });
        }
    }

    // T4: doctor bundle (in-VM diagnostic tar) if present. The bytes are
    // an opaque tar so we don't redact -- they're tar-of-binary-files and
    // any redaction at this layer would break tar's checksums.
    {
        let path = run_dir.join("doctor-latest.tar");
        let entry_path = format!("{bundle_root}/doctor/bundle.tar");
        if let Ok(bytes) = fs::read(&path) {
            let len = bytes.len() as u64;
            add_bytes(&mut tar, &entry_path, &bytes)?;
            sections.push(Section {
                path: entry_path,
                kind: "binary",
                bytes: Some(len),
                missing: false,
                reason: None,
                truncated_to_last_bytes: None,
            });
        }
    }

    // -- manifest (last) --
    let manifest = Manifest {
        schema_version: SCHEMA_VERSION,
        generated_at: chrono_like_rfc3339(),
        generator: Generator {
            binary: "capsem",
            version: env!("CARGO_PKG_VERSION"),
            build_ts: option_env!("CAPSEM_BUILD_TS").unwrap_or("dev"),
            platform: format!("{}/{}", std::env::consts::OS, std::env::consts::ARCH),
        },
        host: HostInfo {
            hostname: hostname(),
            os: format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH),
        },
        capsem_home: home.display().to_string(),
        redacted: !no_redact,
        sessions: included_session_ids,
        sections,
        warnings,
        next_steps: vec![
            "Open manifest.json first.".into(),
            "Then host/service.log around the timestamp of the failure.".into(),
            "sessions/<latest>/process.log for the matching window.".into(),
            "Search by `target=ipc` for IPC handshake / dropped-message events.".into(),
            "Search by `target=service status>=500` for swallowed server errors.".into(),
        ],
    };
    let manifest_bytes = serde_json::to_vec_pretty(&manifest)?;
    add_bytes(
        &mut tar,
        &format!("{bundle_root}/manifest.json"),
        &manifest_bytes,
    )?;

    let gz = tar.into_inner().context("finalize tar")?;
    gz.finish().context("finalize gzip")?;

    // Print a one-line summary to stderr (not stdout -- stdout reserved
    // for scripts that wrap the command and pipe stdout).
    let total_bytes = manifest
        .sections
        .iter()
        .filter_map(|s| s.bytes)
        .sum::<u64>();
    let missing = manifest.sections.iter().filter(|s| s.missing).count();
    eprintln!(
        "wrote support bundle: {} ({} bytes across {} sections, {} missing)",
        output.display(),
        total_bytes,
        manifest.sections.len(),
        missing,
    );
    Ok(output)
}

fn add_bytes<W: Write>(tar: &mut TarBuilder<W>, path: &str, bytes: &[u8]) -> Result<()> {
    let mut header = tar::Header::new_gnu();
    header.set_size(bytes.len() as u64);
    header.set_mode(0o600);
    header.set_mtime(
        SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs(),
    );
    header.set_cksum();
    tar.append_data(&mut header, path, bytes)?;
    Ok(())
}

fn read_tail(path: &Path, max_bytes: u64) -> Option<Vec<u8>> {
    let metadata = fs::metadata(path).ok()?;
    if !metadata.is_file() {
        return None;
    }
    let len = metadata.len();
    let bytes = fs::read(path).ok()?;
    if len <= max_bytes {
        return Some(bytes);
    }
    // Skip leading partial line so the first byte starts mid-record-cleanly.
    let start = (len - max_bytes) as usize;
    let mut tail = bytes[start..].to_vec();
    if let Some(idx) = tail.iter().position(|b| *b == b'\n') {
        tail.drain(..=idx);
    }
    Some(tail)
}

fn config_diagnostics(home: &Path) -> serde_json::Value {
    use capsem_core::net::policy_config::{
        corp_config_paths, corp_provision, ProfileCatalog, ProfileCatalogSource,
    };

    let profiles = match ProfileCatalog::load_default() {
        Ok(catalog) => {
            let source = match catalog.source() {
                ProfileCatalogSource::BuiltIn => "built_in".to_string(),
                ProfileCatalogSource::Directory(path) => format!("directory:{}", path.display()),
            };
            let profiles = catalog
                .profiles()
                .map(|profile| {
                    let mcp_server_count = profile
                        .mcp
                        .as_ref()
                        .map(|mcp| {
                            mcp.servers.len()
                                + usize::from(
                                    mcp.server_enabled.get("local").copied().unwrap_or(false),
                                )
                        })
                        .unwrap_or(0);
                    serde_json::json!({
                        "id": profile.id,
                        "name": profile.name,
                        "description": profile.description,
                        "revision": profile.revision,
                        "refresh_policy": profile.refresh_policy,
                        "availability": profile.availability,
                        "asset_arches": profile.assets.arch.keys().collect::<Vec<_>>(),
                        "default_rule_count": profile.default.len(),
                        "profile_rule_count": profile.profiles.rules.len(),
                        "ai_rule_count": profile.ai.values().map(|provider| provider.rules.len()).sum::<usize>(),
                        "plugin_count": profile.plugins.len(),
                        "mcp_server_count": mcp_server_count,
                    })
                })
                .collect::<Vec<_>>();
            serde_json::json!({
                "ok": true,
                "source": source,
                "profile_count": profiles.len(),
                "profiles": profiles,
            })
        }
        Err(error) => serde_json::json!({
            "ok": false,
            "error": error,
        }),
    };

    let corp_paths = corp_config_paths()
        .into_iter()
        .map(|path| {
            serde_json::json!({
                "path": path.display().to_string(),
                "exists": path.exists(),
            })
        })
        .collect::<Vec<_>>();
    let corp = serde_json::json!({
        "installed": corp_paths.iter().any(|path| path["exists"].as_bool().unwrap_or(false)),
        "paths": corp_paths,
        "source": corp_provision::read_corp_source(home),
    });

    serde_json::json!({
        "profiles": profiles,
        "corp": corp,
    })
}

fn redact_log_bytes(bytes: &[u8]) -> Vec<u8> {
    // Best-effort: split on \n, redact each line. Binary content trips
    // the from_utf8 path -- we leave it untouched.
    match std::str::from_utf8(bytes) {
        Ok(text) => text
            .lines()
            .map(redact::redact_line)
            .collect::<Vec<_>>()
            .join("\n")
            .into_bytes(),
        Err(_) => bytes.to_vec(),
    }
}

fn ts_filename() -> String {
    // YYYYMMDD-HHMMSS in UTC. Re-uses chrono_like_rfc3339() under the hood
    // and strips the separators -- one source of truth for the time math.
    chrono_like_rfc3339()
        .chars()
        .filter(|c| c.is_ascii_digit() || *c == 'T' || *c == '-')
        .collect::<String>()
        .replace("Z", "")
        .replace("T", "-")
        .replace("-", "")
        .chars()
        .enumerate()
        .map(|(i, c)| {
            if i == 8 {
                format!("-{c}")
            } else {
                c.to_string()
            }
        })
        .collect()
}

fn host_label() -> String {
    hostname()
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect::<String>()
        .chars()
        .take(32)
        .collect()
}

fn runtime_boundary_debug_contract() -> serde_json::Value {
    let host_vsock_services: Vec<_> = capsem_core::capsem_proto::host_vsock_services()
        .iter()
        .map(|service| {
            serde_json::json!({
                "service": service.as_str(),
                "port": service.port(),
            })
        })
        .collect();

    serde_json::json!({
        "version": env!("CARGO_PKG_VERSION"),
        "host_vsock_services": host_vsock_services,
        "closed_raw_vsock_ports": [
            {
                "port": 5003,
                "reason": "retired_mcp_raw_port",
            },
            {
                "port": 11434,
                "reason": "guest_tcp_ollama_must_use_mitm_redirect",
            },
            {
                "port": 3128,
                "reason": "guest_tcp_proxy_must_use_mitm_redirect",
            },
            {
                "port": 8080,
                "reason": "guest_tcp_proxy_must_use_mitm_redirect",
            }
        ],
        "debug_routes": [
            "/version",
            "/status",
            "/triage",
            "/panics",
            "/host-logs/{name}",
            "/vms/{id}/status",
            "/vms/{id}/info",
            "/vms/{id}/logs",
            "/vms/{id}/history",
            "/vms/{id}/security/latest",
            "/vms/{id}/security/status",
            "/vms/{id}/detection/latest",
            "/vms/{id}/detection/status",
            "/vms/{id}/enforcement/latest",
            "/vms/{id}/enforcement/status",
            "/profiles/status",
            "/profiles/list",
            "/profiles/{profile_id}/info",
            "/profiles/{profile_id}/assets/status",
            "/profiles/{profile_id}/plugins/info",
            "/profiles/{profile_id}/plugins/{plugin_id}/info",
            "/profiles/{profile_id}/plugins/credential_broker/credentials/info",
            "/profiles/{profile_id}/mcp/info",
            "/profiles/{profile_id}/mcp/servers/list"
        ],
    })
}

fn hostname() -> String {
    std::process::Command::new("hostname")
        .output()
        .ok()
        .and_then(|out| String::from_utf8(out.stdout).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "unknown".into())
}

fn chrono_like_rfc3339() -> String {
    // Pure-stdlib RFC3339 (UTC, second precision) -- avoids a `chrono`
    // dep. Days-since-epoch -> Y/M/D via a Howard Hinnant-style civil
    // calendar conversion (https://howardhinnant.github.io/date_algorithms.html).
    let secs = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let days = secs.div_euclid(86400);
    let secs_in_day = secs.rem_euclid(86400);
    let hh = (secs_in_day / 3600) as u32;
    let mm = ((secs_in_day % 3600) / 60) as u32;
    let ss = (secs_in_day % 60) as u32;

    // days_from_civil inverse: civil_from_days
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = (z - era * 146097) as u32;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };

    format!("{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z", y, m, d, hh, mm, ss)
}

#[derive(Serialize)]
struct Manifest {
    schema_version: u32,
    generated_at: String,
    generator: Generator,
    host: HostInfo,
    capsem_home: String,
    redacted: bool,
    sessions: Vec<String>,
    sections: Vec<Section>,
    warnings: Vec<String>,
    next_steps: Vec<String>,
}

#[derive(Serialize)]
struct Generator {
    binary: &'static str,
    version: &'static str,
    build_ts: &'static str,
    platform: String,
}

#[derive(Serialize)]
struct HostInfo {
    hostname: String,
    os: String,
}

#[derive(Serialize)]
struct Section {
    path: String,
    kind: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    bytes: Option<u64>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    missing: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    truncated_to_last_bytes: Option<u64>,
}

#[cfg(test)]
mod tests;
