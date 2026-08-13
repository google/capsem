//! Boot verified profile images directly and require the guest control-plane connection.
//!
//! This is a release test harness, not a package or profile builder. The Python
//! caller resolves every path and digest from the selected channel manifest.

use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{bail, Context, Result};
use capsem_core::{
    boot_vm, create_virtiofs_session, guest_share_dir, BootOptions, VirtioFsShare,
    VSOCK_PORT_CONTROL,
};

fn arguments() -> Result<BTreeMap<String, String>> {
    let mut values = BTreeMap::new();
    let mut args = std::env::args().skip(1);
    while let Some(flag) = args.next() {
        if !flag.starts_with("--") {
            bail!("unexpected positional argument: {flag}");
        }
        let value = args
            .next()
            .with_context(|| format!("missing value for {flag}"))?;
        if values.insert(flag.clone(), value).is_some() {
            bail!("repeated argument: {flag}");
        }
    }
    Ok(values)
}

fn required<'a>(values: &'a BTreeMap<String, String>, flag: &str) -> Result<&'a str> {
    values
        .get(flag)
        .map(String::as_str)
        .with_context(|| format!("missing required argument {flag}"))
}

fn validate_profile_id(profile: &str) -> Result<()> {
    if profile.is_empty()
        || profile == "."
        || profile == ".."
        || !profile
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
    {
        bail!("unsafe profile identity: {profile:?}");
    }
    Ok(())
}

fn verify_image(path: &Path, digest: &str, label: &str) -> Result<()> {
    if digest.len() != 64 || !digest.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        bail!("{label} BLAKE3 is malformed");
    }
    let mut input =
        std::fs::File::open(path).with_context(|| format!("open {label} {}", path.display()))?;
    let mut hasher = blake3::Hasher::new();
    let mut buffer = vec![0_u8; 64 * 1024];
    loop {
        let read = input
            .read(&mut buffer)
            .with_context(|| format!("read {label} {}", path.display()))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    let actual = hasher.finalize().to_hex();
    if actual.as_str() != digest {
        bail!("{label} does not match the manifest digest: expected {digest}, got {actual}");
    }
    Ok(())
}

fn unique_session_root(profile: &str) -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    std::env::temp_dir().join(format!(
        "capsem-profile-boot-{}-{}-{stamp}",
        profile,
        std::process::id()
    ))
}

fn kernel_cmdline() -> &'static str {
    #[cfg(target_arch = "x86_64")]
    {
        "console=ttyS0 root=/dev/vda ro loglevel=1 quiet init_on_alloc=1 slab_nomerge page_alloc.shuffle=1 random.trust_cpu=1"
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        "console=hvc0 root=/dev/vda ro loglevel=1 quiet init_on_alloc=1 slab_nomerge page_alloc.shuffle=1 random.trust_cpu=1"
    }
}

fn serial_tail(path: &Path) -> String {
    let Ok(bytes) = std::fs::read(path) else {
        return "<serial log unavailable>".to_string();
    };
    let start = bytes.len().saturating_sub(8_000);
    String::from_utf8_lossy(&bytes[start..]).into_owned()
}

fn run() -> Result<()> {
    let values = arguments()?;
    let profile = required(&values, "--profile")?;
    validate_profile_id(profile)?;
    let kernel = PathBuf::from(required(&values, "--kernel")?);
    let initrd = PathBuf::from(required(&values, "--initrd")?);
    let rootfs = PathBuf::from(required(&values, "--rootfs")?);
    // The digests this harness was told to prove. They are also what boot must
    // verify against: this path is not behind the service, so boot's own check is
    // the only one, and it must use the profile under test rather than any
    // channel-wide pointer.
    let kernel_blake3 = required(&values, "--kernel-blake3")?.to_string();
    let initrd_blake3 = required(&values, "--initrd-blake3")?.to_string();
    let rootfs_blake3 = required(&values, "--rootfs-blake3")?.to_string();
    verify_image(&kernel, &kernel_blake3, "kernel")?;
    verify_image(&initrd, &initrd_blake3, "initrd")?;
    verify_image(&rootfs, &rootfs_blake3, "rootfs")?;
    let timeout = required(&values, "--timeout")?
        .parse::<u64>()
        .context("invalid --timeout")?;
    if timeout == 0 {
        bail!("--timeout must be positive");
    }

    let session_root = unique_session_root(profile);
    create_virtiofs_session(&session_root, 1)
        .with_context(|| format!("create boot-proof session {}", session_root.display()))?;
    let guest_dir = guest_share_dir(&session_root);
    let system_overlay = guest_dir.join("system/rootfs.img");
    let serial_log = session_root.join("serial.log");
    let machine_identifier = session_root.join("machine_identifier");
    let shares = [VirtioFsShare {
        tag: "capsem".to_string(),
        host_path: guest_dir,
        read_only: false,
    }];

    let result = (|| -> Result<()> {
        let (vm, mut connections, _) = boot_vm(BootOptions {
            assets: kernel.parent().context("kernel path has no parent")?,
            kernel_override: Some(&kernel),
            initrd_override: Some(&initrd),
            rootfs_override: Some(&rootfs),
            cmdline: kernel_cmdline(),
            system_overlay_disk: Some(&system_overlay),
            virtiofs_shares: &shares,
            cpu_count: 2,
            ram_bytes: 2 * 1024 * 1024 * 1024,
            checkpoint_path: None,
            machine_identifier_path: Some(&machine_identifier),
            serial_log_path: Some(&serial_log),
        expected_asset_hashes: Some(capsem_core::asset_manager::ExpectedAssetHashes {
            kernel: kernel_blake3.clone(),
            initrd: initrd_blake3.clone(),
            rootfs: rootfs_blake3.clone(),
        }),
        })?;
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .context("create profile boot runtime")?;
        let deadline = Instant::now() + Duration::from_secs(timeout);
        let connected = runtime.block_on(async {
            loop {
                let remaining = deadline.saturating_duration_since(Instant::now());
                if remaining.is_zero() {
                    return false;
                }
                match tokio::time::timeout(remaining, connections.recv()).await {
                    Ok(Some(connection)) if connection.port == VSOCK_PORT_CONTROL => return true,
                    Ok(Some(_)) => continue,
                    Ok(None) | Err(_) => return false,
                }
            }
        });
        let stop_result = vm.stop();
        if !connected {
            bail!(
                "guest did not connect to the control plane before timeout; serial tail:\n{}",
                serial_tail(&serial_log)
            );
        }
        stop_result.context("stop boot-proof VM")?;
        Ok(())
    })();

    if result.is_ok() {
        std::fs::remove_dir_all(&session_root).with_context(|| {
            format!(
                "remove successful boot-proof session {}",
                session_root.display()
            )
        })?;
    } else {
        eprintln!(
            "profile boot failure evidence retained at {}",
            session_root.display()
        );
    }
    result
}

fn main() {
    if let Err(error) = run() {
        eprintln!("release profile boot proof failed: {error:#}");
        std::process::exit(1);
    }
    println!("release profile boot proof passed");
}
