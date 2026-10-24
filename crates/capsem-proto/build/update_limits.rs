use std::fmt::Write;

pub fn render(config: &toml::Value) -> String {
    let mut generated = String::new();
    for (field, constant, ceiling) in [
        ("check_timeout_seconds", "UPDATE_CHECK_TIMEOUT_SECONDS", 600),
        ("output_bytes_per_stream", "UPDATE_OUTPUT_BYTES_PER_STREAM", 1048576),
    ] {
        let value = config
            .get("update")
            .and_then(|section| section.get(field))
            .and_then(toml::Value::as_integer)
            .filter(|value| (1..=ceiling).contains(value))
            .unwrap_or_else(|| panic!("runtime limits: update.{field} must be between 1 and {ceiling}"));
        writeln!(generated, "pub const {constant}: usize = {value};").unwrap();
    }
    generated
}
