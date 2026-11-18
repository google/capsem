//! Release metadata is publisher authority; digest-verified artifacts are not.
use anyhow::{bail, Context, Result};
use reqwest::{ClientBuilder, Url};

use crate::platform::InstallLayout;

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

#[cfg(test)]
mod tests;
