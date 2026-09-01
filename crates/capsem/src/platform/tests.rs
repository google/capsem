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
