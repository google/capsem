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
        evidence: "/profiles/releases/1.0.0/co-work/apt-packages.txt".to_string(),
        digest: digest_set(),
    }
}

#[test]
fn release_graph_enums_reject_unknown_status_values() {
    let error = serde_json::from_value::<Status>(serde_json::json!("removed"))
        .expect_err("removed is absence from a newer graph, not a status");

    assert!(
        error.to_string().contains("unknown variant")
            || error.to_string().contains("expected one of"),
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
    let error = serde_json::from_value::<ManifestRecord>(invalid)
        .expect_err("manifest records must not use schema_version");

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
    assert_eq!(
        parsed.channels["nightly"].manifests[0].status,
        Status::Current
    );
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
    let parsed: ChannelsCatalog =
        serde_json::from_value(catalog).expect("JSON shape parses before validation");
    let error = parsed
        .validate()
        .expect_err("duplicate manifest versions are ambiguous");
    assert!(
        error.to_string().contains("duplicate manifest version"),
        "{error}"
    );
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
    let parsed: ChannelsCatalog =
        serde_json::from_value(catalog).expect("JSON shape parses before validation");
    let error = parsed.validate().expect_err("bad sha256 rejected");
    assert!(error.to_string().contains("sha256"), "{error}");
}

#[test]
fn release_graph_digest_verifier_rejects_tampered_profile_ref() {
    let bytes = br#"{"id":"co-work","version":"1.2.0"}"#;
    let digest = DigestSet {
        sha256: format!("{:x}", Sha256::digest(bytes)),
        blake3: blake3::hash(bytes).to_hex().to_string(),
    };

    digest
        .verify_bytes(bytes, "profile co-work")
        .expect("original bytes verify");
    let error = digest
        .verify_bytes(br#"{"id":"co-work","version":"1.2.1"}"#, "profile co-work")
        .expect_err("tampered profile ref is rejected");
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

    catalog
        .validate()
        .expect("revoked manifests remain auditable");
    let selected = catalog
        .select_manifest("stable")
        .expect("supported fallback selected");
    assert_eq!(selected.version, "1.3.0");
    assert_eq!(
        catalog.channels["stable"].manifests[0].status,
        Status::Revoked
    );
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

    let selected = catalog
        .select_manifest("nightly")
        .expect("manifest selected");
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
        status: Status::Current,
        packages: vec![PackageInventoryRow {
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
                url: "/packages/stable/1.4.0/capsem-1-4-0-pkg-sbom.spdx.json".to_string(),
                digest: digest_set(),
            }],
        }],
        profiles: BTreeMap::new(),
    };

    manifest
        .validate_inventory_shape()
        .expect("package and binary inventory is valid");
    assert_ne!(
        manifest.packages[0].name,
        manifest.packages[0].binaries[0].name
    );
    assert_eq!(
        manifest.packages[0].binaries[0].installed_path,
        "/usr/local/bin/capsem"
    );
}

#[test]
fn source_commit_is_optional_for_legacy_graphs_but_strict_when_present() {
    let package = PackageInventoryRow {
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
    let profile = profile_with_image_artifacts("1.0.0", Vec::new());

    let package_legacy = serde_json::to_value(&package).expect("serialize package");
    let profile_legacy = serde_json::to_value(&profile).expect("serialize profile");
    assert!(package_legacy.get("source_commit").is_none());
    assert!(profile_legacy.get("source_commit").is_none());
    serde_json::from_value::<PackageInventoryRow>(package_legacy.clone())
        .expect("legacy package without source commit remains readable");
    serde_json::from_value::<ProfileDocument>(profile_legacy.clone())
        .expect("legacy profile without source commit remains readable");

    for invalid in [
        serde_json::Value::Null,
        serde_json::json!("A".repeat(40)),
        serde_json::json!("a".repeat(39)),
        serde_json::json!("main"),
        serde_json::json!(format!("{} ", "a".repeat(40))),
    ] {
        let mut package_value = package_legacy.clone();
        package_value["source_commit"] = invalid.clone();
        serde_json::from_value::<PackageInventoryRow>(package_value)
            .expect_err("malformed package source commit must fail");

        let mut profile_value = profile_legacy.clone();
        profile_value["source_commit"] = invalid;
        serde_json::from_value::<ProfileDocument>(profile_value)
            .expect_err("malformed profile source commit must fail");
    }
}

#[test]
fn source_commit_belongs_only_to_package_and_profile_families() {
    let package = PackageInventoryRow {
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
        status: Status::Current,
        packages: vec![package],
        profiles: BTreeMap::new(),
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
        .expect_err("package architecture must not enter VM/profile rows");
    assert_eq!(
        serde_json::from_str::<Architecture>(r#""x86_64""#).expect("machine architecture"),
        Architecture::X86_64
    );
}

#[test]
fn package_architecture_parser_rejects_aliases_and_filename_lies() {
    assert_eq!(
        PackageArchitecture::from_package_name("Capsem_1.4.0_amd64.deb")
            .expect("amd64 Debian filename"),
        PackageArchitecture::Amd64
    );
    assert_eq!(
        PackageArchitecture::from_package_name("Capsem_1.4.0_arm64.deb")
            .expect("arm64 Debian filename"),
        PackageArchitecture::Arm64
    );
    PackageArchitecture::from_package_name("Capsem_1.4.0_x86_64.deb")
        .expect_err("machine alias is forbidden in package filenames");
    PackageArchitecture::from_package_name("Capsem_1.4.0.deb")
        .expect_err("missing Debian architecture is rejected");

    let package = PackageInventoryRow {
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
    assert!(
        format!("{error:#}").contains("filename architecture"),
        "{error:#}"
    );
}

#[test]
fn package_inventory_requires_package_sbom() {
    let manifest = ReleaseManifest {
        version: "1.4.0".to_string(),
        status: Status::Current,
        packages: vec![PackageInventoryRow {
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
        profiles: BTreeMap::new(),
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
        status: Status::Current,
        packages: vec![PackageInventoryRow {
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
        profiles: BTreeMap::new(),
    };

    let error = manifest
        .validate_inventory_shape()
        .expect_err("bad package digest is rejected");
    assert!(format!("{error:#}").contains("blake3"), "{error:#}");
}

#[test]
fn executable_inventory_records_every_packaged_binary_with_hashes_and_sbom_refs() {
    let package = PackageInventoryRow {
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
        (
            "/usr/local/bin/capsem".to_string(),
            "SPDXRef-File-capsem".to_string(),
        ),
        (
            "/usr/local/share/capsem/bin/capsem-service".to_string(),
            "SPDXRef-File-capsem-service".to_string(),
        ),
    ]);

    let rows = executable_inventory_from_package_files(&package, &files, &sbom_refs).expect("rows");

    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].name, "capsem");
    assert_eq!(rows[0].installed_path, "/usr/local/bin/capsem");
    assert_eq!(
        rows[0].digest.sha256,
        format!("{:x}", Sha256::digest(b"capsem-bin"))
    );
    assert_eq!(
        rows[0].digest.blake3,
        blake3::hash(b"capsem-bin").to_hex().to_string()
    );
    assert_eq!(rows[0].sbom_component_ref, "SPDXRef-File-capsem");
    assert_eq!(rows[1].sbom_component_ref, "SPDXRef-File-capsem-service");
}

#[test]
fn executable_inventory_rejects_missing_sbom_component_ref() {
    let package = PackageInventoryRow {
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
    let macos_rows =
        executable_inventory_from_package_files(&macos_package, &macos_files, &macos_sbom_refs)
            .expect("macOS package rows");
    verify_package_contents_match_binary_inventory(&macos_package, &macos_files, &macos_rows)
        .expect("macOS package contents match manifest inventory");

    let deb_package = PackageInventoryRow {
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
        (
            "/usr/bin/capsem".to_string(),
            "SPDXRef-File-deb-capsem".to_string(),
        ),
        (
            "/usr/bin/capsem-service".to_string(),
            "SPDXRef-File-deb-capsem-service".to_string(),
        ),
    ]);
    let deb_rows =
        executable_inventory_from_package_files(&deb_package, &deb_files, &deb_sbom_refs)
            .expect("deb package rows");
    verify_package_contents_match_binary_inventory(&deb_package, &deb_files, &deb_rows)
        .expect("deb package contents match manifest inventory");
}

#[test]
fn executable_inventory_rejects_package_content_hash_drift() {
    let package = PackageInventoryRow {
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
    let sbom_refs = BTreeMap::from([(
        "/usr/bin/capsem".to_string(),
        "SPDXRef-File-deb-capsem".to_string(),
    )]);
    let mut rows =
        executable_inventory_from_package_files(&package, &files, &sbom_refs).expect("rows");
    rows[0].digest.sha256 = "0".repeat(64);

    let error = verify_package_contents_match_binary_inventory(&package, &files, &rows)
        .expect_err("tampered package content hash must be rejected");

    assert!(
        format!("{error:#}").contains("sha256 mismatch"),
        "{error:#}"
    );
}

fn profile_with_image_artifacts(
    revision: &str,
    artifacts: Vec<ProfileImageArtifactRef>,
) -> ProfileDocument {
    ProfileDocument {
        version: revision.to_string(),
        id: "co-work".to_string(),
        name: "Co-work".to_string(),
        revision: revision.to_string(),
        source_commit: None,
        status: Status::Current,
        min_capsem_version: Some("1.4.0".to_string()),
        max_capsem_version: None,
        architectures: vec![profile_architecture(revision, artifacts)],
    }
}

fn profile_architecture(
    revision: &str,
    artifacts: Vec<ProfileImageArtifactRef>,
) -> ProfileArchitectureImages {
    ProfileArchitectureImages {
        architecture: Architecture::Arm64,
        software: vec![software_row()],
        config: vec![ProfileConfigRef {
            kind: ProfileConfigKind::Mcp,
            path: "profiles/co-work/mcp.json".to_string(),
            url: format!("/profiles/releases/{revision}/co-work/arm64/mcp.json"),
            bytes: 12,
            digest: digest_set(),
            status: Status::Current,
        }],
        artifacts,
        evidence: vec![
            EvidenceRef {
                kind: "abom".to_string(),
                url: format!("/profiles/releases/{revision}/co-work/arm64/abom.cdx.json"),
                digest: digest_set(),
            },
            EvidenceRef {
                kind: "obom".to_string(),
                url: format!("/profiles/releases/{revision}/co-work/arm64/obom.cdx.json"),
                digest: digest_set(),
            },
            EvidenceRef {
                kind: "software_inventory".to_string(),
                url: format!("/profiles/releases/{revision}/co-work/arm64/software-inventory.json"),
                digest: digest_set_with('c', 'd'),
            },
        ],
    }
}

fn profile_image_artifact(
    kind: ProfileImageArtifactKind,
    name: &str,
    revision: &str,
) -> ProfileImageArtifactRef {
    ProfileImageArtifactRef {
        kind,
        name: name.to_string(),
        url: format!("/profiles/releases/{revision}/co-work/arm64/{name}"),
        bytes: 42,
        digest: digest_set(),
        status: Status::Current,
    }
}

fn profile_image_artifact_set(revision: &str) -> Vec<ProfileImageArtifactRef> {
    vec![
        profile_image_artifact(ProfileImageArtifactKind::Kernel, "vmlinuz", revision),
        profile_image_artifact(ProfileImageArtifactKind::Initrd, "initrd.img", revision),
        profile_image_artifact(ProfileImageArtifactKind::Rootfs, "rootfs.erofs", revision),
    ]
}

#[test]
fn profile_image_versions_append_without_deprecating_previous() {
    let first = profile_with_image_artifacts("1.0.0", profile_image_artifact_set("1.0.0"));
    let second = profile_with_image_artifacts("1.0.1", profile_image_artifact_set("1.0.1"));
    let mut history = ProfileVersionHistory::new("nightly", first).expect("first profile version");

    history
        .append_version(second)
        .expect("new profile image version appends");

    assert_eq!(history.versions.len(), 2);
    assert_eq!(history.versions[0].revision, "1.0.0");
    assert!(history.versions[0].architectures[0]
        .artifacts
        .iter()
        .all(|artifact| artifact.status == Status::Current));
    assert_eq!(history.versions[1].revision, "1.0.1");
}

#[test]
fn profile_image_artifact_sets_require_kernel_initrd_and_rootfs() {
    let profile = profile_with_image_artifacts(
        "1.0.0",
        vec![
            profile_image_artifact(ProfileImageArtifactKind::Initrd, "initrd.img", "1.0.0"),
            profile_image_artifact(ProfileImageArtifactKind::Rootfs, "rootfs.erofs", "1.0.0"),
        ],
    );

    let error = profile
        .validate_profile_ownership()
        .expect_err("profile image sets must include every required image kind");

    assert!(
        error.to_string().contains("images missing kernel"),
        "{error}"
    );
}

#[test]
fn profile_image_evidence_must_match_owning_architecture() {
    let mut profile = profile_with_image_artifacts("1.0.0", profile_image_artifact_set("1.0.0"));
    let abom = profile.architectures[0]
        .evidence
        .iter_mut()
        .find(|evidence| evidence.kind == "abom")
        .expect("abom evidence");
    abom.url = abom.url.replace("/arm64/", "/x86_64/");

    let error = profile
        .validate_profile_ownership()
        .expect_err("image evidence must stay scoped to its owning architecture");

    assert!(
        error
            .to_string()
            .contains("evidence abom url must include /arm64/"),
        "{error}"
    );
}

#[test]
fn profile_image_versions_removed_image_is_absent_not_status_removed() {
    let previous = profile_with_image_artifacts("1.0.0", profile_image_artifact_set("1.0.0"));
    let next = profile_with_image_artifacts(
        "1.0.1",
        vec![
            profile_image_artifact(ProfileImageArtifactKind::Kernel, "vmlinuz", "1.0.1"),
            profile_image_artifact(ProfileImageArtifactKind::Rootfs, "rootfs.erofs", "1.0.1"),
        ],
    );

    let error = diff_profile_image_artifacts(&previous, &next)
        .expect_err("required image artifacts cannot be omitted from a profile revision");

    assert!(
        error.to_string().contains("images missing initrd"),
        "{error}"
    );

    let invalid_removed_status = serde_json::json!({
        "kind": "initrd",
        "name": "initrd.img",
        "url": "/profiles/releases/1.0.1/co-work/arm64/initrd.img",
        "bytes": 42,
        "digest": digest_json(),
        "status": "removed"
    });
    serde_json::from_value::<ProfileImageArtifactRef>(invalid_removed_status)
        .expect_err("removed is represented by absence, not by a status enum");
}

#[test]
fn profile_config_kind_rejects_unknown_values() {
    for kind in [
        "apt_packages",
        "python_requirements",
        "python_requirements_lock",
        "npm_packages",
        "npm_package_lock",
    ] {
        let value = serde_json::json!({
            "kind": kind,
            "path": format!("profiles/co-work/{kind}"),
            "url": format!("/profiles/releases/1.0.0/co-work/arm64/{kind}"),
            "bytes": 42,
            "digest": digest_json(),
            "status": "current"
        });
        serde_json::from_value::<ProfileConfigRef>(value)
            .unwrap_or_else(|error| panic!("profile config kind {kind} must deserialize: {error}"));
    }

    let invalid_kind = serde_json::json!({
        "kind": "misc",
        "path": "profiles/co-work/misc.json",
        "url": "/profiles/releases/1.0.0/co-work/arm64/misc.json",
        "bytes": 42,
        "digest": digest_json(),
        "status": "current"
    });

    serde_json::from_value::<ProfileConfigRef>(invalid_kind)
        .expect_err("profile config kind must be a release graph enum");
}

#[test]
fn profile_json_ownership_has_min_capsem_not_current_binary() {
    let profile = ProfileDocument {
        version: "1.0.0".to_string(),
        id: "co-work".to_string(),
        name: "Co-work".to_string(),
        revision: "1.0.0".to_string(),
        source_commit: None,
        status: Status::Current,
        min_capsem_version: Some("1.4.0".to_string()),
        max_capsem_version: None,
        architectures: vec![ProfileArchitectureImages {
            architecture: Architecture::Arm64,
            software: vec![software_row()],
            config: vec![ProfileConfigRef {
                kind: ProfileConfigKind::Mcp,
                path: "profiles/co-work/mcp.json".to_string(),
                url: "/profiles/releases/1.0.0/co-work/arm64/mcp.json".to_string(),
                bytes: 12,
                digest: digest_set(),
                status: Status::Current,
            }],
            artifacts: vec![
                ProfileImageArtifactRef {
                    kind: ProfileImageArtifactKind::Kernel,
                    name: "vmlinuz".to_string(),
                    url: "/profiles/releases/1.0.0/co-work/arm64/vmlinuz".to_string(),
                    bytes: 42,
                    digest: digest_set(),
                    status: Status::Current,
                },
                ProfileImageArtifactRef {
                    kind: ProfileImageArtifactKind::Initrd,
                    name: "initrd.img".to_string(),
                    url: "/profiles/releases/1.0.0/co-work/arm64/initrd.img".to_string(),
                    bytes: 42,
                    digest: digest_set(),
                    status: Status::Current,
                },
                ProfileImageArtifactRef {
                    kind: ProfileImageArtifactKind::Rootfs,
                    name: "rootfs.erofs".to_string(),
                    url: "/profiles/releases/1.0.0/co-work/arm64/rootfs.erofs".to_string(),
                    bytes: 42,
                    digest: digest_set(),
                    status: Status::Current,
                },
            ],
            evidence: vec![
                EvidenceRef {
                    kind: "abom".to_string(),
                    url: "/profiles/releases/1.0.0/co-work/arm64/abom.cdx.json".to_string(),
                    digest: digest_set(),
                },
                EvidenceRef {
                    kind: "obom".to_string(),
                    url: "/profiles/releases/1.0.0/co-work/arm64/obom.cdx.json".to_string(),
                    digest: digest_set(),
                },
                EvidenceRef {
                    kind: "software_inventory".to_string(),
                    url: "/profiles/releases/1.0.0/co-work/arm64/software-inventory.json"
                        .to_string(),
                    digest: digest_set_with('c', 'd'),
                },
            ],
        }],
    };

    profile
        .validate_profile_ownership()
        .expect("profile-owned graph validates");
    assert_eq!(profile.min_capsem_version.as_deref(), Some("1.4.0"));
    assert_eq!(profile.architectures[0].evidence.len(), 3);
}

#[test]
fn profile_json_ownership_rejects_unversioned_software_rows() {
    let mut profile = profile_with_image_artifacts(
        "1.0.0",
        vec![profile_image_artifact(
            ProfileImageArtifactKind::Rootfs,
            "rootfs.erofs",
            "1.0.0",
        )],
    );
    profile.architectures[0].software[0].version = "unversioned".to_string();

    let error = profile
        .validate_profile_ownership()
        .expect_err("profile software rows must use real versions");

    assert!(error.to_string().contains("unversioned"), "{error}");
}

#[test]
fn profile_json_ownership_rejects_software_machine_architecture_mismatch() {
    let mut profile = profile_with_image_artifacts(
        "1.0.0",
        vec![profile_image_artifact(
            ProfileImageArtifactKind::Rootfs,
            "rootfs.erofs",
            "1.0.0",
        )],
    );
    profile.architectures[0].software[0].architecture = Architecture::X86_64;

    let error = profile
        .validate_profile_ownership()
        .expect_err("software rows must use their owning machine architecture");

    assert!(
        error.to_string().contains("architecture mismatch"),
        "{error}"
    );
}

#[test]
fn profile_json_ownership_rejects_reused_software_inventory_digest() {
    let mut profile = profile_with_image_artifacts(
        "1.0.0",
        vec![profile_image_artifact(
            ProfileImageArtifactKind::Rootfs,
            "rootfs.erofs",
            "1.0.0",
        )],
    );
    let inventory_digest = profile.architectures[0]
        .evidence
        .iter()
        .find(|evidence| evidence.kind == "software_inventory")
        .expect("software inventory evidence")
        .digest
        .clone();
    profile.architectures[0].software[0].digest = inventory_digest;

    let error = profile
        .validate_profile_ownership()
        .expect_err("software rows must not reuse inventory file digests");

    assert!(
        error
            .to_string()
            .contains("reuses software_inventory evidence digest"),
        "{error}"
    );
}

#[test]
fn profile_json_ownership_rejects_current_binary_and_assets() {
    let invalid = serde_json::json!({
        "version": "1.0.0",
        "id": "co-work",
        "name": "Co-work",
        "revision": "1.0.0",
        "status": "current",
        "min_capsem_version": "1.4.0",
        "current_binary": "1.4.0",
        "current_assets": "2026.0627.8"
    });

    let error = serde_json::from_value::<ProfileDocument>(invalid)
        .expect_err("profile JSON must not contain channel-owned current binary/assets");
    assert!(
        error.to_string().contains("current_binary")
            || error.to_string().contains("current_assets"),
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

    let mut profiles = BTreeMap::new();
    profiles.insert(
        "co-work".to_string(),
        ProfileDocument {
            version: "1.0.0".to_string(),
            id: "co-work".to_string(),
            name: "Co-work".to_string(),
            revision: "1.0.0".to_string(),
            source_commit: None,
            status: Status::Current,
            min_capsem_version: Some("1.4.0".to_string()),
            max_capsem_version: None,
            architectures: vec![ProfileArchitectureImages {
                architecture: Architecture::Arm64,
                software: vec![software_row()],
                config: vec![ProfileConfigRef {
                    kind: ProfileConfigKind::Mcp,
                    path: "profiles/co-work/mcp.json".to_string(),
                    url: "/profiles/releases/1.0.0/co-work/arm64/mcp.json".to_string(),
                    bytes: 12,
                    digest: digest_set(),
                    status: Status::Current,
                }],
                artifacts: vec![ProfileImageArtifactRef {
                    kind: ProfileImageArtifactKind::Rootfs,
                    name: "rootfs.erofs".to_string(),
                    url: "/profiles/releases/1.0.0/co-work/arm64/rootfs.erofs".to_string(),
                    bytes: 42,
                    digest: digest_set(),
                    status: Status::Current,
                }],
                evidence: vec![
                    EvidenceRef {
                        kind: "abom".to_string(),
                        url: "/profiles/releases/1.0.0/co-work/arm64/abom.cdx.json".to_string(),
                        digest: digest_set(),
                    },
                    EvidenceRef {
                        kind: "obom".to_string(),
                        url: "/profiles/releases/1.0.0/co-work/arm64/obom.cdx.json".to_string(),
                        digest: digest_set(),
                    },
                    EvidenceRef {
                        kind: "software_inventory".to_string(),
                        url: "/profiles/releases/1.0.0/co-work/arm64/software-inventory.json"
                            .to_string(),
                        digest: digest_set_with('c', 'd'),
                    },
                ],
            }],
        },
    );

    let mut manifests = BTreeMap::new();
    manifests.insert(
        "stable".to_string(),
        BTreeMap::from([(
            "1.4.0".to_string(),
            ReleaseManifest {
                version: "1.4.0".to_string(),
                status: Status::Current,
                packages: vec![PackageInventoryRow {
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
                profiles,
            },
        )]),
    );

    let ledger = ReleaseLedger::derive(&catalog, &manifests);
    assert!(ledger.entries.iter().any(|entry| {
        entry.channel == "stable"
            && entry.kind == ReleaseLedgerKind::Package
            && entry.name == "Capsem-1.4.0.pkg"
    }));
    assert!(ledger.entries.iter().any(|entry| {
        entry.channel == "stable"
            && entry.kind == ReleaseLedgerKind::Binary
            && entry.name == "capsem"
    }));
    assert!(ledger.entries.iter().any(|entry| {
        entry.channel == "stable"
            && entry.kind == ReleaseLedgerKind::Profile
            && entry.profile.as_deref() == Some("co-work")
    }));
    assert!(ledger.entries.iter().any(|entry| {
        entry.channel == "stable"
            && entry.kind == ReleaseLedgerKind::ProfileImage
            && entry.profile.as_deref() == Some("co-work")
            && entry.architecture == Some(ReleaseLedgerArchitecture::Machine(Architecture::Arm64))
    }));
    assert!(ledger
        .entries
        .iter()
        .any(|entry| { entry.channel == "nightly" && entry.kind == ReleaseLedgerKind::Manifest }));
}

// -- Profile revision semver discipline -------------------------------------
//
// A profile's revision is its tag: the thing a corp operator reads, a
// compatibility window is written against, and asset reuse is keyed on. Date
// strings like "2026.06.08.9" cannot carry ordering a resolver can use -- the
// date said June while the build was July, and the trailing counter counted
// hand-edits rather than publications. These tests specify strict semver,
// independently versioned per profile, enforced for first-party and
// corp-authored profiles alike.

#[test]
fn profile_revision_must_be_semver() {
    assert!(parse_profile_revision("0.6.0").is_ok());
    assert!(parse_profile_revision("1.2.3").is_ok());
}

#[test]
fn dated_profile_revisions_are_rejected() {
    // The scheme this replaces. Four components is not semver, and the
    // leading date lied about when the assets were built.
    let error = parse_profile_revision("2026.06.08.9")
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("2026.06.08.9"),
        "rejection must name the offending revision: {error}"
    );
}

#[test]
fn only_the_historical_four_numeric_component_shape_is_legacy() {
    assert!(is_legacy_profile_revision("2026.06.08.7"));
    assert!(!is_legacy_profile_revision("0.6.0"));
    assert!(!is_legacy_profile_revision("legacy"));
    assert!(!is_legacy_profile_revision("2026.06.08.7/escape"));
}

#[test]
fn a_two_component_revision_is_rejected() {
    assert!(parse_profile_revision("0.6").is_err());
}

#[test]
fn an_empty_revision_is_rejected() {
    assert!(parse_profile_revision("").is_err());
}

#[test]
fn profile_revisions_order_numerically_not_lexically() {
    // The bug a string compare hides: "0.10.0" sorts before "0.9.0" as text.
    let ten = parse_profile_revision("0.10.0").unwrap();
    let nine = parse_profile_revision("0.9.0").unwrap();
    assert!(ten > nine, "0.10.0 must outrank 0.9.0");
}

#[test]
fn semver_may_replace_a_published_legacy_revision_once() {
    assert!(ensure_revision_advances("2026.06.08.7", "0.6.0").is_ok());
    assert!(ensure_revision_advances("2026.06.08.7", "2026.06.08.8").is_err());
}

#[test]
fn republishing_the_same_revision_is_rejected() {
    let error = ensure_revision_advances("0.6.0", "0.6.0")
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("0.6.0"),
        "rejection must name the revision that failed to advance: {error}"
    );
}

#[test]
fn a_revision_that_goes_backwards_is_rejected() {
    assert!(ensure_revision_advances("0.6.1", "0.6.0").is_err());
}

#[test]
fn an_advancing_revision_is_accepted() {
    assert!(ensure_revision_advances("0.6.0", "0.6.1").is_ok());
    assert!(ensure_revision_advances("0.6.9", "0.10.0").is_ok());
}

#[test]
fn profiles_version_independently_of_each_other() {
    // Profiles are orthogonal: co-work moving does not constrain code.
    assert!(ensure_revision_advances("0.3.2", "0.3.3").is_ok());
    assert!(ensure_revision_advances("1.4.0", "1.4.1").is_ok());
}

#[test]
fn a_profile_revision_is_not_a_capsem_version() {
    // The profile's own version and the binary window it declares are
    // separate axes. A profile at 0.3.2 may require capsem >= 0.6.0.
    let revision = parse_profile_revision("0.3.2").unwrap();
    let minimum = semver::Version::parse("0.6.0").unwrap();
    assert!(
        revision < minimum,
        "these are different axes, not comparable state"
    );
}
