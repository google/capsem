use crate::SecurityRuleProfile;
use serde_json::json;

fn matches(selection: &str, condition: &str, host: &str, method: &str) -> bool {
    let yaml = format!(
        "title: grouping_test\nlevel: high\nlogsource:\n  product: capsem\n  service: security_event\ndetection:\n{selection}  condition: {condition}\ncapsem:\n  action: allow\n"
    );
    let profile = SecurityRuleProfile::parse_sigma_yaml(&yaml).expect("valid Sigma rule");
    profile.profiles.rules["grouping_test"]
        .matches_security_event(&json!({"http": {"host": host, "method": method}}))
        .expect("event evaluation")
}

#[test]
fn list_alternatives_cannot_escape_other_selection_fields() {
    let selection = "  selection:\n    http.method: GET\n    http.host: [a.example, b.example]\n";
    for host in ["a.example", "b.example"] {
        assert!(matches(selection, "selection", host, "GET"));
        assert!(!matches(selection, "selection", host, "POST"));
    }
    assert!(!matches(selection, "selection", "other.example", "GET"));
}

#[test]
fn selection_alternatives_cannot_escape_a_condition_conjunction() {
    let selection = "  hosts:\n    http.host: [a.example, b.example]\n  method:\n    http.method: GET\n";
    for host in ["a.example", "b.example"] {
        assert!(matches(selection, "hosts and method", host, "GET"));
        assert!(!matches(selection, "hosts and method", host, "POST"));
    }
}

#[test]
fn negated_selection_stays_grouped_with_other_conditions() {
    let selection = "  excluded:\n    http.host: a.example\n    http.method: POST\n  method:\n    http.method: GET\n";
    assert!(matches(selection, "not excluded and method", "b.example", "GET"));
    assert!(!matches(selection, "not excluded and method", "b.example", "POST"));
    assert!(!matches(selection, "not excluded and method", "a.example", "POST"));
}
