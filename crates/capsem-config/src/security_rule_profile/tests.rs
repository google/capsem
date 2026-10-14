use serde_json::json;

use super::*;

mod yaml_migration;

#[test]
fn sigma_sequence_alternatives_still_require_every_selection_field() {
    let profile = SecurityRuleProfile::parse_sigma_yaml(
        "title: Grouped Rule\nlogsource:\n  product: capsem\n  service: security_event\ndetection:\n  selection:\n    http.host: approved.example\n    http.method: [GET, POST]\n  condition: selection\nlevel: high\n",
    ).unwrap();
    let rule = &profile.profiles.rules["grouped_rule"];
    for method in ["GET", "POST"] {
        assert!(evaluate_security_event_match(
            &rule.condition,
            &json!({"http": {"host": "approved.example", "method": method}})
        )
        .unwrap());
        assert!(
            !evaluate_security_event_match(
                &rule.condition,
                &json!({"http": {"host": "unexpected.example", "method": method}})
            )
            .unwrap(),
            "{method} cannot bypass the required host"
        );
    }
}

#[test]
fn sigma_negated_selection_alternatives_still_require_the_guard() {
    let profile = SecurityRuleProfile::parse_sigma_yaml(
        "title: Guarded Rule\nlogsource:\n  product: capsem\n  service: security_event\ndetection:\n  guard:\n    tcp.port: 443\n  selection:\n    http.host: approved.example\n    http.method: [GET, POST]\n  condition: guard and not selection\nlevel: high\n",
    ).unwrap();
    let rule = &profile.profiles.rules["guarded_rule"];
    for (port, host, method, expected) in [
        ("443", "approved.example", "GET", false),
        ("443", "approved.example", "POST", false),
        ("443", "approved.example", "DELETE", true),
        ("443", "unexpected.example", "GET", true),
        ("80", "approved.example", "DELETE", false),
        ("80", "unexpected.example", "GET", false),
    ] {
        assert_eq!(
            evaluate_security_event_match(
                &rule.condition,
                &json!({"tcp": {"port": port}, "http": {"host": host, "method": method}})
            )
            .unwrap(),
            expected,
            "{port} {host} {method}"
        );
    }
}
