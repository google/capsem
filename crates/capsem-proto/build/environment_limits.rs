use std::fmt::Write;

pub fn render(config: &toml::Value) -> String {
    let fields = [
        ("max_name_bytes", 64),
        ("max_description_bytes", 4096),
        ("max_startup_script_bytes", 65536),
        ("max_variables", 128),
        ("max_variable_name_bytes", 128),
        ("max_variable_value_bytes", 16384),
        ("max_variable_total_bytes", 262144),
        ("max_repositories", 64),
        ("max_spec_bytes", 1048576),
    ];
    let section = config
        .get("environment")
        .and_then(toml::Value::as_table)
        .expect("runtime limits: environment table is required");
    let mut generated = String::new();
    for (field, ceiling) in fields {
        let value = section
            .get(field)
            .and_then(toml::Value::as_integer)
            .filter(|value| (1..=ceiling).contains(value))
            .unwrap_or_else(|| panic!("runtime limits: environment.{field} must be between 1 and {ceiling}"));
        writeln!(
            generated,
            "pub const ENVIRONMENT_{}: usize = {value};",
            field.to_ascii_uppercase()
        )
        .unwrap();
    }
    let value_bytes = section["max_variable_value_bytes"].as_integer().unwrap();
    let total_bytes = section["max_variable_total_bytes"].as_integer().unwrap();
    assert!(
        value_bytes <= total_bytes,
        "runtime limits: an environment variable must fit its aggregate budget"
    );
    let spec_bytes = section["max_spec_bytes"].as_integer().unwrap();
    for field in [
        "max_name_bytes",
        "max_description_bytes",
        "max_startup_script_bytes",
        "max_variable_total_bytes",
    ] {
        assert!(
            section[field].as_integer().unwrap() <= spec_bytes,
            "runtime limits: environment.{field} must fit the specification budget"
        );
    }
    generated
}
