use super::*;

#[test]
fn binary_installer_apply_plan_uses_macos_pkg_installer() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("Capsem 99.99.99.pkg");
    std::fs::write(&path, b"pkg").unwrap();
    let installer = BinaryInstaller {
        name: "Capsem-99.99.99.pkg".to_string(),
        url: "https://github.com/google/capsem/releases/download/v99.99.99/Capsem-99.99.99.pkg".to_string(),
        sha256: "0".repeat(64),
        blake3: "0".repeat(64),
        size: 3,
        install_layout: "macos_pkg".to_string(),
    };

    let plan = binary_installer_apply_plan(&installer, &path).unwrap();

    assert_eq!(
        plan.commands,
        vec![BinaryInstallerApplyCommand {
            program: "sudo".to_string(),
            args: vec![
                "/bin/sh".to_string(),
                "-c".to_string(),
                PRIVILEGED_INSTALL_SCRIPT.to_string(),
                "capsem-update".to_string(),
                path.display().to_string(),
                installer.sha256.clone(),
                installer.size.to_string(),
                "macos_pkg".to_string(),
            ],
        }]
    );
    assert!(PRIVILEGED_INSTALL_SCRIPT.contains("/usr/sbin/installer -pkg \"$package\""));
}

#[test]
fn binary_installer_apply_plan_uses_apt_for_linux_deb() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("Capsem_99.99.99_arm64.deb");
    std::fs::write(&path, b"deb").unwrap();
    let installer = BinaryInstaller {
        name: "Capsem_99.99.99_arm64.deb".to_string(),
        url: "https://github.com/google/capsem/releases/download/v99.99.99/Capsem_99.99.99_arm64.deb".to_string(),
        sha256: "0".repeat(64),
        blake3: "0".repeat(64),
        size: 3,
        install_layout: "linux_deb".to_string(),
    };

    let plan = binary_installer_apply_plan(&installer, &path).unwrap();

    assert_eq!(
        plan.commands,
        vec![BinaryInstallerApplyCommand {
            program: "sudo".to_string(),
            args: vec![
                "/bin/sh".to_string(),
                "-c".to_string(),
                PRIVILEGED_INSTALL_SCRIPT.to_string(),
                "capsem-update".to_string(),
                path.display().to_string(),
                installer.sha256.clone(),
                installer.size.to_string(),
                "linux_deb".to_string(),
            ],
        }]
    );
    assert!(PRIVILEGED_INSTALL_SCRIPT.contains("/usr/bin/apt-get install --yes --allow-downgrades \"$package\""));
}

#[test]
fn privileged_installer_rejects_source_changed_after_unprivileged_verification() {
    let dir = tempfile::tempdir().unwrap();
    let layout = if cfg!(target_os = "macos") {
        "macos_pkg"
    } else {
        "linux_deb"
    };
    let path = dir.path().join(if cfg!(target_os = "macos") {
        "installer.pkg"
    } else {
        "installer.deb"
    });
    let verified = b"verified package";
    std::fs::write(&path, verified).unwrap();
    let expected_sha256 = super::super::sha256_hex(verified);
    std::fs::write(&path, b"swapped package").unwrap();

    let output = std::process::Command::new("/bin/sh")
        .arg("-c")
        .arg(PRIVILEGED_INSTALL_SCRIPT)
        .arg("capsem-update")
        .arg(&path)
        .arg(expected_sha256)
        .arg(verified.len().to_string())
        .arg(layout)
        .output()
        .unwrap();

    assert!(!output.status.success(), "mutated package reached the package manager");
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("installer changed before privileged verification"),
        "{:?}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn binary_installer_apply_plan_rejects_unknown_layout() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("Capsem-99.99.99.pkg");
    std::fs::write(&path, b"pkg").unwrap();
    let installer = BinaryInstaller {
        name: "Capsem-99.99.99.pkg".to_string(),
        url: "https://github.com/google/capsem/releases/download/v99.99.99/Capsem-99.99.99.pkg".to_string(),
        sha256: "0".repeat(64),
        blake3: "0".repeat(64),
        size: 3,
        install_layout: "portable_zip".to_string(),
    };

    let err = binary_installer_apply_plan(&installer, &path).unwrap_err();

    assert!(
        format!("{err:#}").contains("unsupported binary installer layout portable_zip"),
        "{err:#}"
    );
}
