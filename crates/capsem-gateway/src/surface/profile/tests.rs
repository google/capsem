use super::*;

#[test]
fn auto_uses_the_parent_connection_and_explicit_profiles_override_it() {
    for parent in [
        "http://localhost:5173",
        "http://127.0.0.1:1234",
        "http://[::1]:3000",
        "tauri://localhost",
    ] {
        assert_eq!(DisplayProfile::Auto.resolve(parent).as_str(), "local");
        assert_eq!(DisplayProfile::Remote.resolve(parent).as_str(), "remote");
    }
    for parent in ["https://capsem.example", "https://localhost.attacker.test", "", "null"] {
        assert_eq!(DisplayProfile::Auto.resolve(parent).as_str(), "remote");
        assert_eq!(DisplayProfile::Local.resolve(parent).as_str(), "local");
    }
}

#[test]
fn bootstrap_profiles_are_bounded_enums_and_not_arbitrary_client_settings() {
    assert_eq!(DisplayProfile::from_form("bootstrap_token=x").unwrap().as_str(), "auto");
    for profile in ["auto", "local", "remote"] {
        assert_eq!(
            DisplayProfile::from_form(&format!("display_profile={profile}"))
                .unwrap()
                .as_str(),
            profile
        );
    }
    for body in [
        "display_profile=raw",
        "display_profile=",
        "display_profile=local&display_profile=remote",
        "display_profile=%22%3E",
    ] {
        assert!(DisplayProfile::from_form(body).is_err(), "{body}");
    }
}
