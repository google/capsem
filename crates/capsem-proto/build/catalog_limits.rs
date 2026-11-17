//! Generate publication and read ceilings from the canonical catalog contract.
use std::fmt::Write;

pub fn render(config: &toml::Value) -> String {
    let fields = [
        ("max_document_bytes", 1048576),
        ("max_entries", 4096),
        ("max_identifier_bytes", 64),
        ("max_name_bytes", 128),
        ("max_version_bytes", 128),
        ("max_description_bytes", 4096),
        ("max_tags", 32),
        ("max_tag_bytes", 64),
        ("max_provenance_bytes", 1024),
        ("max_rule_parameters", 64),
        ("max_skill_files", 128),
        ("max_skill_path_bytes", 256),
        ("max_skill_file_bytes", 262144),
        ("max_skill_total_bytes", 524288),
    ];
    let section = config
        .get("catalog")
        .and_then(toml::Value::as_table)
        .expect("runtime limits: catalog table is required");
    let mut generated = String::new();
    for (field, ceiling) in fields {
        let value = section
            .get(field)
            .and_then(toml::Value::as_integer)
            .filter(|value| (1..=ceiling).contains(value))
            .unwrap_or_else(|| panic!("runtime limits: catalog.{field} must be between 1 and {ceiling}"));
        writeln!(
            generated,
            "pub const CATALOG_{}: usize = {value};",
            field.to_ascii_uppercase()
        )
        .unwrap();
    }
    let document = section["max_document_bytes"].as_integer().unwrap();
    for (field, _) in fields {
        if field.ends_with("_bytes") {
            assert!(
                section[field].as_integer().unwrap() <= document,
                "runtime limits: catalog.{field} must fit the document budget"
            );
        }
    }
    assert!(
        section["max_skill_file_bytes"].as_integer().unwrap() <= section["max_skill_total_bytes"].as_integer().unwrap(),
        "runtime limits: a skill file must fit its complete dictionary budget"
    );
    generated
}
