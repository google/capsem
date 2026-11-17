#[path = "../../build/catalog_limits.rs"]
mod generated;

fn approved() -> toml::Value {
    toml::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/runtime-limits.toml"
    )))
    .unwrap()
}

#[test]
fn catalog_limits_follow_the_canonical_authority() {
    let config = approved();
    let rendered = generated::render(&config);
    for (name, value) in config["catalog"].as_table().unwrap() {
        assert!(rendered.contains(&format!(
            "pub const CATALOG_{}: usize = {};",
            name.to_ascii_uppercase(),
            value.as_integer().unwrap()
        )));
    }
    assert_eq!(crate::CATALOG_MAX_DOCUMENT_BYTES, 1024 * 1024);
    assert_eq!(crate::CATALOG_MAX_IDENTIFIER_BYTES, 64);
    assert_eq!(crate::CATALOG_MAX_SKILL_TOTAL_BYTES, 512 * 1024);
    let mut lower = config;
    lower["catalog"]["max_entries"] = toml::Value::Integer(128);
    assert!(generated::render(&lower).contains("CATALOG_MAX_ENTRIES: usize = 128;"));
}

#[test]
fn catalog_limits_refuse_missing_invalid_and_excessive_values() {
    let config = approved();
    let mut missing = config.clone();
    missing.as_table_mut().unwrap().remove("catalog");
    assert!(std::panic::catch_unwind(|| generated::render(&missing)).is_err());
    for field in config["catalog"].as_table().unwrap().keys() {
        for value in [
            toml::Value::Integer(0),
            toml::Value::Integer(-1),
            toml::Value::Integer(i64::MAX),
            toml::Value::String("128".into()),
        ] {
            let mut invalid = config.clone();
            invalid["catalog"][field] = value;
            assert!(
                std::panic::catch_unwind(|| generated::render(&invalid)).is_err(),
                "accepted invalid {field}"
            );
        }
        let mut missing = config.clone();
        missing["catalog"].as_table_mut().unwrap().remove(field);
        assert!(
            std::panic::catch_unwind(|| generated::render(&missing)).is_err(),
            "accepted missing {field}"
        );
    }
}

#[test]
fn catalog_limits_refuse_inconsistent_complete_skill_budgets() {
    let config = approved();
    let mut invalid = config.clone();
    invalid["catalog"]["max_skill_total_bytes"] = toml::Value::Integer(1024);
    assert!(std::panic::catch_unwind(|| generated::render(&invalid)).is_err());
    let mut invalid = config.clone();
    invalid["catalog"]["max_document_bytes"] = toml::Value::Integer(256 * 1024);
    assert!(std::panic::catch_unwind(|| generated::render(&invalid)).is_err());
    let mut lower = config;
    lower["catalog"]["max_document_bytes"] = toml::Value::Integer(16 * 1024);
    lower["catalog"]["max_skill_file_bytes"] = toml::Value::Integer(4096);
    lower["catalog"]["max_skill_total_bytes"] = toml::Value::Integer(8192);
    generated::render(&lower);
}
