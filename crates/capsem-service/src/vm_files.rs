use super::*;

pub(super) fn main_db_path_for_run_dir(run_dir: &StdPath) -> PathBuf {
    run_dir.parent().unwrap_or(run_dir).join("sessions").join("main.db")
}

pub(super) fn gib(bytes: u64) -> u64 {
    bytes / 1024 / 1024 / 1024
}

pub(super) fn session_rootfs_size_gb(entry: &PersistentVmEntry) -> Result<u32> {
    let rootfs = capsem_core::guest_share_dir(&entry.session_dir).join("system/rootfs.img");
    let metadata = std::fs::metadata(&rootfs)
        .with_context(|| format!("VM '{}' rootfs.img unavailable at {}", entry.name, rootfs.display()))?;
    let gib_bytes = 1024_u64 * 1024 * 1024;
    if metadata.len() == 0 || metadata.len() % gib_bytes != 0 {
        return Err(anyhow!(
            "VM '{}' rootfs.img logical size is not a positive whole GiB: {} bytes",
            entry.name,
            metadata.len()
        ));
    }
    u32::try_from(gib(metadata.len())).map_err(|_| {
        anyhow!(
            "VM '{}' rootfs.img logical size is too large: {} GiB",
            entry.name,
            gib(metadata.len())
        )
    })
}

pub(super) fn validate_saved_active_profile(path: &StdPath, entry: &PersistentVmEntry) -> Result<()> {
    let text =
        std::fs::read_to_string(path).with_context(|| format!("read saved active profile {}", path.display()))?;
    let active: ActiveProfileFile =
        toml::from_str(&text).with_context(|| format!("parse saved active profile {}", path.display()))?;
    active
        .validate()
        .map_err(anyhow::Error::msg)
        .with_context(|| format!("validate saved active profile {}", path.display()))?;
    if active.id != entry.profile_id {
        return Err(anyhow!(
            "saved profile id mismatch for VM '{}': pinned '{}', saved '{}'",
            entry.name,
            entry.profile_id,
            active.id
        ));
    }
    if active.revision != entry.profile_revision {
        return Err(anyhow!(
            "saved profile revision mismatch for VM '{}': pinned '{}', saved '{}'",
            entry.name,
            entry.profile_revision,
            active.revision
        ));
    }
    Ok(())
}

pub(super) fn boot_asset_pin_path(assets_dir: &StdPath, arch: &str, pin: &BootAssetPin) -> PathBuf {
    let bases = [assets_dir.join(arch), assets_dir.to_path_buf()];
    let hash_name = boot_asset_pin_hash_name(pin);
    for base in &bases {
        let path = base.join(&hash_name);
        if path.exists() {
            return path;
        }
    }
    for base in &bases {
        let path = base.join(&pin.name);
        if path.exists() {
            return path;
        }
    }
    bases[0].join(hash_name)
}

pub(super) fn reject_revoked_persistent_pins(assets_dir: &StdPath, entry: &PersistentVmEntry) -> Result<()> {
    let manifest_path = assets_dir.join("manifest.json");
    let Ok(bytes) = std::fs::read(&manifest_path) else {
        return Ok(());
    };
    let manifest: serde_json::Value = serde_json::from_slice(&bytes)
        .with_context(|| format!("parse installed manifest {}", manifest_path.display()))?;
    let Some(profile) = manifest
        .get("profiles")
        .and_then(|profiles| profiles.get(&entry.profile_id))
    else {
        return Ok(());
    };
    if profile
        .get("status")
        .and_then(serde_json::Value::as_str)
        .is_some_and(|status| status.eq_ignore_ascii_case("revoked"))
    {
        return Err(anyhow!(
            "profile '{}' is explicitly revoked for persistent VM '{}'",
            entry.profile_id,
            entry.name
        ));
    }

    let pinned_hashes = [
        &entry.asset_pins.kernel,
        &entry.asset_pins.initrd,
        &entry.asset_pins.rootfs,
    ]
    .into_iter()
    .map(|pin| pin.hash.strip_prefix("blake3:").unwrap_or(&pin.hash))
    .collect::<HashSet<_>>();
    let revoked_hash = profile
        .get("architectures")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|architecture| architecture.get("images"))
        .filter_map(serde_json::Value::as_array)
        .flatten()
        .find_map(|image| {
            let revoked = image
                .get("status")
                .and_then(serde_json::Value::as_str)
                .is_some_and(|status| status.eq_ignore_ascii_case("revoked"));
            let hash = image.pointer("/digest/blake3").and_then(serde_json::Value::as_str)?;
            (revoked && pinned_hashes.contains(hash)).then_some(hash)
        });
    if let Some(hash) = revoked_hash {
        return Err(anyhow!(
            "persistent VM '{}' pins explicitly revoked image blake3:{}",
            entry.name,
            hash
        ));
    }
    Ok(())
}

pub(super) fn profile_asset_pins(profile: &ProfileConfigFile) -> Result<BootAssetPins> {
    let arch = capsem_core::net::policy_config::current_profile_arch();
    let arch_assets = profile
        .assets
        .current_arch_assets()
        .ok_or_else(|| anyhow!("profile {} has no assets for architecture {arch}", profile.id))?;
    Ok(BootAssetPins {
        kernel: descriptor_pin(&arch_assets.kernel)?,
        initrd: descriptor_pin(&arch_assets.initrd)?,
        rootfs: descriptor_pin(&arch_assets.rootfs)?,
    })
}

pub(super) fn profile_payload_hash(profile: &ProfileConfigFile) -> Result<String> {
    let bytes = serde_json::to_vec(profile).context("serialize profile payload for hash")?;
    Ok(format!("blake3:{}", blake3::hash(&bytes).to_hex()))
}

pub(super) fn descriptor_pin(asset: &ProfileAssetDescriptor) -> Result<BootAssetPin> {
    Ok(BootAssetPin {
        name: asset.name.clone(),
        hash: required_profile_asset_hash(asset)?.to_string(),
    })
}

pub(super) fn validate_asset_file_pin(kind: &str, path: &StdPath, pin: &BootAssetPin) -> Result<()> {
    if !path.exists() {
        return Err(anyhow!("{kind} asset '{}' is missing at {}", pin.name, path.display()));
    }
    Ok(())
}

pub(super) fn profile_asset_descriptor_path(
    assets_dir: &StdPath,
    arch: &str,
    asset: &ProfileAssetDescriptor,
) -> Result<PathBuf> {
    let hash_name = profile_asset_hash_name(asset)?;
    let bases = [assets_dir.join(arch), assets_dir.to_path_buf()];

    for base in &bases {
        let path = base.join(&hash_name);
        if path.exists() {
            return Ok(path);
        }
    }
    for base in &bases {
        let path = base.join(&asset.name);
        if path.exists() {
            return Ok(path);
        }
    }

    Ok(bases[0].join(&asset.name))
}

pub(super) fn required_profile_asset_hash(asset: &ProfileAssetDescriptor) -> Result<&str> {
    asset
        .hash
        .as_deref()
        .ok_or_else(|| anyhow!("profile asset '{}' is missing a materialized hash", asset.name))
}

pub(super) fn required_profile_asset_size(asset: &ProfileAssetDescriptor) -> Result<u64> {
    asset
        .size
        .ok_or_else(|| anyhow!("profile asset '{}' is missing a materialized size", asset.name))
}

pub(super) fn profile_asset_hash_hex(asset: &ProfileAssetDescriptor) -> Result<&str> {
    let hash = required_profile_asset_hash(asset)?;
    Ok(hash.strip_prefix("blake3:").unwrap_or(hash))
}

pub(super) fn profile_asset_hash_name(asset: &ProfileAssetDescriptor) -> Result<String> {
    Ok(capsem_assets::asset_manager::hash_filename(
        &asset.name,
        profile_asset_hash_hex(asset)?,
    ))
}

pub(super) fn boot_asset_pin_hash_name(pin: &BootAssetPin) -> String {
    let hash = pin.hash.strip_prefix("blake3:").unwrap_or(&pin.hash);
    capsem_assets::asset_manager::hash_filename(&pin.name, hash)
}

pub(super) fn profile_catalog_asset_filenames(catalog: &ProfileCatalog) -> HashSet<String> {
    let mut filenames = HashSet::new();
    for profile in catalog.profiles() {
        for assets in profile.assets.arch.values() {
            if let Ok(name) = profile_asset_hash_name(&assets.kernel) {
                filenames.insert(name);
            }
            if let Ok(name) = profile_asset_hash_name(&assets.initrd) {
                filenames.insert(name);
            }
            if let Ok(name) = profile_asset_hash_name(&assets.rootfs) {
                filenames.insert(name);
            }
        }
    }
    filenames
}

pub(super) fn persistent_registry_asset_filenames(registry: &PersistentRegistry) -> HashSet<String> {
    let mut filenames = HashSet::new();
    for entry in registry.list() {
        filenames.insert(boot_asset_pin_hash_name(&entry.asset_pins.kernel));
        filenames.insert(boot_asset_pin_hash_name(&entry.asset_pins.initrd));
        filenames.insert(boot_asset_pin_hash_name(&entry.asset_pins.rootfs));
    }
    filenames
}

pub(super) fn profile_asset_download_target(
    assets_dir: &StdPath,
    arch: &str,
    asset: &ProfileAssetDescriptor,
) -> Result<PathBuf> {
    Ok(assets_dir.join(arch).join(profile_asset_hash_name(asset)?))
}

/// Identify the launchd-cleanup-saturation transient that masquerades
/// as an "entitlement missing" error from VZ.
///
/// Apple's `Virtualization.framework` runs a per-VM XPC helper
/// (`com.apple.Virtualization.VirtualMachine.<UUID>`). When capsem-process
/// dies, launchd schedules that XPC's cleanup with a 9s delay. Under
/// rapid VM churn (~3s/cycle) the PETRIFIED-pending queue grows; once
/// `syspolicyd` saturates (we observe `Unable to get certificates
/// array: (null)` in the unified log just before the failure window),
/// the next `VZVirtualMachineConfiguration.validateWithError()`
/// returns NSError code 2 with the misleading
/// `localizedDescription = "...The process doesn't have the
/// 'com.apple.security.virtualization' entitlement."` string -- even
/// though the binary IS entitled. The error message is wrong; the
/// actual cause is launchd cleanup saturation that drains within a
/// second or two.
///
/// Pattern-match on the full VZ-specific phrase (not just the bare
/// word "entitlement") so a real codesign regression -- which we'd
/// also want to surface -- is not silently retried away. The error
/// string is stable across VZ releases since it comes from VZ's
/// localized string table, not our code.
pub(super) fn is_launchd_cleanup_transient(process_log_tail: &str) -> bool {
    process_log_tail.contains("com.apple.security.virtualization") && process_log_tail.contains("entitlement")
}

pub(super) fn is_boot_fatal_log_tail(tail: &str) -> bool {
    tail.contains("FATAL: overlayfs")
        || tail.contains("Stale file handle")
        || tail.contains("failed to verify upper root origin")
        || tail.contains("Kernel panic")
}

/// Last `n` lines of a session log stream.
///
/// Named for what it does rather than shadowing `telemetry::read_log_tail`,
/// which it now delegates to. The old local copy read the bare file name and
/// carried the same name as the shared reader, so it both lost rotated content
/// and made the crate look like it already used the shared one. `serial.log`
/// is written through `CappedLogWriter` and rotates, so the bare name holds
/// only the newest slice.
pub(super) fn read_session_log_lines(session_dir: &std::path::Path, file_name: &str, n: usize) -> Option<String> {
    let content =
        capsem_foundation::telemetry::read_log_tail(&session_dir.join(file_name), SESSION_LOG_TAIL_MAX_BYTES)?;
    let lines: Vec<&str> = content.lines().collect();
    let start = lines.len().saturating_sub(n);
    Some(lines[start..].join("\n"))
}

pub(super) fn read_boot_failure_tail(session_dir: &std::path::Path) -> Option<String> {
    for file_name in ["serial.log", "process.log"] {
        let Some(tail) = read_session_log_lines(session_dir, file_name, 80) else {
            continue;
        };
        if is_boot_fatal_log_tail(&tail) {
            return Some(tail);
        }
    }
    None
}

/// Read the last `n` lines of `<session_dir>/process.log`. Returns a
/// placeholder string when the log is absent or unreadable, so callers
/// can always embed SOMETHING meaningful in a user-facing error.
pub(super) fn read_process_log_tail(session_dir: &std::path::Path, n: usize) -> String {
    let log_path = session_dir.join("process.log");
    let content = match std::fs::read_to_string(&log_path) {
        Ok(c) => c,
        Err(e) => return format!("(could not read {}: {e})", log_path.display()),
    };
    let lines: Vec<&str> = content.lines().collect();
    let tail = if lines.len() > n {
        &lines[lines.len() - n..]
    } else {
        &lines[..]
    };
    tail.join("\n")
}

/// Find the most recent `sessions/<id>-failed-<suffix>/` directory for a
/// given VM id. Returns `None` when no failed session has been preserved
/// (e.g. the VM id is simply unknown). Used by `handle_logs` so a user
/// running `capsem logs <id>` after a boot crash sees the logs that
/// `preserve_failed_session_dir` saved instead of a 404.
pub(super) fn find_failed_session_dir(run_dir: &std::path::Path, id: &str) -> Option<PathBuf> {
    let sessions_dir = run_dir.join("sessions");
    let entries = std::fs::read_dir(&sessions_dir).ok()?;
    let prefix = format!("{id}-failed-");
    let mut best: Option<(PathBuf, std::time::SystemTime)> = None;
    for entry in entries.flatten() {
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if !name.starts_with(&prefix) {
            continue;
        }
        let mtime = entry
            .metadata()
            .and_then(|m| m.modified())
            .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
        match &best {
            Some((_, existing)) if *existing >= mtime => {}
            _ => best = Some((path, mtime)),
        }
    }
    best.map(|(p, _)| p)
}

use axum::http::StatusCode;
use capsem_service::errors::AppError;
use capsem_service::fs_utils::{identify_file_sync, sanitize_file_path};

// ---------------------------------------------------------------------------
// Files API -- workspace path resolver (state-bound; pure helpers live in fs_utils.rs)
// ---------------------------------------------------------------------------

/// Resolve a sanitized relative path to an absolute workspace path on the host.
/// Returns (workspace_root, resolved_path). Verifies the resolved path is
/// inside the workspace via canonicalize + starts_with.
pub(super) fn resolve_workspace_path(
    state: &ServiceState,
    id: &str,
    sanitized: &str,
) -> Result<(PathBuf, PathBuf), AppError> {
    let session_dir = {
        let instances = state.instances.lock().unwrap();
        if let Some(info) = instances.get(id) {
            info.session_dir.clone()
        } else {
            drop(instances);
            // Check persistent registry for stopped VMs
            let reg = state.persistent_registry.lock().unwrap();
            reg.data
                .vms
                .get(id)
                .or_else(|| reg.data.vms.values().find(|e| e.name == id))
                .map(|e| e.session_dir.clone())
                .ok_or_else(|| AppError(StatusCode::NOT_FOUND, format!("sandbox not found: {id}")))?
        }
    };
    let workspace_root = capsem_core::guest_share_dir(&session_dir).join("workspace");
    let target = workspace_root.join(sanitized);

    // Canonicalize requires the path to exist for files; for listing we may
    // also target the workspace root itself. Use the parent if target doesn't exist.
    let canonical = if target.exists() {
        target.canonicalize()
    } else {
        // For upload: parent must exist and be inside workspace
        if let Some(parent) = target.parent() {
            if parent.exists() {
                let canon_parent = parent
                    .canonicalize()
                    .map_err(|e| AppError(StatusCode::INTERNAL_SERVER_ERROR, format!("canonicalize: {e}")))?;
                let ws_canon = workspace_root.canonicalize().map_err(|e| {
                    AppError(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        format!("canonicalize workspace: {e}"),
                    )
                })?;
                if !canon_parent.starts_with(&ws_canon) {
                    return Err(AppError(StatusCode::FORBIDDEN, "path outside workspace".into()));
                }
                return Ok((workspace_root, target));
            }
        }
        return Ok((workspace_root, target));
    }
    .map_err(|e| AppError(StatusCode::INTERNAL_SERVER_ERROR, format!("canonicalize: {e}")))?;

    let ws_canon = workspace_root.canonicalize().map_err(|e| {
        AppError(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("canonicalize workspace: {e}"),
        )
    })?;
    if !canonical.starts_with(&ws_canon) {
        return Err(AppError(StatusCode::FORBIDDEN, "path outside workspace".into()));
    }
    Ok((workspace_root, canonical))
}

// ---------------------------------------------------------------------------
// Files API Handlers (host-side VirtioFS)
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
pub(super) struct FileListQuery {
    #[serde(default)]
    path: Option<String>,
    #[serde(default = "default_file_depth")]
    depth: u32,
}

pub(super) fn default_file_depth() -> u32 {
    1
}

#[derive(Deserialize)]
pub(super) struct FileContentQuery {
    pub(super) path: String,
}

/// Recursively list a directory up to `max_depth`.
pub(super) fn list_dir_recursive(
    base: &std::path::Path,
    rel_prefix: &str,
    current_depth: u32,
    max_depth: u32,
    magika: &Mutex<magika::Session>,
) -> Vec<FileListEntry> {
    let mut entries = Vec::new();
    let read = match std::fs::read_dir(base) {
        Ok(r) => r,
        Err(_) => return entries,
    };

    let mut items: Vec<_> = read.flatten().collect();
    items.sort_by(|a, b| {
        let a_is_dir = a.file_type().map(|t| t.is_dir()).unwrap_or(false);
        let b_is_dir = b.file_type().map(|t| t.is_dir()).unwrap_or(false);
        b_is_dir.cmp(&a_is_dir).then_with(|| a.file_name().cmp(&b.file_name()))
    });

    for item in items {
        let name = item.file_name().to_string_lossy().into_owned();
        // Skip the system directory (rootfs overlay, not user content)
        if name == "system" {
            continue;
        }
        let rel_path = if rel_prefix.is_empty() {
            name.clone()
        } else {
            format!("{rel_prefix}/{name}")
        };
        let meta = match item.metadata() {
            Ok(m) => m,
            Err(_) => continue,
        };
        let mtime = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs())
            .unwrap_or(0);

        if meta.is_dir() {
            let children = if current_depth < max_depth {
                Some(list_dir_recursive(
                    &base.join(&name),
                    &rel_path,
                    current_depth + 1,
                    max_depth,
                    magika,
                ))
            } else {
                None
            };
            entries.push(FileListEntry {
                name,
                path: rel_path,
                entry_type: "directory".into(),
                size: 0,
                mtime,
                mime: None,
                label: None,
                is_text: None,
                children,
            });
        } else if meta.is_file() {
            let (lbl, mime_str, _group, text) = identify_file_sync(magika, &base.join(&name));
            let (mime, label, is_text) = (Some(mime_str), Some(lbl), Some(text));
            entries.push(FileListEntry {
                name,
                path: rel_path,
                entry_type: "file".into(),
                size: meta.len(),
                mtime,
                mime,
                label,
                is_text,
                children: None,
            });
        }
    }
    entries
}

pub(super) async fn handle_list_files(
    State(state): State<Arc<ServiceState>>,
    Path(id): Path<String>,
    Query(params): Query<FileListQuery>,
) -> Result<Json<FileListResponse>, AppError> {
    let depth = params.depth.min(6);
    let rel_path = match params.path.as_deref() {
        Some(p) if !p.is_empty() => sanitize_file_path(p)?,
        _ => String::new(),
    };

    let (workspace_root, target) = if rel_path.is_empty() {
        // List workspace root -- get session_dir directly
        let session_dir = {
            let instances = state.instances.lock().unwrap();
            if let Some(info) = instances.get(&id) {
                info.session_dir.clone()
            } else {
                drop(instances);
                let reg = state.persistent_registry.lock().unwrap();
                reg.data
                    .vms
                    .get(&id)
                    .or_else(|| reg.data.vms.values().find(|e| e.name == id))
                    .map(|e| e.session_dir.clone())
                    .ok_or_else(|| AppError(StatusCode::NOT_FOUND, format!("sandbox not found: {id}")))?
            }
        };
        let ws = capsem_core::guest_share_dir(&session_dir).join("workspace");
        (ws.clone(), ws)
    } else {
        resolve_workspace_path(&state, &id, &rel_path)?
    };

    if !target.exists() {
        return Err(AppError(StatusCode::NOT_FOUND, "path not found".into()));
    }

    // Compute relative prefix for the listing
    let rel_prefix = target
        .strip_prefix(&workspace_root)
        .unwrap_or(std::path::Path::new(""))
        .to_string_lossy()
        .into_owned();

    // read_dir + metadata are blocking I/O -- run in spawn_blocking
    let magika = state.magika.lock().unwrap();
    // We can't send MutexGuard across threads; re-acquire inside spawn_blocking
    drop(magika);
    let magika_ref = {
        // Clone Arc to move into blocking task
        let state_clone = Arc::clone(&state);
        let target = target.clone();
        tokio::task::spawn_blocking(move || list_dir_recursive(&target, &rel_prefix, 1, depth, &state_clone.magika))
            .await
            .map_err(|e| AppError(StatusCode::INTERNAL_SERVER_ERROR, format!("list: {e}")))?
    };

    Ok(Json(FileListResponse { entries: magika_ref }))
}

const MAX_FILE_SIZE: u64 = 10 * 1024 * 1024; // 10MB
const FILE_SECURITY_CONTENT_PREVIEW_MAX: usize = 64 * 1024;

pub(super) fn file_security_preview_bytes(data: &[u8]) -> Vec<u8> {
    data[..data.len().min(FILE_SECURITY_CONTENT_PREVIEW_MAX)].to_vec()
}

pub(super) fn active_instance_uds_path(state: &Arc<ServiceState>, id: &str) -> Result<PathBuf, AppError> {
    let instances = state.instances.lock().unwrap();
    instances.get(id).map(|i| i.uds_path.clone()).ok_or_else(|| {
        AppError(
            StatusCode::CONFLICT,
            "file import/export requires a running sandbox security ledger".into(),
        )
    })
}

pub(super) async fn log_file_boundary(
    state: &Arc<ServiceState>,
    sandbox_id: &str,
    action: FileBoundaryAction,
    path: String,
    data_preview: Vec<u8>,
    size: u64,
    mime_type: Option<String>,
) -> Result<Option<Vec<u8>>, AppError> {
    let uds_path = active_instance_uds_path(state, sandbox_id)?;
    wait_for_vm_ready(&uds_path, 30, Some(state), Some(sandbox_id))
        .await
        .map_err(|e| AppError(StatusCode::INTERNAL_SERVER_ERROR, e))?;

    let id = state.next_job_id();
    let res = send_ipc_command(
        &uds_path,
        ServiceToProcess::LogFileBoundary {
            id,
            action,
            path,
            data: data_preview,
            size,
            mime_type,
        },
        Some(5),
    )
    .await
    .map_err(|e| AppError(StatusCode::INTERNAL_SERVER_ERROR, e))?;

    match res {
        ProcessToService::LogFileBoundaryResult {
            success: true, data, ..
        } => Ok(data),
        ProcessToService::LogFileBoundaryResult { error, .. } => Err(AppError(
            StatusCode::INTERNAL_SERVER_ERROR,
            error.unwrap_or_else(|| "failed to log file boundary".into()),
        )),
        _ => Err(AppError(
            StatusCode::INTERNAL_SERVER_ERROR,
            "unexpected IPC response for file boundary log".into(),
        )),
    }
}

pub(super) async fn handle_download_file(
    State(state): State<Arc<ServiceState>>,
    Path(id): Path<String>,
    Query(params): Query<FileContentQuery>,
) -> Result<axum::response::Response, AppError> {
    let sanitized = sanitize_file_path(&params.path)?;
    let (_ws_root, resolved) = resolve_workspace_path(&state, &id, &sanitized)?;

    if !resolved.is_file() {
        return Err(AppError(StatusCode::NOT_FOUND, "file not found".into()));
    }

    let meta = std::fs::metadata(&resolved)
        .map_err(|e| AppError(StatusCode::INTERNAL_SERVER_ERROR, format!("metadata: {e}")))?;
    if meta.len() > MAX_FILE_SIZE {
        return Err(AppError(
            StatusCode::PAYLOAD_TOO_LARGE,
            format!("file too large: {} bytes (max {})", meta.len(), MAX_FILE_SIZE),
        ));
    }

    // Read file and detect type in spawn_blocking
    let state_clone = Arc::clone(&state);
    let resolved_clone = resolved.clone();
    let (data, mime, filename) = tokio::task::spawn_blocking(move || {
        let data = std::fs::read(&resolved_clone)
            .map_err(|e| AppError(StatusCode::INTERNAL_SERVER_ERROR, format!("read: {e}")))?;
        let (_, mime_str, _, _) = identify_file_sync(&state_clone.magika, &resolved_clone);
        let name = resolved_clone
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "download".into());
        // Sanitize the filename for Content-Disposition
        let safe_name: String = name
            .chars()
            .filter(|c| c.is_ascii_alphanumeric() || *c == '.' || *c == '_' || *c == '-')
            .collect();
        Ok::<_, AppError>((data, mime_str, safe_name))
    })
    .await
    .map_err(|e| AppError(StatusCode::INTERNAL_SERVER_ERROR, format!("task: {e}")))??;

    let rewritten = log_file_boundary(
        &state,
        &id,
        FileBoundaryAction::Export,
        sanitized,
        file_security_preview_bytes(&data),
        data.len() as u64,
        Some(mime.clone()),
    )
    .await?;
    let data = rewritten.unwrap_or(data);

    use axum::response::IntoResponse;
    Ok((
        StatusCode::OK,
        [
            (axum::http::header::CONTENT_TYPE, mime),
            (
                axum::http::header::CONTENT_DISPOSITION,
                format!("attachment; filename=\"{filename}\""),
            ),
            (axum::http::header::CONTENT_LENGTH, data.len().to_string()),
        ],
        data,
    )
        .into_response())
}

pub(super) async fn handle_upload_file(
    State(state): State<Arc<ServiceState>>,
    Path(id): Path<String>,
    Query(params): Query<FileContentQuery>,
    body: axum::body::Bytes,
) -> Result<Json<UploadResponse>, AppError> {
    let sanitized = sanitize_file_path(&params.path)?;
    let (_ws_root, target) = resolve_workspace_path(&state, &id, &sanitized)?;

    let mut data = body.to_vec();
    let size = data.len() as u64;
    let preview = file_security_preview_bytes(&data);
    let target_for_write = target.clone();

    if let Some(rewritten) =
        log_file_boundary(&state, &id, FileBoundaryAction::Import, sanitized, preview, size, None).await?
    {
        data = rewritten;
    }
    let written_size = data.len() as u64;

    // Write file in spawn_blocking (blocking I/O)
    tokio::task::spawn_blocking(move || {
        if let Some(parent) = target_for_write.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| AppError(StatusCode::INTERNAL_SERVER_ERROR, format!("mkdir: {e}")))?;
        }
        use std::os::unix::fs::OpenOptionsExt;
        std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o644)
            .open(&target_for_write)
            .and_then(|f| {
                use std::io::Write;
                let mut f = f;
                f.write_all(&data)?;
                Ok(())
            })
            .map_err(|e| AppError(StatusCode::INTERNAL_SERVER_ERROR, format!("write: {e}")))?;
        Ok::<_, AppError>(())
    })
    .await
    .map_err(|e| AppError(StatusCode::INTERNAL_SERVER_ERROR, format!("task: {e}")))??;

    Ok(Json(UploadResponse {
        success: true,
        size: written_size,
    }))
}

// ---------------------------------------------------------------------------
// Image API Handlers
// ---------------------------------------------------------------------------

pub(super) async fn handle_fork(
    State(state): State<Arc<ServiceState>>,
    Path(id): Path<String>,
    Json(payload): Json<ForkRequest>,
) -> Result<Json<ForkResponse>, AppError> {
    let name = &payload.name;
    validate_vm_name(name).map_err(|e| AppError(StatusCode::BAD_REQUEST, e.to_string()))?;

    // Check name is not taken
    {
        let registry = state.persistent_registry.lock().unwrap();
        if registry.contains(name) {
            return Err(AppError(
                StatusCode::CONFLICT,
                format!("sandbox '{}' already exists", name),
            ));
        }
    }

    // Find source: running instance or stopped persistent VM
    let (
        session_dir,
        profile_id,
        profile_revision,
        profile_payload_hash,
        asset_pins,
        ram_mb,
        cpus,
        base_version,
        uds_path,
    ) = {
        let instances = state.instances.lock().unwrap();
        if let Some(i) = instances.get(&id) {
            (
                i.session_dir.clone(),
                i.profile_id.clone(),
                i.profile_revision.clone(),
                i.profile_payload_hash.clone(),
                i.asset_pins.clone(),
                i.ram_mb,
                i.cpus,
                i.base_version.clone(),
                Some(i.uds_path.clone()),
            )
        } else {
            drop(instances);
            if let Some(p) = find_persistent_entry_by_route_id(&state, &id) {
                (
                    p.session_dir,
                    p.profile_id,
                    p.profile_revision,
                    p.profile_payload_hash,
                    p.asset_pins,
                    p.ram_mb,
                    p.cpus,
                    p.base_version,
                    None,
                )
            } else {
                return Err(AppError(
                    StatusCode::NOT_FOUND,
                    format!("source sandbox not found: {}", id),
                ));
            }
        }
    };
    let profile = state
        .cached_profile_config(&profile_id)
        .map_err(|e| AppError(StatusCode::PRECONDITION_FAILED, e.to_string()))?;
    state
        .validate_profile_pins(&profile, &profile_revision, &profile_payload_hash, &asset_pins)
        .map_err(|e| AppError(StatusCode::PRECONDITION_FAILED, e.to_string()))?;

    // Flush the guest root filesystem so the ext4 system overlay (/dev/vdb
    // backed by rootfs.img) has pushed dirty pages into the host-visible image
    // before fork clone. Do not fsfreeze here: the old shell command thawed
    // before cloning, so it paid freeze latency without actually snapshotting
    // while frozen.
    if let Some(ref uds) = uds_path {
        let flush_id = state.next_job_id();
        if let Err(e) = send_ipc_command(
            uds,
            ServiceToProcess::Exec {
                id: flush_id,
                command: "sync; true".to_string(),
            },
            Some(10),
        )
        .await
        {
            tracing::warn!(error = %e, "pre-fork guest sync failed (non-fatal)");
        }
    }

    // Clone state into new persistent sandbox. The route/runtime id is
    // separate from the human display name.
    let vm_id = new_persistent_vm_id();
    let new_session_dir = state.run_dir.join("persistent").join(&vm_id);
    let _ = std::fs::create_dir_all(state.run_dir.join("persistent"));
    let _ = std::fs::create_dir_all(&new_session_dir);

    // clone_sandbox_state does fsync + APFS clonefile + walkdir -- all blocking.
    // Offload to the blocking pool so axum worker threads aren't starved under
    // concurrent fork load.
    let clone_dst = new_session_dir.clone();
    let size_bytes =
        tokio::task::spawn_blocking(move || capsem_core::auto_snapshot::clone_sandbox_state(&session_dir, &clone_dst))
            .await
            .map_err(|e| {
                capsem_service::app_error_logged!(
                    error,
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "fork: clone-task panic: {e}"
                )
            })?
            .map_err(|e| {
                capsem_service::app_error_logged!(error, StatusCode::INTERNAL_SERVER_ERROR, "fork: clone failed: {e}")
            })?;

    // Register as persistent VM
    {
        let mut registry = state.persistent_registry.lock().unwrap();
        registry
            .register(PersistentVmEntry {
                id: vm_id.clone(),
                name: name.clone(),
                profile_id,
                profile_revision,
                profile_payload_hash,
                asset_pins,
                ram_mb,
                cpus,
                base_version,
                created_at: format!(
                    "{}",
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap()
                        .as_secs()
                ),
                session_dir: new_session_dir,
                forked_from: Some(id.clone()),
                description: payload.description.clone(),
                suspended: false,
                defunct: false,
                last_error: None,
                checkpoint_path: None,
                env: None,
            })
            .map_err(|e| AppError(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
        drop(registry);
    }

    Ok(Json(ForkResponse {
        id: vm_id,
        name: name.clone(),
        size_bytes,
    }))
}

/// Outcome of a single provision attempt inside `handle_provision`.
/// `LaunchdTransient` is the recoverable case: VZ rejected the fresh
/// VM with the misleading entitlement string while launchd's
/// PETRIFIED-cleanup queue was draining. The poll_until loop retries
/// on this; everything else (incl. `Other`) bubbles up unchanged.
pub(super) enum ProvisionAttemptOutcome {
    Ready { uds_path: PathBuf },
    StillBootingTimedOut { uds_path: PathBuf }, // 5s envelope hit; treat as success per pre-existing contract
    LaunchdTransient,
    BootCrash { tail: String },
    ProvisionError(anyhow::Error),
}

/// Decision the retry loop takes after observing one provision attempt.
/// Pure function of the outcome -- no side effects -- so the
/// retry-routing can be unit-tested without spawning a real VM.
#[derive(Debug)]
pub(super) enum AttemptDecision {
    Succeed(PathBuf),
    BailWithError(AppError),
    RetryAfterCleanup,
}

/// Map a single attempt's outcome to the retry loop's next move.
/// The `LaunchdTransient` variant is the only one that triggers retry;
/// `BootCrash` and `ProvisionError` bail with structured errors that
/// match the pre-refactor handle_provision response shape.
pub(super) fn classify_attempt_decision(outcome: ProvisionAttemptOutcome, id: &str) -> AttemptDecision {
    match outcome {
        ProvisionAttemptOutcome::Ready { uds_path } | ProvisionAttemptOutcome::StillBootingTimedOut { uds_path } => {
            AttemptDecision::Succeed(uds_path)
        }
        ProvisionAttemptOutcome::LaunchdTransient => AttemptDecision::RetryAfterCleanup,
        ProvisionAttemptOutcome::BootCrash { tail } => AttemptDecision::BailWithError(AppError(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!(
                "sandbox {id} failed to boot. process.log tail:\n\n{tail}\n\n\
                 (full logs: `capsem logs {id}`)"
            ),
        )),
        ProvisionAttemptOutcome::ProvisionError(e) => {
            let status = if e.to_string().contains("already exists") {
                StatusCode::CONFLICT
            } else {
                StatusCode::INTERNAL_SERVER_ERROR
            };
            AttemptDecision::BailWithError(AppError(status, format!("provision failed: {e}")))
        }
    }
}

pub(super) fn existing_session_names(state: &ServiceState) -> Vec<String> {
    let mut existing: Vec<String> = state
        .instances
        .lock()
        .unwrap()
        .values()
        .map(|instance| instance.name.clone())
        .collect();
    existing.extend(
        state
            .persistent_registry
            .lock()
            .unwrap()
            .list()
            .map(|entry| entry.name.clone()),
    );
    for dir in [state.run_dir.join("sessions"), state.run_dir.join("persistent")] {
        if let Ok(entries) = std::fs::read_dir(dir) {
            existing.extend(
                entries
                    .flatten()
                    .filter(|entry| entry.file_type().map(|ty| ty.is_dir()).unwrap_or(false))
                    .filter_map(|entry| entry.file_name().into_string().ok()),
            );
        }
    }
    existing
}

pub(super) async fn handle_provision(
    State(state): State<Arc<ServiceState>>,
    Json(payload): Json<ProvisionRequest>,
) -> Result<Json<ProvisionResponse>, AppError> {
    let profile_id = validate_profile_route_id(payload.profile_id.clone())?;
    if let Some(reason) = vm_asset_block_reason(&state, &profile_id) {
        return Err(AppError(StatusCode::PRECONDITION_FAILED, reason));
    }

    let name = payload.name.clone().unwrap_or_else(|| {
        let existing = existing_session_names(&state);
        generate_profile_session_name(&profile_id, existing.iter().map(|s| s.as_str()))
    });
    let persistent = payload.persistent || payload.name.is_some() || payload.from.is_some();
    if existing_session_names(&state).iter().any(|existing| existing == &name) {
        return Err(AppError(
            StatusCode::CONFLICT,
            format!("persistent VM \"{}\" already exists", name),
        ));
    }
    let id = new_persistent_vm_id();

    let profile = state
        .cached_profile_config(&profile_id)
        .map_err(|e| AppError(StatusCode::PRECONDITION_FAILED, e.to_string()))?;
    let resources = resolve_profile_vm_resources(&profile, payload.ram_mb, payload.cpus);
    let ram_mb = resources.ram_mb;
    let cpus = resources.cpus;
    let scratch_disk_size_gb = resources.scratch_disk_size_gb;

    // Retry budget for the launchd-cleanup transient. Failed attempts
    // fast-fail in ~500ms (capsem-process spawn -> validateWithError
    // crash -> child-exit handler -> instances-map removal observable
    // here), so 8s covers ~5-8 attempts including backoff. Successful
    // attempts return on the first poll iteration regardless of timeout.
    // Backoff lets launchd tick at least one PETRIFIED-cleanup entry
    // (9s wall-clock per entry) between retries; under a real cascade
    // the second attempt usually lands once one entry has drained.
    let opts = capsem_foundation::poll::PollOpts {
        label: "provision-launchd-drain",
        timeout: std::time::Duration::from_secs(8),
        initial_delay: std::time::Duration::from_millis(200),
        max_delay: std::time::Duration::from_millis(500),
    };

    let id_for_loop = id.clone();
    let attempt_num = std::sync::atomic::AtomicU32::new(0);
    let result = capsem_foundation::poll::poll_until(opts, || {
        let state = Arc::clone(&state);
        let id = id_for_loop.clone();
        let name = name.clone();
        let payload_env = payload.env.clone();
        let payload_from = payload.from.clone();
        let payload_profile_id = profile_id.clone();
        let payload_persistent = persistent;
        let attempt = attempt_num.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
        async move {
            // Before retry attempts (>1), clear any state the prior
            // failed attempt left behind so provision_sandbox does not
            // reject with "already exists". The child-exit handler has
            // already done its own cleanup (instances.remove +
            // preserve_failed_session_dir) by the time we observe
            // crash-before-ready; we only need to undo registration of
            // the persistent entry.
            if attempt > 1 {
                let mut registry = state.persistent_registry.lock().unwrap();
                let _ = registry.unregister(&name);
                drop(registry);
                state.instances.lock().unwrap().remove(&id);
                warn!(id, attempt, "retrying provision after launchd-cleanup transient");
            }

            let outcome = provision_attempt(
                &state,
                &id,
                &name,
                ram_mb,
                cpus,
                scratch_disk_size_gb,
                payload_profile_id,
                payload_persistent,
                payload_env,
                payload_from,
            )
            .await;
            // Log structured context BEFORE losing the outcome to classify_*.
            // BootCrash/ProvisionError still produce a user-facing error
            // body via classify_attempt_decision; these logs are for
            // operators reading service.log.
            if let ProvisionAttemptOutcome::BootCrash { ref tail } = outcome {
                // The tail goes to the caller in the 500 body; without it here
                // service.log records that a boot died but never why, and the
                // reason survives only inside the session's process.log.
                error!(
                    id,
                    cause = capsem_core::session::boot_failure_summary(tail),
                    "capsem-process exited before reaching ready"
                );
            } else if let ProvisionAttemptOutcome::ProvisionError(ref e) = outcome {
                error!(id, error = %e, "provision failed");
            }
            match classify_attempt_decision(outcome, &id) {
                AttemptDecision::Succeed(uds_path) => Some(Ok(uds_path)),
                AttemptDecision::RetryAfterCleanup => None, // poll_until retries
                AttemptDecision::BailWithError(err) => Some(Err(err)),
            }
        }
    })
    .await;

    match result {
        Ok(Ok(uds_path)) => provision_response_for_running(&state, id, uds_path).map(Json),
        Ok(Err(app_err)) => Err(app_err),
        Err(timed_out) => {
            // Exhausted retries on launchd transient. Surface the most
            // recent failed-attempt tail so the user sees what VZ said,
            // even though the actual cause is launchd-side saturation.
            let tail = match find_failed_session_dir(&state.run_dir, &id) {
                Some(dir) => read_process_log_tail(&dir, 20),
                None => "(no preserved log found)".to_string(),
            };
            error!(
                id,
                attempts = timed_out.attempts,
                "provision: launchd-cleanup retries exhausted"
            );
            Err(AppError(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!(
                    "sandbox {id} could not be provisioned after {} attempts ({}). \
                     This typically clears within 10s; please retry. process.log tail:\n\n{tail}\n\n\
                     (full logs: `capsem logs {id}`)",
                    timed_out.attempts, timed_out
                ),
            ))
        }
    }
}

/// Run one provision attempt: spawn capsem-process, then poll briefly
/// for either the `.ready` sentinel or a crash-before-ready signal.
/// Pure bookkeeping; no retry logic here -- caller drives the retry
/// loop on `ProvisionAttemptOutcome::LaunchdTransient`.
#[allow(clippy::too_many_arguments)]
pub(super) async fn provision_attempt(
    state: &Arc<ServiceState>,
    id: &str,
    name: &str,
    ram_mb: u64,
    cpus: u32,
    scratch_disk_size_gb: u32,
    profile_id: String,
    persistent: bool,
    env: Option<std::collections::HashMap<String, String>>,
    from: Option<String>,
) -> ProvisionAttemptOutcome {
    // Creating/starting a VM is an Apple VZ lifecycle operation too. Cold
    // starts take the shared rail so independent boots can overlap, but they
    // still wait behind any in-flight save/restore checkpoint edge.
    let _vz_guard = state.save_restore_lock.read().await;
    let _vz_host_guard = match acquire_vz_host_lock(startup::VzHostLockMode::Shared).await {
        Ok(guard) => guard,
        Err(e) => {
            return ProvisionAttemptOutcome::ProvisionError(anyhow::anyhow!(
                "vz lifecycle lock acquire failed: {}",
                e.1
            ))
        }
    };

    let state_clone = Arc::clone(state);
    let id_owned = id.to_string();
    let name_owned = name.to_string();
    let version = state.current_version.clone();
    let provision_result = match tokio::task::spawn_blocking(move || {
        state_clone.provision_sandbox(ProvisionOptions {
            id: &id_owned,
            name: &name_owned,
            profile_id,
            ram_mb,
            cpus,
            scratch_disk_size_gb,
            version_override: Some(version),
            persistent,
            env,
            from,
            description: None,
        })
    })
    .await
    {
        Ok(r) => r,
        Err(e) => return ProvisionAttemptOutcome::ProvisionError(anyhow::anyhow!("provision task: {e}")),
    };

    if let Err(e) = provision_result {
        return ProvisionAttemptOutcome::ProvisionError(e);
    }

    // Wait briefly for either the `.ready` sentinel or the child-exit
    // handler to remove the VM from the instances map (crash). Without
    // this poll, `capsem create` prints the id and exits 0 while the
    // guest is already dead. The window is deliberately shorter than a
    // normal cold boot: create must catch synchronous launch failures, while
    // exec/file routes own the full readiness wait for valid slow boots.
    let uds_path = state.instance_socket_path(id);
    let ready_path = uds_path.with_extension("ready");
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_millis(500);
    loop {
        if ready_path.exists() {
            return ProvisionAttemptOutcome::Ready { uds_path };
        }
        let still_alive = state.instances.lock().unwrap().contains_key(id);
        if !still_alive {
            // Crash before ready. Prefer the persistent entry's
            // cached last_error (already computed by the child-exit
            // handler) to avoid re-reading the log; fall back to
            // find_failed_session_dir for ephemeral VMs whose dir was
            // renamed to `-failed-*`.
            let cached = find_persistent_entry_by_route_id(state, id).and_then(|e| e.last_error);
            let tail = cached.unwrap_or_else(|| match find_failed_session_dir(&state.run_dir, id) {
                Some(dir) => read_process_log_tail(&dir, 20),
                None => "(no preserved log found)".to_string(),
            });
            return if is_launchd_cleanup_transient(&tail) {
                warn!(
                    id,
                    "provision: detected launchd-cleanup transient (misleading 'entitlement' error)"
                );
                ProvisionAttemptOutcome::LaunchdTransient
            } else {
                ProvisionAttemptOutcome::BootCrash { tail }
            };
        }
        if tokio::time::Instant::now() >= deadline {
            return ProvisionAttemptOutcome::StillBootingTimedOut { uds_path };
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
}

#[cfg(unix)]
pub(super) fn physical_bytes(metadata: &std::fs::Metadata) -> u64 {
    use std::os::unix::fs::MetadataExt;
    metadata.blocks() * 512
}

#[cfg(not(unix))]
pub(super) fn physical_bytes(metadata: &std::fs::Metadata) -> u64 {
    metadata.len()
}

pub(super) fn statvfs_bytes<BlockCount>(blocks: BlockCount, block_size: u64) -> u64
where
    BlockCount: Into<u64>,
{
    blocks.into().saturating_mul(block_size)
}

pub(super) fn storage_diagnostics(session_dir: &StdPath) -> Option<api::StorageDiagnostics> {
    let rootfs_image_path = capsem_core::guest_share_dir(session_dir).join("system/rootfs.img");
    let metadata = std::fs::metadata(&rootfs_image_path).ok()?;
    let stat = nix::sys::statvfs::statvfs(session_dir).ok()?;
    let block_size = stat.block_size();

    Some(api::StorageDiagnostics {
        rootfs_image_path: rootfs_image_path.to_string_lossy().to_string(),
        rootfs_image_logical_bytes: metadata.len(),
        rootfs_image_physical_bytes: physical_bytes(&metadata),
        host_total_bytes: statvfs_bytes(stat.blocks(), block_size),
        host_free_bytes: statvfs_bytes(stat.blocks_free(), block_size),
        host_available_bytes: statvfs_bytes(stat.blocks_available(), block_size),
        guest_overlay_device: "/dev/vdb".into(),
        guest_overlay_mount: "/".into(),
    })
}

pub(super) fn append_fingerprint_field(out: &mut String, value: &str) {
    use std::fmt::Write as _;

    let _ = write!(out, "{}:", value.len());
    out.push_str(value);
    out.push('|');
}

pub(super) fn list_response_fingerprint(state: &ServiceState) -> String {
    use std::fmt::Write as _;

    let mut fingerprint = String::new();
    {
        let instances = state.instances.lock().unwrap();
        let _ = write!(fingerprint, "running={};", instances.len());
        for i in instances.values() {
            append_fingerprint_field(&mut fingerprint, &i.id);
            append_fingerprint_field(&mut fingerprint, &i.profile_id);
            append_fingerprint_field(&mut fingerprint, &i.name);
            let _ = write!(
                fingerprint,
                "{}:{}:{}:{}:{};",
                i.pid,
                i.persistent,
                i.ram_mb,
                i.cpus,
                i.start_time.elapsed().as_secs()
            );
            append_fingerprint_field(&mut fingerprint, &i.base_version);
            append_fingerprint_field(&mut fingerprint, i.forked_from.as_deref().unwrap_or(""));
        }
    }
    {
        let registry = state.persistent_registry.lock().unwrap();
        let instances = state.instances.lock().unwrap();
        let inactive_entries: Vec<&PersistentVmEntry> = registry
            .list()
            .filter(|entry| !instances.contains_key(&persistent_entry_vm_id(entry)))
            .collect();
        let _ = write!(fingerprint, "inactive={};", inactive_entries.len());
        for entry in inactive_entries {
            append_fingerprint_field(&mut fingerprint, &persistent_entry_vm_id(entry));
            append_fingerprint_field(&mut fingerprint, &entry.name);
            append_fingerprint_field(&mut fingerprint, &entry.profile_id);
            append_fingerprint_field(&mut fingerprint, &entry.profile_revision);
            append_fingerprint_field(&mut fingerprint, &entry.profile_payload_hash);
            append_fingerprint_field(&mut fingerprint, &entry.base_version);
            append_fingerprint_field(&mut fingerprint, entry.forked_from.as_deref().unwrap_or(""));
            append_fingerprint_field(&mut fingerprint, entry.description.as_deref().unwrap_or(""));
            append_fingerprint_field(&mut fingerprint, entry.last_error.as_deref().unwrap_or(""));
            let _ = write!(
                fingerprint,
                "{}:{}:{}:{}:{}:{};",
                entry.ram_mb,
                entry.cpus,
                entry.suspended,
                entry.defunct,
                entry.session_dir.display(),
                persistent_resume_state_fingerprint(state, entry)
            );
        }
        drop(instances);
        drop(registry);
    }
    fingerprint
}

pub(super) fn build_list_response(state: &ServiceState) -> ListResponse {
    let mut sandboxes: Vec<SandboxInfo> = Vec::new();

    // Running instances. Keep this list route in-memory only; callers that
    // need ledger-backed counters use explicit stats routes instead of making
    // every UI/TUI poll open session.db.
    {
        let instances = state.instances.lock().unwrap();
        for i in instances.values() {
            let mut info = SandboxInfo::new(
                i.id.clone(),
                i.profile_id.clone(),
                i.pid,
                VmLifecycleState::Running,
                i.persistent,
            );
            info.name = Some(i.name.clone());
            info.ram_mb = Some(i.ram_mb);
            info.cpus = Some(i.cpus);
            info.version = Some(i.base_version.clone());
            info.forked_from = i.forked_from.clone();
            info.uptime_secs = Some(i.start_time.elapsed().as_secs());
            info.can_resume = false;
            info.refresh_available_actions();
            sandboxes.push(info);
        }
    }

    // Stopped/Suspended/Defunct persistent VMs (not in instances map).
    // `Defunct` surfaces a boot failure so users see the problem in
    // `capsem list` instead of a misleading "Stopped" -- last_error
    // carries the tail of process.log for one-line diagnosis.
    let inactive_persistent: Vec<PersistentVmEntry> = {
        let registry = state.persistent_registry.lock().unwrap();
        let instances = state.instances.lock().unwrap();
        registry
            .list()
            .filter(|entry| !instances.contains_key(&persistent_entry_vm_id(entry)))
            .cloned()
            .collect()
    };
    for entry in inactive_persistent {
        let vm_id = persistent_entry_vm_id(&entry);
        let (status, can_resume, blocked_reason) = state.persistent_entry_resume_state_cached(&entry);
        let mut info = SandboxInfo::new(vm_id, entry.profile_id.clone(), 0, status, true);
        info.name = Some(entry.name.clone());
        info.ram_mb = Some(entry.ram_mb);
        info.cpus = Some(entry.cpus);
        info.version = Some(entry.base_version.clone());
        info.forked_from = entry.forked_from.clone();
        info.description = entry.description.clone();
        info.can_resume = can_resume;
        if can_resume {
            info.resume_blocked_reason = None;
        } else if entry.defunct {
            info.last_error = blocked_reason;
        } else {
            info.resume_blocked_reason = blocked_reason;
        }
        info.refresh_available_actions();
        sandboxes.push(info);
    }

    ListResponse { sandboxes }
}

pub(super) async fn handle_list(State(state): State<Arc<ServiceState>>) -> axum::response::Response {
    state.reconcile_persistent_defunct_from_logs();
    let fingerprint = list_response_fingerprint(&state);
    if let Some(cached) = state.list_response_cache.lock().unwrap().clone() {
        if cached.fingerprint == fingerprint {
            return json_bytes_response(cached.bytes);
        }
    }

    let response = build_list_response(&state);
    let bytes = Bytes::from(serde_json::to_vec(&response).unwrap_or_default());
    *state.list_response_cache.lock().unwrap() = Some(CachedListResponse {
        fingerprint,
        bytes: bytes.clone(),
    });
    json_bytes_response(bytes)
}

pub(super) async fn handle_info(
    State(state): State<Arc<ServiceState>>,
    Path(id): Path<String>,
) -> Result<Json<SandboxInfo>, AppError> {
    state.reconcile_persistent_defunct_from_logs();
    // Check running instances first
    {
        let (instance_data, session_dir) = {
            let instances = state.instances.lock().unwrap();
            match instances.get(&id) {
                Some(i) => {
                    let mut info = SandboxInfo::new(
                        i.id.clone(),
                        i.profile_id.clone(),
                        i.pid,
                        VmLifecycleState::Running,
                        i.persistent,
                    );
                    info.name = Some(i.name.clone());
                    info.ram_mb = Some(i.ram_mb);
                    info.cpus = Some(i.cpus);
                    info.version = Some(i.base_version.clone());
                    info.forked_from = i.forked_from.clone();
                    info.uptime_secs = Some(i.start_time.elapsed().as_secs());
                    info.can_resume = false;
                    info.refresh_available_actions();
                    (Some(info), Some(i.session_dir.clone()))
                }
                None => (None, None),
            }
        };
        if let (Some(mut info), Some(dir)) = (instance_data, session_dir) {
            apply_session_db_status(&state, &mut info, &dir).await;
            info.storage = state.storage_diagnostics_cached(&dir);
            return Ok(Json(info));
        }
    }

    // Check stopped/suspended/defunct persistent VMs
    let persistent_entry = find_persistent_entry_by_route_id(&state, &id);
    if let Some(entry) = persistent_entry {
        let vm_id = persistent_entry_vm_id(&entry);
        let (status, can_resume, blocked_reason) = state.persistent_entry_resume_state_cached(&entry);
        let mut info = SandboxInfo::new(vm_id, entry.profile_id.clone(), 0, status, true);
        info.name = Some(entry.name.clone());
        info.ram_mb = Some(entry.ram_mb);
        info.cpus = Some(entry.cpus);
        info.version = Some(entry.base_version.clone());
        info.forked_from = entry.forked_from.clone();
        info.description = entry.description.clone();
        info.can_resume = can_resume;
        if can_resume {
            info.resume_blocked_reason = None;
        } else if entry.defunct {
            info.last_error = blocked_reason;
        } else {
            info.resume_blocked_reason = blocked_reason;
        }
        info.refresh_available_actions();
        // Disk usage is a recursive walk of the session dir (including every
        // snapshot clone). Run it off the async worker so it does not stall the
        // axum runtime, and log rather than silently swallow a failure.
        let session_dir = entry.session_dir.clone();
        info.size_bytes =
            match tokio::task::spawn_blocking(move || capsem_core::auto_snapshot::sandbox_disk_usage(&session_dir))
                .await
            {
                Ok(Ok(bytes)) => Some(bytes),
                Ok(Err(error)) => {
                    tracing::debug!(error = %error, "sandbox disk usage computation failed");
                    None
                }
                Err(error) => {
                    tracing::debug!(error = %error, "sandbox disk usage task failed");
                    None
                }
            };
        apply_session_db_status(&state, &mut info, &entry.session_dir).await;
        info.storage = state.storage_diagnostics_cached(&entry.session_dir);
        return Ok(Json(info));
    }

    Err(AppError(StatusCode::NOT_FOUND, format!("sandbox not found: {id}")))
}

pub(super) async fn handle_vm_status(
    State(state): State<Arc<ServiceState>>,
    Path(id): Path<String>,
) -> Result<Json<api::VmStatusResponse>, AppError> {
    state.reconcile_persistent_defunct_from_logs();
    {
        let instances = state.instances.lock().unwrap();
        if let Some(i) = instances.get(&id) {
            return Ok(Json(api::VmStatusResponse {
                id: i.id.clone(),
                name: i.name.clone(),
                status: VmLifecycleState::Running,
                pid: Some(i.pid),
                persistent: i.persistent,
                uptime_secs: Some(i.start_time.elapsed().as_secs()),
                created_at: None,
                last_error: None,
                can_resume: false,
                resume_blocked_reason: None,
                storage: state.storage_diagnostics_cached(&i.session_dir),
                available_actions: VmLifecycleState::Running.available_actions(false),
            }));
        }
    }

    {
        if let Some(entry) = find_persistent_entry_by_route_id(&state, &id) {
            let vm_id = persistent_entry_vm_id(&entry);
            let (status, can_resume, blocked_reason) = state.persistent_entry_resume_state_cached(&entry);
            return Ok(Json(api::VmStatusResponse {
                id: vm_id,
                name: entry.name.clone(),
                status,
                pid: None,
                persistent: true,
                uptime_secs: None,
                created_at: Some(entry.created_at.clone()),
                last_error: if entry.defunct {
                    blocked_reason.clone()
                } else {
                    entry.last_error.clone()
                },
                can_resume,
                resume_blocked_reason: if can_resume || entry.defunct {
                    None
                } else {
                    blocked_reason
                },
                storage: state.storage_diagnostics_cached(&entry.session_dir),
                available_actions: status.available_actions(can_resume),
            }));
        }
    }

    Err(AppError(StatusCode::NOT_FOUND, format!("sandbox not found: {id}")))
}

pub(super) async fn handle_vm_snapshots_status(
    State(state): State<Arc<ServiceState>>,
    Path(id): Path<String>,
) -> Result<Json<capsem_proto::ipc::SnapshotStatus>, AppError> {
    if let Some(uds_path) = {
        let instances = state.instances.lock().unwrap();
        instances.get(&id).map(|instance| instance.uds_path.clone())
    } {
        let request_id = state.job_counter.fetch_add(1, Ordering::SeqCst);
        let response = send_ipc_command(&uds_path, ServiceToProcess::SnapshotStatus { id: request_id }, Some(5))
            .await
            .map_err(|error| AppError(StatusCode::BAD_GATEWAY, error))?;
        return match response {
            ProcessToService::SnapshotStatusResult {
                id: response_id,
                status,
            } if response_id == request_id => Ok(Json(status)),
            other => Err(AppError(
                StatusCode::BAD_GATEWAY,
                format!("unexpected snapshot status IPC response: {other:?}"),
            )),
        };
    }

    let session_dir = resolve_session_dir(&state, &id)?;
    Ok(Json(snapshot_status_from_session_dir(&session_dir)))
}

pub(super) async fn handle_vm_snapshots_list(
    State(state): State<Arc<ServiceState>>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    let Json(status) = handle_vm_snapshots_status(State(state), Path(id)).await?;
    Ok(Json(serde_json::json!({
        "total": status.total,
        "snapshots": status.snapshots,
    })))
}

pub(super) fn snapshot_status_from_session_dir(session_dir: &std::path::Path) -> capsem_proto::ipc::SnapshotStatus {
    let scheduler = capsem_core::auto_snapshot::AutoSnapshotScheduler::new(
        session_dir.to_path_buf(),
        10,
        12,
        std::time::Duration::from_secs(300),
    );
    let snapshots = scheduler.list_snapshots();
    let auto_count = snapshots
        .iter()
        .filter(|slot| slot.origin == capsem_core::auto_snapshot::SnapshotOrigin::Auto)
        .count();
    let manual_count = snapshots.len().saturating_sub(auto_count);
    let snapshots = snapshots
        .into_iter()
        .map(|slot| capsem_proto::ipc::SnapshotSlotStatus {
            checkpoint: format!("cp-{}", slot.slot),
            slot: slot.slot,
            origin: match slot.origin {
                capsem_core::auto_snapshot::SnapshotOrigin::Auto => "auto",
                capsem_core::auto_snapshot::SnapshotOrigin::Manual => "manual",
            }
            .to_string(),
            name: slot.name,
            timestamp: humantime::format_rfc3339(slot.timestamp).to_string(),
            hash: slot.hash,
        })
        .collect();
    capsem_proto::ipc::SnapshotStatus {
        total: auto_count + manual_count,
        auto_count,
        manual_count,
        manual_available: scheduler.available_manual_slots(),
        snapshots,
    }
}

pub(super) async fn vm_operation_status(
    state: Arc<ServiceState>,
    id: String,
    operation: &'static str,
) -> Result<Json<api::VmOperationStatusResponse>, AppError> {
    let _ = handle_vm_status(State(Arc::clone(&state)), Path(id.clone())).await?;
    Ok(Json(api::VmOperationStatusResponse {
        vm_id: id,
        operation: operation.into(),
        status: "idle".into(),
        in_progress: false,
        message: Some("operation progress is not asynchronous in this build".into()),
    }))
}

pub(super) async fn handle_vm_save_status(
    State(state): State<Arc<ServiceState>>,
    Path(id): Path<String>,
) -> Result<Json<api::VmOperationStatusResponse>, AppError> {
    vm_operation_status(state, id, "save").await
}

pub(super) async fn handle_vm_fork_status(
    State(state): State<Arc<ServiceState>>,
    Path(id): Path<String>,
) -> Result<Json<api::VmOperationStatusResponse>, AppError> {
    vm_operation_status(state, id, "fork").await
}

/// GET /stats -- return global stats from the canonical ledger.
pub(super) async fn handle_stats(State(state): State<Arc<ServiceState>>) -> Result<impl IntoResponse, AppError> {
    let body = read_stats_response_from_main_db_handle(&state).await?;
    Ok((
        StatusCode::OK,
        [(axum::http::header::CONTENT_TYPE, "application/json")],
        Bytes::from(body),
    ))
}

/// GET /vms/{id}/stats/detail -- return fixed UI stats/detail ledgers.
pub(super) async fn handle_stats_detail(
    State(state): State<Arc<ServiceState>>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, AppError> {
    let session_dir = resolve_session_dir(&state, &id)?;
    let db_path = session_dir.join("session.db");
    if let Some(body) = session_response_cache_get(&state, &id, "stats_detail", &db_path) {
        return Ok(json_bytes_response(body));
    }

    let payload = read_stats_detail_payload_from_session_db(&state, &id, &db_path).await?;
    let body = serde_json::to_vec(&payload).map_err(|error| {
        AppError(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("failed to serialize stats detail response: {error}"),
        )
    })?;
    session_response_cache_store(&state, &id, "stats_detail", &db_path, &body);
    Ok(json_bytes_response(Bytes::from(body)))
}

/// GET /vms/{id}/stats/summary -- return compact live toolbar counters.
pub(super) async fn handle_stats_summary(
    State(state): State<Arc<ServiceState>>,
    Path(id): Path<String>,
) -> Result<Json<api::VmStatsSummaryResponse>, AppError> {
    let session_dir = resolve_session_dir(&state, &id)?;
    let db_path = session_dir.join("session.db");
    let db = open_ready_session_db(&state, &id, "stats_summary", &db_path).await?;
    let stats = db
        .session_stats()
        .await
        .map_err(|error| ledger_route_error(&id, "stats_summary", "query", &db_path, error))?;
    Ok(Json(api::VmStatsSummaryResponse {
        total_requests: stats.net_total,
        allowed_requests: stats.net_allowed,
        denied_requests: stats.net_denied,
        total_input_tokens: stats.total_input_tokens,
        total_thinking_tokens: stats.total_usage_details.get("thinking").copied().unwrap_or_default(),
        total_output_tokens: stats.total_output_tokens,
        total_tool_calls: stats.total_tool_calls,
        total_estimated_cost: stats.total_estimated_cost_usd,
    }))
}

pub(super) async fn handle_logs(
    State(state): State<Arc<ServiceState>>,
    Path(id): Path<String>,
) -> Result<Json<LogsResponse>, AppError> {
    let session_dir = {
        let instances = state.instances.lock().unwrap();
        if let Some(i) = instances.get(&id) {
            i.session_dir.clone()
        } else {
            match find_persistent_entry_by_route_id(&state, &id).map(|e| e.session_dir) {
                Some(dir) => dir,
                None => {
                    // VM might have crashed on boot. preserve_failed_session_dir
                    // renames `sessions/<id>` to `sessions/<id>-failed-<suffix>`,
                    // so the most recent `<id>-failed-*` still has the logs the
                    // user needs to debug the crash. Without this branch
                    // `capsem logs <id>` just returns 404 after a boot failure,
                    // which is exactly when logs matter most.
                    match find_failed_session_dir(&state.run_dir, &id) {
                        Some(dir) => dir,
                        None => return Err(AppError(StatusCode::NOT_FOUND, format!("sandbox not found: {id}"))),
                    }
                }
            }
        }
    };

    let serial_log_path = session_dir.join("serial.log");
    let process_log_path = session_dir.join("process.log");

    // Bounded and rotation-aware. `serial.log` is guest-controlled console
    // output written through `CappedLogWriter`, so it both rotates and can be
    // arbitrarily large -- reading the whole bare file lost the rotated slice
    // and let the guest choose the allocation.
    let (serial_logs, process_logs) = tokio::task::spawn_blocking(move || {
        let serial = capsem_foundation::telemetry::read_log_tail(&serial_log_path, SESSION_LOG_TAIL_MAX_BYTES);
        let process = capsem_foundation::telemetry::read_log_tail(&process_log_path, SESSION_LOG_TAIL_MAX_BYTES);
        (serial, process)
    })
    .await
    .map_err(|e| AppError(StatusCode::INTERNAL_SERVER_ERROR, format!("log read failed: {e}")))?;

    Ok(Json(LogsResponse {
        logs: serial_logs.as_deref().unwrap_or("").to_string(),
        serial_logs,
        process_logs,
    }))
}

/// `GET /panics?since=30m&limit=20` -- structured panic + backtrace
/// extractor across all host log files. Returns JSON array. Used by the
/// `capsem_panics` MCP tool.
pub(super) async fn handle_panics(
    State(state): State<Arc<ServiceState>>,
    axum::extract::Query(params): axum::extract::Query<TriageQuery>,
) -> Result<axum::Json<serde_json::Value>, AppError> {
    let since_unix = params
        .since
        .as_deref()
        .and_then(triage::parse_since)
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let limit = params.limit.unwrap_or(20).min(200);

    let run_dir = state.run_dir.clone();
    let home = capsem_foundation::paths::capsem_home();

    let mut all_panics: Vec<triage::PanicEvent> = Vec::new();
    for binary in ["service", "mcp", "gateway", "tray"] {
        if let Some(path) = triage::host_log_path(&run_dir, binary) {
            all_panics.extend(triage::scan_panics_in_file(
                &path,
                &format!("capsem-{binary}"),
                since_unix,
            ));
        }
    }
    if let Some(path) = triage::latest_app_log(&home) {
        all_panics.extend(triage::scan_panics_in_file(&path, "capsem-app", since_unix));
    }

    all_panics.truncate(limit);
    Ok(axum::Json(serde_json::json!({ "panics": all_panics })))
}

/// `GET /triage?id=<vm>&since=30m&limit=20` -- ranked summary of recent
/// panics, errors, and slow ops across host logs (and, when `id` is
/// provided, session.db error rows). Used by the `capsem_triage` MCP
/// tool.
pub(super) async fn handle_triage(
    State(state): State<Arc<ServiceState>>,
    axum::extract::Query(params): axum::extract::Query<TriageQuery>,
) -> Result<axum::Json<serde_json::Value>, AppError> {
    let since_str = params.since.clone().unwrap_or_else(|| "30m".to_string());
    let since_unix = triage::parse_since(&since_str)
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let limit = params.limit.unwrap_or(20).min(200);

    let run_dir = state.run_dir.clone();
    let home = capsem_foundation::paths::capsem_home();

    let mut panics: Vec<triage::PanicEvent> = Vec::new();
    let mut errors: Vec<triage::ErrorEvent> = Vec::new();
    let mut slow_ops: Vec<triage::SlowOpEvent> = Vec::new();

    for binary in ["service", "mcp", "gateway", "tray"] {
        if let Some(path) = triage::host_log_path(&run_dir, binary) {
            let bin_label = format!("capsem-{binary}");
            panics.extend(triage::scan_panics_in_file(&path, &bin_label, since_unix));
            errors.extend(triage::scan_errors_in_file(&path, &bin_label, since_unix, limit));
            slow_ops.extend(triage::scan_slow_ops_in_file(&path, &bin_label, since_unix, 500));
        }
    }
    if let Some(path) = triage::latest_app_log(&home) {
        panics.extend(triage::scan_panics_in_file(&path, "capsem-app", since_unix));
        errors.extend(triage::scan_errors_in_file(&path, "capsem-app", since_unix, limit));
    }

    panics.truncate(limit);
    errors.truncate(limit);
    slow_ops.truncate(limit);

    // When `id` is set, add session-scoped error signals from the canonical
    // session ledger. The future DB-owned mem layer can make this fast; the
    // service route does not own a separate logged-data copy.
    let session_block = if let Some(ref vm_id) = params.id {
        triage_for_vm(&state, vm_id, limit).await?
    } else {
        serde_json::json!({})
    };

    // Build a deterministic ranked-list of the highest-blast-radius items
    // first: panics > unhandled-enum warns > slow_op events > everything else.
    let mut rank: Vec<String> = Vec::new();
    for p in panics.iter().take(5) {
        rank.push(format!(
            "panic {} in {} at {} -- {}",
            p.ts.as_str().chars().take(19).collect::<String>(),
            p.binary,
            p.location.clone().unwrap_or_else(|| "?".into()),
            p.message.chars().take(120).collect::<String>(),
        ));
    }
    for e in errors.iter().filter(|e| e.target.as_deref() == Some("ipc")).take(3) {
        rank.push(format!(
            "ipc-warn {} in {} -- {}",
            e.ts.as_str().chars().take(19).collect::<String>(),
            e.binary,
            e.message.chars().take(120).collect::<String>(),
        ));
    }
    for s in slow_ops.iter().take(3) {
        rank.push(format!(
            "slow_op {} {} {}ms in {}",
            s.ts.as_str().chars().take(19).collect::<String>(),
            s.op,
            s.duration_ms,
            s.binary,
        ));
    }

    let out = serde_json::json!({
        "since": since_str,
        "session_id": params.id,
        "host": {
            "panics": panics,
            "errors": errors,
            "slow_ops": slow_ops,
        },
        "session": session_block,
        "rank": rank,
    });
    Ok(axum::Json(out))
}

pub(super) async fn session_db_triage(
    vm_id: &str,
    db: &capsem_logger::DbHandle,
    db_path: &std::path::Path,
    limit: usize,
) -> anyhow::Result<serde_json::Value> {
    db.ready()
        .await
        .map_err(|error| anyhow!("session triage ledger is not ready for {vm_id}: {error}"))?;
    let denied_net_sql = format!(
        "SELECT timestamp, domain, decision, status_code, duration_ms \
         FROM net_events WHERE decision = 'denied' OR status_code >= 500 \
         ORDER BY timestamp DESC LIMIT {limit}"
    );
    let tool_errors_sql = format!(
        "SELECT timestamp, server_name, method, decision, policy_mode, policy_action, \
                policy_rule, policy_reason, error_message, duration_ms \
         FROM tool_calls \
         WHERE origin IN ('native', 'mcp', 'builtin', 'local') \
           AND (decision IN ('denied','error') OR error_message IS NOT NULL) \
         ORDER BY timestamp DESC LIMIT {limit}"
    );
    let exec_failures_sql = format!(
        "SELECT timestamp, exec_id, command, exit_code, duration_ms \
         FROM exec_events WHERE exit_code IS NOT NULL AND exit_code != 0 \
         ORDER BY timestamp DESC LIMIT {limit}"
    );

    async fn read_query(
        db: &capsem_logger::DbHandle,
        vm_id: &str,
        db_path: &std::path::Path,
        query_name: &str,
        sql: &str,
    ) -> anyhow::Result<serde_json::Value> {
        let raw = db.query(sql, &[]).await.map_err(|error| {
            error!(
                vm_id,
                query_name,
                db_path = %db_path.display(),
                error = %error,
                "session triage ledger query failed"
            );
            anyhow!("session triage query {query_name} failed: {error}")
        })?;
        serde_json::from_str(&raw).map_err(|error| {
            error!(
                vm_id,
                query_name,
                db_path = %db_path.display(),
                error = %error,
                "session triage ledger query returned invalid JSON"
            );
            anyhow!("session triage query {query_name} returned invalid JSON: {error}")
        })
    }

    let denied_net_v = read_query(db, vm_id, db_path, "denied_net", &denied_net_sql).await?;
    let tool_errors_v = read_query(db, vm_id, db_path, "tool_errors", &tool_errors_sql).await?;
    let exec_failures_v = read_query(db, vm_id, db_path, "exec_failures", &exec_failures_sql).await?;

    Ok(serde_json::json!({
        "denied_net": denied_net_v,
        "tool_errors": tool_errors_v,
        "exec_failures": exec_failures_v,
    }))
}

pub(super) fn limit_columnar_query_json(value: &serde_json::Value, limit: usize) -> serde_json::Value {
    let columns = value.get("columns").cloned().unwrap_or_else(|| json!([]));
    let rows = value
        .get("rows")
        .and_then(|value| value.as_array())
        .map(|rows| rows.iter().take(limit).cloned().collect::<Vec<_>>())
        .unwrap_or_default();
    json!({
        "columns": columns,
        "rows": rows,
    })
}

pub(super) fn limit_triage_session_block(value: &serde_json::Value, limit: usize) -> serde_json::Value {
    json!({
        "denied_net": limit_columnar_query_json(&value["denied_net"], limit),
        "tool_errors": limit_columnar_query_json(&value["tool_errors"], limit),
        "exec_failures": limit_columnar_query_json(&value["exec_failures"], limit),
    })
}

pub(super) async fn triage_for_vm(
    state: &ServiceState,
    vm_id: &str,
    limit: usize,
) -> Result<serde_json::Value, AppError> {
    let session_dir = match resolve_session_dir(state, vm_id) {
        Ok(session_dir) => session_dir,
        Err(_) => {
            return Ok(json!({ "missing": true, "reason": "session not found" }));
        }
    };
    let db_path = session_db_path_for_session_dir(&session_dir);
    if !db_path.exists() {
        return Ok(json!({ "missing": true, "reason": "session not found" }));
    }
    let db = open_ready_session_db(state, vm_id, "triage", &db_path).await?;
    let session = session_db_triage(vm_id, &db, &db_path, limit).await.map_err(|error| {
        AppError(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("failed to read triage ledger for {vm_id}: {error}"),
        )
    })?;
    Ok(limit_triage_session_block(&session, limit))
}

#[derive(Deserialize, Debug, Default)]
pub(super) struct TriageQuery {
    /// Lookback window. Default "30m". Accepts "5m", "1h", "24h", or
    /// RFC3339 ("2026-05-02T17:30:00Z").
    since: Option<String>,
    /// Max items per category. Default 20, capped at 200.
    limit: Option<usize>,
    /// Optional session id (reserved for the future session.db query).
    id: Option<String>,
}

/// `GET /host-logs/{name}?grep=&tail=&max_bytes=` -- read a host-side log
/// file by symbolic name. Hard-coded allowlist (no path traversal). Used
/// by the `capsem_host_logs` MCP tool (T3) but the endpoint already lands
/// in this commit so a future T3 sub-sprint can wire the MCP tool without
/// touching the service.
pub(super) async fn handle_host_logs(
    State(state): State<Arc<ServiceState>>,
    axum::extract::Path(name): axum::extract::Path<String>,
    axum::extract::Query(params): axum::extract::Query<HostLogsQuery>,
) -> Result<String, AppError> {
    let path = if name == "app" {
        triage::latest_app_log(&capsem_foundation::paths::capsem_home())
            .ok_or_else(|| AppError(StatusCode::NOT_FOUND, "no app log found".into()))?
    } else {
        triage::host_log_path(&state.run_dir, &name)
            .ok_or_else(|| AppError(StatusCode::BAD_REQUEST, format!("unknown log name: {name}")))?
    };
    let max_bytes = params.max_bytes.unwrap_or(100 * 1024).min(5 * 1024 * 1024);
    // `service.log` names a daily-rotated stream, so opening that exact name
    // returns nothing the moment it has rotated -- this endpoint reported an
    // empty log for a service that was writing normally. Reading through the
    // stream reader also removes the fourth hand-rolled copy of seek-from-end
    // and trim-the-partial-line in this crate.
    let text = tokio::task::spawn_blocking(move || {
        capsem_foundation::telemetry::read_log_tail(&path, max_bytes as usize).unwrap_or_default()
    })
    .await
    .map_err(|e| AppError(StatusCode::INTERNAL_SERVER_ERROR, format!("log read failed: {e}")))?;

    // Apply grep + tail post-filters here so the wire surface to the
    // capsem_host_logs MCP tool can avoid two round-trips.
    let mut text = text;
    if let Some(pat) = &params.grep {
        text = text.lines().filter(|l| l.contains(pat)).collect::<Vec<_>>().join("\n");
    }
    if let Some(n) = params.tail {
        let lines: Vec<&str> = text.lines().collect();
        let start = lines.len().saturating_sub(n);
        text = lines[start..].join("\n");
    }
    Ok(text)
}

#[derive(Deserialize, Debug, Default)]
pub(super) struct HostLogsQuery {
    grep: Option<String>,
    tail: Option<usize>,
    max_bytes: Option<u64>,
}

pub(super) async fn handle_service_logs(State(state): State<Arc<ServiceState>>) -> Result<String, AppError> {
    let log_path = state.run_dir.join("service.log");

    let text = tokio::task::spawn_blocking(move || -> Result<String, String> {
        // `service.log` names a daily-rotated stream, not a file. Resolution
        // and tailing live in one place so every consumer sees the same log.
        capsem_foundation::telemetry::read_log_tail(&log_path, 100 * 1024)
            .ok_or_else(|| format!("no log files in stream {}", log_path.display()))
    })
    .await
    .map_err(|e| AppError(StatusCode::INTERNAL_SERVER_ERROR, format!("log read failed: {e}")))?
    .map_err(|e| AppError(StatusCode::INTERNAL_SERVER_ERROR, e))?;

    Ok(text)
}

#[tracing::instrument(skip_all, fields(cmd = ?std::mem::discriminant(&cmd), timeout_secs = ?timeout_secs))]
pub(super) async fn send_ipc_command(
    uds_path: &std::path::Path,
    cmd: ServiceToProcess,
    timeout_secs: Option<u64>,
) -> Result<ProcessToService, String> {
    let stream = tokio::net::UnixStream::connect(uds_path)
        .await
        .map_err(|e| format!("failed to connect to sandbox: {e}"))?;
    let mut std_stream = stream
        .into_std()
        .map_err(|e| format!("failed to convert stream: {e}"))?;
    capsem_foundation::ipc_handshake::negotiate_initiator(
        &mut std_stream,
        "capsem-service",
        capsem_foundation::telemetry::current_parent_traceparent(),
    )
    .map_err(|e| format!("IPC handshake failed: {e}"))?;
    let (tx, rx): (Sender<ServiceToProcess>, Receiver<ProcessToService>) =
        channel_from_std(std_stream).map_err(|e| format!("failed to create IPC channel: {e}"))?;

    tx.send(cmd.clone())
        .await
        .map_err(|e| format!("failed to send IPC command: {e}"))?;

    let deadline = timeout_secs.map(|secs| tokio::time::Instant::now() + std::time::Duration::from_secs(secs));
    loop {
        let msg = match deadline {
            Some(deadline) => match tokio::time::timeout_at(deadline, rx.recv()).await {
                Ok(Ok(msg)) => msg,
                Ok(Err(e)) => {
                    error!(?e, "IPC receive error");
                    return Err(format!("IPC connection closed: {e}"));
                }
                Err(_) => {
                    let secs = timeout_secs.unwrap_or_default();
                    return Err(format!("IPC command timed out after {secs}s"));
                }
            },
            None => match rx.recv().await {
                Ok(msg) => msg,
                Err(e) => {
                    error!(?e, "IPC receive error");
                    return Err(format!("IPC connection closed: {e}"));
                }
            },
        };

        match msg {
            ProcessToService::Pong => {
                if matches!(cmd, ServiceToProcess::Ping | ServiceToProcess::ReloadConfig) {
                    return Ok(ProcessToService::Pong);
                }
                continue;
            }
            ProcessToService::TerminalOutput { .. } => continue,
            ProcessToService::StateChanged { .. } => continue,
            res => return Ok(res),
        }
    }
}

/// Wait until a VM signals readiness via a `.ready` sentinel file.
/// The capsem-process creates this file once the guest handshake completes.
///
/// If `state` and `id` are provided, also checks on every poll iteration that
/// the VM is still in the instance registry. The resume_sandbox / spawn child-
/// exit handlers remove the instance when capsem-process dies; observing that
/// removal lets us fail fast (within ~50ms) instead of polling the dead
/// sentinel for the full timeout. Without this, a capsem-process that crashes
/// or exits during boot/restore would hang the API for `timeout_secs` (was
/// reproducibly 30s under heavy suspend/resume churn).
#[tracing::instrument(skip_all, fields(timeout_secs))]
pub(super) async fn wait_for_vm_ready(
    uds_path: &std::path::Path,
    timeout_secs: u64,
    state: Option<&Arc<ServiceState>>,
    id: Option<&str>,
) -> Result<(), String> {
    let ready_span = tracing::debug_span!(
        target: "capsem.launch",
        capsem_foundation::telemetry::LAUNCH_VSOCK_READY_SPAN,
        status = tracing::field::Empty,
    );
    let ready_path = uds_path.with_extension("ready");
    // Override the PollOpts::new defaults (50ms / 500ms): VM ready-time is
    // sub-second in the common case and the sentinel check is a single stat,
    // so 500ms max_delay overshoots readiness by ~500ms and blows the
    // exec_ready / boot_ready latency gates. Peer callers (service-connect,
    // gateway-ready) wait for remote processes with seconds-scale startup
    // where 500ms is appropriate; this poll is different.
    let opts = vm_ready_poll_opts(timeout_secs);
    let died: Arc<std::sync::atomic::AtomicBool> = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let res = capsem_foundation::poll::poll_until(opts, || {
        let ready = ready_path.clone();
        let state = state.cloned();
        let id = id.map(|s| s.to_string());
        let died = Arc::clone(&died);
        async move {
            if ready.exists() {
                return Some(());
            }
            if let (Some(st), Some(name)) = (state.as_ref(), id.as_ref()) {
                if !st.instances.lock().unwrap().contains_key(name) {
                    died.store(true, std::sync::atomic::Ordering::Release);
                    // Returning Some short-circuits the poll loop; the
                    // outer caller distinguishes via `died`.
                    return Some(());
                }
            }
            None
        }
    })
    .instrument(ready_span.clone())
    .await;
    if died.load(std::sync::atomic::Ordering::Acquire) {
        ready_span.record("status", "error");
        return Err("capsem-process exited before signalling ready".into());
    }
    match res {
        Ok(()) => {
            ready_span.record("status", "ok");
            Ok(())
        }
        Err(error) => {
            ready_span.record("status", "error");
            Err(format!("{error}"))
        }
    }
}

pub(super) fn vm_ready_poll_opts(timeout_secs: u64) -> capsem_foundation::poll::PollOpts {
    capsem_foundation::poll::PollOpts {
        initial_delay: std::time::Duration::from_millis(5),
        max_delay: std::time::Duration::from_millis(50),
        ..capsem_foundation::poll::PollOpts::new("vm-ready", std::time::Duration::from_secs(timeout_secs))
    }
}

fn running_uds_path(state: &ServiceState, id: &str) -> Result<std::path::PathBuf, AppError> {
    let instances = state.instances.lock().unwrap();
    let path = instances
        .get(id)
        .ok_or_else(|| AppError(StatusCode::NOT_FOUND, format!("sandbox not found: {id}")))?
        .uds_path
        .clone();
    drop(instances);
    Ok(path)
}

pub(super) async fn handle_exec(
    State(state): State<Arc<ServiceState>>,
    Path(id): Path<String>,
    Json(payload): Json<ExecRequest>,
) -> Result<Json<ExecResponse>, AppError> {
    let uds_path = running_uds_path(&state, &id)?;

    wait_for_vm_ready(&uds_path, 30, Some(&state), Some(&id))
        .await
        .map_err(|e| AppError(StatusCode::INTERNAL_SERVER_ERROR, e))?;

    let id_val = state.next_job_id();
    let command = payload.command;
    let res = send_ipc_command(
        &uds_path,
        ServiceToProcess::Exec {
            id: id_val,
            command: command.clone(),
        },
        payload.timeout_secs,
    )
    .await
    .map_err(|e| AppError(StatusCode::INTERNAL_SERVER_ERROR, e))?;

    match res {
        ProcessToService::ExecResult {
            stdout,
            stderr,
            exit_code,
            truncated,
            ..
        } => Ok(Json(ExecResponse {
            stdout: String::from_utf8(stdout).unwrap_or_else(|e| String::from_utf8_lossy(e.as_bytes()).into_owned()),
            stderr: String::from_utf8(stderr).unwrap_or_else(|e| String::from_utf8_lossy(e.as_bytes()).into_owned()),
            exit_code,
            truncated,
        })),
        _ => Err(AppError(
            StatusCode::INTERNAL_SERVER_ERROR,
            "unexpected IPC response for exec".to_string(),
        )),
    }
}

pub(super) async fn handle_write_file(
    State(state): State<Arc<ServiceState>>,
    Path(id): Path<String>,
    Json(payload): Json<WriteFileRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    let uds_path = running_uds_path(&state, &id)?;

    let mut data = payload.content.into_bytes();
    let path = payload.path;
    let size = data.len() as u64;
    if let Some(rewritten) = log_file_boundary(
        &state,
        &id,
        FileBoundaryAction::Import,
        path.clone(),
        file_security_preview_bytes(&data),
        size,
        None,
    )
    .await?
    {
        data = rewritten;
    }

    let id_val = state.next_job_id();
    let res = send_ipc_command(
        &uds_path,
        ServiceToProcess::WriteFile {
            id: id_val,
            path: path.clone(),
            data,
        },
        Some(30),
    )
    .await
    .map_err(|error| {
        AppError(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("VM {id} write_file failed while awaiting the guest completion response: {error}"),
        )
    })?;

    match res {
        ProcessToService::WriteFileResult { success, error, .. } => {
            if success {
                Ok(Json(json!({ "success": true })))
            } else {
                Err(AppError(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    error.unwrap_or_else(|| "unknown write error".into()),
                ))
            }
        }
        _ => Err(AppError(
            StatusCode::INTERNAL_SERVER_ERROR,
            "unexpected IPC response for write_file".to_string(),
        )),
    }
}

pub(super) async fn handle_read_file(
    State(state): State<Arc<ServiceState>>,
    Path(id): Path<String>,
    Json(payload): Json<ReadFileRequest>,
) -> Result<Json<ReadFileResponse>, AppError> {
    let path = &payload.path;
    let uds_path = running_uds_path(&state, &id)?;

    wait_for_vm_ready(&uds_path, 30, Some(&state), Some(&id))
        .await
        .map_err(|e| AppError(StatusCode::INTERNAL_SERVER_ERROR, e))?;

    let id_val = state.next_job_id();
    let res = send_ipc_command(
        &uds_path,
        ServiceToProcess::ReadFile {
            id: id_val,
            path: path.clone(),
        },
        Some(30),
    )
    .await
    .map_err(|e| AppError(StatusCode::INTERNAL_SERVER_ERROR, e))?;

    match res {
        ProcessToService::ReadFileResult { data, error, .. } => {
            if let Some(d) = data {
                Ok(Json(ReadFileResponse {
                    content: String::from_utf8(d)
                        .unwrap_or_else(|e| String::from_utf8_lossy(e.as_bytes()).into_owned()),
                }))
            } else {
                Err(AppError(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    error.unwrap_or_else(|| "unknown read error".into()),
                ))
            }
        }
        _ => Err(AppError(
            StatusCode::INTERNAL_SERVER_ERROR,
            "unexpected IPC response for read_file".to_string(),
        )),
    }
}
