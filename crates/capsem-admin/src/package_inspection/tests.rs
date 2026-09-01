use super::*;
use crate::tests::{write_minimal_deb_with_control, write_minimal_deb_with_file};

#[test]
fn binary_files_from_deb_records_contained_executable_inventory() {
    let temp = tempfile::tempdir().expect("tempdir");
    let deb_path = temp.path().join("Capsem_1.4.1234567890_arm64.deb");
    let executable = b"real capsem executable bytes";
    write_minimal_deb_with_file(
        &deb_path,
        "usr/bin/capsem-app",
        executable,
        release_graph::PackageArchitecture::Arm64,
    );

    let files = binary_files_from_artifacts(&[deb_path]).expect("binary files");

    assert_eq!(files.len(), 1);
    let package = &files[0];
    assert_eq!(package.name, "Capsem_1.4.1234567890_arm64.deb");
    assert_eq!(package.binaries.len(), 1);
    let binary = &package.binaries[0];
    assert_eq!(binary.name, "capsem-app");
    assert_eq!(binary.installed_path, "/usr/bin/capsem-app");
    assert_eq!(binary.size, executable.len() as u64);
    assert_eq!(binary.sha256, format!("{:x}", Sha256::digest(executable)));
    assert_eq!(binary.blake3, blake3::hash(executable).to_hex().to_string());
    assert_eq!(binary.sbom_component_ref, "SPDXRef-File-capsem-app");
}

#[test]
fn binary_files_from_deb_rejects_filename_control_architecture_mismatch() {
    let temp = tempfile::tempdir().expect("tempdir");
    let deb_path = temp.path().join("Capsem_1.4.1234567890_amd64.deb");
    write_minimal_deb_with_file(
        &deb_path,
        "usr/bin/capsem-app",
        b"executable",
        release_graph::PackageArchitecture::Arm64,
    );

    let error = binary_files_from_artifacts(&[deb_path]).expect_err("filename/control architecture mismatch rejected");

    assert!(
        format!("{error:#}").contains("filename architecture amd64 does not match control Architecture arm64"),
        "{error:#}"
    );
}

#[test]
fn binary_files_from_deb_rejects_missing_control_architecture() {
    let temp = tempfile::tempdir().expect("tempdir");
    let deb_path = temp.path().join("Capsem_1.4.1234567890_amd64.deb");
    write_minimal_deb_with_control(
        &deb_path,
        "usr/bin/capsem-app",
        b"executable",
        b"Package: capsem\nVersion: 1.0.0\n",
    );

    let error = binary_files_from_artifacts(&[deb_path]).expect_err("missing control Architecture rejected");

    assert!(format!("{error:#}").contains("missing Architecture"), "{error:#}");
}
