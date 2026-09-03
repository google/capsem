use std::io::{Read, Write};
use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result};
use capsem_proto::{decode_guest_msg, encode_host_msg, GuestToHost, HostToGuest, MAX_FRAME_SIZE};
use tokio::sync::mpsc;
use tracing::{debug_span, info, info_span, warn};

use crate::host_state::{HostState, HostStateMachine};
use crate::hypervisor::{Hypervisor, VmHandle, VsockConnection};
use crate::vm::config::VmConfig;

#[cfg(target_os = "macos")]
use crate::hypervisor::apple_vz::AppleVzHypervisor;
#[cfg(target_os = "linux")]
use crate::hypervisor::kvm::KvmHypervisor;
use crate::net::cert_authority::CertAuthority;
use crate::net::mitm_proxy;
use crate::net::policy_config;
use crate::VirtioFsShare;
use capsem_logger::DbWriter;

use super::registry::SandboxNetworkState;

/// Static CA keypair embedded at compile time.
pub const CA_KEY_PEM: &str = include_str!("../../resources/ca/capsem-ca.key");
pub const CA_CERT_PEM: &str = include_str!("../../resources/ca/capsem-ca.crt");

/// Create per-sandbox network state (CA + policy for MITM proxy).
pub fn create_net_state(vm_id: &str, db: Arc<DbWriter>) -> Result<SandboxNetworkState> {
    let policy = policy_config::load_merged_network_policy();
    create_net_state_with_policy(vm_id, db, policy)
}

/// Create per-sandbox network state with a pre-loaded policy (avoids redundant disk reads).
pub fn create_net_state_with_policy(
    vm_id: &str,
    db: Arc<DbWriter>,
    mechanics: crate::net::policy::NetworkMechanics,
) -> Result<SandboxNetworkState> {
    let ca = CertAuthority::load(CA_KEY_PEM, CA_CERT_PEM).context("failed to load MITM CA")?;
    info!(vm_id, "loaded MITM CA");
    info!(
        vm_id,
        http_upstream_ports = ?mechanics.http_upstream_ports,
        dns_redirects = mechanics.dns_redirects.len(),
        "loaded network mechanics"
    );

    Ok(SandboxNetworkState {
        policy: Arc::new(std::sync::RwLock::new(Arc::new(mechanics))),
        db,
        ca: Arc::new(ca),
        upstream_tls: mitm_proxy::make_upstream_tls_config(),
    })
}

pub struct BootOptions<'a> {
    pub assets: &'a Path,
    pub kernel_override: Option<&'a Path>,
    pub initrd_override: Option<&'a Path>,
    pub rootfs_override: Option<&'a Path>,
    pub cmdline: &'a str,
    /// Path to a sparse host file attached as the second virtio-blk device
    /// (`/dev/vdb` in the guest). In VirtioFS mode this is the system-overlay
    /// disk that the guest formats ext4 on first boot and uses as the
    /// overlayfs upper. Bypasses the loop-on-VirtioFS sandwich whose closed-
    /// source virtiofsd EIOs under writeback pressure on resume.
    pub system_overlay_disk: Option<&'a Path>,
    pub virtiofs_shares: &'a [VirtioFsShare],
    pub cpu_count: u32,
    pub ram_bytes: u64,
    pub checkpoint_path: Option<std::path::PathBuf>,
    pub machine_identifier_path: Option<&'a Path>,
    pub serial_log_path: Option<&'a Path>,
    /// Asset hashes pinned by the profile this VM is booting.
    ///
    /// A channel carries one image set per profile, so no channel-wide pointer
    /// can answer this: the caller knows which profile it is starting and must
    /// say what that profile pins. Absent means the caller could not determine
    /// them, which is a hard error rather than a licence to boot unverified.
    pub expected_asset_hashes: Option<capsem_assets::asset_manager::ExpectedAssetHashes>,
}

/// Build config, boot the VM via the hypervisor trait, and return the handle +
/// vsock receiver + state machine.
///
/// If `system_overlay_disk` is provided, the file is attached as the second
/// virtio-blk device. In VirtioFS mode the guest mounts it at `/mnt/system`
/// as the overlayfs upper.
/// If `virtiofs_shares` is non-empty, VirtioFS directory sharing devices are
/// attached and `capsem.storage=virtiofs` is appended to the kernel cmdline.
pub fn boot_vm(
    options: BootOptions,
) -> Result<(
    Box<dyn VmHandle>,
    mpsc::UnboundedReceiver<VsockConnection>,
    HostStateMachine,
)> {
    let BootOptions {
        assets,
        kernel_override,
        initrd_override,
        rootfs_override,
        cmdline,
        system_overlay_disk,
        virtiofs_shares,
        cpu_count,
        ram_bytes,
        checkpoint_path,
        machine_identifier_path,
        serial_log_path,
        expected_asset_hashes,
    } = options;
    let _span = info_span!("boot_vm").entered();
    let mut sm = HostStateMachine::new_host();

    info!(
        "[boot-audit] boot_vm: cpu={cpu_count} ram_bytes={ram_bytes} virtiofs_shares={}",
        virtiofs_shares.len()
    );

    let effective_cmdline = effective_kernel_cmdline(cmdline, virtiofs_shares, rootfs_override);

    let config = {
        let _span = debug_span!("config_build").entered();
        info!("[boot-audit] building VmConfig");

        let kernel_path = kernel_override
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| assets.join("vmlinuz"));
        info!(
            "[boot-audit] kernel: {} (exists={})",
            kernel_path.display(),
            kernel_path.exists()
        );

        let mut builder = VmConfig::builder()
            .cpu_count(cpu_count)
            .ram_bytes(ram_bytes)
            .kernel_path(kernel_path)
            .kernel_cmdline(&effective_cmdline);

        if let Some(cp) = checkpoint_path {
            builder = builder.checkpoint_path(cp);
        }

        if let Some(mi_path) = machine_identifier_path {
            builder = builder.machine_identifier_path(mi_path);
        }

        if let Some(slp) = serial_log_path {
            builder = builder.serial_log_path(slp);
        }

        // Verify against the hashes the booting profile pins, supplied by the
        // caller. Reading them from the manifest's channel-wide pointer instead
        // verified every profile against whichever one that pointer named, which
        // is correct only while a channel carries exactly one profile.
        let expected_hashes = expected_asset_hashes.context(
            "refusing to boot without the booting profile's pinned asset hashes: \
             an unverified kernel is worse than a failed boot",
        )?;
        // Logged in full, not truncated. Pins reach here in two spellings --
        // bare hex and `blake3:<hex>` -- and a truncated line renders both as
        // plausible-looking prefixes, which is precisely how a spelling
        // mismatch between pin and digest hides in an audit trail.
        info!(
            "[boot-audit] asset hash verification enabled (kernel={}, initrd={}, rootfs={})",
            expected_hashes.kernel, expected_hashes.initrd, expected_hashes.rootfs,
        );

        builder = builder.expected_kernel_hash(&expected_hashes.kernel);

        let initrd_path = initrd_override
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| assets.join("initrd.img"));
        if initrd_path.exists() {
            info!("[boot-audit] initrd: {} (exists=true)", initrd_path.display());
            builder = builder.initrd_path(initrd_path);
            builder = builder.expected_initrd_hash(&expected_hashes.initrd);
        } else {
            info!("[boot-audit] initrd: {} (exists=false)", initrd_path.display());
        }

        // Use explicit rootfs override if provided (e.g. from ~/.capsem/assets/),
        // otherwise use the release EROFS rootfs contract.
        let rootfs_path = rootfs_override
            .map(|p| p.to_path_buf())
            .or_else(|| Some(assets.join("rootfs.erofs")).filter(|p| p.exists()));

        if let Some(ref rootfs) = rootfs_path {
            info!("[boot-audit] rootfs: {} (exists={})", rootfs.display(), rootfs.exists());
            builder = builder.disk_path(rootfs);
            builder = builder.expected_disk_hash(&expected_hashes.rootfs);
        } else {
            info!("[boot-audit] rootfs: none");
        }

        if let Some(overlay) = system_overlay_disk {
            info!("[boot-audit] system overlay disk: {}", overlay.display());
            builder = builder.scratch_disk_path(overlay);
        }

        for share in virtiofs_shares {
            info!(
                "[boot-audit] VirtioFS share: tag={} path={}",
                share.tag,
                share.host_path.display()
            );
            builder = builder.virtio_fs_share(&share.tag, &share.host_path, share.read_only);
        }

        info!("[boot-audit] calling VmConfig::build()");
        builder.build().context("failed to build VmConfig")?
    };
    info!("[boot-audit] VmConfig built successfully");

    info!("[boot-audit] calling hypervisor boot");
    let boot_span = debug_span!(
        target: "capsem.launch",
        capsem_foundation::telemetry::LAUNCH_VM_BOOT_SPAN,
        status = tracing::field::Empty,
    );
    let (vm, vsock_rx) = {
        let _span = boot_span.clone().entered();
        #[cfg(target_os = "macos")]
        let result = AppleVzHypervisor.boot(&config, capsem_proto::host_vsock_ports());
        #[cfg(target_os = "linux")]
        let result = KvmHypervisor.boot(&config, capsem_proto::host_vsock_ports());
        match result {
            Ok(value) => {
                boot_span.record("status", "ok");
                value
            }
            Err(error) => {
                boot_span.record("status", "error");
                return Err(error).context("failed to boot VM");
            }
        }
    };
    info!("[boot-audit] hypervisor boot returned OK");

    sm.transition(HostState::Booting, "vm_started")?;

    Ok((vm, vsock_rx, sm))
}

fn effective_kernel_cmdline(base: &str, virtiofs_shares: &[VirtioFsShare], rootfs_override: Option<&Path>) -> String {
    effective_kernel_cmdline_with_erofs_mode(
        base,
        virtiofs_shares,
        rootfs_override,
        std::env::var("CAPSEM_EXPERIMENTAL_EROFS_DAX")
            .ok()
            .is_some_and(|v| matches!(v.as_str(), "1" | "true" | "TRUE" | "yes" | "on")),
    )
}

fn effective_kernel_cmdline_with_erofs_mode(
    base: &str,
    virtiofs_shares: &[VirtioFsShare],
    rootfs_override: Option<&Path>,
    erofs_dax: bool,
) -> String {
    let mut cmdline = base.to_string();
    if !virtiofs_shares.is_empty() {
        cmdline.push_str(" capsem.storage=virtiofs");
    }
    if rootfs_override
        .and_then(|p| p.extension())
        .is_some_and(|ext| ext == "erofs")
    {
        if erofs_dax {
            cmdline.push_str(" capsem.rootfs=erofs-dax");
        } else {
            cmdline.push_str(" capsem.rootfs=erofs");
        }
    }
    cmdline
}

/// A guest control frame longer than `MAX_FRAME_SIZE`.
///
/// The payload has been drained, so the stream is aligned on the next frame
/// and the reader may continue. Dropping the connection instead was a wedge:
/// the guest never got an ack for the frame, replayed it on the fresh
/// connection, and the host dropped that one too, for the life of the VM.
#[derive(Debug, thiserror::Error)]
#[error("control frame too large ({0} bytes > {MAX_FRAME_SIZE}); frame discarded")]
pub struct ControlFrameTooLarge(pub usize);

/// Read one guest-to-host control message from an fd (blocking).
pub fn read_control_msg(file: &mut std::fs::File) -> Result<GuestToHost> {
    let mut len_buf = [0u8; 4];
    file.read_exact(&mut len_buf)?;
    let len = u32::from_be_bytes(len_buf) as usize;
    if len > MAX_FRAME_SIZE as usize {
        std::io::copy(&mut Read::by_ref(file).take(len as u64), &mut std::io::sink())?;
        return Err(ControlFrameTooLarge(len).into());
    }
    let mut payload = vec![0u8; len];
    file.read_exact(&mut payload)?;
    decode_guest_msg(&payload)
}

/// Write one host-to-guest control message to an fd.
pub fn write_control_msg(file: &mut std::fs::File, msg: &HostToGuest) -> Result<()> {
    let frame = encode_host_msg(msg)?;
    file.write_all(&frame)?;
    Ok(())
}

/// Detect the host timezone by reading the `/etc/localtime` symlink.
/// Returns the Olson timezone name (e.g., `America/Los_Angeles`).
fn detect_host_timezone() -> Option<String> {
    let target = std::fs::read_link("/etc/localtime").ok()?;
    let s = target.to_str()?;
    let marker = "/zoneinfo/";
    let idx = s.find(marker)?;
    Some(s[idx + marker.len()..].to_string())
}

/// Send the boot configuration as individual vsock messages.
/// If `preloaded_guest_config` is provided, uses it instead of reading from disk.
pub fn send_boot_config(
    file: &mut std::fs::File,
    cli_env: &[(String, String)],
    preloaded_guest_config: Option<policy_config::GuestConfig>,
) -> Result<()> {
    use capsem_proto::{
        validate_env_key, validate_env_value, validate_file_path, MAX_BOOT_ENV_VARS, MAX_BOOT_FILES,
        MAX_BOOT_FILE_BYTES,
    };

    let epoch_secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    // 1. Send BootConfig with clock.
    let traceparent = capsem_foundation::telemetry::current_parent_traceparent().to_string();
    write_control_msg(
        file,
        &HostToGuest::BootConfig {
            epoch_secs,
            traceparent,
        },
    )?;

    // 1b. Inject host timezone (TZ env var + /etc/localtime binary).
    if let Some(tz) = detect_host_timezone() {
        info!("injecting host timezone: {tz}");
        write_control_msg(
            file,
            &HostToGuest::SetEnv {
                key: "TZ".into(),
                value: tz,
            },
        )?;
        if let Ok(tz_data) = std::fs::read("/etc/localtime") {
            write_control_msg(
                file,
                &HostToGuest::FileWrite {
                    id: 0,
                    path: "/etc/localtime".into(),
                    data: tz_data,
                    mode: 0o644,
                },
            )?;
        }
    }

    // 2. Send metadata-driven env vars from settings UI metadata.
    let guest_config = preloaded_guest_config.unwrap_or_else(policy_config::load_merged_guest_config);
    let mut env_count: usize = 0;

    // Track what we actually send for the injection test manifest.
    let mut sent_env: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    let mut sent_files: Vec<serde_json::Value> = Vec::new();

    if let Some(env) = guest_config.env {
        for (key, value) in env {
            if env_count >= MAX_BOOT_ENV_VARS {
                warn!("boot env var cap reached ({MAX_BOOT_ENV_VARS}), skipping remaining");
                break;
            }
            if let Err(e) = validate_env_key(&key) {
                warn!(error = %e, "skipping invalid boot env var key");
                continue;
            }
            if let Err(e) = validate_env_value(&value) {
                warn!(error = %e, "skipping boot env var {key}");
                continue;
            }
            sent_env.insert(key.clone(), value.clone());
            write_control_msg(file, &HostToGuest::SetEnv { key, value })?;
            env_count += 1;
        }
    }

    // 3. CLI --env overrides (last wins).
    for (key, value) in cli_env {
        if env_count >= MAX_BOOT_ENV_VARS {
            warn!("boot env var cap reached ({MAX_BOOT_ENV_VARS}), skipping remaining CLI --env");
            break;
        }
        if let Err(e) = validate_env_key(key) {
            warn!(error = %e, "skipping invalid CLI --env key");
            continue;
        }
        if let Err(e) = validate_env_value(value) {
            warn!(error = %e, "skipping CLI --env {key}");
            continue;
        }
        sent_env.insert(key.clone(), value.clone());
        write_control_msg(
            file,
            &HostToGuest::SetEnv {
                key: key.clone(),
                value: value.clone(),
            },
        )?;
        env_count += 1;
    }

    // 4. Send each boot file (with caps).
    let mut file_count: usize = 0;
    let mut total_file_bytes: usize = 0;

    for f in guest_config.files.unwrap_or_default() {
        if file_count >= MAX_BOOT_FILES {
            warn!("boot file cap reached ({MAX_BOOT_FILES}), skipping remaining");
            break;
        }
        let data = f.content.into_bytes();
        if total_file_bytes + data.len() > MAX_BOOT_FILE_BYTES {
            warn!(
                "boot file bytes cap reached ({MAX_BOOT_FILE_BYTES}), skipping {}",
                f.path
            );
            continue;
        }
        if let Err(e) = validate_file_path(&f.path) {
            warn!(error = %e, "skipping invalid boot file path");
            continue;
        }
        total_file_bytes += data.len();
        file_count += 1;
        sent_files.push(serde_json::json!({
            "path": &f.path,
            "mode": f.mode,
        }));
        write_control_msg(
            file,
            &HostToGuest::FileWrite {
                id: 0,
                path: f.path,
                data,
                mode: f.mode,
            },
        )?;
    }

    // 5. Send injection manifest (for in-VM injection tests).
    let manifest = serde_json::json!({
        "env": &sent_env,
        "files": &sent_files,
    });
    write_control_msg(
        file,
        &HostToGuest::FileWrite {
            id: 0,
            path: "/tmp/capsem-injection-manifest.json".to_string(),
            data: serde_json::to_string_pretty(&manifest)
                .unwrap_or_else(|_| "{}".to_string())
                .into_bytes(),
            mode: 0o644,
        },
    )?;

    // 6. Signal done.
    write_control_msg(file, &HostToGuest::BootConfigDone)?;

    Ok(())
}

#[cfg(test)]
mod tests;
