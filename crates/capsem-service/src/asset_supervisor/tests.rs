use std::sync::Arc;
use std::time::Duration;
use std::{collections::HashMap, ffi::OsString};

use capsem_core::asset_manager::{hash_file, hash_filename, DownloadProgress, ManifestV2};
use serde_json::json;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use super::*;

static RELEASE_URL_ENV_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

fn manifest_for(kernel: &[u8], initrd: &[u8], rootfs: &[u8]) -> ManifestV2 {
    let dir = tempfile::tempdir().unwrap();
    let kernel_path = dir.path().join("kernel");
    let initrd_path = dir.path().join("initrd");
    let rootfs_path = dir.path().join("rootfs");
    std::fs::write(&kernel_path, kernel).unwrap();
    std::fs::write(&initrd_path, initrd).unwrap();
    std::fs::write(&rootfs_path, rootfs).unwrap();

    let manifest = json!({
        "format": 2,
        "assets": {
            "current": "2026.0513.1",
            "releases": {
                "2026.0513.1": {
                    "date": "2026-05-13",
                    "deprecated": false,
                    "min_binary": "1.0.0",
                    "arches": {
                        "arm64": {
                            "vmlinuz": { "hash": hash_file(&kernel_path).unwrap(), "size": kernel.len() },
                            "initrd.img": { "hash": hash_file(&initrd_path).unwrap(), "size": initrd.len() },
                            "rootfs.squashfs": { "hash": hash_file(&rootfs_path).unwrap(), "size": rootfs.len() }
                        }
                    }
                }
            }
        },
        "binaries": {
            "current": "1.0.0",
            "releases": {
                "1.0.0": {
                    "date": "2026-05-13",
                    "deprecated": false,
                    "min_assets": "2026.0513.1"
                }
            }
        }
    });
    ManifestV2::from_json(&manifest.to_string()).unwrap()
}

fn supervisor_for(manifest: ManifestV2, assets_dir: &std::path::Path) -> AssetSupervisor {
    supervisor_for_with_interval(manifest, assets_dir, Duration::from_secs(60))
}

fn supervisor_for_with_interval(
    manifest: ManifestV2,
    assets_dir: &std::path::Path,
    check_interval: Duration,
) -> AssetSupervisor {
    AssetSupervisor::new(
        assets_dir.to_path_buf(),
        Some(Arc::new(manifest)),
        "1.0.0".to_string(),
        "arm64".to_string(),
        check_interval,
    )
}

async fn start_asset_server(
    files: HashMap<String, Vec<u8>>,
) -> (String, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let files = Arc::new(files);
    let handle = tokio::spawn(async move {
        loop {
            let Ok((mut stream, _)) = listener.accept().await else {
                break;
            };
            let files = Arc::clone(&files);
            tokio::spawn(async move {
                let mut buf = [0_u8; 2048];
                let n = stream.read(&mut buf).await.unwrap_or(0);
                let request = String::from_utf8_lossy(&buf[..n]);
                let path = request
                    .lines()
                    .next()
                    .and_then(|line| line.split_whitespace().nth(1))
                    .unwrap_or("/")
                    .trim_start_matches('/')
                    .to_string();
                if let Some(body) = files.get(&path) {
                    let header =
                        format!("HTTP/1.1 200 OK\r\ncontent-length: {}\r\n\r\n", body.len());
                    let _ = stream.write_all(header.as_bytes()).await;
                    let _ = stream.write_all(body).await;
                } else {
                    let _ = stream
                        .write_all(b"HTTP/1.1 404 Not Found\r\ncontent-length: 0\r\n\r\n")
                        .await;
                }
            });
        }
    });
    (format!("http://{addr}"), handle)
}

fn set_release_url(url: &str) -> Option<OsString> {
    let old = std::env::var_os("CAPSEM_RELEASE_URL");
    std::env::set_var("CAPSEM_RELEASE_URL", url);
    old
}

fn restore_release_url(old: Option<OsString>) {
    if let Some(old) = old {
        std::env::set_var("CAPSEM_RELEASE_URL", old);
    } else {
        std::env::remove_var("CAPSEM_RELEASE_URL");
    }
}

#[test]
fn local_check_reports_updating_when_required_assets_are_missing() {
    let dir = tempfile::tempdir().unwrap();
    let manifest = manifest_for(b"kernel", b"initrd", b"rootfs");
    let supervisor = supervisor_for(manifest, dir.path());

    supervisor.refresh_local_state();

    let health = supervisor.snapshot();
    assert_eq!(health.state, AssetHealthState::Updating);
    assert!(!health.ready);
    assert_eq!(health.version.as_deref(), Some("2026.0513.1"));
    assert_eq!(health.arch.as_deref(), Some("arm64"));
    assert_eq!(
        health.missing,
        vec!["vmlinuz", "initrd.img", "rootfs.squashfs"]
    );
}

#[test]
fn local_check_reports_ready_when_required_assets_are_present() {
    let dir = tempfile::tempdir().unwrap();
    let manifest = manifest_for(b"kernel", b"initrd", b"rootfs");
    let release = manifest.assets.releases.get("2026.0513.1").unwrap();
    let arch_assets = release.arches.get("arm64").unwrap();
    for (name, bytes) in [
        ("vmlinuz", b"kernel".as_slice()),
        ("initrd.img", b"initrd".as_slice()),
        ("rootfs.squashfs", b"rootfs".as_slice()),
    ] {
        let entry = arch_assets.get(name).unwrap();
        std::fs::write(dir.path().join(hash_filename(name, &entry.hash)), bytes).unwrap();
    }
    let supervisor = supervisor_for(manifest, dir.path());

    supervisor.refresh_local_state();

    let health = supervisor.snapshot();
    assert_eq!(health.state, AssetHealthState::Ready);
    assert!(health.ready);
    assert!(health.missing.is_empty());
    assert!(health.progress.is_none());
    assert!(health.error.is_none());
}

#[test]
fn download_progress_is_visible_in_snapshot() {
    let dir = tempfile::tempdir().unwrap();
    let manifest = manifest_for(b"kernel", b"initrd", b"rootfs");
    let supervisor = supervisor_for(manifest, dir.path());

    supervisor.record_download_progress(DownloadProgress {
        logical_name: "rootfs.squashfs".to_string(),
        bytes_done: 12,
        bytes_total: Some(24),
        done: false,
    });

    let health = supervisor.snapshot();
    assert_eq!(health.state, AssetHealthState::Updating);
    assert!(!health.ready);
    let progress = health.progress.expect("progress should be present");
    assert_eq!(progress.logical_name, "rootfs.squashfs");
    assert_eq!(progress.bytes_done, 12);
    assert_eq!(progress.bytes_total, Some(24));
    assert!(!progress.done);
}

#[test]
fn retryable_download_error_is_reported_as_error_state() {
    let dir = tempfile::tempdir().unwrap();
    let manifest = manifest_for(b"kernel", b"initrd", b"rootfs");
    let supervisor = supervisor_for(manifest, dir.path());

    supervisor.record_error("GET fixture returned 503", true);

    let health = supervisor.snapshot();
    assert_eq!(health.state, AssetHealthState::Error);
    assert!(!health.ready);
    assert!(health.retryable);
    assert_eq!(health.retry_count, 1);
    assert_eq!(health.error.as_deref(), Some("GET fixture returned 503"));
}

#[tokio::test]
async fn ensure_assets_once_downloads_missing_assets_and_reports_ready() {
    let _guard = RELEASE_URL_ENV_LOCK.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let manifest = manifest_for(b"kernel", b"initrd", b"rootfs");
    let release = manifest.assets.releases.get("2026.0513.1").unwrap();
    let arch_assets = release.arches.get("arm64").unwrap().clone();
    let mut files = HashMap::new();
    files.insert("v1.0.0/arm64-vmlinuz".to_string(), b"kernel".to_vec());
    files.insert("v1.0.0/arm64-initrd.img".to_string(), b"initrd".to_vec());
    files.insert(
        "v1.0.0/arm64-rootfs.squashfs".to_string(),
        b"rootfs".to_vec(),
    );
    let (base_url, server) = start_asset_server(files).await;
    let old_url = set_release_url(&base_url);
    let supervisor = supervisor_for(manifest, dir.path());

    supervisor.ensure_assets_once().await;

    restore_release_url(old_url);
    server.abort();
    let health = supervisor.snapshot();
    assert_eq!(health.state, AssetHealthState::Ready);
    assert!(health.ready);
    assert!(health.missing.is_empty());
    for name in ["vmlinuz", "initrd.img", "rootfs.squashfs"] {
        let entry = arch_assets.get(name).unwrap();
        assert!(
            dir.path()
                .join("arm64")
                .join(hash_filename(name, &entry.hash))
                .exists(),
            "{name} should be downloaded"
        );
    }
}

#[tokio::test]
async fn ensure_assets_once_reports_retryable_error_when_release_source_fails() {
    let _guard = RELEASE_URL_ENV_LOCK.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let manifest = manifest_for(b"kernel", b"initrd", b"rootfs");
    let (base_url, server) = start_asset_server(HashMap::new()).await;
    let old_url = set_release_url(&base_url);
    let supervisor = supervisor_for(manifest, dir.path());

    supervisor.ensure_assets_once().await;

    restore_release_url(old_url);
    server.abort();
    let health = supervisor.snapshot();
    assert_eq!(health.state, AssetHealthState::Error);
    assert!(!health.ready);
    assert!(health.retryable);
    assert_eq!(health.retry_count, 1);
    assert!(
        health.error.as_deref().unwrap_or_default().contains("404"),
        "error should preserve release-source failure, got {:?}",
        health.error
    );
}

#[tokio::test]
async fn spawned_background_loop_downloads_missing_assets() {
    let _guard = RELEASE_URL_ENV_LOCK.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let manifest = manifest_for(b"kernel", b"initrd", b"rootfs");
    let mut files = HashMap::new();
    files.insert("v1.0.0/arm64-vmlinuz".to_string(), b"kernel".to_vec());
    files.insert("v1.0.0/arm64-initrd.img".to_string(), b"initrd".to_vec());
    files.insert(
        "v1.0.0/arm64-rootfs.squashfs".to_string(),
        b"rootfs".to_vec(),
    );
    let (base_url, server) = start_asset_server(files).await;
    let old_url = set_release_url(&base_url);
    let supervisor = Arc::new(supervisor_for_with_interval(
        manifest,
        dir.path(),
        Duration::from_millis(10),
    ));

    let supervisor_task = Arc::clone(&supervisor).spawn();
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if supervisor.snapshot().ready {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("background supervisor should make assets ready");

    restore_release_url(old_url);
    supervisor_task.abort();
    server.abort();
    let health = supervisor.snapshot();
    assert_eq!(health.state, AssetHealthState::Ready);
    assert!(health.ready);
}
