use super::*;

fn digest_json() -> serde_json::Value {
    serde_json::json!({
        "sha256": "a".repeat(64),
        "blake3": "b".repeat(64)
    })
}

fn digest_set() -> DigestSet {
    digest_set_with('a', 'b')
}

fn digest_set_with(sha256: char, blake3: char) -> DigestSet {
    DigestSet {
        sha256: sha256.to_string().repeat(64),
        blake3: blake3.to_string().repeat(64),
    }
}

fn software_row() -> SoftwareInventoryRow {
    SoftwareInventoryRow {
        name: "python".to_string(),
        version: "3.12.11".to_string(),
        source: "apt".to_string(),
        architecture: Architecture::Arm64,
        evidence: "/runtime/releases/stable/1.0.0/arm64/software-inventory.json".to_string(),
        digest: digest_set(),
    }
}

#[test]
fn release_graph_enums_reject_unknown_status_values() {
    let error = serde_json::from_value::<Status>(serde_json::json!("removed"))
        .expect_err("removed is absence from a newer graph, not a status");

    assert!(
        error.to_string().contains("unknown variant") || error.to_string().contains("expected one of"),
        "{error}"
    );
}

#[test]
fn release_graph_enums_accept_only_canonical_status_values() {
    for (raw, expected) in [
        ("current", Status::Current),
        ("supported", Status::Supported),
        ("deprecated", Status::Deprecated),
        ("revoked", Status::Revoked),
    ] {
        let parsed: Status = serde_json::from_value(serde_json::json!(raw)).expect(raw);
        assert_eq!(parsed, expected);
    }
}

#[test]
fn release_graph_manifest_records_use_version_not_schema_version() {
    let valid = serde_json::json!({
        "version": "1.4.0",
        "status": "current",
        "url": "/manifests/stable/1.4.0/manifest.json",
        "digest": digest_json(),
        "min_capsem_version": "1.4.0"
    });
    serde_json::from_value::<ManifestRecord>(valid).expect("version is the manifest record key");

    let invalid = serde_json::json!({
        "schema_version": 2,
        "status": "current",
        "url": "/manifests/stable/1.4.0/manifest.json",
        "digest": digest_json()
    });
    let error =
        serde_json::from_value::<ManifestRecord>(invalid).expect_err("manifest records must not use schema_version");

    assert!(error.to_string().contains("schema_version"), "{error}");
}

#[test]
fn release_graph_channels_catalog_lists_manifest_records() {
    let catalog = serde_json::json!({
        "version": 1,
        "generated_at": "2030-01-01T00:00:00Z",
        "channels": {
            "stable": {
                "label": "Stable",
                "manifests": [
                    {
                        "version": "1.4.0",
                        "status": "current",
                        "url": "/manifests/stable/1.4.0/manifest.json",
                        "digest": digest_json()
                    },
                    {
                        "version": "1.3.0",
                        "status": "supported",
                        "url": "/manifests/stable/1.3.0/manifest.json",
                        "digest": digest_json()
                    }
                ]
            },
            "nightly": {
                "label": "Nightly",
                "manifests": [
                    {
                        "version": "1.5.0-nightly.20300101",
                        "status": "current",
                        "url": "/manifests/nightly/1.5.0-nightly.20300101/manifest.json",
                        "digest": digest_json()
                    }
                ]
            }
        }
    });

    let parsed: ChannelsCatalog = serde_json::from_value(catalog).expect("channels catalog parses");
    assert_eq!(parsed.channels["stable"].manifests.len(), 2);
    assert_eq!(parsed.channels["nightly"].manifests[0].status, Status::Current);
    parsed.validate().expect("catalog validates");
}

#[test]
fn release_graph_channels_catalog_rejects_duplicate_manifest_versions() {
    let catalog = serde_json::json!({
        "version": 1,
        "generated_at": "2030-01-01T00:00:00Z",
        "channels": {
            "stable": {
                "label": "Stable",
                "manifests": [
                    {
                        "version": "1.4.0",
                        "status": "current",
                        "url": "/manifests/stable/1.4.0/manifest.json",
                        "digest": digest_json()
                    },
                    {
                        "version": "1.4.0",
                        "status": "supported",
                        "url": "/manifests/stable/1.4.0-copy/manifest.json",
                        "digest": digest_json()
                    }
                ]
            }
        }
    });
    let parsed: ChannelsCatalog = serde_json::from_value(catalog).expect("JSON shape parses before validation");
    let error = parsed
        .validate()
        .expect_err("duplicate manifest versions are ambiguous");
    assert!(error.to_string().contains("duplicate manifest version"), "{error}");
}

#[test]
fn release_graph_channels_catalog_rejects_bad_digest_shape() {
    let catalog = serde_json::json!({
        "version": 1,
        "generated_at": "2030-01-01T00:00:00Z",
        "channels": {
            "nightly": {
                "label": "Nightly",
                "manifests": [
                    {
                        "version": "1.5.0-nightly.20300101",
                        "status": "current",
                        "url": "/manifests/nightly/1.5.0-nightly.20300101/manifest.json",
                        "digest": {
                            "sha256": "a".repeat(40),
                            "blake3": "b".repeat(64)
                        }
                    }
                ]
            }
        }
    });
    let parsed: ChannelsCatalog = serde_json::from_value(catalog).expect("JSON shape parses before validation");
    let error = parsed.validate().expect_err("bad sha256 rejected");
    assert!(error.to_string().contains("sha256"), "{error}");
}

#[test]
fn release_graph_digest_verifier_rejects_tampered_runtime_ref() {
    let bytes = br#"{"revision":"1.2.0"}"#;
    let digest = DigestSet {
        sha256: format!("{:x}", Sha256::digest(bytes)),
        blake3: blake3::hash(bytes).to_hex().to_string(),
    };

    digest
        .verify_bytes(bytes, "runtime 1.2.0")
        .expect("original bytes verify");
    let error = digest
        .verify_bytes(br#"{"revision":"1.2.1"}"#, "runtime 1.2.0")
        .expect_err("tampered runtime ref is rejected");
    assert!(error.to_string().contains("sha256 mismatch"), "{error}");
}

#[test]
fn release_graph_revoked_manifest_is_listed_but_not_selectable() {
    let catalog: ChannelsCatalog = serde_json::from_value(serde_json::json!({
        "version": 1,
        "generated_at": "2030-01-01T00:00:00Z",
        "channels": {
            "stable": {
                "label": "Stable",
                "manifests": [
                    {
                        "version": "1.4.0-bad",
                        "status": "revoked",
                        "url": "/manifests/stable/1.4.0-bad/manifest.json",
                        "digest": digest_json()
                    },
                    {
                        "version": "1.3.0",
                        "status": "supported",
                        "url": "/manifests/stable/1.3.0/manifest.json",
                        "digest": digest_json()
                    }
                ]
            }
        }
    }))
    .expect("catalog shape");

    catalog.validate().expect("revoked manifests remain auditable");
    let selected = catalog.select_manifest("stable").expect("supported fallback selected");
    assert_eq!(selected.version, "1.3.0");
    assert_eq!(catalog.channels["stable"].manifests[0].status, Status::Revoked);
}

#[test]
fn release_graph_current_manifest_is_preferred_over_supported_and_deprecated() {
    let catalog: ChannelsCatalog = serde_json::from_value(serde_json::json!({
        "version": 1,
        "generated_at": "2030-01-01T00:00:00Z",
        "channels": {
            "nightly": {
                "label": "Nightly",
                "manifests": [
                    {
                        "version": "1.5.0-nightly.old",
                        "status": "deprecated",
                        "url": "/manifests/nightly/1.5.0-nightly.old/manifest.json",
                        "digest": digest_json()
                    },
                    {
                        "version": "1.5.0-nightly.supported",
                        "status": "supported",
                        "url": "/manifests/nightly/1.5.0-nightly.supported/manifest.json",
                        "digest": digest_json()
                    },
                    {
                        "version": "1.5.0-nightly.current",
                        "status": "current",
                        "url": "/manifests/nightly/1.5.0-nightly.current/manifest.json",
                        "digest": digest_json()
                    }
                ]
            }
        }
    }))
    .expect("catalog shape");

    let selected = catalog.select_manifest("nightly").expect("manifest selected");
    assert_eq!(selected.version, "1.5.0-nightly.current");
}

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

#[test]
fn executable_inventory_records_every_packaged_binary_with_hashes_and_sbom_refs() {
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
    let files = vec![
        PackagedExecutableFile {
            name: "capsem-service".to_string(),
            description: "Capsem executable fixture".to_string(),
            installed_path: "/usr/local/share/capsem/bin/capsem-service".to_string(),
            bytes: b"service-bin".to_vec(),
        },
        PackagedExecutableFile {
            name: "capsem".to_string(),
            description: "Capsem executable fixture".to_string(),
            installed_path: "/usr/local/bin/capsem".to_string(),
            bytes: b"capsem-bin".to_vec(),
        },
    ];
    let sbom_refs = BTreeMap::from([
        ("/usr/local/bin/capsem".to_string(), "SPDXRef-File-capsem".to_string()),
        (
            "/usr/local/share/capsem/bin/capsem-service".to_string(),
            "SPDXRef-File-capsem-service".to_string(),
        ),
    ]);

    let rows = executable_inventory_from_package_files(&package, &files, &sbom_refs).expect("rows");

    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].name, "capsem");
    assert_eq!(rows[0].installed_path, "/usr/local/bin/capsem");
    assert_eq!(rows[0].digest.sha256, format!("{:x}", Sha256::digest(b"capsem-bin")));
    assert_eq!(rows[0].digest.blake3, blake3::hash(b"capsem-bin").to_hex().to_string());
    assert_eq!(rows[0].sbom_component_ref, "SPDXRef-File-capsem");
    assert_eq!(rows[1].sbom_component_ref, "SPDXRef-File-capsem-service");
}

#[test]
fn executable_inventory_rejects_missing_sbom_component_ref() {
    let package = PackageInventoryRow {
        id: "capsem-package".to_string(),
        name: "capsem_1.4.0_arm64.deb".to_string(),
        version: "1.4.0".to_string(),
        source_commit: None,
        kind: PackageKind::DebianPackage,
        platform: "linux".to_string(),
        architecture: PackageArchitecture::Arm64,
        url: "/packages/stable/1.4.0/capsem_1.4.0_arm64.deb".to_string(),
        bytes: 42,
        digest: digest_set(),
        status: Status::Current,
        binaries: Vec::new(),
        evidence: Vec::new(),
    };
    let files = vec![PackagedExecutableFile {
        name: "capsem".to_string(),
        description: "Capsem executable fixture".to_string(),
        installed_path: "/usr/bin/capsem".to_string(),
        bytes: b"capsem-bin".to_vec(),
    }];

    let error = executable_inventory_from_package_files(&package, &files, &BTreeMap::new())
        .expect_err("missing SBOM component ref rejected");

    assert!(
        format!("{error:#}").contains("missing SBOM component reference"),
        "{error:#}"
    );
}

#[test]
fn executable_inventory_matches_macos_and_deb_package_contents() {
    let macos_package = PackageInventoryRow {
        id: "capsem-package".to_string(),
        name: "Capsem-1.4.0.pkg".to_string(),
        version: "1.4.0".to_string(),
        source_commit: None,
        kind: PackageKind::MacosPkg,
        platform: "macos".to_string(),
        architecture: PackageArchitecture::Arm64,
        url: "/packages/stable/1.4.0/Capsem-1.4.0.pkg".to_string(),
        bytes: 99,
        digest: digest_set(),
        status: Status::Current,
        binaries: Vec::new(),
        evidence: Vec::new(),
    };
    let macos_files = vec![
        PackagedExecutableFile {
            name: "capsem".to_string(),
            description: "Capsem executable fixture".to_string(),
            installed_path: "/usr/local/share/capsem/bin/capsem".to_string(),
            bytes: b"macos-capsem".to_vec(),
        },
        PackagedExecutableFile {
            name: "capsem-service".to_string(),
            description: "Capsem executable fixture".to_string(),
            installed_path: "/usr/local/share/capsem/bin/capsem-service".to_string(),
            bytes: b"macos-service".to_vec(),
        },
    ];
    let macos_sbom_refs = BTreeMap::from([
        (
            "/usr/local/share/capsem/bin/capsem".to_string(),
            "SPDXRef-File-macos-capsem".to_string(),
        ),
        (
            "/usr/local/share/capsem/bin/capsem-service".to_string(),
            "SPDXRef-File-macos-capsem-service".to_string(),
        ),
    ]);
    let macos_rows = executable_inventory_from_package_files(&macos_package, &macos_files, &macos_sbom_refs)
        .expect("macOS package rows");
    verify_package_contents_match_binary_inventory(&macos_package, &macos_files, &macos_rows)
        .expect("macOS package contents match manifest inventory");

    let deb_package = PackageInventoryRow {
        id: "capsem-package".to_string(),
        name: "Capsem_1.4.0_arm64.deb".to_string(),
        version: "1.4.0".to_string(),
        source_commit: None,
        kind: PackageKind::DebianPackage,
        platform: "linux".to_string(),
        architecture: PackageArchitecture::Arm64,
        url: "/packages/stable/1.4.0/Capsem_1.4.0_arm64.deb".to_string(),
        bytes: 101,
        digest: digest_set(),
        status: Status::Current,
        binaries: Vec::new(),
        evidence: Vec::new(),
    };
    let deb_files = vec![
        PackagedExecutableFile {
            name: "capsem".to_string(),
            description: "Capsem executable fixture".to_string(),
            installed_path: "/usr/bin/capsem".to_string(),
            bytes: b"deb-capsem".to_vec(),
        },
        PackagedExecutableFile {
            name: "capsem-service".to_string(),
            description: "Capsem executable fixture".to_string(),
            installed_path: "/usr/bin/capsem-service".to_string(),
            bytes: b"deb-service".to_vec(),
        },
    ];
    let deb_sbom_refs = BTreeMap::from([
        ("/usr/bin/capsem".to_string(), "SPDXRef-File-deb-capsem".to_string()),
        (
            "/usr/bin/capsem-service".to_string(),
            "SPDXRef-File-deb-capsem-service".to_string(),
        ),
    ]);
    let deb_rows =
        executable_inventory_from_package_files(&deb_package, &deb_files, &deb_sbom_refs).expect("deb package rows");
    verify_package_contents_match_binary_inventory(&deb_package, &deb_files, &deb_rows)
        .expect("deb package contents match manifest inventory");
}

#[test]
fn executable_inventory_rejects_package_content_hash_drift() {
    let package = PackageInventoryRow {
        id: "capsem-package".to_string(),
        name: "Capsem_1.4.0_arm64.deb".to_string(),
        version: "1.4.0".to_string(),
        source_commit: None,
        kind: PackageKind::DebianPackage,
        platform: "linux".to_string(),
        architecture: PackageArchitecture::Arm64,
        url: "/packages/stable/1.4.0/Capsem_1.4.0_arm64.deb".to_string(),
        bytes: 101,
        digest: digest_set(),
        status: Status::Current,
        binaries: Vec::new(),
        evidence: Vec::new(),
    };
    let files = vec![PackagedExecutableFile {
        name: "capsem".to_string(),
        description: "Capsem executable fixture".to_string(),
        installed_path: "/usr/bin/capsem".to_string(),
        bytes: b"deb-capsem".to_vec(),
    }];
    let sbom_refs = BTreeMap::from([("/usr/bin/capsem".to_string(), "SPDXRef-File-deb-capsem".to_string())]);
    let mut rows = executable_inventory_from_package_files(&package, &files, &sbom_refs).expect("rows");
    rows[0].digest.sha256 = "0".repeat(64);

    let error = verify_package_contents_match_binary_inventory(&package, &files, &rows)
        .expect_err("tampered package content hash must be rejected");

    assert!(format!("{error:#}").contains("sha256 mismatch"), "{error:#}");
}

fn runtime_url(revision: &str, file: &str) -> String {
    format!("/runtime/releases/stable/{revision}/arm64/{file}")
}

fn runtime_evidence(kind: &str, file: &str, revision: &str, digest: DigestSet) -> EvidenceRef {
    EvidenceRef {
        kind: kind.to_string(),
        name: None,
        url: runtime_url(revision, file),
        bytes: 12,
        digest,
        status: Status::Current,
    }
}

fn runtime_image(kind: RuntimeImageArtifactKind, name: &str, revision: &str) -> RuntimeImageArtifactRef {
    RuntimeImageArtifactRef {
        kind,
        name: name.to_string(),
        url: runtime_url(revision, name),
        bytes: 42,
        digest: digest_set(),
        status: Status::Current,
    }
}

fn runtime_image_set(revision: &str) -> Vec<RuntimeImageArtifactRef> {
    vec![
        runtime_image(RuntimeImageArtifactKind::Kernel, "vmlinuz", revision),
        runtime_image(RuntimeImageArtifactKind::Initrd, "initrd.img", revision),
        runtime_image(RuntimeImageArtifactKind::Rootfs, "rootfs.erofs", revision),
    ]
}

fn runtime_document(revision: &str) -> RuntimeDocument {
    RuntimeDocument {
        revision: revision.to_string(),
        source_commit: None,
        status: Status::Current,
        min_capsem_version: Some("1.4.0".to_string()),
        max_capsem_version: None,
        architectures: vec![RuntimeArchitecture {
            architecture: Architecture::Arm64,
            package_inventory_revision: revision.to_string(),
            image_revision: revision.to_string(),
            software: vec![software_row()],
            images: runtime_image_set(revision),
            evidence: vec![
                runtime_evidence("abom", "abom.cdx.json", revision, digest_set()),
                runtime_evidence("obom", "obom.cdx.json", revision, digest_set()),
                runtime_evidence(
                    "software_inventory",
                    "software-inventory.json",
                    revision,
                    digest_set_with('c', 'd'),
                ),
            ],
        }],
    }
}

#[test]
fn runtime_document_validates_and_carries_min_capsem_not_current_binary() {
    let runtime = runtime_document("0.7.0-0123456789ab");

    runtime.validate().expect("runtime graph validates");
    assert_eq!(runtime.min_capsem_version.as_deref(), Some("1.4.0"));
    assert_eq!(runtime.architectures[0].evidence.len(), 3);
}

#[test]
fn runtime_source_commit_is_optional_but_strict_when_present() {
    let runtime = serde_json::to_value(runtime_document("1.0.0")).expect("serialize runtime");
    assert!(runtime.get("source_commit").is_none());
    serde_json::from_value::<RuntimeDocument>(runtime.clone()).expect("runtime without source commit parses");

    let mut with_commit = runtime.clone();
    with_commit["source_commit"] = serde_json::json!("a".repeat(40));
    serde_json::from_value::<RuntimeDocument>(with_commit).expect("runtime with source commit parses");

    for invalid in [
        serde_json::Value::Null,
        serde_json::json!("A".repeat(40)),
        serde_json::json!("main"),
    ] {
        let mut value = runtime.clone();
        value["source_commit"] = invalid;
        serde_json::from_value::<RuntimeDocument>(value).expect_err("malformed runtime source commit must fail");
    }
}

#[test]
fn release_graph_refuses_a_profiles_map() {
    let graph = serde_json::json!({
        "version": "1.0.0",
        "channel": "stable",
        "status": "current",
        "packages": [],
        "profiles": {}
    });

    let error = serde_json::from_value::<ReleaseManifest>(graph).expect_err("profiles are no release unit");

    assert!(error.to_string().contains("profiles"), "{error}");
}

#[test]
fn runtime_image_sets_require_kernel_initrd_and_rootfs() {
    let mut runtime = runtime_document("1.0.0");
    runtime.architectures[0]
        .images
        .retain(|image| image.kind != RuntimeImageArtifactKind::Kernel);

    let error = runtime
        .validate()
        .expect_err("runtime image sets must include every required image kind");

    assert!(error.to_string().contains("images missing kernel"), "{error}");

    let invalid_removed_status = serde_json::json!({
        "kind": "initrd",
        "name": "initrd.img",
        "url": runtime_url("1.0.0", "initrd.img"),
        "bytes": 42,
        "digest": digest_json(),
        "status": "removed"
    });
    serde_json::from_value::<RuntimeImageArtifactRef>(invalid_removed_status)
        .expect_err("removed is represented by absence, not by a status enum");
}

#[test]
fn runtime_architecture_revisions_must_be_the_runtime_revision() {
    for field in ["image_revision", "package_inventory_revision"] {
        let mut runtime = serde_json::to_value(runtime_document("1.0.0")).expect("serialize runtime");
        runtime["architectures"][0][field] = serde_json::json!("1.0.1");
        let runtime: RuntimeDocument = serde_json::from_value(runtime).expect("shape parses");

        let error = runtime.validate().expect_err("one runtime has one revision");

        assert!(error.to_string().contains(field), "{error}");
    }
}

#[test]
fn runtime_refuses_repeated_architectures_and_unsafe_revisions() {
    let mut repeated = runtime_document("1.0.0");
    repeated.architectures.push(repeated.architectures[0].clone());
    let error = repeated.validate().expect_err("one image set per architecture");
    assert!(error.to_string().contains("repeats architecture arm64"), "{error}");

    for revision in ["", "..", "1.0.0/../x", "1.0 0"] {
        let error = runtime_document(revision)
            .validate()
            .expect_err("revision is a path component");
        assert!(error.to_string().contains("URL-path-safe"), "{revision:?}: {error}");
    }
    runtime_document("0.7.0-0123456789ab")
        .validate()
        .expect("workspace version plus commit is a revision");
}

#[test]
fn runtime_capsem_window_must_be_semver_and_ordered() {
    let mut runtime = runtime_document("1.0.0");
    runtime.min_capsem_version = Some("not-a-version".to_string());
    assert!(runtime.validate().is_err());

    runtime.min_capsem_version = Some("2.0.0".to_string());
    runtime.max_capsem_version = Some("1.0.0".to_string());
    let error = runtime.validate().expect_err("minimum above maximum");
    assert!(error.to_string().contains("exceeds maximum"), "{error}");
}

#[test]
fn runtime_image_evidence_must_match_owning_architecture() {
    let mut runtime = runtime_document("1.0.0");
    let abom = runtime.architectures[0]
        .evidence
        .iter_mut()
        .find(|evidence| evidence.kind == "abom")
        .expect("abom evidence");
    abom.url = abom.url.replace("/arm64/", "/x86_64/");

    let error = runtime
        .validate()
        .expect_err("image evidence must stay scoped to its owning architecture");

    assert!(
        error.to_string().contains("evidence abom url must include /arm64/"),
        "{error}"
    );
}

#[test]
fn runtime_rejects_unversioned_software_rows() {
    let mut runtime = runtime_document("1.0.0");
    runtime.architectures[0].software[0].version = "unversioned".to_string();

    let error = runtime.validate().expect_err("software rows must use real versions");

    assert!(error.to_string().contains("unversioned"), "{error}");
}

#[test]
fn runtime_rejects_software_machine_architecture_mismatch() {
    let mut runtime = runtime_document("1.0.0");
    runtime.architectures[0].software[0].architecture = Architecture::X86_64;

    let error = runtime
        .validate()
        .expect_err("software rows must use their owning machine architecture");

    assert!(error.to_string().contains("architecture mismatch"), "{error}");
}

#[test]
fn runtime_rejects_reused_software_inventory_digest() {
    let mut runtime = runtime_document("1.0.0");
    let inventory_digest = runtime.architectures[0]
        .evidence
        .iter()
        .find(|evidence| evidence.kind == "software_inventory")
        .expect("software inventory evidence")
        .digest
        .clone();
    runtime.architectures[0].software[0].digest = inventory_digest;

    let error = runtime
        .validate()
        .expect_err("software rows must not reuse inventory file digests");

    assert!(
        error.to_string().contains("reuses software_inventory evidence digest"),
        "{error}"
    );
}

#[test]
fn release_ledger_is_derived_from_channels_and_manifests() {
    let catalog: ChannelsCatalog = serde_json::from_value(serde_json::json!({
        "version": 1,
        "generated_at": "2030-01-01T00:00:00Z",
        "channels": {
            "stable": {
                "label": "Stable",
                "manifests": [
                    {
                        "version": "1.4.0",
                        "status": "current",
                        "url": "/manifests/stable/1.4.0/manifest.json",
                        "digest": digest_json()
                    }
                ]
            },
            "nightly": {
                "label": "Nightly",
                "manifests": [
                    {
                        "version": "1.5.0-nightly.20300101",
                        "status": "current",
                        "url": "/manifests/nightly/1.5.0-nightly.20300101/manifest.json",
                        "digest": digest_json()
                    }
                ]
            }
        }
    }))
    .expect("catalog shape");

    let mut manifests = BTreeMap::new();
    manifests.insert(
        "stable".to_string(),
        BTreeMap::from([(
            "1.4.0".to_string(),
            ReleaseManifest {
                version: "1.4.0".to_string(),
                channel: "stable".to_string(),
                status: Status::Current,
                packages: vec![PackageInventoryRow {
                    id: "capsem-1-4-0-pkg".to_string(),
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
                runtime: Some(runtime_document("1.0.0")),
            },
        )]),
    );

    let ledger = ReleaseLedger::derive(&catalog, &manifests);
    assert!(ledger.entries.iter().any(|entry| {
        entry.channel == "stable" && entry.kind == ReleaseLedgerKind::Package && entry.name == "Capsem-1.4.0.pkg"
    }));
    assert!(ledger.entries.iter().any(|entry| {
        entry.channel == "stable" && entry.kind == ReleaseLedgerKind::Binary && entry.name == "capsem"
    }));
    assert!(ledger.entries.iter().any(|entry| {
        entry.channel == "stable" && entry.kind == ReleaseLedgerKind::Runtime && entry.version == "1.0.0"
    }));
    assert_eq!(
        ledger
            .entries
            .iter()
            .filter(|entry| {
                entry.channel == "stable"
                    && entry.kind == ReleaseLedgerKind::RuntimeImage
                    && entry.architecture == Some(ReleaseLedgerArchitecture::Machine(Architecture::Arm64))
            })
            .count(),
        3
    );
    assert!(ledger
        .entries
        .iter()
        .any(|entry| { entry.channel == "nightly" && entry.kind == ReleaseLedgerKind::Manifest }));
    let serialized = serde_json::to_value(&ledger).expect("serialize ledger");
    assert!(serialized.to_string().contains("\"runtime_image\""));
    assert!(!serialized.to_string().contains("\"profile"));
}
