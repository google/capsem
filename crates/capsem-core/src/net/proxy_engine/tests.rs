use std::collections::BTreeMap;
use std::sync::Arc;

use super::*;
use crate::net::policy::NetworkMechanics;
use crate::net::policy_config::{
    ModelEndpointRegistry, SecurityPluginConfig, SecurityRuleProfile, SecurityRuleSet, SecurityRuleSource,
};

fn revision(marker: usize) -> ProxyPolicySnapshot {
    let profile = SecurityRuleProfile::parse_toml(&format!(
        r#"
[profiles.rules.marker_{marker}]
name = "marker_{marker}"
action = "allow"
match = 'http.host == "marker-{marker}.example"'
"#
    ))
    .unwrap();
    let rules = SecurityRuleSet::compile_profile(&profile, SecurityRuleSource::User).unwrap();
    let mut network = NetworkMechanics::new();
    network.max_body_capture = marker;
    let plugins = BTreeMap::<String, SecurityPluginConfig>::new();

    ProxyPolicySnapshot::new(
        format!("blake3:{marker:064x}"),
        network,
        rules,
        plugins,
        ModelEndpointRegistry::default(),
    )
}

#[test]
fn policy_handle_replaces_one_immutable_revision_atomically() {
    let handle = ProxyPolicyHandle::new(revision(0));

    for marker in 1..=256 {
        handle.replace(revision(marker));
        let snapshot = handle.snapshot();
        assert_eq!(snapshot.network().max_body_capture, marker);
        assert_eq!(snapshot.digest(), format!("blake3:{marker:064x}"));
        assert_eq!(
            snapshot.security_rules().rules()[0].rule_id,
            format!("profiles.rules.marker_{marker}")
        );
    }
}

#[test]
fn request_snapshot_cannot_mix_concurrent_policy_revisions() {
    let handle = Arc::new(ProxyPolicyHandle::new(revision(0)));
    let writer_handle = Arc::clone(&handle);
    let writer = std::thread::spawn(move || {
        for marker in 1..=10_000 {
            writer_handle.replace(revision(marker % 257));
        }
    });

    for _ in 0..10_000 {
        let snapshot = handle.snapshot();
        let marker = snapshot.network().max_body_capture;
        assert_eq!(snapshot.digest(), format!("blake3:{marker:064x}"));
        assert_eq!(
            snapshot.security_rules().rules()[0].rule_id,
            format!("profiles.rules.marker_{marker}")
        );
    }
    writer.join().unwrap();
}
