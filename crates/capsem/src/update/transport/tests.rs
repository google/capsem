use super::*;

#[test]
fn remote_http_never_carries_manifest_authority() {
    for source in [
        "http://updates.example/assets/stable/manifest.json",
        "HTTP://updates.example/assets/stable/manifest.json",
        "http://127.0.0.1.evil.example/assets/stable/manifest.json",
        "http://[::ffff:192.0.2.1]/assets/stable/manifest.json",
    ] {
        let url = Url::parse(source).unwrap();
        for layout in [
            InstallLayout::Development,
            InstallLayout::LinuxDeb,
            InstallLayout::MacosPkg,
            InstallLayout::UserDir,
        ] {
            assert!(
                validate_authority(&url, &layout).is_err(),
                "accepted {source} for {layout:?}"
            );
        }
    }
}

#[test]
fn only_development_literal_loopback_may_use_http_authority() {
    for source in ["http://127.0.0.1/manifest.json", "http://[::1]/manifest.json"] {
        let url = Url::parse(source).unwrap();
        assert!(validate_authority(&url, &InstallLayout::Development).is_ok());
        for layout in [InstallLayout::LinuxDeb, InstallLayout::MacosPkg, InstallLayout::UserDir] {
            assert!(validate_authority(&url, &layout).is_err());
        }
    }
    for source in [
        "http://localhost/manifest.json",
        "https://user:password@updates.example/manifest.json",
    ] {
        assert!(validate_authority(&Url::parse(source).unwrap(), &InstallLayout::Development).is_err());
    }
}

#[test]
fn manifest_entrypoints_reject_remote_http_but_preserve_file_diagnostics() {
    let remote = "http://updates.example/assets/stable/manifest.json";
    assert!(super::super::validate_manifest_source_url_arg(remote).is_err());
    assert!(super::super::validate_release_manifest_url(remote).is_err());
    assert!(super::super::channel_manifest_url(remote).is_err());
    assert!(super::super::validate_manifest_source_url_arg("file:///tmp/manifest.json").is_ok());

    let error = super::super::channel_manifest_url("file:///tmp/assets/stable/manifest.json").unwrap_err();
    assert!(error.to_string().contains("file"), "{error:#}");
}

#[tokio::test]
async fn authority_client_refuses_a_cleartext_request_before_network() {
    let secure = Url::parse("https://release.capsem.org/assets/stable/manifest.json").unwrap();
    let client = client_builder(&secure, Trust::ManifestAuthority)
        .unwrap()
        .build()
        .unwrap();
    let error = client.get("http://192.0.2.1/manifest.json").send().await.unwrap_err();
    assert!(
        error.is_builder(),
        "HTTPS-only client must refuse before connecting: {error}"
    );
}

#[test]
fn digest_verified_artifacts_do_not_gain_authority_from_transport() {
    let artifact = Url::parse("http://artifacts.example/Capsem.pkg").unwrap();
    let _builder = client_builder(&artifact, Trust::DigestVerifiedArtifact)
        .expect("artifact bytes are independently bound by the trusted manifest");
}
