use super::*;

// -------------------------------------------------------------------
// Boot handshake validation: env key
// -------------------------------------------------------------------

#[test]
fn validate_env_key_accepts_normal_keys() {
    assert!(validate_env_key("HOME").is_ok());
    assert!(validate_env_key("PATH").is_ok());
    assert!(validate_env_key("ANTHROPIC_API_KEY").is_ok());
    assert!(validate_env_key("MY_VAR_123").is_ok());
    assert!(validate_env_key("a").is_ok());
}

#[test]
fn validate_env_key_rejects_empty() {
    assert!(validate_env_key("").is_err());
}

#[test]
fn validate_env_key_rejects_equals() {
    assert!(validate_env_key("FOO=BAR").is_err());
    assert!(validate_env_key("=").is_err());
    assert!(validate_env_key("KEY=").is_err());
}

#[test]
fn validate_env_key_rejects_nul() {
    assert!(validate_env_key("FOO\0BAR").is_err());
    assert!(validate_env_key("\0").is_err());
}

#[test]
fn validate_env_key_rejects_oversized() {
    let long_key = "X".repeat(MAX_ENV_KEY_LEN + 1);
    assert!(validate_env_key(&long_key).is_err());
    // Exactly at limit should pass.
    let ok_key = "X".repeat(MAX_ENV_KEY_LEN);
    assert!(validate_env_key(&ok_key).is_ok());
}

#[test]
fn validate_env_key_rejects_every_blocked_var() {
    for &var in BLOCKED_ENV_VARS {
        assert!(validate_env_key(var).is_err(), "should reject blocked var: {var}");
    }
}

#[test]
fn validate_env_key_rejects_ld_prefix_vars() {
    // LD_ prefix catch-all blocks unknown linker vars.
    assert!(validate_env_key("LD_TRACE_LOADED_OBJECTS").is_err());
    assert!(validate_env_key("LD_WHATEVER").is_err());
}

#[test]
fn validate_env_key_rejects_bash_func_export() {
    assert!(validate_env_key("BASH_FUNC_myfunc%%").is_err());
    assert!(validate_env_key("BASH_FUNC_evil").is_err());
}

#[test]
fn validate_env_key_case_sensitive() {
    // Linux env vars are case-sensitive. Lowercase variants are harmless.
    assert!(validate_env_key("ld_preload").is_ok());
    assert!(validate_env_key("Ld_Preload").is_ok());
    assert!(validate_env_key("ifs").is_ok());
    assert!(validate_env_key("bash_env").is_ok());
}

// -------------------------------------------------------------------
// Boot handshake validation: env value
// -------------------------------------------------------------------

#[test]
fn validate_env_value_accepts_normal() {
    assert!(validate_env_value("hello world").is_ok());
    assert!(validate_env_value("").is_ok()); // empty value is valid
    assert!(validate_env_value("/usr/bin:/usr/local/bin").is_ok());
    assert!(validate_env_value("sk-test-abc123").is_ok());
}

#[test]
fn validate_env_value_rejects_nul() {
    assert!(validate_env_value("foo\0bar").is_err());
    assert!(validate_env_value("\0").is_err());
}

#[test]
fn validate_env_value_rejects_oversized() {
    let long_val = "X".repeat(MAX_ENV_VALUE_LEN + 1);
    assert!(validate_env_value(&long_val).is_err());
    // Exactly at limit should pass.
    let ok_val = "X".repeat(MAX_ENV_VALUE_LEN);
    assert!(validate_env_value(&ok_val).is_ok());
}

// -------------------------------------------------------------------
// Boot handshake validation: file path
// -------------------------------------------------------------------

#[test]
fn validate_file_path_accepts_normal() {
    assert!(validate_file_path("/workspace/test.txt").is_ok());
    assert!(validate_file_path("/etc/ssl/certs/ca-certificates.crt").is_ok());
    assert!(validate_file_path("/root/.bashrc").is_ok());
    assert!(validate_file_path("/workspace/data..backup.txt").is_ok());
    assert!(validate_file_path("report..v2.csv").is_ok());
}

#[test]
fn validate_file_path_rejects_empty() {
    assert!(validate_file_path("").is_err());
}

#[test]
fn validate_file_path_rejects_nul() {
    assert!(validate_file_path("/workspace/\0evil").is_err());
}

#[test]
fn validate_file_path_rejects_traversal() {
    assert!(validate_file_path("/workspace/../etc/passwd").is_err());
    assert!(validate_file_path("../escape").is_err());
    assert!(validate_file_path("/workspace/..").is_err());
    assert!(validate_file_path("..").is_err());
}

// -------------------------------------------------------------------
// is_blocked_env_var
// -------------------------------------------------------------------

#[test]
fn is_blocked_catches_all_listed_vars() {
    assert!(is_blocked_env_var("LD_PRELOAD"));
    assert!(is_blocked_env_var("LD_LIBRARY_PATH"));
    assert!(is_blocked_env_var("LD_AUDIT"));
    assert!(is_blocked_env_var("IFS"));
    assert!(is_blocked_env_var("BASH_ENV"));
    assert!(is_blocked_env_var("ENV"));
    assert!(is_blocked_env_var("CDPATH"));
    assert!(is_blocked_env_var("GLOBIGNORE"));
    assert!(is_blocked_env_var("SHELLOPTS"));
    assert!(is_blocked_env_var("BASHOPTS"));
    assert!(is_blocked_env_var("PROMPT_COMMAND"));
    assert!(is_blocked_env_var("PS4"));
}

#[test]
fn is_blocked_allows_safe_vars() {
    assert!(!is_blocked_env_var("HOME"));
    assert!(!is_blocked_env_var("PATH"));
    assert!(!is_blocked_env_var("TERM"));
    assert!(!is_blocked_env_var("EDITOR"));
    assert!(!is_blocked_env_var("ANTHROPIC_API_KEY"));
}

#[test]
fn is_blocked_case_sensitive() {
    assert!(!is_blocked_env_var("ld_preload"));
    assert!(!is_blocked_env_var("Ld_Preload"));
    assert!(!is_blocked_env_var("ifs"));
}
