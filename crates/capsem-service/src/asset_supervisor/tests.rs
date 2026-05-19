use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use capsem_core::asset_manager::{hash_file, hash_filename, DownloadProgress};
use capsem_core::settings_profiles::{VmArchAssets, VmAssetDeclaration};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use super::*;

fn profile_assets_for(
    kernel: &[u8],
    initrd: &[u8],
    rootfs: &[u8],
    base_url: &str,
) -> ProfileAssetRequirement {
    let dir = tempfile::tempdir().unwrap();
    let kernel_path = dir.path().join("kernel");
    let initrd_path = dir.path().join("initrd");
    let rootfs_path = dir.path().join("rootfs");
    std::fs::write(&kernel_path, kernel).unwrap();
    std::fs::write(&initrd_path, initrd).unwrap();
    std::fs::write(&rootfs_path, rootfs).unwrap();

    let asset = |name: &str, path: &std::path::Path, size: usize| VmAssetDeclaration {
        url: format!("{base_url}/{name}"),
        hash: format!("blake3:{}", hash_file(path).unwrap()),
        signature_url: format!("{base_url}/{name}.minisig"),
        size: size as u64,
        content_type: "application/octet-stream".to_string(),
    };

    ProfileAssetRequirement {
        profile_id: "everyday-work".to_string(),
        revision: Some("2026.0513.1".to_string()),
        arch: "arm64".to_string(),
        assets: VmArchAssets {
            kernel: asset("vmlinuz", &kernel_path, kernel.len()),
            initrd: asset("initrd.img", &initrd_path, initrd.len()),
            rootfs: asset("rootfs.squashfs", &rootfs_path, rootfs.len()),
        },
    }
}

fn supervisor_for(
    required: ProfileAssetRequirement,
    assets_dir: &std::path::Path,
) -> AssetSupervisor {
    supervisor_for_with_interval(required, assets_dir, Duration::from_secs(60))
}

fn supervisor_for_with_interval(
    required: ProfileAssetRequirement,
    assets_dir: &std::path::Path,
    check_interval: Duration,
) -> AssetSupervisor {
    AssetSupervisor::new(
        assets_dir.to_path_buf(),
        AssetRequirement::Profile(required),
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

#[test]
fn local_check_reports_updating_when_required_assets_are_missing() {
    let dir = tempfile::tempdir().unwrap();
    let required = profile_assets_for(
        b"kernel",
        b"initrd",
        b"rootfs",
        "https://assets.example.test",
    );
    let supervisor = supervisor_for(required, dir.path());

    supervisor.refresh_local_state();

    let health = supervisor.snapshot();
    assert_eq!(health.state, AssetHealthState::Updating);
    assert!(!health.ready);
    assert_eq!(health.version.as_deref(), Some("everyday-work@2026.0513.1"));
    assert_eq!(health.arch.as_deref(), Some("arm64"));
    assert_eq!(
        health.missing,
        vec!["vmlinuz", "initrd.img", "rootfs.squashfs"]
    );
}

#[test]
fn local_check_reports_ready_when_required_assets_are_present() {
    let dir = tempfile::tempdir().unwrap();
    let required = profile_assets_for(
        b"kernel",
        b"initrd",
        b"rootfs",
        "https://assets.example.test",
    );
    for (name, bytes, asset) in [
        ("vmlinuz", b"kernel".as_slice(), &required.assets.kernel),
        ("initrd.img", b"initrd".as_slice(), &required.assets.initrd),
        (
            "rootfs.squashfs",
            b"rootfs".as_slice(),
            &required.assets.rootfs,
        ),
    ] {
        let hash = profile_asset_hash_hex(asset);
        std::fs::write(dir.path().join(hash_filename(name, hash)), bytes).unwrap();
    }
    let supervisor = supervisor_for(required, dir.path());

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
    let required = profile_assets_for(
        b"kernel",
        b"initrd",
        b"rootfs",
        "https://assets.example.test",
    );
    let supervisor = supervisor_for(required, dir.path());

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
    let required = profile_assets_for(
        b"kernel",
        b"initrd",
        b"rootfs",
        "https://assets.example.test",
    );
    let supervisor = supervisor_for(required, dir.path());

    supervisor.record_error("GET fixture returned 503", true);

    let health = supervisor.snapshot();
    assert_eq!(health.state, AssetHealthState::Error);
    assert!(!health.ready);
    assert!(health.retryable);
    assert_eq!(health.retry_count, 1);
    assert_eq!(health.error.as_deref(), Some("GET fixture returned 503"));
}

#[test]
fn log_url_redaction_strips_query_and_credentials() {
    assert_eq!(
        redacted_url_for_log(
            "https://token:secret@assets.example.test/path/rootfs.squashfs?sig=secret"
        ),
        "https://assets.example.test/path/rootfs.squashfs"
    );
}

#[tokio::test]
async fn ensure_assets_once_downloads_missing_assets_and_reports_ready() {
    let dir = tempfile::tempdir().unwrap();
    let mut files = HashMap::new();
    files.insert("vmlinuz".to_string(), b"kernel".to_vec());
    files.insert("initrd.img".to_string(), b"initrd".to_vec());
    files.insert("rootfs.squashfs".to_string(), b"rootfs".to_vec());
    let (base_url, server) = start_asset_server(files).await;
    let required = profile_assets_for(b"kernel", b"initrd", b"rootfs", &base_url);
    let expected_assets = required.assets.clone();
    let supervisor = supervisor_for(required, dir.path());

    supervisor.ensure_assets_once().await;

    server.abort();
    let health = supervisor.snapshot();
    assert_eq!(health.state, AssetHealthState::Ready);
    assert!(health.ready);
    assert!(health.missing.is_empty());
    for (name, asset) in [
        ("vmlinuz", &expected_assets.kernel),
        ("initrd.img", &expected_assets.initrd),
        ("rootfs.squashfs", &expected_assets.rootfs),
    ] {
        assert!(
            dir.path()
                .join("arm64")
                .join(hash_filename(name, profile_asset_hash_hex(asset)))
                .exists(),
            "{name} should be downloaded"
        );
    }
}

#[tokio::test]
async fn ensure_assets_once_reports_retryable_error_when_release_source_fails() {
    let dir = tempfile::tempdir().unwrap();
    let (base_url, server) = start_asset_server(HashMap::new()).await;
    let required = profile_assets_for(b"kernel", b"initrd", b"rootfs", &base_url);
    let supervisor = supervisor_for(required, dir.path());

    supervisor.ensure_assets_once().await;

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
    let dir = tempfile::tempdir().unwrap();
    let mut files = HashMap::new();
    files.insert("vmlinuz".to_string(), b"kernel".to_vec());
    files.insert("initrd.img".to_string(), b"initrd".to_vec());
    files.insert("rootfs.squashfs".to_string(), b"rootfs".to_vec());
    let (base_url, server) = start_asset_server(files).await;
    let required = profile_assets_for(b"kernel", b"initrd", b"rootfs", &base_url);
    let supervisor = Arc::new(supervisor_for_with_interval(
        required,
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

    supervisor_task.abort();
    server.abort();
    let health = supervisor.snapshot();
    assert_eq!(health.state, AssetHealthState::Ready);
    assert!(health.ready);
}
