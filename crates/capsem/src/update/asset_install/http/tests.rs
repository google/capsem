use super::*;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

async fn image_server(payload: Vec<u8>) -> (String, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/rootfs.erofs", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        for _ in 0..super::super::RELEASE_HTTP_ATTEMPTS {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = [0u8; 1024];
            let _ = stream.read(&mut request).await;
            let headers = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                payload.len()
            );
            if stream.write_all(headers.as_bytes()).await.is_err() {
                continue;
            }
            let midpoint = payload.len() / 2;
            if stream.write_all(&payload[..midpoint]).await.is_err() {
                continue;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
            let _ = stream.write_all(&payload[midpoint..]).await;
        }
    });
    (url, task)
}

fn download(url: &str, payload: &[u8], size: u64) -> ReleaseChannelAssetDownload {
    ReleaseChannelAssetDownload {
        logical_name: "rootfs.erofs".to_string(),
        url: url.to_string(),
        size,
        blake3: capsem_crypto::blake3::hash(payload).to_hex().to_string(),
        sha256: format!("{:x}", Sha256::digest(payload)),
    }
}

#[tokio::test]
async fn release_channel_runtime_asset_streams_and_verifies_chunks() {
    let payload = b"runtime-image-from-two-chunks";
    let (url, server) = image_server(payload.to_vec()).await;
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("rootfs.erofs");
    download_release_channel_runtime_asset_from_http(&target, &url, &download(&url, payload, payload.len() as u64))
        .await
        .unwrap();
    assert_eq!(std::fs::read(&target).unwrap(), payload);
    assert!(!target.with_extension("tmp").exists());
    server.abort();
}

#[tokio::test]
async fn release_channel_runtime_asset_refuses_bytes_past_manifest_size() {
    let payload = b"too-long";
    let (url, server) = image_server(payload.to_vec()).await;
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("rootfs.erofs");
    let error = download_release_channel_runtime_asset_from_http(&target, &url, &download(&url, payload, 4))
        .await
        .unwrap_err();
    assert!(format!("{error:#}").contains("exceeds manifest size"));
    assert!(!target.exists());
    assert!(!target.with_extension("tmp").exists());
    server.abort();
}
