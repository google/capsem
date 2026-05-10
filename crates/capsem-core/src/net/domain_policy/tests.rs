//! Tests for `domain_policy` (extracted from inline `mod tests`).

use super::*;

fn dev_policy() -> DomainPolicy {
    DomainPolicy::default_dev()
}

// -- Exact match --

#[test]
fn allow_exact_match() {
    let policy = dev_policy();
    let (action, _) = policy.evaluate("github.com");
    assert_eq!(action, Action::Allow);
}

#[test]
fn allow_elie_net() {
    let policy = dev_policy();
    let (action, _) = policy.evaluate("elie.net");
    assert_eq!(action, Action::Allow);
}

#[test]
fn allow_pypi() {
    let policy = dev_policy();
    let (action, _) = policy.evaluate("pypi.org");
    assert_eq!(action, Action::Allow);
}

// -- Wildcard match --

#[test]
fn allow_wildcard_subdomain() {
    let policy = dev_policy();
    let (action, _) = policy.evaluate("api.github.com");
    assert_eq!(action, Action::Allow);
}

#[test]
fn allow_deep_wildcard_subdomain() {
    let policy = dev_policy();
    let (action, _) = policy.evaluate("raw.githubusercontent.com");
    assert_eq!(action, Action::Allow);
}

#[test]
fn wildcard_does_not_match_base_domain() {
    // "*.github.com" should NOT match "github.com" itself
    // (github.com is allowed via exact match, not wildcard)
    let policy = DomainPolicy::new(&["*.example.org".to_string()], &[], Action::Deny);
    let (action, _) = policy.evaluate("example.org");
    assert_eq!(action, Action::Deny);
}

// -- Block-list --

#[test]
fn block_anthropic_api() {
    let policy = dev_policy();
    let (action, reason) = policy.evaluate("api.anthropic.com");
    assert_eq!(action, Action::Deny);
    assert_eq!(reason, "domain in block-list");
}

#[test]
fn block_openai_api() {
    let policy = dev_policy();
    let (action, _) = policy.evaluate("api.openai.com");
    assert_eq!(action, Action::Deny);
}

#[test]
fn allow_google_ai_api() {
    let policy = dev_policy();
    let (action, _) = policy.evaluate("generativelanguage.googleapis.com");
    assert_eq!(action, Action::Allow);
}

#[test]
fn allow_wikipedia() {
    let policy = dev_policy();
    let (action, _) = policy.evaluate("en.wikipedia.org");
    assert_eq!(action, Action::Allow);
}

#[test]
fn allow_wikipedia_wildcard_subdomain() {
    let policy = dev_policy();
    let (action, _) = policy.evaluate("fr.wikipedia.org");
    assert_eq!(action, Action::Allow);
}

#[test]
fn block_takes_priority_over_allow() {
    // If a domain is in both lists, block wins
    let policy = DomainPolicy::new(
        &["evil.com".to_string()],
        &["evil.com".to_string()],
        Action::Allow,
    );
    let (action, reason) = policy.evaluate("evil.com");
    assert_eq!(action, Action::Deny);
    assert_eq!(reason, "domain in block-list");
}

// -- Default deny --

#[test]
fn deny_unknown_domain() {
    let policy = dev_policy();
    let (action, reason) = policy.evaluate("example.com");
    assert_eq!(action, Action::Deny);
    assert_eq!(reason, "domain not in allow-list");
}

#[test]
fn deny_rfc2606_example_net() {
    let policy = dev_policy();
    let (action, _) = policy.evaluate("example.net");
    assert_eq!(action, Action::Deny);
}

#[test]
fn deny_rfc2606_example_org() {
    let policy = dev_policy();
    let (action, _) = policy.evaluate("example.org");
    assert_eq!(action, Action::Deny);
}

// -- Case insensitivity --

#[test]
fn case_insensitive_match() {
    let policy = dev_policy();
    let (action, _) = policy.evaluate("GitHub.COM");
    assert_eq!(action, Action::Allow);
}

#[test]
fn case_insensitive_block() {
    let policy = dev_policy();
    let (action, _) = policy.evaluate("API.ANTHROPIC.COM");
    assert_eq!(action, Action::Deny);
}

// -- Edge cases --

#[test]
fn empty_domain_denied() {
    let policy = dev_policy();
    let (action, reason) = policy.evaluate("");
    assert_eq!(action, Action::Deny);
    assert_eq!(reason, "empty domain");
}

#[test]
fn default_allow_policy() {
    let policy = DomainPolicy::new(&[], &[], Action::Allow);
    let (action, reason) = policy.evaluate("anything.com");
    assert_eq!(action, Action::Allow);
    assert_eq!(reason, "default allow");
}

#[test]
fn empty_policy_denies_all() {
    let policy = DomainPolicy::new(&[], &[], Action::Deny);
    let (action, _) = policy.evaluate("github.com");
    assert_eq!(action, Action::Deny);
}

// -- Pattern list accessors --

#[test]
fn allowed_patterns_returned() {
    let policy = dev_policy();
    let patterns = policy.allowed_patterns();
    assert!(patterns.contains(&"github.com".to_string()));
    assert!(patterns.contains(&"*.github.com".to_string()));
}

#[test]
fn blocked_patterns_returned() {
    let policy = dev_policy();
    let patterns = policy.blocked_patterns();
    assert!(patterns.contains(&"api.anthropic.com".to_string()));
}

// -----------------------------------------------------------------------
// Stress: block always beats allow
// -----------------------------------------------------------------------

#[test]
fn block_beats_allow_exact_same_domain() {
    let policy = DomainPolicy::new(&["evil.com".into()], &["evil.com".into()], Action::Allow);
    let (action, reason) = policy.evaluate("evil.com");
    assert_eq!(action, Action::Deny);
    assert_eq!(reason, "domain in block-list");
}

#[test]
fn block_beats_allow_wildcard_same_domain() {
    let policy = DomainPolicy::new(
        &["*.example.com".into()],
        &["*.example.com".into()],
        Action::Allow,
    );
    let (action, reason) = policy.evaluate("sub.example.com");
    assert_eq!(action, Action::Deny);
    assert_eq!(reason, "domain in block-list");
}

#[test]
fn exact_block_beats_wildcard_allow() {
    // Block "api.example.com" exactly, allow "*.example.com" via wildcard.
    let policy = DomainPolicy::new(
        &["*.example.com".into()],
        &["api.example.com".into()],
        Action::Allow,
    );
    let (action, reason) = policy.evaluate("api.example.com");
    assert_eq!(action, Action::Deny);
    assert_eq!(reason, "domain in block-list");
    // Other subdomains still allowed
    let (action, _) = policy.evaluate("web.example.com");
    assert_eq!(action, Action::Allow);
}

#[test]
fn wildcard_block_beats_exact_allow() {
    // Allow "api.example.com" exactly, block "*.example.com" via wildcard.
    let policy = DomainPolicy::new(
        &["api.example.com".into()],
        &["*.example.com".into()],
        Action::Allow,
    );
    let (action, reason) = policy.evaluate("api.example.com");
    assert_eq!(action, Action::Deny);
    assert_eq!(reason, "domain in block-list");
}

#[test]
fn block_beats_allow_with_default_allow() {
    // Default is allow, domain is in both lists -- block wins.
    let policy = DomainPolicy::new(
        &["target.com".into()],
        &["target.com".into()],
        Action::Allow,
    );
    let (action, _) = policy.evaluate("target.com");
    assert_eq!(action, Action::Deny);
}

#[test]
fn block_beats_allow_with_default_deny() {
    // Default is deny, domain is in both lists -- block wins.
    let policy = DomainPolicy::new(&["target.com".into()], &["target.com".into()], Action::Deny);
    let (action, _) = policy.evaluate("target.com");
    assert_eq!(action, Action::Deny);
}

#[test]
fn block_many_overlapping_wildcards() {
    // Multiple wildcard overlaps: block should always win.
    let policy = DomainPolicy::new(
        &["*.a.com".into(), "*.b.com".into(), "*.c.com".into()],
        &["*.a.com".into(), "*.c.com".into()],
        Action::Allow,
    );
    let (action, _) = policy.evaluate("x.a.com");
    assert_eq!(action, Action::Deny);
    let (action, _) = policy.evaluate("x.b.com");
    assert_eq!(action, Action::Allow);
    let (action, _) = policy.evaluate("x.c.com");
    assert_eq!(action, Action::Deny);
}

// -----------------------------------------------------------------------
// Stress: explicit lists beat default action
// -----------------------------------------------------------------------

#[test]
fn allow_list_beats_default_deny() {
    let policy = DomainPolicy::new(&["safe.com".into()], &[], Action::Deny);
    let (action, reason) = policy.evaluate("safe.com");
    assert_eq!(action, Action::Allow);
    assert_eq!(reason, "domain in allow-list");
}

#[test]
fn block_list_beats_default_allow() {
    let policy = DomainPolicy::new(&[], &["dangerous.com".into()], Action::Allow);
    let (action, reason) = policy.evaluate("dangerous.com");
    assert_eq!(action, Action::Deny);
    assert_eq!(reason, "domain in block-list");
}

#[test]
fn wildcard_allow_beats_default_deny() {
    let policy = DomainPolicy::new(&["*.safe.org".into()], &[], Action::Deny);
    let (action, reason) = policy.evaluate("api.safe.org");
    assert_eq!(action, Action::Allow);
    assert_eq!(reason, "domain in allow-list");
    // Base domain not matched by wildcard -- falls to default deny
    let (action, _) = policy.evaluate("safe.org");
    assert_eq!(action, Action::Deny);
}

#[test]
fn wildcard_block_beats_default_allow() {
    let policy = DomainPolicy::new(&[], &["*.evil.org".into()], Action::Allow);
    let (action, reason) = policy.evaluate("sub.evil.org");
    assert_eq!(action, Action::Deny);
    assert_eq!(reason, "domain in block-list");
    // Base domain not matched by wildcard -- falls to default allow
    let (action, _) = policy.evaluate("evil.org");
    assert_eq!(action, Action::Allow);
}

#[test]
fn unlisted_domain_uses_default_deny() {
    let policy = DomainPolicy::new(
        &["allowed.com".into()],
        &["blocked.com".into()],
        Action::Deny,
    );
    let (action, reason) = policy.evaluate("unlisted.com");
    assert_eq!(action, Action::Deny);
    assert_eq!(reason, "domain not in allow-list");
}

#[test]
fn unlisted_domain_uses_default_allow() {
    let policy = DomainPolicy::new(
        &["allowed.com".into()],
        &["blocked.com".into()],
        Action::Allow,
    );
    let (action, reason) = policy.evaluate("unlisted.com");
    assert_eq!(action, Action::Allow);
    assert_eq!(reason, "default allow");
}

// -----------------------------------------------------------------------
// Stress: priority ordering (block > allow > default)
// -----------------------------------------------------------------------

#[test]
fn full_priority_chain_block_allow_default() {
    // Three domains: one blocked, one allowed, one unlisted.
    // Default = Allow. Verify each gets the right outcome.
    let policy = DomainPolicy::new(
        &["allowed.com".into(), "both.com".into()],
        &["blocked.com".into(), "both.com".into()],
        Action::Allow,
    );
    let (action, reason) = policy.evaluate("blocked.com");
    assert_eq!(action, Action::Deny);
    assert_eq!(reason, "domain in block-list");

    let (action, reason) = policy.evaluate("allowed.com");
    assert_eq!(action, Action::Allow);
    assert_eq!(reason, "domain in allow-list");

    let (action, reason) = policy.evaluate("unlisted.com");
    assert_eq!(action, Action::Allow);
    assert_eq!(reason, "default allow");

    // "both.com" in both lists -- block wins
    let (action, reason) = policy.evaluate("both.com");
    assert_eq!(action, Action::Deny);
    assert_eq!(reason, "domain in block-list");
}

#[test]
fn full_priority_chain_with_default_deny() {
    let policy = DomainPolicy::new(
        &["allowed.com".into(), "both.com".into()],
        &["blocked.com".into(), "both.com".into()],
        Action::Deny,
    );
    let (action, _) = policy.evaluate("blocked.com");
    assert_eq!(action, Action::Deny);
    let (action, _) = policy.evaluate("allowed.com");
    assert_eq!(action, Action::Allow);
    let (action, _) = policy.evaluate("unlisted.com");
    assert_eq!(action, Action::Deny);
    let (action, _) = policy.evaluate("both.com");
    assert_eq!(action, Action::Deny);
}

#[test]
fn many_domains_block_always_wins_over_allow() {
    // Stress: 100 domains in both allow and block. All must be denied.
    let domains: Vec<String> = (0..100).map(|i| format!("d{i}.test.com")).collect();
    let policy = DomainPolicy::new(&domains, &domains, Action::Allow);
    for d in &domains {
        let (action, reason) = policy.evaluate(d);
        assert_eq!(action, Action::Deny, "block must beat allow for {d}");
        assert_eq!(reason, "domain in block-list");
    }
}
