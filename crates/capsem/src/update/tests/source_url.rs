//! Release and corporate source URL parsing.

use super::*;

#[test]
fn release_manifest_url_env_rejects_bare_paths() {
    let err = validate_release_manifest_url("/tmp/release/assets/stable/manifest.json").unwrap_err();
    assert!(
        format!("{err:#}").contains("CAPSEM_RELEASE_MANIFEST_URL must be a URL"),
        "{err:#}"
    );
}

#[test]
fn update_source_url_flags_are_url_only() {
    for flag in ["--manifest", "--corp"] {
        for source in [
            "https://release.capsem.org/assets/stable/manifest.json",
            "http://127.0.0.1:8080/assets/stable/manifest.json",
            "file:///tmp/capsem/assets/stable/manifest.json",
        ] {
            assert_eq!(
                validate_source_url_arg(flag, source),
                Ok(source.to_string()),
                "{flag} should accept {source}"
            );
        }

        for source in [
            "/tmp/capsem/assets/stable/manifest.json",
            "assets/stable/manifest.json",
            "file:assets/stable/manifest.json",
            "file://relative/manifest.json",
            "ssh://updates.example/assets/stable/manifest.json",
            "https:release.capsem.org/assets/stable/manifest.json",
        ] {
            let err = validate_source_url_arg(flag, source).expect_err("source should be rejected");
            assert!(err.contains(flag), "error for {source} should mention {flag}: {err}");
        }
    }
}

#[test]
fn corporate_config_source_requires_https_or_an_absolute_file_url() {
    for source in ["https://corp.example/capsem/corp.toml", "file:///etc/capsem/corp.toml"] {
        assert_eq!(validate_corp_source_url_arg(source), Ok(source.to_string()));
    }
    for source in [
        "http://corp.example/capsem/corp.toml",
        "https://user:secret@corp.example/capsem/corp.toml",
    ] {
        let error = validate_corp_source_url_arg(source).expect_err("unauthenticated corp source must be rejected");
        assert!(error.contains("--corp"), "{error}");
    }
}
