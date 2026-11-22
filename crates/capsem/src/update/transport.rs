//! Release metadata is publisher authority; digest-verified artifacts are not.
use std::time::Duration;

use anyhow::{bail, Context, Result};
use reqwest::{ClientBuilder, Url};
use tracing::warn;

use crate::platform::InstallLayout;

use super::RELEASE_HTTP_ATTEMPTS;

const RELEASE_HTTP_INITIAL_BACKOFF_MS: u64 = 1_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Trust {
    ManifestAuthority,
    DigestVerifiedArtifact,
}

pub(super) fn validate_authority(url: &Url, layout: &InstallLayout) -> Result<()> {
    validate_mode(url, matches!(layout, InstallLayout::Development))
}

fn validate_mode(url: &Url, development: bool) -> Result<()> {
    if !matches!(url.scheme(), "https" | "http") {
        bail!(
            "release authority URL has unsupported scheme {}: use HTTPS",
            url.scheme()
        );
    }
    if !url.username().is_empty() || url.password().is_some() || !url.has_host() {
        bail!("release authority requires an HTTPS origin without URL credentials");
    }
    let local = url.host_str().is_some_and(|host| {
        host.trim_start_matches('[')
            .trim_end_matches(']')
            .parse::<std::net::IpAddr>()
            .is_ok_and(|ip| ip.is_loopback())
    });
    if url.scheme() == "https" || (development && url.scheme() == "http" && local) {
        return Ok(());
    }
    bail!("release authority requires HTTPS; HTTP is restricted to literal loopback in development builds")
}

pub(super) fn client_builder(url: &Url, trust: Trust) -> Result<ClientBuilder> {
    if trust == Trust::DigestVerifiedArtifact {
        if !matches!(url.scheme(), "https" | "http") {
            bail!(
                "release artifact URL has unsupported scheme {}: use https or http",
                url.scheme()
            );
        }
        return Ok(reqwest::Client::builder());
    }

    let development = matches!(crate::platform::detect_install_layout(), InstallLayout::Development);
    validate_mode(url, development)?;
    // A secure initial source never delegates publisher authority to cleartext,
    // including in development. Local development fixtures cannot redirect to
    // an unauthenticated remote host either.
    let local_http = development && url.scheme() == "http";
    Ok(reqwest::Client::builder()
        .https_only(!local_http)
        .redirect(reqwest::redirect::Policy::custom(move |attempt| {
            if attempt.previous().len() >= 10 {
                return attempt.error("release source redirect limit exceeded");
            }
            match validate_mode(attempt.url(), local_http) {
                Ok(()) => attempt.follow(),
                Err(error) => attempt.error(error.to_string()),
            }
        })))
}

pub(super) fn validate_authority_current(url: &Url) -> Result<()> {
    validate_authority(url, &crate::platform::detect_install_layout()).context("invalid release authority transport")
}

/// Validate user-provided update source flags.
///
/// The CLI intentionally accepts URL strings only. Local files must be written
/// as `file://` so normal release channels and corporate channels share one
/// update/provenance mechanism.
pub fn validate_source_url_arg(flag: &str, value: &str) -> std::result::Result<String, String> {
    let parsed = reqwest::Url::parse(value).map_err(|_| {
        format!("{flag} must be a URL: use https://..., http://..., or file:///absolute/path, got {value}")
    })?;
    match parsed.scheme() {
        "https" | "http" => {
            if !has_scheme_authority_prefix(value, parsed.scheme()) {
                return Err(format!(
                    "{flag} must use https://, http://, or file:// URLs, got {value}"
                ));
            }
            Ok(value.to_string())
        }
        "file" => {
            if !has_scheme_authority_prefix(value, "file") {
                return Err(format!("{flag} file URL must start with file://: {value}"));
            }
            parsed
                .to_file_path()
                .map(|_| value.to_string())
                .map_err(|_| format!("{flag} file URL must be absolute: {value}"))
        }
        scheme => Err(format!(
            "unsupported {flag} URL scheme {scheme}: use https://, http://, or file://"
        )),
    }
}

pub fn validate_manifest_source_url_arg(value: &str) -> std::result::Result<String, String> {
    let value = validate_source_url_arg("--manifest", value)?;
    let parsed = reqwest::Url::parse(&value).expect("source URL was validated");
    if matches!(parsed.scheme(), "https" | "http") {
        validate_authority_current(&parsed).map_err(|error| format!("--manifest: {error:#}"))?;
    }
    Ok(value)
}

pub fn validate_corp_source_url_arg(value: &str) -> std::result::Result<String, String> {
    let value = validate_source_url_arg("--corp", value)?;
    let parsed = reqwest::Url::parse(&value).expect("source URL was validated");
    if parsed.scheme() == "http" {
        return Err("--corp remote authority requires HTTPS; use file:///absolute/path for local config".to_string());
    }
    if parsed.scheme() == "https"
        && (!parsed.username().is_empty() || parsed.password().is_some() || !parsed.has_host())
    {
        return Err("--corp requires an HTTPS origin without URL credentials".to_string());
    }
    Ok(value)
}

pub(super) fn has_scheme_authority_prefix(value: &str, scheme: &str) -> bool {
    let prefix = format!("{scheme}://");
    value
        .get(..prefix.len())
        .is_some_and(|candidate| candidate.eq_ignore_ascii_case(&prefix))
}

pub(super) async fn release_http_get_bytes(
    url: reqwest::Url,
    accept: Option<&'static str>,
    display_url: &str,
    trust: Trust,
) -> Result<Vec<u8>> {
    let client = client_builder(&url, trust)?
        .user_agent("capsem")
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(120))
        .build()
        .context("build release HTTP client")?;

    let mut last_error: Option<anyhow::Error> = None;
    for attempt in 1..=RELEASE_HTTP_ATTEMPTS {
        let mut request = client.get(url.clone());
        if let Some(accept) = accept {
            request = request.header("Accept", accept);
        }

        match request.send().await {
            Ok(response) => {
                let status = response.status();
                if status.is_success() {
                    match response.bytes().await {
                        Ok(bytes) => return Ok(bytes.to_vec()),
                        Err(error) => {
                            let error = anyhow::Error::new(error).context(format!("read {display_url}"));
                            if attempt == RELEASE_HTTP_ATTEMPTS {
                                return Err(error);
                            }
                            warn!(
                                attempt,
                                max_attempts = RELEASE_HTTP_ATTEMPTS,
                                url = %display_url,
                                error = %error,
                                "release HTTP body read failed; retrying"
                            );
                            last_error = Some(error);
                        }
                    }
                } else if release_http_status_is_retryable(status) {
                    let error = anyhow::anyhow!("GET {} returned {}", display_url, status);
                    if attempt == RELEASE_HTTP_ATTEMPTS {
                        return Err(error);
                    }
                    warn!(
                        attempt,
                        max_attempts = RELEASE_HTTP_ATTEMPTS,
                        url = %display_url,
                        status = %status,
                        "release HTTP status is retryable"
                    );
                    last_error = Some(error);
                } else {
                    anyhow::bail!("GET {} returned {}", display_url, status);
                }
            }
            Err(error) => {
                let error = anyhow::Error::new(error).context(format!("GET {display_url}"));
                if attempt == RELEASE_HTTP_ATTEMPTS {
                    return Err(error);
                }
                warn!(
                    attempt,
                    max_attempts = RELEASE_HTTP_ATTEMPTS,
                    url = %display_url,
                    error = %error,
                    "release HTTP request failed; retrying"
                );
                last_error = Some(error);
            }
        }

        tokio::time::sleep(release_http_retry_backoff(attempt)).await;
    }

    Err(last_error.unwrap_or_else(|| anyhow::anyhow!("GET {display_url} failed")))
}

pub(super) fn release_http_status_is_retryable(status: reqwest::StatusCode) -> bool {
    status == reqwest::StatusCode::REQUEST_TIMEOUT
        || status == reqwest::StatusCode::TOO_MANY_REQUESTS
        || status.is_server_error()
}

pub(super) fn release_http_retry_backoff(attempt: usize) -> Duration {
    let multiplier = 1u64 << attempt.saturating_sub(1).min(4);
    Duration::from_millis(RELEASE_HTTP_INITIAL_BACKOFF_MS * multiplier)
}

pub(super) async fn read_manifest_source(source: &str) -> Result<Vec<u8>> {
    let url = reqwest::Url::parse(source).with_context(|| {
        format!("--manifest must be a URL: use https://..., http://..., or file:///absolute/path, got {source}")
    })?;
    match url.scheme() {
        "file" => {
            if !has_scheme_authority_prefix(source, "file") {
                anyhow::bail!("--manifest file URL must start with file://: {source}");
            }
            let path = url
                .to_file_path()
                .map_err(|_| anyhow::anyhow!("--manifest file URL must be absolute: {source}"))?;
            std::fs::read(&path).with_context(|| format!("read manifest {}", path.display()))
        }
        "http" | "https" => {
            if !has_scheme_authority_prefix(source, url.scheme()) {
                anyhow::bail!("--manifest must use https://, http://, or file:// URLs, got {source}");
            }
            release_http_get_bytes(
                url.clone(),
                Some("application/json"),
                source,
                Trust::ManifestAuthority,
            )
            .await
            .with_context(|| format!("read manifest body from {source}"))
        }
        scheme => anyhow::bail!("unsupported --manifest URL scheme {scheme}: use https://, http://, or file://"),
    }
}

#[cfg(test)]
mod tests;
