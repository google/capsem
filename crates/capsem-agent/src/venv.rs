//! The default Python venv path, published without blocking boot.
//!
//! capsem-init creates the venv in the background and touches a ready flag
//! when done. The shell environment points at the stable `/root/.venv`
//! contract at once, but BootReady must not wait for Python packaging when
//! the first command is often a simple readiness probe.

/// Point the boot environment at the venv and, in the background, make sure
/// one exists once init has had its chance.
pub fn activate(boot_env: &mut Vec<(String, String)>) {
    const VENV_DIR: &str = "/root/.venv";
    const VENV_TARGET: &str = "/run/capsem-venv";
    const VENV_READY: &str = "/run/capsem-venv-ready";
    boot_env.push(("VIRTUAL_ENV".into(), VENV_DIR.into()));
    if let Some((_, path_val)) = boot_env.iter_mut().find(|(k, _)| k == "PATH") {
        *path_val = format!("{VENV_DIR}/bin:{path_val}");
    }
    std::thread::spawn(move || {
        let venv_activate = std::path::Path::new(VENV_DIR).join("bin/activate");
        for _ in 0..30 {
            if std::path::Path::new(VENV_READY).exists() || venv_activate.exists() {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        if !venv_activate.exists() {
            eprintln!("[capsem-agent] venv missing after init wait; creating fallback");
            let _ = std::fs::remove_file(VENV_TARGET);
            let _ = std::fs::remove_dir_all(VENV_TARGET);
            let _ = std::fs::remove_file(VENV_DIR);
            let _ = std::os::unix::fs::symlink(VENV_TARGET, VENV_DIR);
            let created = std::process::Command::new("uv")
                .args(["venv", "--system-site-packages", VENV_TARGET])
                .status()
                .map(|status| status.success())
                .unwrap_or(false)
                || std::process::Command::new("python3")
                    .args(["-m", "venv", "--system-site-packages", VENV_TARGET])
                    .status()
                    .map(|status| status.success())
                    .unwrap_or(false);
            if created {
                let _ = std::fs::write(VENV_READY, b"");
            }
        }
    });
}
