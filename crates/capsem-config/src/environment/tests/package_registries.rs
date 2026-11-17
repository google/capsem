use super::*;

#[test]
fn environment_package_registry_selections_have_an_exact_shared_count_bound() {
    let mut doc = document();
    let mut draft: EnvironmentDraft = toml::from_str("name='development'").unwrap();
    let registries: Vec<_> = (0..capsem_proto::ENVIRONMENT_MAX_PACKAGE_REGISTRIES)
        .map(|index| format!("registry-{index}"))
        .collect();
    doc.specification.packages.registries = registries.clone();
    draft.packages.registries = registries;
    doc.to_toml().unwrap();
    draft.validate().unwrap();
    doc.specification.packages.registries.push("extra".into());
    draft.packages.registries.push("extra".into());
    assert!(doc.to_toml().is_err());
    assert!(draft.validate().is_err());
    for registries in [vec!["npm".into(), "npm".into()], vec!["../untrusted".into()]] {
        doc.specification.packages.registries = registries.clone();
        draft.packages.registries = registries;
        assert!(doc.to_toml().is_err());
        assert!(draft.validate().is_err());
    }
}
