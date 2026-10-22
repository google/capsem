//! Bounded, incremental release-image download. The manifest owns size and digests.

use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result};
use capsem_crypto::Sha256;
use tokio::io::AsyncWriteExt;
use tracing::warn;

use super::{
    finish_release_channel_asset_download, release_http_retry_backoff, release_http_status_is_retryable,
    ReleaseChannelAssetDownload,
};

const CHUNK_IDLE_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug)]
struct ManifestSizeExceeded;

impl std::fmt::Display for ManifestSizeExceeded {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "response exceeds manifest size")
    }
}

impl std::error::Error for ManifestSizeExceeded {}

#[derive(Debug)]
struct PermanentHttpStatus(reqwest::StatusCode);

impl std::fmt::Display for PermanentHttpStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "runtime image GET returned {}", self.0)
    }
}

impl std::error::Error for PermanentHttpStatus {}

pub(super) async fn download_release_channel_runtime_asset_from_http(
    target: &Path,
    url: &str,
    download: &ReleaseChannelAssetDownload,
) -> Result<()> {
    let parsed = reqwest::Url::parse(url).with_context(|| format!("parse runtime image URL {url}"))?;
    let client = reqwest::Client::builder()
        .user_agent("capsem")
        .connect_timeout(Duration::from_secs(15))
        .build()
        .context("build release image HTTP client")?;
    let tmp = target.with_extension("tmp");
    let mut last_error = None;

    for attempt in 1..=super::RELEASE_HTTP_ATTEMPTS {
        let _ = std::fs::remove_file(&tmp);
        let result = async {
            let mut response = client
                .get(parsed.clone())
                .send()
                .await
                .with_context(|| format!("GET {url}"))?;
            if !response.status().is_success() {
                if !release_http_status_is_retryable(response.status()) {
                    return Err(PermanentHttpStatus(response.status()).into());
                }
                anyhow::bail!("GET {url} returned {}", response.status());
            }
            if response.content_length().is_some_and(|length| length > download.size) {
                return Err(ManifestSizeExceeded.into());
            }
            let mut file = tokio::fs::File::create(&tmp)
                .await
                .with_context(|| format!("create {}", tmp.display()))?;
            let mut blake3 = capsem_crypto::blake3::Hasher::new();
            let mut sha256 = Sha256::new();
            let mut bytes_done = 0u64;
            loop {
                let chunk = tokio::time::timeout(CHUNK_IDLE_TIMEOUT, response.chunk())
                    .await
                    .context("runtime image download stalled")?
                    .with_context(|| format!("read runtime image from {url}"))?;
                let Some(chunk) = chunk else { break };
                bytes_done = bytes_done
                    .checked_add(chunk.len() as u64)
                    .context("runtime image byte count overflow")?;
                if bytes_done > download.size {
                    return Err(ManifestSizeExceeded.into());
                }
                file.write_all(&chunk)
                    .await
                    .with_context(|| format!("write {}", tmp.display()))?;
                blake3.update(&chunk);
                sha256.update(&chunk);
            }
            file.flush().await.with_context(|| format!("flush {}", tmp.display()))?;
            drop(file);
            finish_release_channel_asset_download(
                target,
                &tmp,
                download,
                bytes_done,
                blake3.finalize().to_hex().to_string(),
                format!("{:x}", sha256.finalize()),
            )
        }
        .await;

        match result {
            Ok(()) => return Ok(()),
            Err(error) => {
                let _ = std::fs::remove_file(&tmp);
                if error.downcast_ref::<ManifestSizeExceeded>().is_some()
                    || error.downcast_ref::<PermanentHttpStatus>().is_some()
                {
                    return Err(error.context(format!("{} runtime image", download.logical_name)));
                }
                if attempt == super::RELEASE_HTTP_ATTEMPTS {
                    return Err(error);
                }
                warn!(attempt, url, error = %error, "runtime image download failed; retrying");
                last_error = Some(error);
                tokio::time::sleep(release_http_retry_backoff(attempt)).await;
            }
        }
    }
    Err(last_error.expect("at least one runtime image download attempt"))
}

#[cfg(test)]
mod tests;
