#[path = "../../build/environment_limits.rs"]
mod generated;

fn approved() -> toml::Value {
    toml::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/runtime-limits.toml"
    )))
    .unwrap()
}

#[test]
fn environment_limits_are_generated_from_the_canonical_authority() {
    let rendered = generated::render(&approved());
    for (name, value) in [
        ("NAME_BYTES", 64),
        ("DESCRIPTION_BYTES", 4096),
        ("STARTUP_SCRIPT_BYTES", 65536),
        ("VARIABLES", 128),
        ("VARIABLE_NAME_BYTES", 128),
        ("VARIABLE_VALUE_BYTES", 16384),
        ("VARIABLE_TOTAL_BYTES", 262144),
        ("REPOSITORIES", 64),
        ("SPEC_BYTES", 1048576),
    ] {
        assert!(
            rendered.contains(&format!("pub const ENVIRONMENT_MAX_{name}: usize = {value};")),
            "missing {name}"
        );
    }
}

#[test]
fn invalid_environment_limits_fail_before_publication() {
    let config = approved();
    for field in [
        "max_name_bytes",
        "max_description_bytes",
        "max_startup_script_bytes",
        "max_variables",
        "max_variable_name_bytes",
        "max_variable_value_bytes",
        "max_variable_total_bytes",
        "max_repositories",
        "max_spec_bytes",
    ] {
        for value in [
            toml::Value::Integer(0),
            toml::Value::Integer(-1),
            toml::Value::Integer(i64::MAX),
            toml::Value::String("64".into()),
        ] {
            let mut invalid = config.clone();
            invalid["environment"][field] = value;
            assert!(
                std::panic::catch_unwind(|| generated::render(&invalid)).is_err(),
                "accepted invalid {field}"
            );
        }
        let mut missing = config.clone();
        missing["environment"].as_table_mut().unwrap().remove(field);
        assert!(
            std::panic::catch_unwind(|| generated::render(&missing)).is_err(),
            "accepted missing {field}"
        );
    }
    let mut invalid = config;
    invalid["environment"]["max_variable_total_bytes"] = toml::Value::Integer(8192);
    assert!(std::panic::catch_unwind(|| generated::render(&invalid)).is_err());
}

#[test]
fn compiled_environment_limits_match_guest_and_persisted_contracts() {
    assert_eq!(crate::ENVIRONMENT_MAX_VARIABLES, crate::MAX_BOOT_ENV_VARS);
    assert_eq!(crate::ENVIRONMENT_MAX_NAME_BYTES, 64);
    assert_eq!(crate::ENVIRONMENT_MAX_DESCRIPTION_BYTES, 4096);
    assert_eq!(crate::ENVIRONMENT_MAX_STARTUP_SCRIPT_BYTES, 65536);
    assert_eq!(crate::ENVIRONMENT_MAX_VARIABLE_NAME_BYTES, 128);
    assert_eq!(crate::ENVIRONMENT_MAX_VARIABLE_VALUE_BYTES, 16384);
    assert_eq!(crate::ENVIRONMENT_MAX_VARIABLE_TOTAL_BYTES, 262144);
    assert_eq!(crate::ENVIRONMENT_MAX_REPOSITORIES, 64);
    assert_eq!(crate::ENVIRONMENT_MAX_SPEC_BYTES, 1048576);
}

#[test]
fn boot_cap_constants() {
    assert_eq!(crate::MAX_BOOT_ENV_VARS, 128);
    assert_eq!(crate::MAX_BOOT_FILES, 64);
    assert_eq!(crate::MAX_BOOT_FILE_BYTES, 10_485_760);
    assert_eq!(crate::MAX_ENV_KEY_LEN, 256);
    assert_eq!(crate::MAX_ENV_VALUE_LEN, 131_072);
}
