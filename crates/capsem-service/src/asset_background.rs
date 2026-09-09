use super::*;

pub(super) fn start_profile_ensure(state: &Arc<ServiceState>, profile: &ProfileConfigFile) -> Result<bool, AppError> {
    if claim_asset_reconcile(state).is_err() {
        return Ok(false);
    }
    if let Err(error) = update_asset_reconcile_state(state, |status| {
        *status = AssetReconcileState {
            in_progress: true,
            ..Default::default()
        };
    }) {
        state.asset_reconcile_inflight.store(false, Ordering::Release);
        return Err(AppError(StatusCode::INTERNAL_SERVER_ERROR, error));
    }

    let state = Arc::clone(state);
    let profile = profile.clone();
    tokio::spawn(async move {
        if let Err(error) = ensure_profile_assets_after_claim(Arc::clone(&state), &profile).await {
            warn!(profile = %profile.id, error = %error, "profile asset reconciliation failed");
        }
        if let Err(AppError(_, error)) = rebuild_profile_status_cache(&state) {
            warn!(profile = %profile.id, error = %error, "failed to refresh profile status after asset reconciliation");
        }
        state.asset_reconcile_inflight.store(false, Ordering::Release);
    });
    Ok(true)
}

pub(super) fn start_startup_ensure(state: Arc<ServiceState>) {
    if let Err(error) = claim_asset_reconcile(&state) {
        warn!(error = %error, "startup asset reconciliation was already running");
        return;
    }
    if let Err(error) = update_asset_reconcile_state(&state, |status| {
        *status = AssetReconcileState {
            in_progress: true,
            ..Default::default()
        };
    }) {
        state.asset_reconcile_inflight.store(false, Ordering::Release);
        warn!(error = %error, "failed to start startup asset reconciliation");
        return;
    }
    tokio::spawn(async move {
        match ensure_assets_after_claim(Arc::clone(&state)).await {
            Ok(downloaded) => info!(downloaded, "startup asset reconciliation finished"),
            Err(error) => warn!(error = %error, "startup asset reconciliation failed"),
        }
        if let Err(AppError(_, error)) = rebuild_profile_status_cache(&state) {
            warn!(error = %error, "failed to refresh profile status after startup asset reconciliation");
        }
        state.asset_reconcile_inflight.store(false, Ordering::Release);
    });
}

pub(super) async fn download_profile_asset<F>(
    asset: &ProfileAssetDescriptor,
    target: &StdPath,
    mut on_progress: F,
) -> Result<()>
where
    F: FnMut(u64, Option<u64>, bool),
{
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    if let Some(parent) = target.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .with_context(|| format!("create {}", parent.display()))?;
    }
    let tmp = target.with_file_name(format!(
        "{}.tmp",
        target.file_name().and_then(|name| name.to_str()).unwrap_or("asset")
    ));
    let _ = tokio::fs::remove_file(&tmp).await;
    let mut output = tokio::fs::File::create(&tmp)
        .await
        .with_context(|| format!("create {}", tmp.display()))?;
    let mut bytes_done = 0u64;
    let expected_hash = profile_asset_hash_hex(asset)?.to_string();
    let total = Some(required_profile_asset_size(asset)?);

    if asset.url.starts_with("file://") {
        let path = reqwest::Url::parse(&asset.url)?
            .to_file_path()
            .map_err(|()| anyhow!("invalid local profile asset URL: {}", asset.url))?;
        let mut input = tokio::fs::File::open(&path)
            .await
            .with_context(|| format!("open profile asset source {}", path.display()))?;
        let mut buf = vec![0u8; 256 * 1024];
        loop {
            let n = input
                .read(&mut buf)
                .await
                .with_context(|| format!("read profile asset source {}", path.display()))?;
            if n == 0 {
                break;
            }
            output
                .write_all(&buf[..n])
                .await
                .with_context(|| format!("write {}", tmp.display()))?;
            bytes_done += n as u64;
            on_progress(bytes_done, total, false);
        }
    } else {
        use futures::StreamExt;
        let client = reqwest::Client::builder()
            .user_agent(concat!("capsem/", env!("CARGO_PKG_VERSION")))
            .build()
            .context("build reqwest client")?;
        let resp = client
            .get(&asset.url)
            .send()
            .await
            .with_context(|| format!("GET {}", asset.url))?;
        if !resp.status().is_success() {
            anyhow::bail!("GET {} returned {}", asset.url, resp.status());
        }
        let total = resp.content_length().or(total);
        let mut stream = resp.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.with_context(|| format!("stream {}", asset.url))?;
            output
                .write_all(&chunk)
                .await
                .with_context(|| format!("write {}", tmp.display()))?;
            bytes_done += chunk.len() as u64;
            on_progress(bytes_done, total, false);
        }
    }

    output
        .flush()
        .await
        .with_context(|| format!("flush {}", tmp.display()))?;
    drop(output);

    let hash_path = tmp.clone();
    let actual = tokio::task::spawn_blocking(move || capsem_assets::asset_manager::hash_file(&hash_path))
        .await
        .context("hash task failed")??;
    if actual != expected_hash {
        let _ = tokio::fs::remove_file(&tmp).await;
        anyhow::bail!(
            "{}: hash mismatch (expected {}, got {})",
            asset.name,
            expected_hash,
            actual
        );
    }
    tokio::fs::rename(&tmp, target)
        .await
        .with_context(|| format!("rename {} -> {}", tmp.display(), target.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = tokio::fs::set_permissions(target, std::fs::Permissions::from_mode(0o444)).await;
    }
    on_progress(bytes_done, total, true);
    Ok(())
}
