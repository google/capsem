use super::*;

#[test]
fn update_status_reports_binary_and_asset_tracks_from_cache_and_manifest() {
    let dir = tempfile::tempdir().unwrap();
    let assets_dir = dir.path().join("assets");
    std::fs::create_dir_all(&assets_dir).unwrap();
    std::fs::write(
        assets_dir.join("manifest.json"),
        serde_json::json!({
            "format": 2,
            "refresh_policy": "24h",
            "assets": {
                "current": "2026.0627.1",
                "releases": {}
            },
            "binaries": {
                "current": "1.3.1782582155",
                "releases": {}
            }
        })
        .to_string(),
    )
    .unwrap();
    let manifest_hash = capsem_assets::asset_manager::hash_file(&assets_dir.join("manifest.json"))
        .expect("manifest hash should be computable");
    std::fs::write(
        assets_dir.join("manifest-metadata.json"),
        serde_json::json!({
            "schema": "capsem.manifest_metadata.v1",
            "origin": "update",
            "manifest_url": "https://release.capsem.org/assets/stable/manifest.json"
        })
        .to_string(),
    )
    .unwrap();
    let cache_path = assets_dir.join("manifest-metadata.json");
    std::fs::write(
        &cache_path,
        serde_json::json!({
            "schema": "capsem.manifest_metadata.v1",
            "origin": "update",
            "manifest_url": "https://release.capsem.org/assets/stable/manifest.json",
            "checked_at": 1000,
            "latest_version": "1.3.1782600000",
            "update_available": true,
            "latest_assets": "2026.0628.1",
            "assets_update_available": true,
            "checked_url": "https://release.capsem.org/assets/stable/manifest.json",
            "channel_hash": "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
            "validation_status": "valid"
        })
        .to_string(),
    )
    .unwrap();

    let status = update_status_response_from_paths("1.3.1782582155", &assets_dir, &cache_path, 1200);

    assert_eq!(status.checked_at, Some(1000));
    assert!(!status.stale);
    assert_eq!(
        status.channel_url.as_deref(),
        Some("https://release.capsem.org/assets/stable/manifest.json")
    );
    assert_eq!(
        status.channel_hash.as_deref(),
        Some("0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef")
    );
    assert_eq!(status.supply_chain.manifest.origin.as_deref(), Some("update"));
    assert_eq!(
        status.supply_chain.manifest.source.as_deref(),
        Some("https://release.capsem.org/assets/stable/manifest.json")
    );
    assert_eq!(
        status.supply_chain.manifest.path,
        assets_dir.join("manifest.json").display().to_string()
    );
    assert_eq!(
        status.supply_chain.manifest.blake3.as_deref(),
        Some(manifest_hash.as_str())
    );
    assert_eq!(
        status.supply_chain.channel_index.url.as_deref(),
        Some("https://release.capsem.org/assets/stable/manifest.json")
    );
    assert_eq!(
        status.supply_chain.channel_index.sha256.as_deref(),
        Some("0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef")
    );
    assert_eq!(status.supply_chain.host_sbom.name, "host_sbom");
    assert_eq!(
        status.supply_chain.host_sbom.release_artifact.as_deref(),
        Some("capsem-sbom.spdx.json")
    );
    assert_eq!(
        status.supply_chain.vm_obom.route.as_deref(),
        Some("/profiles/{profile_id}/obom")
    );
    assert!(
        status
            .supply_chain
            .attestations
            .iter()
            .any(|reference| reference.name == "github_attestations_vm_assets"),
        "asset rail attestation reference should be explicit"
    );
    assert_eq!(status.validation_status.as_deref(), Some("valid"));
    assert_eq!(status.validation_error, None);
    assert_eq!(status.last_error, None);
    assert_eq!(status.binary.current.as_deref(), Some("1.3.1782582155"));
    assert_eq!(status.binary.latest.as_deref(), Some("1.3.1782600000"));
    assert_eq!(status.binary.state, api::UpdateTrackState::UpdateAvailable);
    assert_eq!(status.binary.compatibility, api::UpdateCompatibilityState::Compatible);
    assert_eq!(status.assets.current.as_deref(), Some("2026.0627.1"));
    assert_eq!(status.assets.latest.as_deref(), Some("2026.0628.1"));
    assert_eq!(status.assets.state, api::UpdateTrackState::UpdateAvailable);
    assert_eq!(status.profiles.state, api::UpdateTrackState::NotPublished);
    assert_eq!(status.images.state, api::UpdateTrackState::NotPublished);
}

#[test]
fn current_asset_state_keeps_independent_release_graph_profiles() {
    let dir = tempfile::tempdir().unwrap();
    let assets_dir = dir.path().join("assets");
    std::fs::create_dir_all(&assets_dir).unwrap();
    let images = |seed: char| {
        serde_json::json!([
            {"kind":"kernel","name":"vmlinuz","bytes":10,"status":"current","digest":{"blake3":seed.to_string().repeat(64),"sha256":"1".repeat(64)}},
            {"kind":"initrd","name":"initrd.img","bytes":20,"status":"current","digest":{"blake3":"b".repeat(64),"sha256":"2".repeat(64)}},
            {"kind":"rootfs","name":"rootfs.erofs","bytes":30,"status":"current","digest":{"blake3":"c".repeat(64),"sha256":"3".repeat(64)}}
        ])
    };
    let manifest = serde_json::json!({
        "profiles": {
            "co-work": {
                "revision": "2030.0101.1",
                "status": "current",
                "architectures": [{"architecture":"arm64","image_revision":"2030.0101.10","images":images('a')}]
            },
            "code": {
                "revision": "2030.0101.2",
                "status": "current",
                "architectures": [{"architecture":"arm64","image_revision":"2030.0101.20","images":images('9')}]
            }
        }
    });
    std::fs::write(assets_dir.join("manifest.json"), serde_json::to_vec(&manifest).unwrap()).unwrap();
    let expected = capsem_assets::asset_manager::release_graph_profile_state(&manifest).unwrap();

    assert_eq!(
        current_asset_version_from_manifest(&assets_dir),
        Some(expected.images_revision)
    );
}

#[test]
fn update_status_reports_profile_and_image_tracks_from_release_cache() {
    let dir = tempfile::tempdir().unwrap();
    let assets_dir = dir.path().join("assets");
    std::fs::create_dir_all(&assets_dir).unwrap();
    let cache_path = assets_dir.join("manifest-metadata.json");
    std::fs::write(
        &cache_path,
        serde_json::json!({
            "schema": "capsem.manifest_metadata.v1",
            "checked_at": 1000,
            "latest_version": "1.3.1782582155",
            "update_available": false,
            "latest_profiles": "profiles-2030.0101.1",
            "current_profiles": "profiles-2030.0101.0",
            "profiles_update_available": true,
            "profiles_state": "update_available",
            "latest_images": "images-2030.0101.1",
            "images_update_available": false,
            "images_state": "published",
            "checked_url": "https://release.capsem.org/health.json"
        })
        .to_string(),
    )
    .unwrap();

    let status = update_status_response_from_paths("1.3.1782582155", &assets_dir, &cache_path, 1200);

    assert_eq!(status.profiles.current.as_deref(), Some("profiles-2030.0101.0"));
    assert_eq!(status.profiles.latest.as_deref(), Some("profiles-2030.0101.1"));
    assert!(status.profiles.update_available);
    assert_eq!(status.profiles.state, api::UpdateTrackState::UpdateAvailable);
    assert_eq!(status.profiles.compatibility, api::UpdateCompatibilityState::Compatible);
    assert_eq!(status.profiles.blocked_reason, None);
    assert_eq!(status.images.latest.as_deref(), Some("images-2030.0101.1"));
    assert!(!status.images.update_available);
    assert_eq!(status.images.state, api::UpdateTrackState::Current);
}

#[test]
fn update_status_reports_blocked_profile_track_from_release_cache() {
    let dir = tempfile::tempdir().unwrap();
    let assets_dir = dir.path().join("assets");
    std::fs::create_dir_all(&assets_dir).unwrap();
    let cache_path = assets_dir.join("manifest-metadata.json");
    std::fs::write(
        &cache_path,
        serde_json::json!({
            "schema": "capsem.manifest_metadata.v1",
            "checked_at": 1000,
            "latest_version": "1.3.1782582155",
            "update_available": false,
            "latest_profiles": "profiles-2030.0101.1",
            "current_profiles": "profiles-2030.0101.0",
            "profiles_update_available": false,
            "profiles_state": "published",
            "profiles_blocked_reason": "requires binary 1.4.0 or newer",
            "checked_url": "https://release.capsem.org/health.json"
        })
        .to_string(),
    )
    .unwrap();

    let status = update_status_response_from_paths("1.3.1782582155", &assets_dir, &cache_path, 1200);

    assert_eq!(status.profiles.current.as_deref(), Some("profiles-2030.0101.0"));
    assert_eq!(status.profiles.latest.as_deref(), Some("profiles-2030.0101.1"));
    assert!(!status.profiles.update_available);
    assert_eq!(status.profiles.state, api::UpdateTrackState::Unknown);
    assert_eq!(status.profiles.compatibility, api::UpdateCompatibilityState::Unknown);
    assert_eq!(
        status.profiles.blocked_reason.as_deref(),
        Some("requires binary 1.4.0 or newer")
    );
}

#[test]
fn update_status_reports_blocked_asset_track_from_release_cache() {
    let dir = tempfile::tempdir().unwrap();
    let assets_dir = dir.path().join("assets");
    std::fs::create_dir_all(&assets_dir).unwrap();
    std::fs::write(
        assets_dir.join("manifest.json"),
        serde_json::json!({
            "format": 2,
            "refresh_policy": "24h",
            "assets": {
                "current": "2026.0627.1",
                "releases": {}
            },
            "binaries": {
                "current": "1.3.1782582155",
                "releases": {}
            }
        })
        .to_string(),
    )
    .unwrap();
    let cache_path = assets_dir.join("manifest-metadata.json");
    std::fs::write(
        &cache_path,
        serde_json::json!({
            "schema": "capsem.manifest_metadata.v1",
            "checked_at": 1000,
            "latest_version": "1.3.1782582155",
            "update_available": false,
            "latest_assets": "2030.0101.1",
            "assets_update_available": false,
            "assets_state": "published",
            "assets_blocked_reason": "requires binary 99.99.99 or newer",
            "checked_url": "https://release.capsem.org/health.json"
        })
        .to_string(),
    )
    .unwrap();

    let status = update_status_response_from_paths("1.3.1782582155", &assets_dir, &cache_path, 1200);

    assert_eq!(status.assets.current.as_deref(), Some("2026.0627.1"));
    assert_eq!(status.assets.latest.as_deref(), Some("2030.0101.1"));
    assert!(!status.assets.update_available);
    assert_eq!(status.assets.state, api::UpdateTrackState::Unknown);
    assert_eq!(status.assets.compatibility, api::UpdateCompatibilityState::Unknown);
    assert_eq!(
        status.assets.blocked_reason.as_deref(),
        Some("requires binary 99.99.99 or newer")
    );
}

#[test]
fn update_status_reports_unknown_when_cache_is_missing_and_keeps_manifest_channel() {
    let dir = tempfile::tempdir().unwrap();
    let assets_dir = dir.path().join("assets");
    std::fs::create_dir_all(&assets_dir).unwrap();
    std::fs::write(
        assets_dir.join("manifest-metadata.json"),
        serde_json::json!({
            "schema": "capsem.manifest_metadata.v1",
            "origin": "package",
            "manifest_url": "https://corp.example/capsem/assets/internal/manifest.json"
        })
        .to_string(),
    )
    .unwrap();

    let status = update_status_response_from_paths(
        "1.3.1782582155",
        &assets_dir,
        &assets_dir.join("missing-manifest-metadata.json"),
        1200,
    );

    assert_eq!(status.checked_at, None);
    assert!(status.stale);
    assert_eq!(
        status.channel_url.as_deref(),
        Some("https://corp.example/capsem/assets/internal/manifest.json")
    );
    assert_eq!(
        status.supply_chain.channel_index.url.as_deref(),
        Some("https://corp.example/capsem/assets/internal/manifest.json")
    );
    assert_eq!(
        status.supply_chain.manifest.source.as_deref(),
        Some("https://corp.example/capsem/assets/internal/manifest.json")
    );
    assert_eq!(status.last_error, None);
    assert_eq!(status.binary.current.as_deref(), Some("1.3.1782582155"));
    assert_eq!(status.binary.latest, None);
    assert_eq!(status.binary.state, api::UpdateTrackState::Current);
    assert_eq!(status.assets.current, None);
    assert_eq!(status.assets.state, api::UpdateTrackState::Unknown);
}

#[test]
fn update_status_uses_manifest_url_from_metadata_when_check_state_is_missing() {
    let dir = tempfile::tempdir().unwrap();
    let assets_dir = dir.path().join("assets");
    std::fs::create_dir_all(&assets_dir).unwrap();
    std::fs::write(
        assets_dir.join("manifest-metadata.json"),
        serde_json::json!({
            "schema": "capsem.manifest_metadata.v1",
            "origin": "package",
            "manifest_url": "https://updates.corp.example/releases/assets/stable/manifest.json"
        })
        .to_string(),
    )
    .unwrap();

    let status = update_status_response_from_paths(
        "1.3.1782582155",
        &assets_dir,
        &assets_dir.join("missing-manifest-metadata.json"),
        1200,
    );

    assert_eq!(
        status.channel_url.as_deref(),
        Some("https://updates.corp.example/releases/assets/stable/manifest.json")
    );
    assert_eq!(
        status.supply_chain.channel_index.url.as_deref(),
        Some("https://updates.corp.example/releases/assets/stable/manifest.json")
    );
    assert_eq!(
        status.supply_chain.manifest.source.as_deref(),
        Some("https://updates.corp.example/releases/assets/stable/manifest.json")
    );
}

#[test]
fn update_status_uses_only_manifest_metadata_for_provenance_and_check_state() {
    let dir = tempfile::tempdir().unwrap();
    let assets_dir = dir.path().join("assets");
    std::fs::create_dir_all(&assets_dir).unwrap();
    let metadata_path = assets_dir.join("manifest-metadata.json");
    std::fs::write(
        &metadata_path,
        serde_json::json!({
            "schema": "capsem.manifest_metadata.v1",
            "origin": "package",
            "manifest_url": "https://release.capsem.org/assets/stable/manifest.json",
            "installed_at": 900,
            "checked_at": 1100,
            "checked_url": "https://release.capsem.org/assets/stable/manifest.json",
            "latest_version": "1.3.1782600000",
            "update_available": true,
            "latest_profiles": "profiles-2030.0101.1",
            "current_profiles": "profiles-2030.0101.0",
            "profiles_update_available": true,
            "profiles_state": "update_available",
            "channel_hash": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            "validation_status": "valid"
        })
        .to_string(),
    )
    .unwrap();

    let status = update_status_response_from_paths("1.3.1782582155", &assets_dir, &metadata_path, 1200);

    assert_eq!(status.checked_at, Some(1100));
    assert_eq!(
        status.channel_url.as_deref(),
        Some("https://release.capsem.org/assets/stable/manifest.json")
    );
    assert_eq!(status.validation_status.as_deref(), Some("valid"));
    assert_eq!(status.validation_error, None);
    assert_eq!(
        status.channel_hash.as_deref(),
        Some("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb")
    );
    assert_eq!(status.binary.latest.as_deref(), Some("1.3.1782600000"));
    assert_eq!(status.profiles.state, api::UpdateTrackState::UpdateAvailable);
    assert_eq!(
        status.supply_chain.manifest.source.as_deref(),
        Some("https://release.capsem.org/assets/stable/manifest.json")
    );
}

#[test]
fn update_status_reports_cache_parse_errors_without_panicking() {
    let dir = tempfile::tempdir().unwrap();
    let assets_dir = dir.path().join("assets");
    std::fs::create_dir_all(&assets_dir).unwrap();
    let cache_path = assets_dir.join("manifest-metadata.json");
    std::fs::write(&cache_path, "not json").unwrap();

    let status = update_status_response_from_paths("1.3.1782582155", dir.path(), &cache_path, 1200);

    assert!(status.stale);
    assert!(status
        .last_error
        .as_deref()
        .is_some_and(|error| error.contains("parse")));
}

#[test]
fn update_status_reports_cached_channel_validation_errors() {
    let dir = tempfile::tempdir().unwrap();
    let assets_dir = dir.path().join("assets");
    std::fs::create_dir_all(&assets_dir).unwrap();
    let cache_path = assets_dir.join("manifest-metadata.json");
    std::fs::write(
        &cache_path,
        serde_json::json!({
            "schema": "capsem.manifest_metadata.v1",
            "checked_at": 1000,
            "checked_url": "https://release.capsem.org/health.json",
            "channel_hash": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "validation_status": "fetch_error",
            "validation_error": "GET https://release.capsem.org/health.json timed out"
        })
        .to_string(),
    )
    .unwrap();

    let status = update_status_response_from_paths("1.3.1782582155", dir.path(), &cache_path, 1200);

    assert_eq!(
        status.channel_hash.as_deref(),
        Some("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
    );
    assert_eq!(status.validation_status.as_deref(), Some("fetch_error"));
    assert_eq!(
        status.validation_error.as_deref(),
        Some("GET https://release.capsem.org/health.json timed out")
    );
    assert_eq!(
        status.last_error.as_deref(),
        Some("GET https://release.capsem.org/health.json timed out")
    );
}
