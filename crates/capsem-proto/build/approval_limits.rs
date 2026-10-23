use std::fmt::Write;

pub fn render(config: &toml::Value) -> String {
    let mut generated = String::new();
    for (field, constant, ceiling) in [
        ("timeout_seconds", "APPROVAL_TIMEOUT_SECONDS", 300),
        ("pending_per_session", "APPROVAL_PENDING_PER_SESSION", 64),
        ("pending_total", "APPROVAL_PENDING_TOTAL", 1024),
        ("context_bytes", "APPROVAL_CONTEXT_BYTES", 16384),
        ("held_bytes_per_request", "APPROVAL_HELD_BYTES_PER_REQUEST", 4194304),
        ("held_bytes_total", "APPROVAL_HELD_BYTES_TOTAL", 67108864),
    ] {
        let value = config
            .get("approval")
            .and_then(|section| section.get(field))
            .and_then(toml::Value::as_integer)
            .filter(|value| (1..=ceiling).contains(value))
            .unwrap_or_else(|| panic!("runtime limits: approval.{field} must be between 1 and {ceiling}"));
        writeln!(generated, "pub const {constant}: usize = {value};").unwrap();
    }
    generated
}
