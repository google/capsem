use super::*;
use crate::environment::effective_catalog_source;
use capsem_config::catalogs::{CatalogKind, CatalogSourceOrigin, CatalogSources};

fn source(kind: &str, reference: &str) -> String {
    format!("[catalogs.{kind}]\nreference='{reference}'\nschema_version=1\n")
}

#[test]
fn corporate_catalog_sources_follow_real_policy_file_precedence_per_kind() {
    let root = tempfile::tempdir().unwrap();
    let system = root.path().join("system-corp.toml");
    let local = root.path().join("local-corp.toml");
    std::fs::write(&system, source("skills", "registry.example/system/skills:approved")).unwrap();
    std::fs::write(
        &local,
        source("skills", "registry.example/local/skills:approved")
            + &source("rules", "registry.example/local/rules:approved"),
    )
    .unwrap();
    let (_, corp) = policy_files_from_paths(None, &[system.clone(), local.clone()]).unwrap();
    let skills = effective_catalog_source(&CatalogSources::default(), &corp, CatalogKind::Skills)
        .unwrap()
        .unwrap();
    assert_eq!(skills.origin(), CatalogSourceOrigin::Corporate);
    assert_eq!(
        skills.reference().to_string(),
        "registry.example/system/skills:approved"
    );
    let rules = effective_catalog_source(&CatalogSources::default(), &corp, CatalogKind::Rules)
        .unwrap()
        .unwrap();
    assert_eq!(rules.reference().to_string(), "registry.example/local/rules:approved");
    assert!(
        effective_catalog_source(&CatalogSources::default(), &corp, CatalogKind::Mcp)
            .unwrap()
            .is_none()
    );
    for invalid in [
        source("skills", "private-canary"),
        source("skills", "registry.example/catalog:stable").replace("schema_version=1", "schema_version=2"),
        source("mcp", "registry.example/mcp:approved"),
    ] {
        std::fs::write(&system, &invalid).unwrap();
        let error = policy_files_from_paths(None, &[system.clone(), local.clone()]).unwrap_err();
        assert!(!error.contains("canary"));
        assert!(crate::net::policy_config::corp_provision::validate_corp_toml(&invalid).is_err());
    }
}

#[test]
fn corporate_catalog_configuration_accepts_an_exact_mcp_source() {
    let root = tempfile::tempdir().unwrap();
    let system = root.path().join("system-corp.toml");
    let reference = format!("registry.example/capsem/mcp@sha256:{}", "a".repeat(64));
    let content = source("mcp", &reference);
    std::fs::write(&system, &content).unwrap();
    crate::net::policy_config::corp_provision::validate_corp_toml(&content).unwrap();
    let (_, corp) = policy_files_from_paths(None, &[system]).unwrap();
    let selected = effective_catalog_source(&CatalogSources::default(), &corp, CatalogKind::Mcp)
        .unwrap()
        .unwrap();
    assert_eq!(selected.origin(), CatalogSourceOrigin::Corporate);
    assert_eq!(selected.reference().to_string(), reference);
}

#[test]
fn ordinary_settings_cannot_read_or_write_corporate_catalog_overrides() {
    let root = tempfile::tempdir().unwrap();
    let user = root.path().join("settings.toml");
    let content = source("skills", "registry.example/unauthorized/skills:stable");
    std::fs::write(&user, &content).unwrap();
    let error = load_local_settings_file(&user).unwrap_err();
    assert!(error.contains("catalog"));
    let document: SettingsFile = toml::from_str(&content).unwrap();
    std::fs::write(&user, "# existing settings\n").unwrap();
    assert!(write_local_settings_document(&user, &document).is_err());
    assert_eq!(std::fs::read_to_string(&user).unwrap(), "# existing settings\n");
}
