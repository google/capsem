use std::io::{Read, Write};
use std::mem::ManuallyDrop;
use std::os::unix::io::{FromRawFd, RawFd};
use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result};
use capsem_core::{
    GuestToHost, Hypervisor, HostState, HostStateMachine, HostToGuest,
    VirtioFsShare, VmConfig, VmHandle, VsockConnection, decode_guest_msg, encode_host_msg,
    MAX_FRAME_SIZE, VSOCK_PORT_CONTROL, VSOCK_PORT_MCP_GATEWAY, VSOCK_PORT_SNI_PROXY,
    VSOCK_PORT_TERMINAL,
};
#[cfg(target_os = "macos")]
use capsem_core::AppleVzHypervisor;
#[cfg(target_os = "linux")]
use capsem_core::KvmHypervisor;
use capsem_core::net::cert_authority::CertAuthority;
use capsem_core::net::mitm_proxy;
use capsem_core::net::policy_config;
use capsem_logger::DbWriter;
use tokio::sync::mpsc;
use tracing::{debug_span, info, info_span, warn};

use crate::state::VmNetworkState;

/// Clone a raw fd into an independently-owned File.
/// The original fd remains open and unaffected.
pub(crate) fn clone_fd(fd: RawFd) -> std::io::Result<std::fs::File> {
    // Safety: fd is valid (checked by caller context)
    let file = ManuallyDrop::new(unsafe { std::fs::File::from_raw_fd(fd) });
    file.try_clone() // creates a dup'd fd owned by the returned File
}

/// Static CA keypair embedded at compile time.
pub(crate) const CA_KEY_PEM: &str = include_str!("../../../config/capsem-ca.key");
pub(crate) const CA_CERT_PEM: &str = include_str!("../../../config/capsem-ca.crt");

pub(crate) fn create_net_state(vm_id: &str, db: Arc<DbWriter>) -> Result<VmNetworkState> {
    let ca = CertAuthority::load(CA_KEY_PEM, CA_CERT_PEM)
        .context("failed to load MITM CA")?;
    info!(vm_id, "loaded MITM CA");

    let policy = policy_config::load_merged_network_policy();
    info!(
        vm_id,
        "loaded network policy ({} rules)",
        policy.rules.len()
    );

    Ok(VmNetworkState {
        policy: Arc::new(std::sync::RwLock::new(Arc::new(policy))),
        db,
        ca: Arc::new(ca),
        upstream_tls: mitm_proxy::make_upstream_tls_config(),
    })
}

/// Build config, boot the VM via the hypervisor trait, and return the handle +
/// vsock receiver + state machine.
///
/// If `scratch_disk_path` is provided, the scratch disk is attached as a second
/// block device (read-write) for the guest `/root` workspace.
/// If `virtiofs_shares` is non-empty, VirtioFS directory sharing devices are
/// attached and `capsem.storage=virtiofs` is appended to the kernel cmdline.
pub(crate) fn boot_vm(
    assets: &Path,
    rootfs_override: Option<&Path>,
    cmdline: &str,
    scratch_disk_path: Option<&Path>,
    virtiofs_shares: &[VirtioFsShare],
    cpu_count: u32,
    ram_bytes: u64,
) -> Result<(Box<dyn VmHandle>, mpsc::UnboundedReceiver<VsockConnection>, HostStateMachine)> {
    let _span = info_span!("boot_vm").entered();
    let mut sm = HostStateMachine::new_host();

    info!("[boot-audit] boot_vm: cpu={cpu_count} ram_bytes={ram_bytes} virtiofs_shares={}", virtiofs_shares.len());

    // In VirtioFS mode, append storage flag to kernel cmdline.
    let effective_cmdline = if virtiofs_shares.is_empty() {
        cmdline.to_string()
    } else {
        format!("{cmdline} capsem.storage=virtiofs")
    };

    let config = {
        let _span = debug_span!("config_build").entered();
        info!("[boot-audit] building VmConfig");

        let kernel_path = assets.join("vmlinuz");
        info!("[boot-audit] kernel: {} (exists={})", kernel_path.display(), kernel_path.exists());

        let mut builder = VmConfig::builder()
            .cpu_count(cpu_count)
            .ram_bytes(ram_bytes)
            .kernel_path(kernel_path)
            .kernel_cmdline(&effective_cmdline);

        if let Some(hash) = option_env!("VMLINUZ_HASH") {
            info!("[boot-audit] kernel hash verification enabled");
            builder = builder.expected_kernel_hash(hash);
        }

        let initrd_path = assets.join("initrd.img");
        if initrd_path.exists() {
            info!("[boot-audit] initrd: {} (exists=true)", initrd_path.display());
            builder = builder.initrd_path(initrd_path);
            if let Some(hash) = option_env!("INITRD_HASH") {
                builder = builder.expected_initrd_hash(hash);
            }
        } else {
            info!("[boot-audit] initrd: {} (exists=false)", initrd_path.display());
        }

        // Use explicit rootfs override if provided (e.g. from ~/.capsem/assets/),
        // otherwise check bundled assets dir for both squashfs and legacy img.
        let rootfs_path = rootfs_override
            .map(|p| p.to_path_buf())
            .or_else(|| {
                Some(assets.join("rootfs.squashfs"))
                    .filter(|p| p.exists())
            });

        if let Some(ref rootfs) = rootfs_path {
            info!("[boot-audit] rootfs: {} (exists={})", rootfs.display(), rootfs.exists());
            builder = builder.disk_path(rootfs);
            if let Some(hash) = option_env!("ROOTFS_HASH") {
                builder = builder.expected_disk_hash(hash);
            }
        } else {
            info!("[boot-audit] rootfs: none");
        }

        if let Some(scratch) = scratch_disk_path {
            info!("[boot-audit] scratch disk: {}", scratch.display());
            builder = builder.scratch_disk_path(scratch);
        }

        for share in virtiofs_shares {
            info!("[boot-audit] VirtioFS share: tag={} path={}", share.tag, share.host_path.display());
            builder = builder.virtio_fs_share(
                &share.tag,
                &share.host_path,
                share.read_only,
            );
        }

        info!("[boot-audit] calling VmConfig::build()");
        builder.build().context("failed to build VmConfig")?
    };
    info!("[boot-audit] VmConfig built successfully");

    let vsock_ports = [
        VSOCK_PORT_CONTROL,
        VSOCK_PORT_TERMINAL,
        VSOCK_PORT_SNI_PROXY,
        VSOCK_PORT_MCP_GATEWAY,
    ];

    info!("[boot-audit] calling hypervisor boot");
    let (vm, vsock_rx) = {
        let _span = debug_span!("hypervisor_boot").entered();
        #[cfg(target_os = "macos")]
        let result = AppleVzHypervisor.boot(&config, &vsock_ports);
        #[cfg(target_os = "linux")]
        let result = KvmHypervisor.boot(&config, &vsock_ports);
        result
            .context("failed to boot VM")?
    };
    info!("[boot-audit] hypervisor boot returned OK");

    sm.transition(HostState::Booting, "vm_started")?;

    Ok((vm, vsock_rx, sm))
}

/// Read one guest-to-host control message from an fd (blocking).
pub(crate) fn read_control_msg(file: &mut std::fs::File) -> Result<GuestToHost> {
    let mut len_buf = [0u8; 4];
    file.read_exact(&mut len_buf)?;
    let len = u32::from_be_bytes(len_buf) as usize;
    if len > MAX_FRAME_SIZE as usize {
        anyhow::bail!("control frame too large ({len} bytes)");
    }
    let mut payload = vec![0u8; len];
    file.read_exact(&mut payload)?;
    decode_guest_msg(&payload)
}

/// Write one host-to-guest control message to an fd.
pub(crate) fn write_control_msg(file: &mut std::fs::File, msg: &HostToGuest) -> Result<()> {
    let frame = encode_host_msg(msg)?;
    file.write_all(&frame)?;
    Ok(())
}

/// Send the boot configuration as individual vsock messages.
///
/// Sends BootConfig (clock), then SetEnv for each env var, FileWrite for each
/// boot file, and BootConfigDone to signal completion. Each message is its own
/// frame, eliminating the old single-frame size constraint.
///
/// Validates all env vars and file paths before sending. Invalid entries are
/// logged and skipped. Enforces allocation caps (MAX_BOOT_ENV_VARS,
/// MAX_BOOT_FILES, MAX_BOOT_FILE_BYTES) to prevent unbounded allocations.
///
/// Env var priority: settings registry defaults < user.toml overrides < CLI --env flags.
pub(crate) fn send_boot_config(file: &mut std::fs::File, cli_env: &[(String, String)]) -> Result<()> {
    use capsem_core::capsem_proto::{
        validate_env_key, validate_env_value, validate_file_path,
        MAX_BOOT_ENV_VARS, MAX_BOOT_FILES, MAX_BOOT_FILE_BYTES,
    };

    let epoch_secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    // 1. Send BootConfig with clock.
    write_control_msg(file, &HostToGuest::BootConfig { epoch_secs })?;

    // 2. Send metadata-driven env vars from settings registry.
    let guest_config = policy_config::load_merged_guest_config();
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
                warn!("skipping invalid boot env var key: {e}");
                continue;
            }
            if let Err(e) = validate_env_value(&value) {
                warn!("skipping boot env var {key}: {e}");
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
            warn!("skipping invalid CLI --env key: {e}");
            continue;
        }
        if let Err(e) = validate_env_value(value) {
            warn!("skipping CLI --env {key}: {e}");
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
            warn!("skipping invalid boot file path: {e}");
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
