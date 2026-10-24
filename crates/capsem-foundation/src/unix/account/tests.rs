use super::*;

#[test]
fn real_os_account_metadata_matches_uid_record() {
    let actual = nix::unistd::User::from_uid(nix::unistd::getuid()).unwrap().unwrap();
    let resolved = by_name(&actual.name).unwrap().unwrap();
    assert_eq!(resolved.uid, actual.uid.as_raw());
    assert_eq!(resolved.home, actual.dir);
    assert!(resolved.home.is_absolute());
}

#[test]
fn invalid_and_missing_names_are_not_guessed() {
    for name in ["", "root\0ignored"] {
        assert_eq!(by_name(name).unwrap_err().kind(), io::ErrorKind::InvalidInput);
    }
    assert!(by_name("capsem-no-such-account-54f37e96").unwrap().is_none());
}

#[test]
fn spoofed_home_child() {
    if std::env::var_os("CAPSEM_ACCOUNT_METADATA_CHILD").is_none() {
        return;
    }
    assert_eq!(std::env::var_os("HOME").unwrap(), "/capsem-synthetic-home-never-read");
    real_os_account_metadata_matches_uid_record();
}

#[test]
fn authoritative_lookup_ignores_inherited_home_in_real_child() {
    assert!(std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "unix::account::tests::spoofed_home_child"])
        .env("CAPSEM_ACCOUNT_METADATA_CHILD", "1")
        .env("HOME", "/capsem-synthetic-home-never-read")
        .status()
        .unwrap()
        .success());
}
