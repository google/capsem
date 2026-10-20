use super::*;
use crate::net::policy_config::{ActivePolicyFile, SettingsFile};
use capsem_credentials::{GrantCapability, GrantTarget};
use std::collections::BTreeSet;

fn capability(id: &str, targets: &[(&str, &str)]) -> GrantCapability {
    GrantCapability::new(
        id.into(),
        BTreeSet::from(["openid".into()]),
        targets
            .iter()
            .map(|(method, url)| GrantTarget::new(method, url).unwrap())
            .collect(),
    )
    .unwrap()
}
fn active(corp: &str) -> ActivePolicyFile {
    ActivePolicyFile::from_settings_and_corp(&SettingsFile::default(), &toml::from_str(corp).unwrap()).unwrap()
}

#[test]
fn credential_target_preflight_obeys_current_and_applied_policy_and_exact_facts() {
    let allowed = capability("probe", &[("GET", "https://example.test/probe?mode=read")]);
    let denied = capability("probe", &[("POST", "https://example.test/probe?mode=write")]);
    let wrong_port = capability("other", &[("GET", "https://example.test:8443/probe?mode=read")]);
    let blocked = active(
        r#"
[corp.rules.no_write]
name = "no_writes"
action = "block"
priority = -10
match = 'http.method == "POST" || http.query == "mode=write" || tcp.port == "8443"'
"#,
    );
    let open = active("");
    for (current, applied) in [(&blocked, &open), (&open, &blocked)] {
        let admission =
            CredentialGrantAdmission::new(current, applied, [&allowed, &denied, &wrong_port].into_iter()).unwrap();
        assert!(admission.admits(&allowed));
        assert!(!admission.admits(&denied));
        assert!(!admission.admits(&wrong_port));
        assert!(!admission.admits(&capability("probe", &[("GET", "https://other.test/probe")])));
    }
}

#[test]
fn credential_target_preflight_refuses_plugin_ask_block_and_any_denied_target() {
    let clean = capability("probe", &[("GET", "https://example.test/probe")]);
    let mixed = capability(
        "probe",
        &[
            ("GET", "https://example.test/probe"),
            ("GET", "https://blocked.test/probe"),
        ],
    );
    let open = active("");
    let blocked = active(
        r#"
[corp.rules.blocked]
name = "blocked_host"
action = "block"
priority = -10
match = 'http.host == "blocked.test"'
"#,
    );
    let admission = CredentialGrantAdmission::new(&blocked, &open, [&clean, &mixed].into_iter()).unwrap();
    assert!(admission.admits(&clean));
    assert!(!admission.admits(&mixed));
    for mode in ["ask", "block"] {
        let policy = active(&format!("[plugins.dummy_post_allow]\nmode = '{mode}'\n"));
        assert!(!CredentialGrantAdmission::new(&policy, &open, [&clean].into_iter())
            .unwrap()
            .admits(&clean));
    }
    let disabled = active("[plugins.credential_broker]\nmode = 'disable'\n");
    assert!(!CredentialGrantAdmission::new(&disabled, &open, [&clean].into_iter())
        .unwrap()
        .admits(&clean));
    let asked = active("[corp.rules.confirm]\nname = 'confirm'\naction = 'ask'\npriority = -10\nmatch = 'http.host == \"example.test\"'\n");
    for (current, applied) in [(&asked, &open), (&open, &asked)] {
        assert!(!CredentialGrantAdmission::new(current, applied, [&clean].into_iter())
            .unwrap()
            .admits(&clean));
    }
}
