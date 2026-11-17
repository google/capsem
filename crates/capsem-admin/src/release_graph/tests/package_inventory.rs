//! Package/manifest schema identity and evidence invariants.
use super::*;

#[test]
fn package_inventory_rows_are_separate_from_binary_rows() {
    let binary = BinaryInventoryRow {
        name: "capsem".to_string(),
        version: "1.4.0".to_string(),
        description: "Capsem executable fixture".to_string(),
        installed_path: "/usr/local/bin/capsem".to_string(),
        platform: "macos".to_string(),
        architecture: PackageArchitecture::Arm64,
        bytes: 7,
        digest: digest_set(),
        status: Status::Current,
        sbom_component_ref: "SPDXRef-File-capsem".to_string(),
    };
    let manifest = ReleaseManifest {
        catalogs: None,
        version: "1.4.0".to_string(),
        channel: "stable".to_string(),
        status: Status::Current,
        packages: vec![PackageInventoryRow {
            id: "capsem-package".to_string(),
            name: "Capsem-1.4.0.pkg".to_string(),
            version: "1.4.0".to_string(),
            source_commit: None,
            kind: PackageKind::MacosPkg,
            platform: "macos".to_string(),
            architecture: PackageArchitecture::Arm64,
            url: "/packages/stable/1.4.0/Capsem-1.4.0.pkg".to_string(),
            bytes: 42,
            digest: digest_set(),
            status: Status::Current,
            binaries: vec![binary],
            evidence: vec![EvidenceRef {
                kind: "sbom".to_string(),
                name: None,
                url: "/packages/stable/1.4.0/capsem-1-4-0-pkg-sbom.spdx.json".to_string(),
                bytes: 1,
                digest: digest_set(),
                status: Status::Current,
            }],
        }],
        runtime: None,
    };

    manifest
        .validate_inventory_shape()
        .expect("package and binary inventory is valid");
    assert_ne!(manifest.packages[0].name, manifest.packages[0].binaries[0].name);
    assert_eq!(manifest.packages[0].binaries[0].installed_path, "/usr/local/bin/capsem");
}

#[test]
fn release_graph_source_commit_belongs_only_to_package_and_runtime_families() {
    let package = PackageInventoryRow {
        id: "capsem-package".to_string(),
        name: "Capsem-1.4.0.pkg".to_string(),
        version: "1.4.0".to_string(),
        source_commit: None,
        kind: PackageKind::MacosPkg,
        platform: "macos".to_string(),
        architecture: PackageArchitecture::Arm64,
        url: "/packages/stable/1.4.0/Capsem-1.4.0.pkg".to_string(),
        bytes: 42,
        digest: digest_set(),
        status: Status::Current,
        binaries: Vec::new(),
        evidence: Vec::new(),
    };
    let manifest = ReleaseManifest {
        catalogs: None,
        version: "1.4.0".to_string(),
        channel: "stable".to_string(),
        status: Status::Current,
        packages: vec![package],
        runtime: None,
    };
    let mut top_level = serde_json::to_value(&manifest).expect("serialize manifest");
    top_level["source_commit"] = serde_json::json!("a".repeat(40));
    serde_json::from_value::<ReleaseManifest>(top_level)
        .expect_err("a graph-wide source commit would claim ownership it does not have");

    let mut binary = serde_json::to_value(BinaryInventoryRow {
        name: "capsem".to_string(),
        version: "1.4.0".to_string(),
        description: "Capsem executable fixture".to_string(),
        installed_path: "/usr/local/bin/capsem".to_string(),
        platform: "macos".to_string(),
        architecture: PackageArchitecture::Arm64,
        bytes: 7,
        digest: digest_set(),
        status: Status::Current,
        sbom_component_ref: "SPDXRef-File-capsem".to_string(),
    })
    .expect("serialize binary");
    binary["source_commit"] = serde_json::json!("a".repeat(40));
    serde_json::from_value::<BinaryInventoryRow>(binary)
        .expect_err("per-binary source commit duplicates package-family provenance");
}

#[test]
fn package_and_machine_architecture_enums_reject_each_others_vocabulary() {
    assert_eq!(
        serde_json::from_str::<PackageArchitecture>(r#""amd64""#).expect("Debian architecture"),
        PackageArchitecture::Amd64
    );
    serde_json::from_str::<PackageArchitecture>(r#""x86_64""#)
        .expect_err("machine architecture must not enter package rows");
    serde_json::from_str::<Architecture>(r#""amd64""#)
        .expect_err("package architecture must not enter VM/runtime rows");
    assert_eq!(
        serde_json::from_str::<Architecture>(r#""x86_64""#).expect("machine architecture"),
        Architecture::X86_64
    );
}

#[test]
fn package_architecture_parser_rejects_aliases_and_filename_lies() {
    assert_eq!(
        PackageArchitecture::from_package_name("Capsem_1.4.0_amd64.deb").expect("amd64 Debian filename"),
        PackageArchitecture::Amd64
    );
    assert_eq!(
        PackageArchitecture::from_package_name("Capsem_1.4.0_arm64.deb").expect("arm64 Debian filename"),
        PackageArchitecture::Arm64
    );
    PackageArchitecture::from_package_name("Capsem_1.4.0_x86_64.deb")
        .expect_err("machine alias is forbidden in package filenames");
    PackageArchitecture::from_package_name("Capsem_1.4.0.deb").expect_err("missing Debian architecture is rejected");

    let package = PackageInventoryRow {
        id: "capsem-package".to_string(),
        name: "Capsem_1.4.0_amd64.deb".to_string(),
        version: "1.4.0".to_string(),
        source_commit: None,
        kind: PackageKind::DebianPackage,
        platform: "linux".to_string(),
        architecture: PackageArchitecture::Arm64,
        url: "/packages/stable/1.4.0/Capsem_1.4.0_amd64.deb".to_string(),
        bytes: 42,
        digest: digest_set(),
        status: Status::Current,
        binaries: Vec::new(),
        evidence: Vec::new(),
    };
    let error = package
        .validate()
        .expect_err("filename and graph architecture mismatch is rejected");
    assert!(format!("{error:#}").contains("filename architecture"), "{error:#}");
}

#[test]
fn package_inventory_requires_package_sbom() {
    let manifest = ReleaseManifest {
        catalogs: None,
        version: "1.4.0".to_string(),
        channel: "stable".to_string(),
        status: Status::Current,
        packages: vec![PackageInventoryRow {
            id: "capsem-package".to_string(),
            name: "Capsem-1.4.0.pkg".to_string(),
            version: "1.4.0".to_string(),
            source_commit: None,
            kind: PackageKind::MacosPkg,
            platform: "macos".to_string(),
            architecture: PackageArchitecture::Arm64,
            url: "/packages/stable/1.4.0/Capsem-1.4.0.pkg".to_string(),
            bytes: 42,
            digest: digest_set(),
            status: Status::Current,
            binaries: vec![BinaryInventoryRow {
                name: "capsem".to_string(),
                version: "1.4.0".to_string(),
                description: "Capsem executable fixture".to_string(),
                installed_path: "/usr/local/bin/capsem".to_string(),
                platform: "macos".to_string(),
                architecture: PackageArchitecture::Arm64,
                bytes: 7,
                digest: digest_set(),
                status: Status::Current,
                sbom_component_ref: "SPDXRef-File-capsem".to_string(),
            }],
            evidence: Vec::new(),
        }],
        runtime: None,
    };

    let error = manifest
        .validate_inventory_shape()
        .expect_err("missing package SBOM evidence is rejected");
    assert!(
        format!("{error:#}").contains("must include package SBOM evidence"),
        "{error:#}"
    );
}

#[test]
fn package_inventory_requires_sha256_and_blake3() {
    let manifest = ReleaseManifest {
        catalogs: None,
        version: "1.4.0".to_string(),
        channel: "stable".to_string(),
        status: Status::Current,
        packages: vec![PackageInventoryRow {
            id: "capsem-package".to_string(),
            name: "capsem_1.4.0_arm64.deb".to_string(),
            version: "1.4.0".to_string(),
            source_commit: None,
            kind: PackageKind::DebianPackage,
            platform: "linux".to_string(),
            architecture: PackageArchitecture::Arm64,
            url: "/packages/stable/1.4.0/capsem_1.4.0_arm64.deb".to_string(),
            bytes: 42,
            digest: DigestSet {
                sha256: "a".repeat(64),
                blake3: "not-a-blake3-digest".to_string(),
            },
            status: Status::Current,
            binaries: vec![BinaryInventoryRow {
                name: "capsem".to_string(),
                version: "1.4.0".to_string(),
                description: "Capsem executable fixture".to_string(),
                installed_path: "/usr/bin/capsem".to_string(),
                platform: "linux".to_string(),
                architecture: PackageArchitecture::Arm64,
                bytes: 7,
                digest: digest_set(),
                status: Status::Current,
                sbom_component_ref: "SPDXRef-File-capsem".to_string(),
            }],
            evidence: Vec::new(),
        }],
        runtime: None,
    };

    let error = manifest
        .validate_inventory_shape()
        .expect_err("bad package digest is rejected");
    assert!(format!("{error:#}").contains("blake3"), "{error:#}");
}
