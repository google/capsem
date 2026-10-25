use super::*;
use std::collections::BTreeMap;

fn no_details() -> BTreeMap<String, u64> {
    BTreeMap::new()
}

fn at(year: i32, month: u8, day: u8) -> std::time::SystemTime {
    time::Date::from_calendar_date(year, month.try_into().unwrap(), day)
        .unwrap()
        .midnight()
        .assume_utc()
        .into()
}

#[test]
fn dated_rates_use_the_event_date_instead_of_the_first_or_current_price() {
    let table = PricingTable::load();
    let price = |date| {
        table
            .estimate_cost_at(
                "openai",
                Some("o3"),
                Some(1_000_000),
                Some(1_000_000),
                &no_details(),
                date,
            )
            .unwrap()
    };
    assert_eq!(price(at(2025, 6, 9)), 50.0);
    assert_eq!(price(at(2025, 6, 10)), 10.0);
    assert_eq!(price(at(2026, 10, 23)), 10.0);
}

#[test]
fn invalid_or_unsupported_date_constraints_fail_closed() {
    for constraint in [
        r#"{"start_date":"2026-02-30"}"#,
        r#"{"start_date":"123é-1-12"}"#,
        r#"{"service_tier":"priority"}"#,
    ] {
        let text = format!(r#"[{{"constraint":{constraint},"prices":{{"input_mtok":2,"output_mtok":10}}}}]"#);
        assert!(serde_json::from_str::<PriceSpec>(&text).is_err());
    }
    let prices: PriceSpec = serde_json::from_str(
        r#"[{"constraint":{"start_date":"2026-12-01"},"prices":{"input_mtok":4,"output_mtok":20}}]"#,
    )
    .unwrap();
    assert!(prices
        .price(time::Date::from_calendar_date(2026, time::Month::November, 30).unwrap())
        .is_none());
}

#[test]
fn compiled_regex_and_ascii_case_follow_the_upstream_match_contract() {
    let clause: MatchClause = serde_json::from_str(r#"{"regex":"^gemini-3\\.5-flash-\\d"}"#).unwrap();
    assert!(clause.matches("GEMINI-3.5-FLASH-1"));
    assert!(!clause.matches("gemini-3.5-flash-lite"));
    assert!(MatchClause::Equals { equals: "o3".into() }.matches("O3"));
}

#[test]
fn prompt_size_tiers_price_the_entire_call_and_include_cached_input() {
    let table = PricingTable::load();
    let before = table
        .estimate_cost_at(
            "google",
            Some("gemini-1.5-pro"),
            Some(128_000),
            Some(1_000_000),
            &no_details(),
            at(2026, 10, 23),
        )
        .unwrap();
    let after = table
        .estimate_cost_at(
            "google",
            Some("gemini-1.5-pro"),
            Some(128_001),
            Some(1_000_000),
            &no_details(),
            at(2026, 10, 23),
        )
        .unwrap();
    assert!((before - 5.16).abs() < 1e-9);
    assert!((after - 10.3200025).abs() < 1e-9);
}

fn cache_details(cache_read: u64) -> BTreeMap<String, u64> {
    BTreeMap::from([("cache_read".into(), cache_read)])
}

#[test]
fn demo_opus_one_hour_cache_usage_is_priced_without_double_charging_writes() {
    // Captured demo usage only: no prompts, account IDs, headers or credentials.
    let (model, input, output, details) = crate::net::ai_traffic::events::parse_non_streaming_usage(
        crate::net::ai_traffic::provider::ModelProtocol::Anthropic,
        include_bytes!("fixtures/demo-opus-cache.json"),
    );
    assert_eq!(input, Some(45149));
    let cost = PricingTable::load()
        .estimate_cost_at("anthropic", model.as_deref(), input, output, &details, at(2026, 10, 24))
        .unwrap();
    let expected = (504.0 * 5.0 + 44611.0 * 0.5 + 34.0 * 10.0 + 8.0 * 25.0) / 1_000_000.0;
    assert!((cost - expected).abs() < 1e-12, "{cost} != {expected}");
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
    assert!(cost.unwrap() > 0.0, "cost should be positive for known model");
}

#[test]
fn estimate_cost_known_google_model() {
    let table = PricingTable::load();
    let cost = table.estimate_cost("google", Some("gemini-2.0-flash"), Some(1000), Some(500), &no_details());
    assert!(cost.unwrap() > 0.0, "cost should be positive for known model");
}

#[test]
fn estimate_cost_known_openai_model() {
    let table = PricingTable::load();
    let cost = table.estimate_cost("openai", Some("gpt-4o"), Some(1000), Some(500), &no_details());
    assert!(cost.unwrap() > 0.0, "cost should be positive for known model");
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
    assert_eq!(cost, None, "unknown model must remain unpriced");
}

#[test]
fn estimate_cost_unknown_provider() {
    let table = PricingTable::load();
    let cost = table.estimate_cost("azure", Some("gpt-4o"), Some(1000), Some(500), &no_details());
    assert_eq!(cost, None, "unknown provider must remain unpriced");
}

#[test]
fn estimate_cost_no_model() {
    let table = PricingTable::load();
    assert_eq!(
        table.estimate_cost("anthropic", None, Some(1000), Some(500), &no_details()),
        None
    );
    assert_eq!(
        table.estimate_cost("anthropic", Some(""), Some(1000), Some(500), &no_details()),
        None
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
        Some(0.0)
    );
    assert_eq!(
        table.estimate_cost("anthropic", Some("claude-sonnet-4-20250514"), None, None, &no_details()),
        None
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
    assert!(cost.unwrap() > 0.0, "tiered model should still return positive cost");
}

#[test]
fn uses_upstream_match_without_suffix_guessing() {
    let table = PricingTable::load();
    let exact = table.estimate_cost("openai", Some("gpt-5-nano"), Some(1000), Some(250), &no_details());
    let guessed = table.estimate_cost(
        "openai",
        Some("gpt-5-private-fork"),
        Some(1000),
        Some(250),
        &no_details(),
    );
    assert!(exact.unwrap() > 0.0, "exact match should have a cost");
    assert_eq!(
        guessed, None,
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
    assert_eq!(cost, None, "unrelated model should not fuzzy-match (prefix too short)");
}

#[test]
fn unknown_deep_suffix_model_has_no_estimate() {
    let table = PricingTable::load();
    let cost = table.estimate_cost(
        "openai",
        Some("gpt-4o-z-z-z-z-z"),
        Some(1_000_000),
        Some(500_000),
        &no_details(),
    );
    assert_eq!(
        cost, None,
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
    assert!(full_cost.unwrap() > 0.0, "full cost should be positive");
    assert!(cached_cost < full_cost, "cached cost should be lower than full cost");
    assert!(
        cached_cost.unwrap() > 0.0,
        "cached cost should still be positive (output tokens)"
    );
}

#[test]
fn cache_read_tokens_use_the_published_cache_rate() {
    let table = PricingTable::load();
    let cost = table.estimate_cost(
        "anthropic",
        Some("claude-sonnet-4-20250514"),
        Some(1000),
        Some(0),
        &cache_details(1000),
    );
    assert!(
        (cost.unwrap() - 0.0003).abs() < 1e-12,
        "cached tokens use the bundled cache rate"
    );
}

#[test]
fn oversized_model_string_rejected() {
    let table = PricingTable::load();
    let huge = "claude-sonnet-4-".to_string() + &"x".repeat(8192);
    let cost = table.estimate_cost("anthropic", Some(&huge), Some(1000), Some(500), &no_details());
    assert_eq!(cost, None, "oversized model string should be rejected");
}

#[test]
fn reviewed_trial_model_is_priced_and_output_only_usage_is_retained() {
    let table = PricingTable::load();
    assert!(
        (table
            .estimate_cost(
                "google",
                Some("gemini-4-argon-trial"),
                Some(34298),
                Some(250),
                &no_details()
            )
            .unwrap()
            - 0.071096)
            .abs()
            < 1e-10
    );
    assert!(
        table
            .estimate_cost(
                "google",
                Some("gemini-3.5-flash-lite"),
                Some(0),
                Some(56),
                &no_details()
            )
            .unwrap()
            > 0.0
    );
}

#[test]
fn missing_rates_never_turn_cached_or_partial_usage_into_free_calls() {
    let model: ModelData =
        serde_json::from_str(r#"{"id":"known","match":{"equals":"known"},"prices":{"input_mtok":2,"output_mtok":4}}"#)
            .unwrap();
    let table = PricingTable {
        providers: vec![ProviderData {
            id: "test".into(),
            models: vec![model],
        }],
    };
    assert_eq!(
        table.estimate_cost("test", Some("known"), Some(100), Some(0), &cache_details(100)),
        None
    );
    assert_eq!(
        table.estimate_cost("test", Some("known"), None, Some(100), &no_details()),
        None
    );
    assert_eq!(
        table.estimate_cost("test", Some("known"), Some(0), Some(0), &no_details()),
        Some(0.0)
    );
    assert_eq!(
        table.estimate_cost(
            "test",
            Some("known"),
            Some(100),
            Some(1),
            &BTreeMap::from([("cache_write_1h".into(), 100)])
        ),
        None
    );
}
