use super::*;

// ---- validate_vm_name (moved from main.rs) ----

#[test]
fn validate_vm_name_valid() {
    assert!(validate_vm_name("my-vm").is_ok());
    assert!(validate_vm_name("project_alpha").is_ok());
    assert!(validate_vm_name("vm123").is_ok());
    assert!(validate_vm_name("a").is_ok());
}

#[test]
fn validate_vm_name_empty() {
    assert!(validate_vm_name("").is_err());
}

#[test]
fn validate_vm_name_path_separator() {
    assert!(validate_vm_name("my/vm").is_err());
    assert!(validate_vm_name("../escape").is_err());
}

#[test]
fn validate_vm_name_starts_with_hyphen() {
    assert!(validate_vm_name("-foo").is_err());
}

#[test]
fn validate_vm_name_spaces() {
    assert!(validate_vm_name("my vm").is_err());
}

#[test]
fn validate_vm_name_too_long() {
    let long = "a".repeat(65);
    assert!(validate_vm_name(&long).is_err());
    let max = "a".repeat(64);
    assert!(validate_vm_name(&max).is_ok());
}

// ---- new tests ----

#[test]
fn validate_vm_name_starts_with_underscore() {
    assert!(validate_vm_name("_foo").is_err());
}

#[test]
fn validate_vm_name_starts_with_digit_ok() {
    assert!(validate_vm_name("9lives").is_ok());
}

#[test]
fn validate_vm_name_rejects_non_ascii() {
    // Non-ASCII letters are allowed by `char::is_alphanumeric` but NOT by
    // `is_ascii_alphanumeric`, so the validator should reject them.
    assert!(validate_vm_name("nai\u{00ef}ve").is_err());
    assert!(validate_vm_name("\u{4e2d}").is_err());
}

#[test]
fn validate_vm_name_rejects_dot() {
    assert!(validate_vm_name("my.vm").is_err());
}

#[test]
fn session_naming_takes_the_first_free_counter() {
    assert_eq!(generate_session_name(std::iter::empty::<&str>()), "vm-1");
    assert_eq!(generate_session_name(["vm-1", "VM-2", "code-3"]), "vm-3");
}

#[test]
fn validate_vm_labels_accepts_valid_and_rejects_invalid_keys_values_and_counts() {
    assert!(validate_vm_labels(None).is_ok());
    let mut map = std::collections::HashMap::new();
    map.insert("inspect-capsem-prefix".to_string(), "value".to_string());
    map.insert("role_1-a".to_string(), "value".to_string());
    map.insert("k".repeat(64), "v".repeat(255));
    assert!(validate_vm_labels(Some(&map)).is_ok());
    assert_eq!(non_empty_labels(Some(std::collections::HashMap::new())), None);
    assert_eq!(non_empty_labels(Some(map.clone())), Some(map));

    let mut too_many = std::collections::HashMap::new();
    for i in 0..65 {
        too_many.insert(format!("k{i}"), "v".to_string());
    }
    assert!(validate_vm_labels(Some(&too_many)).is_err());

    for bad_key in [
        "",
        &"k".repeat(65),
        "-leading",
        "_leading",
        "suite.name",
        "a/b",
        "bad key",
        "bad:key",
        "café",
    ] {
        let map = std::collections::HashMap::from([(bad_key.to_string(), "ok".to_string())]);
        assert!(
            validate_vm_labels(Some(&map)).is_err(),
            "expected error for key {bad_key:?}"
        );
    }

    let ctrl_key = std::collections::HashMap::from([("bad\nkey".to_string(), "ok".to_string())]);
    let ctrl_err = validate_vm_labels(Some(&ctrl_key)).unwrap_err().to_string();
    assert!(ctrl_err.contains("\"bad\\nkey\""), "got: {ctrl_err}");
    assert!(
        !ctrl_err.contains('\n'),
        "error must not contain raw newline: {ctrl_err:?}"
    );

    let long_val = std::collections::HashMap::from([("k".to_string(), "v".repeat(256))]);
    assert!(validate_vm_labels(Some(&long_val)).is_err());

    for bad_val in ["bad\nval", "bad\0val", "bad\x7fval", "bad\u{0085}val"] {
        let map = std::collections::HashMap::from([("k".to_string(), bad_val.to_string())]);
        let err = validate_vm_labels(Some(&map))
            .expect_err("control characters in label values must be rejected")
            .to_string();
        assert!(err.contains("control characters"), "got: {err}");
    }
}
