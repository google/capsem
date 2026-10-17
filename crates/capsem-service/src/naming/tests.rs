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
