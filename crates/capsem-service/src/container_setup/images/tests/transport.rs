//! Registry trust follows the configured authority without granting admission.
use super::*;
use crate::container_setup::registry::{read_catalog_ca, MAX_CA_BYTES};

#[tokio::test]
async fn catalog_ca_reaches_the_same_registry_published_filesystem() {
    let directory = tempfile::tempdir().unwrap();
    let ca = directory.path().join("registry.pem");
    std::fs::write(&ca, "configured-registry-ca").unwrap();
    let fx = Fixture::new(ImagePolicyConfig {
        catalog_ca: Some(ca.display().to_string()),
        ..Default::default()
    });
    fx.catalog.lock().unwrap().as_mut().unwrap()["entries"]["codex-cli"]["versions"][1]["erofs"] =
        json!({host(): digest('8')});
    let (status, body) = fx.pull("codex-cli").await;
    assert_eq!(status, StatusCode::BAD_GATEWAY, "{body}");
    let roots = fx.roots.lock().unwrap();
    assert_eq!(roots.len(), 1);
    assert_eq!(roots[0].2.ca_pem.as_deref(), Some("configured-registry-ca"));
    drop(roots);
}

fn policy(authority: &str, ca: &std::path::Path) -> ImagePolicyConfig {
    ImagePolicyConfig {
        sources: vec![format!("{authority}/app")],
        catalog: Some(CatalogSetting::Reference(format!("{authority}/catalog:stable"))),
        catalog_ca: Some(ca.display().to_string()),
        ..Default::default()
    }
}

#[tokio::test]
async fn a_catalog_ca_is_not_read_or_forwarded_to_another_host_or_port() {
    let directory = tempfile::tempdir().unwrap();
    for authority in ["other.example:5443", "registry.example:5444", "registry.example"] {
        let mut config = policy("registry.example:5443", &directory.path().join("absent.pem"));
        config.sources.push(format!("{authority}/app"));
        let fx = Fixture::new(config);
        let mut decision = Decision::new(&fx.state).await.unwrap();
        let image = requested(&format!("{authority}/app:latest")).unwrap();
        let access = decision
            .registry_access(&image, RegistryAccess::default())
            .await
            .unwrap();
        assert_eq!(access, RegistryAccess::default());
        assert_eq!(fx.reads(), 0, "explicit source admission needs no catalog");
    }
}

#[tokio::test]
async fn explicit_ca_and_credentials_win_without_reading_the_configured_file() {
    let directory = tempfile::tempdir().unwrap();
    let fx = Fixture::new(policy("registry.example:5443", &directory.path().join("absent.pem")));
    let mut decision = Decision::new(&fx.state).await.unwrap();
    let image = requested("registry.example:5443/app:latest").unwrap();
    let access = RegistryAccess {
        username: Some("account".into()),
        password: Some("private-password".into()),
        ca_pem: Some("explicit-ca".into()),
    };
    assert_eq!(decision.registry_access(&image, access.clone()).await.unwrap(), access);
    assert_eq!(fx.reads(), 0);
}

#[tokio::test]
async fn matching_port_inherits_ca_and_preserves_request_credentials() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("ca.pem");
    std::fs::write(&path, "operator-ca").unwrap();
    let fx = Fixture::new(policy("registry.example:5443", &path));
    let mut decision = Decision::new(&fx.state).await.unwrap();
    let image = requested("registry.example:5443/app:latest").unwrap();
    let access = RegistryAccess {
        username: Some("account".into()),
        password: Some("private-password".into()),
        ca_pem: None,
    };
    let expected = RegistryAccess {
        ca_pem: Some("operator-ca".into()),
        ..access.clone()
    };
    assert_eq!(decision.registry_access(&image, access).await.unwrap(), expected);
}

#[tokio::test]
async fn source_denial_precedes_ca_file_access_even_with_explicit_trust() {
    let directory = tempfile::tempdir().unwrap();
    let mut config = policy("registry.example:5443", &directory.path().join("absent.pem"));
    config.sources.clear();
    let fx = Fixture::new(config);
    *fx.catalog.lock().unwrap() = None;
    let mut decision = Decision::new(&fx.state).await.unwrap();
    let image = requested("registry.example:5443/app:latest").unwrap();
    for ca_pem in [None, Some("explicit-ca".into())] {
        let access = RegistryAccess {
            ca_pem,
            ..Default::default()
        };
        assert!(matches!(
            decision.registry_access(&image, access).await,
            Err(ImageError::Refused(_))
        ));
    }
    assert!(fx.pulls().is_empty());
}

#[tokio::test]
async fn corporate_catalog_trust_replaces_user_catalog_trust() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("ca.pem");
    std::fs::write(&path, "corporate-ca").unwrap();
    let user = SettingsFile {
        images: Some(policy("user.example", &directory.path().join("absent.pem"))),
        ..Default::default()
    };
    let corp = SettingsFile {
        images: Some(policy("corp.example", &path)),
        ..Default::default()
    };
    let fx = Fixture::default_policy();
    let mut decision = Decision::new(&fx.state).await.unwrap();
    decision.policy = ImagePolicy::from_files(&user, &corp).unwrap();
    let image = requested("corp.example/app:latest").unwrap();
    let access = decision
        .registry_access(&image, RegistryAccess::default())
        .await
        .unwrap();
    assert_eq!(access.ca_pem.as_deref(), Some("corporate-ca"));
}

#[tokio::test]
async fn ca_reader_supports_configured_symlinks_and_enforces_exact_size_bound() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("ca.pem");
    let link = directory.path().join("linked.pem");
    std::os::unix::fs::symlink(&path, &link).unwrap();
    let source = CatalogSource {
        reference: "registry.example/catalog:stable".into(),
        ca: Some(link),
    };
    std::fs::write(&path, vec![b'a'; MAX_CA_BYTES]).unwrap();
    assert_eq!(read_catalog_ca(&source).await.unwrap().unwrap().len(), MAX_CA_BYTES);
    std::fs::write(&path, vec![b'a'; MAX_CA_BYTES + 1]).unwrap();
    assert!(read_catalog_ca(&source)
        .await
        .unwrap_err()
        .to_string()
        .contains("exceeds"));
    std::fs::write(&path, [0xff]).unwrap();
    assert!(read_catalog_ca(&source)
        .await
        .unwrap_err()
        .to_string()
        .contains("UTF-8"));
}

#[tokio::test]
async fn ca_reader_rejects_special_missing_and_directory_inputs() {
    let directory = tempfile::tempdir().unwrap();
    for path in [
        std::path::PathBuf::from("/dev/null"),
        directory.path().to_owned(),
        directory.path().join("missing"),
    ] {
        let source = CatalogSource {
            reference: "registry.example/catalog:stable".into(),
            ca: Some(path),
        };
        assert!(read_catalog_ca(&source).await.is_err());
    }
}
