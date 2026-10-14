use anyhow::{Context, Result};
use capsem_core::net::upstream_grant::UpstreamGrantClient;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use super::{aggregator_log_path, Args};

fn grant_owner_path(
    policy: capsem_foundation::unix::worker_sandbox::Policy,
    path: &Path,
    access: capsem_foundation::unix::worker_sandbox::Access,
) -> Result<capsem_foundation::unix::worker_sandbox::Policy> {
    let canonical = path
        .canonicalize()
        .with_context(|| format!("resolve sandbox grant {}", path.display()))?;
    Ok(policy.allow(canonical, access))
}

fn grant_owner_session_paths(
    mut policy: capsem_foundation::unix::worker_sandbox::Policy,
    session_dir: &Path,
) -> Result<capsem_foundation::unix::worker_sandbox::Policy> {
    use capsem_foundation::unix::worker_sandbox::Access;

    for (path, access) in [
        (capsem_core::guest_share_dir(session_dir), Access::ReadWrite),
        (
            capsem_core::session::system_overlay_image_path(session_dir),
            Access::ReadWrite,
        ),
        (capsem_core::session::image_share_path(session_dir), Access::ReadOnly),
        (
            session_dir.join(capsem_core::session::OWNER_STATE_DIR),
            Access::ReadWrite,
        ),
        (session_dir.join("serial.log"), Access::ReadWrite),
        (session_dir.join("pty.log"), Access::ReadWrite),
        (aggregator_log_path(session_dir), Access::ReadWrite),
    ] {
        policy = grant_owner_path(policy, &path, access)?;
    }
    Ok(policy)
}

fn grant_owner_null(
    policy: capsem_foundation::unix::worker_sandbox::Policy,
) -> Result<capsem_foundation::unix::worker_sandbox::Policy> {
    use capsem_foundation::unix::worker_sandbox::Access;

    let null = Path::new("/dev/null");
    if null.exists() {
        grant_owner_path(policy, null, Access::ReadWrite)
    } else {
        Ok(policy)
    }
}

#[cfg(target_os = "linux")]
pub(super) fn confine_owner(args: &Args, session_dir: &Path) -> Result<()> {
    use capsem_foundation::unix::worker_sandbox::{Access, Policy, Role};

    let mut policy = Policy::new(Role::VmOwner);
    policy = grant_owner_session_paths(policy, session_dir)?;
    policy = grant_owner_path(policy, &args.assets_dir, Access::ReadOnly)?;
    policy = grant_owner_path(policy, &args.rootfs, Access::ReadOnly)?;
    for path in args.kernel.iter().chain(args.initrd.iter()) {
        policy = grant_owner_path(policy, path, Access::ReadOnly)?;
    }
    for path in ["/dev/kvm", "/dev/vhost-vsock"] {
        let path = Path::new(path);
        if path.exists() {
            policy = grant_owner_path(policy, path, Access::ReadWriteDevice)?;
        }
    }
    policy = grant_owner_null(policy)?;
    let executable = std::env::current_exe().context("locate VM-owner executable")?;
    let router = executable.with_file_name("capsem-router");
    if router.exists() {
        policy = grant_owner_path(policy, &router, Access::Executable)?;
    }
    for path in ["/lib", "/lib64", "/usr/lib", "/usr/lib64"] {
        let path = Path::new(path);
        if path.exists() {
            policy = grant_owner_path(policy, path, Access::ReadOnly)?;
        }
    }
    let linker_cache = Path::new("/etc/ld.so.cache");
    if linker_cache.exists() {
        policy = grant_owner_path(policy, linker_cache, Access::ReadOnly)?;
    }
    for path in ["/lib64/ld-linux-x86-64.so.2", "/lib/ld-linux-aarch64.so.1"] {
        let path = Path::new(path);
        if path.exists() {
            policy = grant_owner_path(policy, path, Access::Executable)?;
        }
    }
    capsem_foundation::unix::worker_sandbox::confine(&policy)?;
    Ok(())
}

#[cfg(target_os = "macos")]
pub(super) fn confine_owner(args: &Args, session_dir: &Path) -> Result<()> {
    use capsem_foundation::unix::worker_sandbox::{Access, Policy, Role};

    let mut policy = Policy::new(Role::VmOwner);
    policy = grant_owner_session_paths(policy, session_dir)?;
    policy = grant_owner_path(policy, &args.assets_dir, Access::ReadOnly)?;
    policy = grant_owner_path(policy, &args.rootfs, Access::ReadOnly)?;
    for path in args.kernel.iter().chain(args.initrd.iter()) {
        policy = grant_owner_path(policy, path, Access::ReadOnly)?;
    }
    let router = std::env::current_exe()
        .context("locate VM-owner executable")?
        .with_file_name("capsem-router");
    if router.exists() {
        policy = grant_owner_path(policy, &router, Access::Executable)?;
    }
    for path in [
        "/System/Library",
        "/usr/lib",
        "/private/var/db/dyld",
        "/private/var/db/timezone",
        "/etc/localtime",
        "/dev/urandom",
    ] {
        let path = Path::new(path);
        if path.exists() {
            policy = grant_owner_path(policy, path, Access::ReadOnly)?;
        }
    }
    policy = grant_owner_null(policy)?;
    capsem_foundation::unix::worker_sandbox::confine(&policy)?;
    Ok(())
}

pub(super) struct OwnerSandboxAttestation {
    direct_path: PathBuf,
    direct_file: std::fs::File,
    guest_path: PathBuf,
    guest_relative: Vec<u8>,
    ledger_path: PathBuf,
}

pub(super) fn prepare_owner_sandbox_attestation(session_dir: &Path) -> Result<OwnerSandboxAttestation> {
    use std::os::unix::ffi::OsStrExt as _;
    use std::os::unix::fs::OpenOptionsExt as _;

    let create = |path: &Path| {
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(path)
            .with_context(|| format!("prepare owner sandbox attestation {}", path.display()))
    };
    let nonce = format!(
        "{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    );
    let direct_path = session_dir
        .join(capsem_core::session::OWNER_STATE_DIR)
        .join(format!(".owner-sandbox-attestation-{nonce}"));
    let guest_relative = format!("workspace/.owner-metadata-attestation-{nonce}").into_bytes();
    let guest_path = session_dir
        .join(capsem_core::GUEST_SHARE_DIR)
        .join(std::ffi::OsStr::from_bytes(&guest_relative));
    let direct_file = create(&direct_path)?;
    drop(create(&guest_path)?);
    Ok(OwnerSandboxAttestation {
        direct_path,
        direct_file,
        guest_path,
        guest_relative,
        ledger_path: session_dir.join("session.db"),
    })
}

pub(super) async fn attest_owner(
    attestation: OwnerSandboxAttestation,
    service_socket: PathBuf,
    upstream_grants: Arc<UpstreamGrantClient>,
) -> Result<()> {
    use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};

    let require_denied = |result: std::io::Result<()>, authority: &str| -> Result<()> {
        match result {
            Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => Ok(()),
            Err(error) => anyhow::bail!("VM owner sandbox returned {error} while denying {authority}"),
            Ok(()) => anyhow::bail!("VM owner sandbox retained authority to {authority}"),
        }
    };
    require_denied(
        std::fs::File::open("/etc/passwd").map(|_| ()),
        "read unrelated host files",
    )?;
    require_denied(
        std::fs::File::open(&attestation.ledger_path).map(|_| ()),
        "open session ledger storage",
    )?;
    require_denied(
        std::net::TcpStream::connect((std::net::Ipv4Addr::LOCALHOST, 9)).map(|_| ()),
        "dial TCP sockets directly",
    )?;
    require_denied(
        std::os::unix::net::UnixStream::connect(service_socket).map(|_| ()),
        "dial control sockets by path",
    )?;
    require_denied(
        attestation
            .direct_file
            .set_permissions(std::fs::Permissions::from_mode(0o777)),
        "change host modes directly",
    )?;
    require_denied(
        std::process::Command::new("/bin/true").status().map(|_| ()),
        "execute arbitrary processes",
    )?;

    tokio::task::spawn_blocking(move || {
        capsem_core::GuestMetadataAuthority::set_mode(upstream_grants.as_ref(), &attestation.guest_relative, 0o701)
    })
    .await
    .context("join brokered metadata attestation")??;
    let mode = std::fs::metadata(&attestation.guest_path)?.mode() & 0o7777;
    anyhow::ensure!(mode == 0o701, "brokered metadata attestation returned mode {mode:o}");
    std::fs::remove_file(&attestation.guest_path)?;
    std::fs::remove_file(&attestation.direct_path)?;
    Ok(())
}

#[cfg(test)]
mod tests;
