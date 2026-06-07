use super::*;

use hickory_proto::op::{Message, MessageType, OpCode, Query};
use hickory_proto::rr::{Name, RecordType};

fn build_query_bytes(name: &str, qtype: RecordType, id: u16) -> Vec<u8> {
    let mut msg = Message::new(id, MessageType::Query, OpCode::Query);
    msg.metadata.recursion_desired = true;
    let name = Name::from_ascii(name).unwrap();
    msg.add_query(Query::query(name, qtype));
    msg.to_vec().unwrap()
}

fn shared_policy() -> SharedPolicy {
    Arc::new(std::sync::RwLock::new(Arc::new(NetworkPolicy::new(
        Vec::new(),
        true,
        true,
    ))))
}

fn security_rules(toml: &str) -> SharedSecurityRules {
    let profile = crate::net::policy_config::SecurityRuleProfile::parse_toml(toml).unwrap();
    let rules = SecurityRuleSet::compile_profile(
        &profile,
        crate::net::policy_config::SecurityRuleSource::User,
    )
    .unwrap();
    Arc::new(std::sync::RwLock::new(Arc::new(rules)))
}

fn plugin_policy() -> SharedPluginPolicy {
    Arc::new(std::sync::RwLock::new(BTreeMap::new()))
}

#[tokio::test]
async fn dns_handler_blocks_query_through_security_event_rules() {
    let handler = DnsHandler::new(
        shared_policy(),
        security_rules(
            r#"
            [profiles.rules.block_dns_example]
            name = "block_dns_example"
            action = "block"
            reason = "dns test block"
            match = 'dns.qname == "blocked.example.com"'
            "#,
        ),
        plugin_policy(),
        Arc::new(DnsResolver::new()),
    );

    let result = handler
        .handle(&build_query_bytes(
            "blocked.example.com.",
            RecordType::A,
            0xCAFE,
        ))
        .await;

    assert_eq!(result.decision, Decision::Denied);
    assert_eq!(result.rcode, 3);
    assert_eq!(result.upstream_resolver_ms, 0);
    assert_eq!(
        result.matched_rule.as_deref(),
        Some("profiles.rules.block_dns_example")
    );
    assert_eq!(result.policy_mode.as_deref(), Some("security_event"));
    assert_eq!(result.policy_action.as_deref(), Some("block"));
    assert_eq!(
        result.policy_rule.as_deref(),
        Some("profiles.rules.block_dns_example")
    );
    assert_eq!(result.policy_reason.as_deref(), Some("dns test block"));
}
