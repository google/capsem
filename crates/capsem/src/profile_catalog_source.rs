use std::net::IpAddr;
use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, Result};

const MAX_PROFILE_CATALOG_MANIFEST_BYTES: u64 = 2 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ProfileCatalogManifestSource {
    File(PathBuf),
    Url(reqwest::Url),
}

pub(crate) fn profile_catalog_manifest_source(
    manifest: Option<PathBuf>,
    manifest_url: Option<String>,
) -> Result<ProfileCatalogManifestSource> {
    match (manifest, manifest_url) {
        (Some(_), Some(_)) => anyhow::bail!(
            "`capsem profile reconcile-catalog` accepts either --manifest or --manifest-url, not both"
        ),
        (Some(path), None) => Ok(ProfileCatalogManifestSource::File(path)),
        (None, Some(raw_url)) => {
            let url = reqwest::Url::parse(&raw_url)
                .with_context(|| format!("parse profile catalog manifest URL {raw_url}"))?;
            validate_profile_catalog_manifest_url(&url)?;
            Ok(ProfileCatalogManifestSource::Url(url))
        }
        (None, None) => anyhow::bail!(
            "`capsem profile reconcile-catalog` requires --manifest or --manifest-url"
        ),
    }
}

pub(crate) async fn read_profile_catalog_manifest(
    manifest: Option<PathBuf>,
    manifest_url: Option<String>,
) -> Result<String> {
    let source = profile_catalog_manifest_source(manifest, manifest_url)?;
    read_profile_catalog_manifest_from_source(source).await
}

async fn read_profile_catalog_manifest_from_source(
    source: ProfileCatalogManifestSource,
) -> Result<String> {
    match source {
        ProfileCatalogManifestSource::File(path) => std::fs::read_to_string(&path)
            .with_context(|| format!("read profile catalog manifest {}", path.display())),
        ProfileCatalogManifestSource::Url(url) => fetch_profile_catalog_manifest(url).await,
    }
}

fn validate_profile_catalog_manifest_url(url: &reqwest::Url) -> Result<()> {
    match url.scheme() {
        "https" => Ok(()),
        "http" if is_loopback_manifest_host(url.host_str()) => Ok(()),
        scheme => anyhow::bail!(
            "profile catalog manifest URL must use https://; http:// is only allowed for loopback development hosts (got {scheme}://)"
        ),
    }
}

fn is_loopback_manifest_host(host: Option<&str>) -> bool {
    let Some(host) = host else {
        return false;
    };
    if host.eq_ignore_ascii_case("localhost") {
        return true;
    }
    host.parse::<IpAddr>().is_ok_and(|addr| addr.is_loopback())
}

async fn fetch_profile_catalog_manifest(url: reqwest::Url) -> Result<String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .redirect(reqwest::redirect::Policy::limited(3))
        .build()
        .context("build profile catalog manifest HTTP client")?;
    let response = client
        .get(url.clone())
        .header("Accept", "application/json")
        .header("User-Agent", "capsem")
        .send()
        .await
        .with_context(|| format!("fetch profile catalog manifest from {url}"))?;
    let status = response.status();
    if !status.is_success() {
        anyhow::bail!("profile catalog manifest fetch failed with HTTP {status}");
    }
    if let Some(content_length) = response.content_length() {
        if content_length > MAX_PROFILE_CATALOG_MANIFEST_BYTES {
            anyhow::bail!(
                "profile catalog manifest is too large: {content_length} bytes exceeds {MAX_PROFILE_CATALOG_MANIFEST_BYTES} bytes"
            );
        }
    }
    let bytes = response
        .bytes()
        .await
        .context("read profile catalog manifest response body")?;
    if bytes.len() as u64 > MAX_PROFILE_CATALOG_MANIFEST_BYTES {
        anyhow::bail!(
            "profile catalog manifest is too large: {} bytes exceeds {} bytes",
            bytes.len(),
            MAX_PROFILE_CATALOG_MANIFEST_BYTES
        );
    }
    String::from_utf8(bytes.to_vec()).context("profile catalog manifest response is not UTF-8")
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::thread;

    use super::*;

    #[test]
    fn profile_catalog_manifest_source_requires_one_source() {
        let err = profile_catalog_manifest_source(None, None).unwrap_err();
        assert!(err
            .to_string()
            .contains("requires --manifest or --manifest-url"));
    }

    #[test]
    fn profile_catalog_manifest_source_rejects_conflicting_sources() {
        let err = profile_catalog_manifest_source(
            Some(PathBuf::from("manifest.json")),
            Some("https://profiles.example.test/manifest.json".to_string()),
        )
        .unwrap_err();
        assert!(err.to_string().contains("not both"));
    }

    #[test]
    fn profile_catalog_manifest_source_rejects_non_loopback_http() {
        let err = profile_catalog_manifest_source(
            None,
            Some("http://profiles.example.test/manifest.json".to_string()),
        )
        .unwrap_err();
        assert!(err.to_string().contains("must use https://"));
    }

    #[tokio::test]
    async fn read_profile_catalog_manifest_reads_file_source() {
        let temp = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(temp.path(), r#"{"format":1}"#).unwrap();

        let manifest = read_profile_catalog_manifest(Some(temp.path().to_path_buf()), None)
            .await
            .unwrap();

        assert_eq!(manifest, r#"{"format":1}"#);
    }

    #[tokio::test]
    async fn read_profile_catalog_manifest_fetches_loopback_url() {
        let url = spawn_manifest_server(r#"{"format":1,"profiles":[]}"#);

        let manifest = read_profile_catalog_manifest(None, Some(url))
            .await
            .unwrap();

        assert_eq!(manifest, r#"{"format":1,"profiles":[]}"#);
    }

    #[tokio::test]
    async fn read_profile_catalog_manifest_rejects_oversized_fetch() {
        let body = "x".repeat((MAX_PROFILE_CATALOG_MANIFEST_BYTES + 1) as usize);
        let url = spawn_manifest_server(&body);

        let err = read_profile_catalog_manifest(None, Some(url))
            .await
            .unwrap_err();

        assert!(err.to_string().contains("too large"));
    }

    fn spawn_manifest_server(body: &str) -> String {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let addr = listener.local_addr().unwrap();
        let body = body.to_string();
        thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buffer = [0; 1024];
            let _ = stream.read(&mut buffer).unwrap();
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            stream.write_all(response.as_bytes()).unwrap();
        });
        format!("http://{addr}/profile-catalog.json")
    }
}
