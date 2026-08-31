use super::*;

#[test]
fn resolve_asset_paths_prefers_erofs_when_present() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("vmlinuz"), b"kernel").unwrap();
    std::fs::write(dir.path().join("initrd.img"), b"initrd").unwrap();
    std::fs::write(dir.path().join("rootfs.erofs"), b"erofs").unwrap();
    let state = make_asset_state(dir.path().to_path_buf());

    let resolved = state.resolve_asset_paths().unwrap();
    assert_eq!(resolved.rootfs, dir.path().join("rootfs.erofs"));
}

#[test]
fn resolve_asset_paths_does_not_accept_squashfs() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("vmlinuz"), b"kernel").unwrap();
    std::fs::write(dir.path().join("initrd.img"), b"initrd").unwrap();
    std::fs::write(dir.path().join("rootfs.squashfs"), b"squashfs").unwrap();
    let state = make_asset_state(dir.path().to_path_buf());

    let resolved = state.resolve_asset_paths().unwrap();
    assert_eq!(resolved.rootfs, dir.path().join("rootfs.erofs"));
    assert!(!resolved.rootfs.exists());
}

#[test]
fn asset_status_reports_reconcile_progress_fields() {
    let dir = tempfile::tempdir().unwrap();
    let arch = capsem_core::net::policy_config::current_profile_arch();
    let arch_dir = dir.path().join(arch);
    std::fs::create_dir_all(&arch_dir).unwrap();
    let state = make_asset_state(dir.path().to_path_buf());
    let profile = materialized_test_profile();
    let arch_assets = profile.assets.current_arch_assets().unwrap();
    for asset in [&arch_assets.kernel, &arch_assets.initrd, &arch_assets.rootfs] {
        std::fs::write(
            arch_dir.join(profile_asset_hash_name(asset).expect("profile asset hash name")),
            b"asset",
        )
        .unwrap();
    }
    {
        let mut reconcile = state.asset_reconcile.lock().unwrap();
        *reconcile = AssetReconcileState {
            in_progress: true,
            current_asset: Some("rootfs.erofs".to_string()),
            bytes_done: 128,
            bytes_total: Some(256),
            last_error: None,
            last_downloaded: None,
        };
    }

    let status = profile_asset_status_value(&state, &profile);
    assert_eq!(status["profile_id"], "code");
    assert_eq!(status["manifest"]["origin"], "missing");
    assert_eq!(status["ready"], true);
    assert_eq!(status["downloading"], true);
    assert_eq!(status["current_asset"], "rootfs.erofs");
    assert_eq!(status["bytes_done"], 128);
    assert_eq!(status["bytes_total"], 256);
}

#[test]
fn profile_asset_status_uses_profile_current_arch_contract() {
    let dir = tempfile::tempdir().unwrap();
    let arch = capsem_core::net::policy_config::current_profile_arch();
    let arch_dir = dir.path().join(arch);
    std::fs::create_dir_all(&arch_dir).unwrap();
    let state = make_asset_state(dir.path().to_path_buf());
    let profile = materialized_test_profile();
    let arch_assets = profile.assets.current_arch_assets().unwrap();
    for asset in [&arch_assets.kernel, &arch_assets.rootfs] {
        let hash = asset
            .hash
            .as_deref()
            .expect("profile asset hash")
            .strip_prefix("blake3:")
            .unwrap();
        let name = capsem_assets::asset_manager::hash_filename(&asset.name, hash);
        std::fs::write(arch_dir.join(name), b"asset").unwrap();
    }

    let status = profile_asset_status_value(&state, &profile);

    assert_eq!(status["profile_id"], "code");
    assert_eq!(status["revision"], profile.revision);
    assert_eq!(status["profile_payload_hash"], test_profile_payload_hash());
    assert_eq!(status["current_arch"], arch);
    assert_eq!(status["manifest"]["origin"], "missing");
    assert_eq!(status["ready"], false, "initrd is intentionally missing");
    assert!(
        status.get("filesystem").is_none(),
        "asset status must not expose build filesystem metadata"
    );
    assert!(
        status.get("compression").is_none(),
        "asset status must not expose build compression metadata"
    );
    let assets = status["assets"].as_array().unwrap();
    assert_eq!(assets.len(), 3);
    assert!(assets.iter().any(|asset| {
        asset["kind"] == "kernel"
            && asset["name"] == "vmlinuz"
            && asset["resolved_name"]
                .as_str()
                .is_some_and(|name| name.starts_with("vmlinuz-"))
            && asset["status"] == "present"
            && asset["hash"].as_str().is_some_and(|hash| hash.starts_with("blake3:"))
    }));
    assert!(assets
        .iter()
        .any(|asset| { asset["kind"] == "initrd" && asset["name"] == "initrd.img" && asset["status"] == "missing" }));
    assert!(assets.iter().any(|asset| {
        asset["kind"] == "rootfs"
            && asset["name"] == "rootfs.erofs"
            && asset["resolved_name"]
                .as_str()
                .is_some_and(|name| name.starts_with("rootfs-"))
            && asset["status"] == "present"
            && asset.get("compression").is_none()
            && asset.get("compression_level").is_none()
    }));
}

#[test]
fn profile_asset_status_rejects_unmaterialized_asset_descriptors() {
    let dir = tempfile::tempdir().unwrap();
    let arch = capsem_core::net::policy_config::current_profile_arch();
    let arch_dir = dir.path().join(arch);
    std::fs::create_dir_all(&arch_dir).unwrap();
    let state = make_asset_state(dir.path().to_path_buf());
    let mut profile = ProfileConfigFile::builtin_primary();
    let arch_assets = profile.assets.arch.get_mut(arch).unwrap();

    for asset in [
        &mut arch_assets.kernel,
        &mut arch_assets.initrd,
        &mut arch_assets.rootfs,
    ] {
        std::fs::write(arch_dir.join(&asset.name), b"stale logical asset").unwrap();
        asset.hash = None;
        asset.size = None;
    }

    let status = profile_asset_status_value(&state, &profile);

    assert_eq!(status["ready"], false);
    let assets = status["assets"].as_array().unwrap();
    assert_eq!(assets.len(), 3);
    assert!(assets.iter().all(|asset| asset["status"] == "error"));
    assert!(assets.iter().all(|asset| asset["error"]
        .as_str()
        .is_some_and(|error| error.contains("missing a materialized hash"))));
}

#[test]
fn profile_asset_status_reports_installed_manifest_metadata_and_hash() {
    let dir = tempfile::tempdir().unwrap();
    let arch = capsem_core::net::policy_config::current_profile_arch();
    std::fs::create_dir_all(dir.path().join(arch)).unwrap();
    let manifest_json = serde_json::json!({
        "format": 2,
        "refresh_policy": "24h",
        "assets": {
            "current": "2026.0609.11",
            "releases": {
                "2026.0609.11": {
                    "date": "2026-06-09",
                    "deprecated": false,
                    "min_binary": "1.0.0",
                    "arches": {}
                }
            }
        },
        "binaries": {
            "current": "1.3.1781035201",
            "releases": {
                "1.3.1781035201": {
                    "date": "2026-06-09",
                    "deprecated": false,
                    "min_assets": "2026.0609.11"
                }
            }
        }
    })
    .to_string();
    let manifest_path = dir.path().join("manifest.json");
    std::fs::write(&manifest_path, manifest_json).unwrap();
    let origin_path = dir.path().join("manifest-metadata.json");
    std::fs::write(
        &origin_path,
        serde_json::json!({
            "schema": "capsem.manifest_metadata.v1",
            "origin": "package",
            "manifest_url": "/tmp/corp/manifest.json",
            "packaged_at": "2026-06-09T12:00:00Z"
        })
        .to_string(),
    )
    .unwrap();
    let expected_hash = capsem_assets::asset_manager::hash_file(&manifest_path).unwrap();

    let state = make_asset_state(dir.path().to_path_buf());
    let profile = ProfileConfigFile::builtin_primary();
    let status = profile_asset_status_value(&state, &profile);

    assert_eq!(status["manifest"]["origin"], "package");
    assert_eq!(status["manifest"]["path"], manifest_path.display().to_string());
    assert_eq!(status["manifest"]["origin_path"], origin_path.display().to_string());
    assert_eq!(status["manifest"]["origin_source"], "/tmp/corp/manifest.json");
    assert_eq!(status["manifest"]["packaged_at"], "2026-06-09T12:00:00Z");
    assert_eq!(status["manifest"]["blake3"], expected_hash);
    assert_eq!(status["manifest"]["validation_status"], "valid");
    assert!(status["manifest"]["refreshed_at"].as_str().is_some());
    assert_eq!(status["manifest"]["format"], 2);
    assert_eq!(status["manifest"]["assets_current"], "2026.0609.11");
    assert_eq!(status["manifest"]["binaries_current"], "1.3.1781035201");
}

#[test]
fn profile_asset_status_reports_invalid_manifest_without_stale_truth() {
    let dir = tempfile::tempdir().unwrap();
    let manifest_path = dir.path().join("manifest.json");
    std::fs::write(
        &manifest_path,
        serde_json::json!({
            "format": 2,
            "refresh_policy": "24h",
            "assets": {
                "current": "2026.0609.stale",
                "releases": {
                    "2026.0609.stale": {
                        "date": "2026-06-09",
                        "deprecated": false,
                        "min_binary": "1.0.0",
                        "arches": {
                            "arm64": {
                                "vmlinuz": {
                                    "hash": "1111111111111111111111111111111111111111111111111111111111111111",
                                    "size": 1
                                }
                            }
                        }
                    }
                }
            },
            "binaries": {
                "current": "1.3.stale",
                "releases": {
                    "1.3.stale": {
                        "date": "2026-06-09",
                        "deprecated": false,
                        "min_assets": "2026.0609.stale"
                    }
                }
            }
        })
        .to_string(),
    )
    .unwrap();
    let state = make_asset_state(dir.path().to_path_buf());
    std::fs::write(&manifest_path, r#"{"format":2}"#).unwrap();

    let profile = ProfileConfigFile::builtin_primary();
    let status = profile_asset_status_value(&state, &profile);

    assert_eq!(status["manifest"]["origin"], "installed");
    assert_eq!(status["manifest"]["validation_status"], "invalid");
    assert!(!status["manifest"]["validation_error"].as_str().unwrap().is_empty());
    assert_eq!(status["manifest"]["path"], manifest_path.display().to_string());
    assert!(status["manifest"].get("assets_current").is_none());
    assert!(status["manifest"].get("binaries_current").is_none());
}

#[test]
fn asset_cleanup_preserves_profile_catalog_and_persistent_vm_pins() {
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path();
    let profile_dir = tempfile::tempdir().unwrap();
    let (config_root, profile) = install_file_asset_profile_fixture(&profile_dir);
    let catalog = ProfileCatalog::load_from_dir(&config_root.join("profiles")).unwrap();
    let catalog_rootfs = profile_asset_hash_name(
        &profile
            .assets
            .current_arch_assets()
            .expect("built-in profile has current arch assets")
            .rootfs,
    )
    .expect("catalog rootfs hash name");
    let pinned_rootfs = "rootfs-dddddddddddddddd.erofs";
    let disposable_rootfs = "rootfs-1111111111111111.erofs";
    for filename in [catalog_rootfs.as_str(), pinned_rootfs, disposable_rootfs] {
        std::fs::write(base.join(filename), filename.as_bytes()).unwrap();
    }

    let mut pins = test_asset_pins();
    pins.rootfs.hash = "blake3:dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd".into();
    let registry_path = base.join("persistent_registry.json");
    let mut registry = PersistentRegistry::load(registry_path);
    registry.data.vms.insert(
        "saved-vm".into(),
        PersistentVmEntry {
            id: new_persistent_vm_id(),
            name: "saved-vm".into(),
            profile_id: "code".into(),
            profile_revision: test_profile_revision(),
            profile_payload_hash: test_profile_payload_hash(),
            asset_pins: pins,
            ram_mb: 2048,
            cpus: 2,
            base_version: "0.0.0".into(),
            created_at: "0".into(),
            session_dir: base.join("persistent/saved-vm"),
            forked_from: None,
            description: None,
            suspended: false,
            defunct: false,
            last_error: None,
            checkpoint_path: None,
            env: None,
        },
    );

    let manifest = capsem_assets::asset_manager::ManifestV2 {
        format: 2,
        refresh_policy: "24h".into(),
        asset_base: None,
        assets: capsem_assets::asset_manager::AssetsSection {
            current: "empty".into(),
            releases: HashMap::new(),
        },
        binaries: capsem_assets::asset_manager::BinariesSection {
            current: "1.0.0".into(),
            releases: HashMap::new(),
        },
    };
    let mut preserve = profile_catalog_asset_filenames(&catalog);
    preserve.extend(persistent_registry_asset_filenames(&registry));

    let removed = capsem_assets::asset_manager::cleanup_unused_assets_preserving(base, &manifest, preserve).unwrap();

    assert_eq!(removed, vec![base.join(disposable_rootfs)]);
    assert!(base.join(catalog_rootfs).exists());
    assert!(base.join(pinned_rootfs).exists());
    assert!(!base.join(disposable_rootfs).exists());
}

#[test]
fn deprecated_asset_cleanup_preserves_persistent_vm_pins() {
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path();
    let pinned_rootfs = "rootfs-dddddddddddddddd.erofs";
    let deprecated_unpinned_rootfs = "rootfs-eeeeeeeeeeeeeeee.erofs";
    for filename in [pinned_rootfs, deprecated_unpinned_rootfs] {
        std::fs::write(base.join(filename), filename.as_bytes()).unwrap();
    }

    let mut pins = test_asset_pins();
    pins.rootfs.hash = "blake3:dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd".into();
    let registry_path = base.join("persistent_registry.json");
    let mut registry = PersistentRegistry::load(registry_path);
    registry.data.vms.insert(
        "saved-vm".into(),
        PersistentVmEntry {
            id: new_persistent_vm_id(),
            name: "saved-vm".into(),
            profile_id: "code".into(),
            profile_revision: test_profile_revision(),
            profile_payload_hash: test_profile_payload_hash(),
            asset_pins: pins,
            ram_mb: 2048,
            cpus: 2,
            base_version: "0.0.0".into(),
            created_at: "0".into(),
            session_dir: base.join("persistent/saved-vm"),
            forked_from: None,
            description: None,
            suspended: false,
            defunct: false,
            last_error: None,
            checkpoint_path: None,
            env: None,
        },
    );

    let manifest = capsem_assets::asset_manager::ManifestV2 {
        format: 2,
        refresh_policy: "24h".into(),
        asset_base: None,
        assets: capsem_assets::asset_manager::AssetsSection {
            current: "2030.0101.1".into(),
            releases: [(
                "2030.0101.1".into(),
                capsem_assets::asset_manager::AssetRelease {
                    date: "2030-01-01".into(),
                    deprecated: true,
                    deprecated_date: Some("2030-01-02".into()),
                    min_binary: "1.0.0".into(),
                    arches: [(
                        "arm64".into(),
                        [
                            (
                                "rootfs.erofs".into(),
                                capsem_assets::asset_manager::AssetEntry {
                                    hash: "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee".into(),
                                    sha256: String::new(),
                                    size: 1,
                                },
                            ),
                            (
                                "rootfs-pinned.erofs".into(),
                                capsem_assets::asset_manager::AssetEntry {
                                    hash: "dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd".into(),
                                    sha256: String::new(),
                                    size: 1,
                                },
                            ),
                        ]
                        .into_iter()
                        .collect(),
                    )]
                    .into_iter()
                    .collect(),
                },
            )]
            .into_iter()
            .collect(),
        },
        binaries: capsem_assets::asset_manager::BinariesSection {
            current: "1.0.0".into(),
            releases: HashMap::new(),
        },
    };
    let preserve = persistent_registry_asset_filenames(&registry);

    let removed = capsem_assets::asset_manager::cleanup_unused_assets_preserving(base, &manifest, preserve).unwrap();

    assert_eq!(removed, vec![base.join(deprecated_unpinned_rootfs)]);
    assert!(base.join(pinned_rootfs).exists());
    assert!(!base.join(deprecated_unpinned_rootfs).exists());
}

#[test]
fn resolve_profile_asset_paths_uses_profile_hash_prefixed_assets() {
    let dir = tempfile::tempdir().unwrap();
    let profile = materialized_test_profile();
    let arch = capsem_core::net::policy_config::current_profile_arch();
    let arch_dir = dir.path().join(arch);
    std::fs::create_dir_all(&arch_dir).unwrap();
    let arch_assets = profile.assets.current_arch_assets().unwrap();
    for asset in [&arch_assets.kernel, &arch_assets.initrd, &arch_assets.rootfs] {
        let hash = asset
            .hash
            .as_deref()
            .expect("profile asset hash")
            .strip_prefix("blake3:")
            .unwrap();
        let name = capsem_assets::asset_manager::hash_filename(&asset.name, hash);
        std::fs::write(arch_dir.join(name), b"asset").unwrap();
    }
    let state = make_asset_state(dir.path().to_path_buf());

    let resolved = state.resolve_profile_asset_paths(&profile).unwrap();

    assert!(resolved.kernel.exists());
    assert!(resolved.initrd.exists());
    assert!(resolved.rootfs.exists());
    assert!(resolved.asset_version.starts_with("profile:code@"));
    assert_ne!(resolved.rootfs.file_name().unwrap(), "rootfs.erofs");
}

#[test]
fn vm_asset_block_reason_reports_unmaterialized_profile_asset_pins() {
    let dir = tempfile::tempdir().unwrap();
    let state = make_asset_state(dir.path().to_path_buf());
    let mut profile = ProfileConfigFile::builtin_primary();
    let arch = capsem_core::net::policy_config::current_profile_arch();
    profile.assets.arch.get_mut(arch).unwrap().rootfs.hash = None;

    let reason = state
        .validate_profile_asset_files(&profile, &test_asset_pins())
        .expect_err("unmaterialized profile asset pins must block VM start");

    assert!(reason.to_string().contains("missing a materialized hash"));
}

#[tokio::test]
async fn ensure_profile_assets_downloads_profile_descriptors() {
    let dir = tempfile::tempdir().unwrap();
    let source_dir = dir.path().join("sources");
    let assets_dir = dir.path().join("assets");
    std::fs::create_dir_all(&source_dir).unwrap();

    let mut profile = ProfileConfigFile::builtin_primary();
    let arch = capsem_core::net::policy_config::current_profile_arch();
    let replacements = [
        ("kernel", "kernel-bytes".as_bytes()),
        ("initrd", "initrd-bytes".as_bytes()),
        ("rootfs", "rootfs-bytes".as_bytes()),
    ];
    {
        let arch_assets = profile.assets.arch.get_mut(arch).unwrap();
        for (kind, bytes) in replacements {
            let descriptor = match kind {
                "kernel" => &mut arch_assets.kernel,
                "initrd" => &mut arch_assets.initrd,
                "rootfs" => &mut arch_assets.rootfs,
                _ => unreachable!(),
            };
            let source = source_dir.join(&descriptor.name);
            std::fs::write(&source, bytes).unwrap();
            descriptor.url = format!("file://{}", source.display());
            descriptor.hash = Some(format!(
                "blake3:{}",
                capsem_assets::asset_manager::hash_file(&source).unwrap()
            ));
            descriptor.size = Some(bytes.len() as u64);
        }
    }
    let state = make_asset_state(assets_dir.clone());

    let downloaded = ensure_profile_assets_for_state(Arc::clone(&state), &profile)
        .await
        .expect("profile ensure should download file fixtures");

    assert_eq!(downloaded, 3);
    let resolved = state.resolve_profile_asset_paths(&profile).unwrap();
    assert!(resolved.kernel.exists());
    assert!(resolved.initrd.exists());
    assert!(resolved.rootfs.exists());
    assert!(
        resolved
            .rootfs
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("rootfs-"),
        "profile ensure stores hash-prefixed assets"
    );
    let reconcile = state.asset_reconcile.lock().unwrap().clone();
    assert_eq!(reconcile.last_downloaded, Some(3));
    assert!(reconcile.last_error.is_none());

    let status = profile_asset_status_value(&state, &profile);
    assert_eq!(status["ready"], true);
    assert_eq!(status["profile_payload_hash"], profile_payload_hash(&profile).unwrap());
    let assets = status["assets"].as_array().unwrap();
    assert!(assets.iter().all(|asset| asset["status"] == "present"));
    assert!(assets.iter().any(|asset| {
        asset["kind"] == "rootfs"
            && asset["resolved_name"]
                .as_str()
                .is_some_and(|name| name.starts_with("rootfs-"))
    }));

    let downloaded = ensure_profile_assets_for_state(state, &profile)
        .await
        .expect("already verified profile assets should skip download");
    assert_eq!(downloaded, 0);
}

#[tokio::test]
async fn ensure_profile_assets_rejects_unmaterialized_profile_descriptors() {
    let dir = tempfile::tempdir().unwrap();
    let source_dir = dir.path().join("sources");
    let assets_dir = dir.path().join("assets");
    std::fs::create_dir_all(&source_dir).unwrap();
    let mut profile = ProfileConfigFile::builtin_primary();
    let arch = capsem_core::net::policy_config::current_profile_arch();
    let kernel = &mut profile.assets.arch.get_mut(arch).unwrap().kernel;
    let source = source_dir.join(&kernel.name);
    std::fs::write(&source, b"rootfs").unwrap();
    kernel.url = format!("file://{}", source.display());
    kernel.hash = None;
    kernel.size = None;
    let state = make_asset_state(assets_dir);

    let error = ensure_profile_assets_for_state(Arc::clone(&state), &profile)
        .await
        .expect_err("unmaterialized profile descriptors must not be downloaded");

    assert!(error.contains("missing a materialized hash"));
    let reconcile = state.asset_reconcile.lock().unwrap().clone();
    assert_eq!(reconcile.last_downloaded, Some(0));
    assert!(reconcile
        .last_error
        .as_deref()
        .is_some_and(|error| error.contains("missing a materialized hash")));
}

#[test]
fn vm_asset_block_reason_reports_missing_assets() {
    let dir = tempfile::tempdir().unwrap();
    let state = make_asset_state(dir.path().to_path_buf());
    let profile = materialized_test_profile();
    install_test_profile_catalog(&state, &profile);

    let reason = vm_asset_block_reason(&state, "code").expect("missing assets must block VM start");

    assert!(reason.contains("VM assets are not ready"));
    assert!(reason.contains("vmlinuz"));
    assert!(reason.contains("initrd.img"));
}

#[test]
fn vm_asset_block_reason_reports_downloading_assets() {
    let dir = tempfile::tempdir().unwrap();
    let state = make_asset_state(dir.path().to_path_buf());
    let profile = materialized_test_profile();
    install_test_profile_catalog(&state, &profile);
    state.asset_reconcile.lock().unwrap().in_progress = true;

    let reason = vm_asset_block_reason(&state, "code").expect("missing assets must block VM start");

    assert!(reason.contains("VM assets are still downloading"));
}

#[test]
fn vm_asset_block_reason_allows_ready_assets() {
    let dir = tempfile::tempdir().unwrap();
    let state = make_asset_state(dir.path().to_path_buf());
    install_test_profile_assets(&state);

    assert!(vm_asset_block_reason(&state, "code").is_none());
}

#[test]
fn load_asset_reconcile_state_resets_stale_in_progress() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("asset-status.json");
    std::fs::write(
        &path,
        r#"{
          "in_progress": true,
          "current_asset": "rootfs.erofs",
          "bytes_done": 512,
          "bytes_total": 1024,
          "last_error": "prior failure",
          "last_downloaded": 2
        }"#,
    )
    .unwrap();

    let loaded = load_asset_reconcile_state(&path);

    assert!(
        !loaded.in_progress,
        "startup must not preserve stale active download state"
    );
    assert!(loaded.current_asset.is_none());
    assert_eq!(loaded.bytes_done, 0);
    assert!(loaded.bytes_total.is_none());
    assert_eq!(loaded.last_error.as_deref(), Some("prior failure"));
    assert_eq!(loaded.last_downloaded, Some(2));
}

#[test]
fn persist_asset_reconcile_state_roundtrips_failure() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("nested").join("asset-status.json");
    let status = AssetReconcileState {
        in_progress: false,
        current_asset: None,
        bytes_done: 0,
        bytes_total: None,
        last_error: Some("GET failed".to_string()),
        last_downloaded: Some(0),
    };

    persist_asset_reconcile_state(&path, &status).unwrap();
    let loaded = load_asset_reconcile_state(&path);

    assert_eq!(loaded.last_error.as_deref(), Some("GET failed"));
    assert_eq!(loaded.last_downloaded, Some(0));
    assert!(!loaded.in_progress);
}

#[tokio::test]
async fn ensure_assets_without_manifest_is_noop_success() {
    let dir = tempfile::tempdir().unwrap();
    let state = make_asset_state(dir.path().to_path_buf());

    let downloaded = ensure_assets_for_state(Arc::clone(&state)).await.unwrap();

    assert_eq!(downloaded, 0);
    let reconcile = state.asset_reconcile.lock().unwrap();
    assert!(!reconcile.in_progress);
    assert_eq!(reconcile.last_downloaded, Some(0));
    assert!(reconcile.last_error.is_none());
    drop(reconcile);

    let persisted = load_asset_reconcile_state(&state.asset_status_path);
    assert!(!persisted.in_progress);
    assert_eq!(persisted.last_downloaded, Some(0));
    assert!(persisted.last_error.is_none());
}

#[tokio::test]
async fn ensure_assets_rejects_concurrent_reconcile() {
    let dir = tempfile::tempdir().unwrap();
    let state = make_asset_state(dir.path().to_path_buf());
    state.asset_reconcile_inflight.store(true, Ordering::Release);

    let err = ensure_assets_for_state(Arc::clone(&state))
        .await
        .expect_err("second reconcile must be rejected");

    assert!(err.contains("already in progress"), "unexpected error: {err}");
    assert!(state.asset_reconcile_inflight.load(Ordering::Acquire));
    state.asset_reconcile_inflight.store(false, Ordering::Release);
}

// -----------------------------------------------------------------------
// next_job_id
// -----------------------------------------------------------------------

#[test]
fn next_job_id_starts_at_1() {
    let state = make_test_state();
    assert_eq!(state.next_job_id(), 1);
}

#[test]
fn next_job_id_increments() {
    let state = make_test_state();
    let a = state.next_job_id();
    let b = state.next_job_id();
    let c = state.next_job_id();
    assert_eq!(b, a + 1);
    assert_eq!(c, a + 2);
}

#[test]
fn next_job_id_unique_across_many() {
    let state = make_test_state();
    let ids: Vec<u64> = (0..1000).map(|_| state.next_job_id()).collect();
    let unique: std::collections::HashSet<u64> = ids.iter().copied().collect();
    assert_eq!(unique.len(), 1000);
}

// -----------------------------------------------------------------------
// Instance map CRUD
// -----------------------------------------------------------------------

#[test]
fn instance_insert_and_lookup() {
    let state = make_test_state();
    insert_fake_instance(&state, "test-vm", std::process::id());
    let instances = state.instances.lock().unwrap();
    assert!(instances.contains_key("test-vm"));
    assert_eq!(instances["test-vm"].ram_mb, 2048);
}

#[test]
fn instance_remove() {
    let state = make_test_state();
    insert_fake_instance(&state, "test-vm", std::process::id());
    state.instances.lock().unwrap().remove("test-vm");
    assert!(!state.instances.lock().unwrap().contains_key("test-vm"));
}

#[test]
fn instance_lookup_missing() {
    let state = make_test_state();
    assert!(!state.instances.lock().unwrap().contains_key("no-such-vm"));
}

#[test]
fn instance_count() {
    let state = make_test_state();
    insert_fake_instance(&state, "vm-1", std::process::id());
    insert_fake_instance(&state, "vm-2", std::process::id());
    insert_fake_instance(&state, "vm-3", std::process::id());
    assert_eq!(state.instances.lock().unwrap().len(), 3);
}

// -----------------------------------------------------------------------
// cleanup_stale_instances
// -----------------------------------------------------------------------

#[test]
fn cleanup_removes_dead_pid() {
    let state = make_test_state();
    // PID 99999999 should not exist
    insert_fake_instance(&state, "dead-vm", 99999999);
    assert_eq!(state.instances.lock().unwrap().len(), 1);
    state.cleanup_stale_instances();
    assert_eq!(state.instances.lock().unwrap().len(), 0);
}

#[test]
fn cleanup_keeps_live_pid() {
    let state = make_test_state();
    // Current process PID should be alive
    insert_fake_instance(&state, "live-vm", std::process::id());
    state.cleanup_stale_instances();
    assert_eq!(state.instances.lock().unwrap().len(), 1);
}

#[test]
fn cleanup_mixed_live_and_dead() {
    let state = make_test_state();
    insert_fake_instance(&state, "live", std::process::id());
    insert_fake_instance(&state, "dead", 99999999);
    state.cleanup_stale_instances();
    let instances = state.instances.lock().unwrap();
    assert_eq!(instances.len(), 1);
    assert!(instances.contains_key("live"));
}

// -----------------------------------------------------------------------
// drain_dead_instances: probe-and-evict contract, filesystem work is the
// caller's responsibility. Exists so `cleanup_stale_instances` can release
// the instances mutex BEFORE performing remove_dir_all -- otherwise every
// handler that touches instances.lock() blocks on slow fs I/O.
// -----------------------------------------------------------------------

#[test]
fn drain_dead_instances_returns_only_dead_entries() {
    let state = make_test_state();
    insert_fake_instance(&state, "live", std::process::id());
    insert_fake_instance(&state, "dead", 99999999);

    let evicted = state.drain_dead_instances();

    assert_eq!(evicted.len(), 1);
    assert_eq!(evicted[0].0, "dead");
    let map = state.instances.lock().unwrap();
    assert!(map.contains_key("live"));
    assert!(!map.contains_key("dead"));
}

#[test]
fn drain_dead_instances_empty_when_all_alive() {
    let state = make_test_state();
    insert_fake_instance(&state, "live-1", std::process::id());
    insert_fake_instance(&state, "live-2", std::process::id());

    let evicted = state.drain_dead_instances();

    assert!(evicted.is_empty());
    assert_eq!(state.instances.lock().unwrap().len(), 2);
}

#[test]
fn drain_dead_instances_releases_mutex_before_returning() {
    // Regression guard: the whole point of splitting drain from the
    // filesystem scrub is that the mutex must be FREE by the time
    // drain returns. If this test ever fails, the locking protocol
    // has regressed and concurrent handlers will block on cleanup I/O.
    let state = make_test_state();
    insert_fake_instance(&state, "dead", 99999999);

    let _evicted = state.drain_dead_instances();

    assert!(
        state.instances.try_lock().is_ok(),
        "mutex still held after drain_dead_instances returned"
    );
}

// -----------------------------------------------------------------------
// preserve_failed_session_dir + cull_failed_sessions
//
// The post-mortem pipeline: when any of the three loss paths
// (wait_for_vm_ready timeout, dead-process cleanup, unexpected
// child exit) would have silently `remove_dir_all`'d a session dir,
// it's renamed to a `-failed-*` sibling instead so process.log,
// mcp-aggregator.stderr.log, serial.log, and session.db survive.
// Cap: MAX_FAILED_SESSIONS (5).
// -----------------------------------------------------------------------

pub(crate) fn make_state_in(test_root: PathBuf) -> Arc<ServiceState> {
    let run_dir = test_root.join("run");
    let registry_path = run_dir.join("persistent_registry.json");
    let asset_status_path = asset_status_path_for_run_dir(&run_dir);
    std::fs::create_dir_all(run_dir.join("sessions")).unwrap();
    Arc::new(ServiceState {
        instances: Mutex::new(HashMap::new()),
        session_db_handles: Mutex::new(HashMap::new()),
        persistent_registry: Mutex::new(PersistentRegistry::load(registry_path)),
        process_binary: PathBuf::from("/nonexistent/capsem-process"),
        assets_dir: PathBuf::from("/nonexistent/assets"),
        run_dir: run_dir.clone(),
        job_counter: AtomicU64::new(1),
        manifest: RwLock::new(None),
        current_version: "0.0.0".into(),
        asset_reconcile: Mutex::new(AssetReconcileState::default()),
        asset_reconcile_inflight: AtomicBool::new(false),
        asset_status_path,
        magika: test_magika(),
        plugin_policy_by_profile: Mutex::new(HashMap::new()),
        profile_summary_cache: Mutex::new(test_profile_summary_cache()),
        profile_cache: Mutex::new(test_profile_cache()),
        profile_status_cache: Mutex::new(None),
        profile_rule_cache: test_profile_rule_cache(),
        profile_mcp_default_cache: test_profile_mcp_default_cache(),
        profile_plugin_policy_cache: test_profile_plugin_policy_cache(),
        mcp_tool_cache: Mutex::new(capsem_core::mcp::load_tool_cache()),
        profile_mutation_db: test_profile_mutation_db(&run_dir),
        last_defunct_reconcile_ms: AtomicU64::new(0),
        stats_response_cache: Mutex::new(None),
        stats_detail_response_cache: Mutex::new(HashMap::new()),
        storage_diagnostics_cache: Mutex::new(HashMap::new()),
        persistent_resume_state_cache: Mutex::new(HashMap::new()),
        evaluate_rule_cache: Mutex::new(HashMap::new()),
        profile_rule_response_cache: Mutex::new(HashMap::new()),
        profile_plugin_response_cache: Mutex::new(HashMap::new()),
        evaluate_response_cache: Mutex::new(HashMap::new()),
        list_response_cache: Mutex::new(None),
        evaluate_last_response_cache: Mutex::new(None),
        save_restore_lock: tokio::sync::RwLock::new(()),
        shutdown_lock: tokio::sync::Mutex::new(()),
        update_lock: tokio::sync::Mutex::new(()),
        update_restart: tokio::sync::Notify::new(),
        _test_tempdir: None,
    })
}

#[test]
fn preserve_renames_session_dir_and_keeps_logs() {
    let dir = tempfile::tempdir().unwrap();
    let state = make_state_in(dir.path().to_path_buf());
    let session_dir = state.run_dir.join("sessions").join("vm-abc");
    std::fs::create_dir_all(&session_dir).unwrap();
    std::fs::write(session_dir.join("process.log"), b"boot failed: ...").unwrap();
    std::fs::write(session_dir.join("serial.log"), b"kernel panic").unwrap();

    state
        .preserve_failed_session_dir(&session_dir, "vm-abc")
        .expect("session evidence should be preserved");

    assert!(!session_dir.exists(), "original dir should have been renamed");
    let entries: Vec<_> = std::fs::read_dir(state.run_dir.join("sessions"))
        .unwrap()
        .flatten()
        .collect();
    let failed = entries
        .iter()
        .find(|e| e.file_name().to_string_lossy().starts_with("vm-abc-failed-"))
        .expect("a vm-abc-failed-* dir must exist");
    let preserved = failed.path().join("process.log");
    assert_eq!(
        capsem_foundation::telemetry::read_log_tail(&preserved, usize::MAX)
            .unwrap()
            .into_bytes(),
        b"boot failed: ..."
    );
    let preserved_serial = failed.path().join("serial.log");
    assert_eq!(
        capsem_foundation::telemetry::read_log_tail(&preserved_serial, usize::MAX)
            .unwrap()
            .into_bytes(),
        b"kernel panic"
    );
}

#[test]
fn cull_keeps_newest_and_prunes_oldest() {
    let dir = tempfile::tempdir().unwrap();
    let state = make_state_in(dir.path().to_path_buf());
    let sessions = state.run_dir.join("sessions");

    // Create MAX_FAILED_SESSIONS + 2 failed dirs with staggered mtimes.
    // Using filetime to set mtime lets us assert deterministically
    // which ones get pruned (oldest) vs kept (newest).
    let total = MAX_FAILED_SESSIONS + 2;
    for i in 0..total {
        let name = format!("vm-{i}-failed-20260101-00000{i}-aaaa");
        let p = sessions.join(&name);
        std::fs::create_dir_all(&p).unwrap();
        std::fs::write(p.join("process.log"), format!("run {i}")).unwrap();
        // Older i -> older mtime.
        let when = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_700_000_000 + i as u64 * 10);
        filetime::set_file_mtime(&p, filetime::FileTime::from_system_time(when)).unwrap();
    }

    state.cull_failed_sessions().unwrap();

    let remaining: std::collections::HashSet<String> = std::fs::read_dir(&sessions)
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();

    assert_eq!(
        remaining.len(),
        MAX_FAILED_SESSIONS,
        "should keep exactly MAX_FAILED_SESSIONS, got {remaining:?}"
    );
    // Oldest two (i=0, i=1) must be pruned; newest MAX_FAILED_SESSIONS kept.
    for i in 0..2 {
        let name = format!("vm-{i}-failed-20260101-00000{i}-aaaa");
        assert!(!remaining.contains(&name), "oldest dir {name} should have been culled");
    }
    for i in 2..total {
        let name = format!("vm-{i}-failed-20260101-00000{i}-aaaa");
        assert!(remaining.contains(&name), "newer dir {name} should have been kept");
    }
}

#[test]
fn cull_is_noop_when_under_cap() {
    let dir = tempfile::tempdir().unwrap();
    let state = make_state_in(dir.path().to_path_buf());
    let sessions = state.run_dir.join("sessions");

    for i in 0..3 {
        let name = format!("vm-{i}-failed-20260101-00000{i}-aaaa");
        std::fs::create_dir_all(sessions.join(&name)).unwrap();
    }

    state.cull_failed_sessions().unwrap();

    assert_eq!(std::fs::read_dir(&sessions).unwrap().count(), 3);
}

#[test]
fn cull_ignores_non_failed_dirs() {
    // Running sessions (no `-failed-` in the name) must never be
    // culled. This is the safety property: a misnamed cull is a
    // production outage.
    let dir = tempfile::tempdir().unwrap();
    let state = make_state_in(dir.path().to_path_buf());
    let sessions = state.run_dir.join("sessions");

    std::fs::create_dir_all(sessions.join("vm-alive")).unwrap();
    for i in 0..(MAX_FAILED_SESSIONS + 3) {
        let name = format!("vm-{i}-failed-20260101-00000{i}-aaaa");
        std::fs::create_dir_all(sessions.join(&name)).unwrap();
    }

    state.cull_failed_sessions().unwrap();

    assert!(sessions.join("vm-alive").exists(), "active VM dir must not be culled");
}

#[tokio::test]
async fn delete_route_destroys_retained_state_before_success() {
    let dir = tempfile::tempdir().unwrap();
    let state = make_state_in(dir.path().to_path_buf());
    let id = new_persistent_vm_id();
    let name = "delete-contract";
    let session_dir = state.run_dir.join("persistent").join(&id);
    std::fs::create_dir_all(&session_dir).unwrap();
    std::fs::write(session_dir.join("session.db"), b"retained state").unwrap();
    std::fs::write(session_dir.join("process.log"), b"retained logs").unwrap();

    let mut entry = test_persistent_entry(name, session_dir.clone());
    entry.id.clone_from(&id);
    state
        .persistent_registry
        .lock()
        .unwrap()
        .data
        .vms
        .insert(name.into(), entry);

    let app = build_service_router(Arc::clone(&state));
    let (status, body) = route_request(app, axum::http::Method::DELETE, &format!("/vms/{id}/delete"), None).await;

    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["success"], true);
    assert!(
        !session_dir.exists(),
        "DELETE must not respond before the retained session directory is gone"
    );
    let failed_prefix = format!("{id}-failed-");
    let failed_dirs: Vec<_> = std::fs::read_dir(state.run_dir.join("sessions"))
        .unwrap()
        .flatten()
        .filter(|entry| entry.file_name().to_string_lossy().starts_with(&failed_prefix))
        .collect();
    assert!(
        failed_dirs.is_empty(),
        "clean DELETE must destroy state, not relabel it as failed: {failed_dirs:?}"
    );
    assert!(
        !state.persistent_registry.lock().unwrap().data.vms.contains_key(name),
        "DELETE must unregister the retained session"
    );
}

#[tokio::test]
async fn delete_route_accepts_canonical_alias_to_trusted_run_dir() {
    let dir = tempfile::tempdir().unwrap();
    let state = make_state_in(dir.path().join("service"));
    let id = new_persistent_vm_id();
    let name = "canonical-alias-delete-contract";
    let trusted_session_dir = state.run_dir.join("persistent").join(&id);
    std::fs::create_dir_all(&trusted_session_dir).unwrap();
    std::fs::write(trusted_session_dir.join("owner-data"), b"delete me").unwrap();

    let run_dir_alias = dir.path().join("run-dir-alias");
    std::os::unix::fs::symlink(&state.run_dir, &run_dir_alias).unwrap();
    let aliased_session_dir = run_dir_alias.join("persistent").join(&id);
    let mut entry = test_persistent_entry(name, aliased_session_dir);
    entry.id.clone_from(&id);
    state
        .persistent_registry
        .lock()
        .unwrap()
        .data
        .vms
        .insert(name.into(), entry);

    let app = build_service_router(Arc::clone(&state));
    let (status, body) = route_request(app, axum::http::Method::DELETE, &format!("/vms/{id}/delete"), None).await;

    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["success"], true);
    assert!(
        !trusted_session_dir.exists(),
        "a canonical alias to the trusted run directory must delete the owned session"
    );
}

#[tokio::test]
async fn delete_route_rejects_registry_path_outside_run_dir() {
    let dir = tempfile::tempdir().unwrap();
    let state = make_state_in(dir.path().join("service"));
    let id = new_persistent_vm_id();
    let name = "unsafe-delete-contract";
    let outside_dir = dir.path().join("must-not-delete");
    std::fs::create_dir_all(&outside_dir).unwrap();
    std::fs::write(outside_dir.join("owner-data"), b"preserve me").unwrap();

    let mut entry = test_persistent_entry(name, outside_dir.clone());
    entry.id.clone_from(&id);
    state
        .persistent_registry
        .lock()
        .unwrap()
        .data
        .vms
        .insert(name.into(), entry);

    let app = build_service_router(Arc::clone(&state));
    let (status, body) = route_request(app, axum::http::Method::DELETE, &format!("/vms/{id}/delete"), None).await;

    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "{body}");
    assert!(
        outside_dir.join("owner-data").exists(),
        "a registry path outside the service run directory must never be deleted"
    );
    assert!(
        state.persistent_registry.lock().unwrap().data.vms.contains_key(name),
        "an unsafe path rejection must leave the registry entry intact"
    );
}

#[tokio::test]
async fn delete_route_rejects_symlinked_session_root() {
    let dir = tempfile::tempdir().unwrap();
    let state = make_state_in(dir.path().join("service"));
    let id = new_persistent_vm_id();
    let name = "symlink-delete-contract";
    let outside_root = dir.path().join("outside-persistent");
    let outside_session = outside_root.join(&id);
    std::fs::create_dir_all(&outside_session).unwrap();
    std::fs::write(outside_session.join("owner-data"), b"preserve me").unwrap();
    std::os::unix::fs::symlink(&outside_root, state.run_dir.join("persistent")).unwrap();

    let session_dir = state.run_dir.join("persistent").join(&id);
    let mut entry = test_persistent_entry(name, session_dir);
    entry.id.clone_from(&id);
    state
        .persistent_registry
        .lock()
        .unwrap()
        .data
        .vms
        .insert(name.into(), entry);

    let app = build_service_router(Arc::clone(&state));
    let (status, body) = route_request(app, axum::http::Method::DELETE, &format!("/vms/{id}/delete"), None).await;

    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "{body}");
    assert!(
        outside_session.join("owner-data").exists(),
        "a symlinked service root must never redirect recursive deletion"
    );
    assert!(
        state.persistent_registry.lock().unwrap().data.vms.contains_key(name),
        "a symlink-root rejection must leave the registry entry intact"
    );
}

// -----------------------------------------------------------------------
// Auto-ID generation format
// -----------------------------------------------------------------------

#[test]
fn auto_id_format() {
    // Verify the auto-ID pattern used in handle_provision
    let id = format!(
        "vm-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs()
    );
    assert!(id.starts_with("vm-"));
    // Should be "vm-" followed by digits
    let suffix = &id[3..];
    assert!(suffix.chars().all(|c| c.is_ascii_digit()));
}

// -----------------------------------------------------------------------
// Input validation edge cases (DTO level)
// -----------------------------------------------------------------------

#[test]
fn provision_request_no_name() {
    let json = serde_json::json!({"profile_id": "code", "ram_mb": 2048, "cpus": 2});
    let req: ProvisionRequest = serde_json::from_value(json).unwrap();
    assert!(req.name.is_none());
}

#[test]
fn provision_request_rejects_missing_profile_id() {
    let json = serde_json::json!({"ram_mb": 2048, "cpus": 2});
    let err = serde_json::from_value::<ProvisionRequest>(json).unwrap_err();
    assert!(err.to_string().contains("profile_id"));
}

#[test]
fn provision_request_empty_name() {
    let json = serde_json::json!({"name": "", "profile_id": "code", "ram_mb": 2048, "cpus": 2});
    let req: ProvisionRequest = serde_json::from_value(json).unwrap();
    assert_eq!(req.name.unwrap(), "");
}

#[test]
fn provision_request_name_with_path_separator() {
    // This is a security edge case -- names with / could create path traversal
    let json = serde_json::json!({"name": "../escape", "profile_id": "code", "ram_mb": 2048, "cpus": 2});
    let req: ProvisionRequest = serde_json::from_value(json).unwrap();
    assert_eq!(req.name.unwrap(), "../escape");
    // Note: the service SHOULD reject this, but currently doesn't validate
}

#[test]
fn exec_request_empty_command() {
    let json = serde_json::json!({"command": ""});
    let req: ExecRequest = serde_json::from_value(json).unwrap();
    assert_eq!(req.command, "");
}

#[test]
fn exec_request_shell_metacharacters() {
    let json = serde_json::json!({"command": "echo $(whoami) && rm -rf /"});
    let req: ExecRequest = serde_json::from_value(json).unwrap();
    assert_eq!(req.command, "echo $(whoami) && rm -rf /");
}

#[test]
fn write_file_request_path_traversal() {
    let json = serde_json::json!({"path": "../../etc/passwd", "content": "evil"});
    let req: WriteFileRequest = serde_json::from_value(json).unwrap();
    assert_eq!(req.path, "../../etc/passwd");
    // Note: no validation at DTO level -- relies on guest-side enforcement
}

// -----------------------------------------------------------------------
// Asset path resolution
// -----------------------------------------------------------------------

#[test]
fn asset_version_path_construction() {
    let base = PathBuf::from("/home/user/.capsem/assets");
    let version = "0.16.1";
    let v_path = base.join(format!("v{}", version));
    assert_eq!(v_path, PathBuf::from("/home/user/.capsem/assets/v0.16.1"));
}

#[test]
fn arch_detection_aarch64() {
    let arch = if cfg!(target_arch = "aarch64") {
        "arm64"
    } else {
        "x86_64"
    };
    assert!(arch == "arm64" || arch == "x86_64");
}

// -----------------------------------------------------------------------
// UDS path length validation (macOS 104, Linux 108 including null)
// -----------------------------------------------------------------------

#[test]
fn long_vm_name_falls_back_to_tmp_socket() {
    let state = make_test_state();
    // A 100-char name exceeds SUN_PATH_MAX via run_dir/instances/ path,
    // but instance_socket_path should fall back to /tmp/capsem/.
    let long_name = "a".repeat(100);
    let path = state.instance_socket_path(&long_name);
    assert!(
        path.starts_with("/tmp/capsem/"),
        "expected /tmp/capsem/ fallback, got: {}",
        path.display()
    );
    assert!(
        path.as_os_str().len() < 104,
        "fallback path still too long: {}",
        path.as_os_str().len()
    );
}

#[test]
fn short_vm_name_uses_run_dir() {
    let state = make_test_state();
    let path = state.instance_socket_path("test-vm");
    assert_eq!(path, state.run_dir.join("instances/test-vm.sock"));
}

#[test]
fn provision_accepts_name_just_under_uds_limit() {
    let state = make_test_state();
    let prefix = state.run_dir.join("instances").join("").as_os_str().len();
    let suffix_len = ".sock".len();
    let sun_path_max: usize = if cfg!(target_os = "macos") { 104 } else { 108 };
    // One byte shorter than the limit -- should pass path validation
    let name_len = sun_path_max - prefix - suffix_len - 1;
    let ok_name = "x".repeat(name_len);
    let result = state.provision_sandbox(ProvisionOptions {
        id: &ok_name,
        name: &ok_name,
        profile_id: "code".into(),
        ram_mb: 2048,
        cpus: 2,
        scratch_disk_size_gb: 16,
        version_override: None,
        persistent: false,
        env: None,
        from: None,
        description: None,
    });
    // Will fail later (missing rootfs), but NOT for path length
    if let Err(e) = &result {
        let msg = e.to_string();
        assert!(
            !msg.contains("socket path"),
            "short name should not hit path limit: {msg}"
        );
    }
}

#[test]
fn provision_short_name_passes_path_check() {
    let state = make_test_state();
    let result = state.provision_sandbox(ProvisionOptions {
        id: "my-vm",
        name: "my-vm",
        profile_id: "code".into(),
        ram_mb: 2048,
        cpus: 2,
        scratch_disk_size_gb: 16,
        version_override: None,
        persistent: false,
        env: None,
        from: None,
        description: None,
    });
    // Fails for missing assets, not path length
    if let Err(e) = &result {
        let msg = e.to_string();
        assert!(
            !msg.contains("socket path"),
            "normal name should not hit path limit: {msg}"
        );
    }
}

#[test]
fn provision_rejects_unknown_profile_before_boot() {
    let (state, _dir) = make_test_state_with_tempdir();
    let result = state.provision_sandbox(ProvisionOptions {
        id: "my-vm",
        name: "my-vm",
        profile_id: "missing-profile".into(),
        ram_mb: 2048,
        cpus: 2,
        scratch_disk_size_gb: 16,
        version_override: None,
        persistent: false,
        env: None,
        from: None,
        description: None,
    });
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("profile not found: missing-profile"),
        "unknown profile must fail before boot, got: {err}"
    );
    assert!(
        !state.run_dir.join("sessions/my-vm").exists(),
        "unknown profile must not create session state"
    );
}

// -----------------------------------------------------------------------
// Provision rejects duplicate persistent VM
// -----------------------------------------------------------------------

#[test]
fn provision_persistent_rejects_duplicate_name() {
    let state = make_test_state();
    // Pre-register a persistent VM directly in the registry data
    {
        let mut reg = state.persistent_registry.lock().unwrap();
        reg.data.vms.insert(
            "taken".into(),
            PersistentVmEntry {
                id: new_persistent_vm_id(),
                name: "taken".into(),
                profile_id: "code".into(),
                profile_revision: test_profile_revision(),
                profile_payload_hash: test_profile_payload_hash(),
                asset_pins: test_asset_pins(),
                ram_mb: 2048,
                cpus: 2,
                base_version: "0.0.0".into(),
                created_at: "0".into(),
                session_dir: PathBuf::from("/tmp/taken"),
                forked_from: None,
                description: None,
                suspended: false,
                defunct: false,
                last_error: None,
                checkpoint_path: None,
                env: None,
            },
        );
    }
    let result = state.provision_sandbox(ProvisionOptions {
        id: "taken",
        name: "taken",
        profile_id: "code".into(),
        ram_mb: 2048,
        cpus: 2,
        scratch_disk_size_gb: 16,
        version_override: None,
        persistent: true,
        env: None,
        from: None,
        description: None,
    });
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(err.contains("already exists"), "expected duplicate error, got: {err}");
    assert!(err.contains("resume"), "should suggest resume, got: {err}");
}

#[tokio::test]
async fn purge_default_removes_defunct_persistent_and_keeps_healthy_stopped() {
    let dir = tempfile::tempdir().unwrap();
    let state = make_asset_state(dir.path().join("assets"));
    let defunct_dir = state.run_dir.join("persistent/defunct-vm");
    let healthy_dir = state.run_dir.join("persistent/healthy-vm");
    std::fs::create_dir_all(&defunct_dir).unwrap();
    std::fs::create_dir_all(&healthy_dir).unwrap();
    std::fs::write(defunct_dir.join("process.log"), "boot failed").unwrap();
    std::fs::write(healthy_dir.join("process.log"), "stopped cleanly").unwrap();

    {
        let mut reg = state.persistent_registry.lock().unwrap();
        reg.data.vms.insert(
            "defunct-vm".into(),
            PersistentVmEntry {
                id: new_persistent_vm_id(),
                name: "defunct-vm".into(),
                profile_id: "code".into(),
                profile_revision: test_profile_revision(),
                profile_payload_hash: test_profile_payload_hash(),
                asset_pins: test_asset_pins(),
                ram_mb: 2048,
                cpus: 2,
                base_version: "0.0.0".into(),
                created_at: "0".into(),
                session_dir: defunct_dir.clone(),
                forked_from: None,
                description: None,
                suspended: false,
                defunct: true,
                last_error: Some("boot failed".into()),
                checkpoint_path: None,
                env: None,
            },
        );
        reg.data.vms.insert(
            "healthy-vm".into(),
            PersistentVmEntry {
                id: new_persistent_vm_id(),
                name: "healthy-vm".into(),
                profile_id: "code".into(),
                profile_revision: test_profile_revision(),
                profile_payload_hash: test_profile_payload_hash(),
                asset_pins: test_asset_pins(),
                ram_mb: 2048,
                cpus: 2,
                base_version: "0.0.0".into(),
                created_at: "0".into(),
                session_dir: healthy_dir.clone(),
                forked_from: None,
                description: None,
                suspended: false,
                defunct: false,
                last_error: None,
                checkpoint_path: None,
                env: None,
            },
        );
    }

    let app = build_service_router(Arc::clone(&state));
    let (status, body) = route_request(app, axum::http::Method::POST, "/purge", Some(json!({ "all": false }))).await;

    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["purged"], 1);
    assert_eq!(body["persistent_purged"], 1);
    assert_eq!(body["ephemeral_purged"], 0);

    let registry = state.persistent_registry.lock().unwrap();
    assert!(registry.get("defunct-vm").is_none());
    assert!(registry.get("healthy-vm").is_some());
    assert!(!defunct_dir.exists());
    assert!(healthy_dir.exists());
}

// -----------------------------------------------------------------------
