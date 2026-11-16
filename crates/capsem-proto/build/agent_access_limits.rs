use std::fmt::Write;

pub fn render(config: &toml::Value) -> String {
    let mut generated = String::new();
    for (field, constant, ceiling) in [
        ("max_environment_ids", "AGENT_ACCESS_MAX_ENVIRONMENT_IDS", 1024),
        ("max_session_bindings", "AGENT_ACCESS_MAX_SESSION_BINDINGS", 1024),
        ("max_active", "AGENT_ACCESS_MAX_ACTIVE", 16384),
        ("max_issued_per_epoch", "AGENT_ACCESS_MAX_ISSUED_PER_EPOCH", 65536),
        ("max_lease_seconds", "AGENT_ACCESS_MAX_LEASE_SECONDS", 86400),
    ] {
        let value = config
            .get("agent_access")
            .and_then(|section| section.get(field))
            .and_then(toml::Value::as_integer)
            .filter(|value| (1..=ceiling).contains(value))
            .unwrap_or_else(|| panic!("runtime limits: agent_access.{field} must be between 1 and {ceiling}"));
        writeln!(generated, "pub const {constant}: usize = {value};").unwrap();
    }
    generated
}
