use super::*;
use std::path::Path;

#[test]
fn detect_returns_development_in_test() {
    let layout = detect_install_layout();
    assert_eq!(layout, InstallLayout::Development);
}

#[test]
fn detect_macos_pkg_layout() {
    let path = Path::new("/usr/local/bin/capsem");
    assert_eq!(detect_layout_from_path(path), InstallLayout::MacosPkg);
}

#[test]
fn detect_user_dir_layout() {
    let path = Path::new("/Users/elie/.capsem/bin/capsem");
    assert_eq!(detect_layout_from_path(path), InstallLayout::UserDir);
}

#[test]
fn detect_macos_pkg_runtime_copy_from_native_payload_marker() {
    let path = Path::new("/Users/elie/.capsem/bin/capsem");
    assert_eq!(
        detect_layout_from_path_with_macos_pkg_marker(path, true),
        InstallLayout::MacosPkg
    );
    assert_eq!(
        detect_layout_from_path_with_macos_pkg_marker(path, false),
        InstallLayout::UserDir
    );
}

#[test]
fn detect_user_dir_linux() {
    let path = Path::new("/home/user/.capsem/bin/capsem-service");
    assert_eq!(detect_layout_from_path(path), InstallLayout::UserDir);
}

#[test]
fn detect_development_layout() {
    let path = Path::new("/Users/elie/git/capsem/cache/target/cargo/debug/capsem");
    assert_eq!(detect_layout_from_path(path), InstallLayout::Development);
}

#[test]
fn detect_linux_deb_layout() {
    let path = Path::new("/usr/bin/capsem");
    assert_eq!(detect_layout_from_path(path), InstallLayout::LinuxDeb);
}

/// A distribution's own package installs to /usr/bin as well. Without apt
/// the .deb the release channel offers cannot be installed, so that host is
/// not a .deb install.
#[test]
fn usr_bin_without_apt_is_a_system_package_not_a_deb() {
    assert_eq!(
        layout_for_package_manager(InstallLayout::LinuxDeb, false),
        InstallLayout::SystemPackage
    );
    assert_eq!(
        layout_for_package_manager(InstallLayout::LinuxDeb, true),
        InstallLayout::LinuxDeb
    );
}

#[test]
fn a_missing_apt_changes_no_other_layout() {
    for layout in [
        InstallLayout::MacosPkg,
        InstallLayout::UserDir,
        InstallLayout::Development,
    ] {
        assert_eq!(layout_for_package_manager(layout.clone(), false), layout);
    }
}

/// The package manager owns a system package's binaries: the updater has no
/// directory to install into and uninstall removes nothing from /usr/bin.
#[test]
fn a_system_package_has_no_install_bin_dir() {
    assert_eq!(install_bin_dir_for(&InstallLayout::SystemPackage), None);
}

#[test]
fn detect_no_false_positive_on_substring() {
    // Path that contains "/usr/local/bin" as a substring of a component name
    let path = Path::new("/home/usr/local/bin-tools/capsem");
    // "bin-tools" != "bin", so this should NOT match MacosPkg
    assert_eq!(detect_layout_from_path(path), InstallLayout::Development);
}

#[test]
fn detect_no_false_positive_capsem_in_name() {
    // ".capsem" appears but not followed by "bin" component
    let path = Path::new("/home/user/.capsem/data/capsem");
    assert_eq!(detect_layout_from_path(path), InstallLayout::Development);
}

#[test]
fn install_bin_dir_development_returns_none() {
    // In test context we're Development
    assert_eq!(install_bin_dir(), None);
}

/// The updater runs the binary a package just installed, so the directory
/// must come from the layout it detected before the package replaced the
/// running executable, not from re-reading `current_exe` afterwards.
#[test]
fn install_bin_dir_for_names_each_layout_package_directory() {
    assert_eq!(
        install_bin_dir_for(&InstallLayout::LinuxDeb),
        Some(PathBuf::from("/usr/bin"))
    );
    for layout in [InstallLayout::MacosPkg, InstallLayout::UserDir] {
        assert_eq!(
            install_bin_dir_for(&layout),
            Some(capsem_foundation::paths::capsem_bin_dir())
        );
    }
    assert_eq!(install_bin_dir_for(&InstallLayout::Development), None);
}

#[test]
fn a_system_package_is_told_to_update_with_its_package_manager() {
    let message = InstallLayout::SystemPackage.no_installer_message();
    assert!(message.contains("package manager"), "{message}");
    assert!(
        InstallLayout::UserDir
            .no_installer_message()
            .contains("No installer package in release manifest"),
        "other layouts keep the existing wording"
    );
}
