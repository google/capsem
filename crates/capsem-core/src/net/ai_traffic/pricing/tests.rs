use super::*;
use std::collections::BTreeMap;

fn no_details() -> BTreeMap<String, u64> {
    BTreeMap::new()
}

fn cache_details(cache_read: u64) -> BTreeMap<String, u64> {
    BTreeMap::from([("cache_read".into(), cache_read)])
}

#[test]
fn load_succeeds() {
    let table = PricingTable::load();
    assert!(
        table.providers.len() >= 3,
        "should have at least anthropic, openai, google"
    );
}

#[test]
fn estimate_cost_known_anthropic_model() {
    let table = PricingTable::load();
    let cost = table.estimate_cost(
        "anthropic",
        Some("claude-sonnet-4-20250514"),
        Some(1000),
        Some(500),
        &no_details(),
    );
    assert!(cost > 0.0, "cost should be positive for known model");
}

#[test]
fn estimate_cost_known_google_model() {
    let table = PricingTable::load();
    let cost = table.estimate_cost(
        "google",
        Some("gemini-2.0-flash"),
        Some(1000),
        Some(500),
        &no_details(),
    );
    assert!(cost > 0.0, "cost should be positive for known model");
}

#[test]
fn estimate_cost_known_openai_model() {
    let table = PricingTable::load();
    let cost = table.estimate_cost(
        "openai",
        Some("gpt-4o"),
        Some(1000),
        Some(500),
        &no_details(),
    );
    assert!(cost > 0.0, "cost should be positive for known model");
}

#[test]
fn estimate_cost_unknown_model() {
    let table = PricingTable::load();
    let cost = table.estimate_cost(
        "anthropic",
        Some("nonexistent-model-xyz"),
        Some(1000),
        Some(500),
        &no_details(),
    );
    assert_eq!(cost, 0.0, "unknown model should return 0");
}

#[test]
fn estimate_cost_unknown_provider() {
    let table = PricingTable::load();
    let cost = table.estimate_cost(
        "azure",
        Some("gpt-4o"),
        Some(1000),
        Some(500),
        &no_details(),
    );
    assert_eq!(cost, 0.0, "unknown provider should return 0");
}

#[test]
fn estimate_cost_no_model() {
    let table = PricingTable::load();
    assert_eq!(
        table.estimate_cost("anthropic", None, Some(1000), Some(500), &no_details()),
        0.0
    );
    assert_eq!(
        table.estimate_cost("anthropic", Some(""), Some(1000), Some(500), &no_details()),
        0.0
    );
}

#[test]
fn estimate_cost_zero_tokens() {
    let table = PricingTable::load();
    assert_eq!(
        table.estimate_cost(
            "anthropic",
            Some("claude-sonnet-4-20250514"),
            Some(0),
            Some(0),
            &no_details()
        ),
        0.0
    );
    assert_eq!(
        table.estimate_cost(
            "anthropic",
            Some("claude-sonnet-4-20250514"),
            None,
            None,
            &no_details()
        ),
        0.0
    );
}

#[test]
fn match_clause_equals() {
    let mc = MatchClause::Equals {
        equals: "gpt-4o".to_string(),
    };
    assert!(mc.matches("gpt-4o"));
    assert!(!mc.matches("gpt-4o-mini"));
}

#[test]
fn match_clause_starts_with() {
    let mc = MatchClause::StartsWith {
        starts_with: "claude-3".to_string(),
    };
    assert!(mc.matches("claude-3-opus"));
    assert!(mc.matches("claude-3-sonnet"));
    assert!(!mc.matches("claude-2"));
}

#[test]
fn match_clause_contains() {
    let mc = MatchClause::Contains {
        contains: "haiku".to_string(),
    };
    assert!(mc.matches("claude-3-5-haiku-20241022"));
    assert!(!mc.matches("claude-3-sonnet"));
}

#[test]
fn match_clause_or() {
    let mc = MatchClause::Or {
        or: vec![
            MatchClause::Equals {
                equals: "gpt-4".to_string(),
            },
            MatchClause::StartsWith {
                starts_with: "gpt-4-".to_string(),
            },
        ],
    };
    assert!(mc.matches("gpt-4"));
    assert!(mc.matches("gpt-4-turbo"));
    assert!(!mc.matches("gpt-3.5"));
}

#[test]
fn tiered_price_uses_base_rate() {
    let table = PricingTable::load();
    let cost = table.estimate_cost(
        "anthropic",
        Some("claude-opus-4-6"),
        Some(1000),
        Some(500),
        &no_details(),
    );
    assert!(cost > 0.0, "tiered model should still return positive cost");
}

#[test]
fn uses_upstream_match_without_suffix_guessing() {
    let table = PricingTable::load();
    let exact = table.estimate_cost(
        "openai",
        Some("gpt-5-nano"),
        Some(1000),
        Some(250),
        &no_details(),
    );
    let guessed = table.estimate_cost(
        "openai",
        Some("gpt-5-private-fork"),
        Some(1000),
        Some(250),
        &no_details(),
    );
    assert!(exact > 0.0, "exact match should have a cost");
    assert_eq!(
        guessed, 0.0,
        "unknown suffixed variants must not inherit pricing by guesswork"
    );
}

#[test]
fn unknown_model_does_not_prefix_match() {
    let table = PricingTable::load();
    let cost = table.estimate_cost(
        "anthropic",
        Some("claude-sonnet-4.future"),
        Some(1000),
        Some(500),
        &no_details(),
    );
    assert_eq!(
        cost, 0.0,
        "unrelated model should not fuzzy-match (prefix too short)"
    );
}

#[test]
fn unknown_deep_suffix_model_is_zero() {
    let table = PricingTable::load();
    let cost = table.estimate_cost(
        "openai",
        Some("gpt-4o-z-z-z-z-z"),
        Some(1_000_000),
        Some(500_000),
        &no_details(),
    );
    assert_eq!(
        cost, 0.0,
        "too many segments should exhaust strip budget and prefix too short"
    );
}

#[test]
fn cache_read_tokens_reduce_cost() {
    let table = PricingTable::load();
    let full_cost = table.estimate_cost(
        "anthropic",
        Some("claude-sonnet-4-20250514"),
        Some(1000),
        Some(500),
        &no_details(),
    );
    let cached_cost = table.estimate_cost(
        "anthropic",
        Some("claude-sonnet-4-20250514"),
        Some(1000),
        Some(500),
        &cache_details(800), // 800 of 1000 input tokens are cached
    );
    assert!(full_cost > 0.0, "full cost should be positive");
    assert!(
        cached_cost < full_cost,
        "cached cost should be lower than full cost"
    );
    assert!(
        cached_cost > 0.0,
        "cached cost should still be positive (output tokens)"
    );
}

#[test]
fn cache_read_tokens_all_cached_zero_input_cost() {
    let table = PricingTable::load();
    let cost = table.estimate_cost(
        "anthropic",
        Some("claude-sonnet-4-20250514"),
        Some(1000),
        Some(0),
        &cache_details(1000),
    );
    assert_eq!(cost, 0.0, "fully cached with no output should be free");
}

#[test]
fn oversized_model_string_rejected() {
    let table = PricingTable::load();
    let huge = "claude-sonnet-4-".to_string() + &"x".repeat(8192);
    let cost = table.estimate_cost(
        "anthropic",
        Some(&huge),
        Some(1000),
        Some(500),
        &no_details(),
    );
    assert_eq!(cost, 0.0, "oversized model string should be rejected");
}
