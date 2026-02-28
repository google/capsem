/// Model pricing: estimates cost per API call using bundled pricing data
/// from pydantic/genai-prices. Update via `just update_prices`.
use serde::Deserialize;

/// Embedded pricing data (updated via `just update_prices`).
const PRICING_JSON: &str = include_str!("../../../../config/genai-prices.json");

/// Pre-parsed pricing lookup table.
pub struct PricingTable {
    providers: Vec<ProviderData>,
}

#[derive(Deserialize)]
struct ProviderData {
    id: String,
    models: Vec<ModelData>,
}

#[derive(Deserialize)]
struct ModelData {
    #[allow(dead_code)]
    id: String,
    #[serde(rename = "match")]
    match_rule: MatchClause,
    prices: PriceSpec,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum MatchClause {
    Equals { equals: String },
    StartsWith { starts_with: String },
    EndsWith { ends_with: String },
    Contains { contains: String },
    Or { or: Vec<MatchClause> },
}

impl MatchClause {
    fn matches(&self, model: &str) -> bool {
        match self {
            MatchClause::Equals { equals } => model == equals,
            MatchClause::StartsWith { starts_with } => model.starts_with(starts_with.as_str()),
            MatchClause::EndsWith { ends_with } => model.ends_with(ends_with.as_str()),
            MatchClause::Contains { contains } => model.contains(contains.as_str()),
            MatchClause::Or { or } => or.iter().any(|c| c.matches(model)),
        }
    }
}

/// Price spec: either a flat `{ input_mtok, output_mtok }` or a conditional
/// list `[{ prices: { ... } }, ...]`. We take the first entry for lists.
#[derive(Deserialize)]
#[serde(untagged)]
enum PriceSpec {
    Direct(ModelPrice),
    Conditional(Vec<ConditionalPrice>),
}

impl PriceSpec {
    fn price(&self) -> Option<&ModelPrice> {
        match self {
            PriceSpec::Direct(p) => Some(p),
            PriceSpec::Conditional(list) => list.first().map(|c| &c.prices),
        }
    }
}

#[derive(Deserialize)]
struct ConditionalPrice {
    prices: ModelPrice,
}

#[derive(Deserialize)]
struct ModelPrice {
    #[serde(default)]
    input_mtok: PriceValue,
    #[serde(default)]
    output_mtok: PriceValue,
}

/// A price per million tokens: either a flat f64 or tiered with a base rate.
#[derive(Deserialize, Default)]
#[serde(untagged)]
enum PriceValue {
    Flat(f64),
    #[serde(rename_all = "snake_case")]
    Tiered {
        base: f64,
        // We ignore 'tiers' for now but allow them to exist in the JSON
    },
    #[default]
    Missing,
}

impl PriceValue {
    fn rate(&self) -> f64 {
        match self {
            PriceValue::Flat(v) => *v,
            PriceValue::Tiered { base } => *base,
            PriceValue::Missing => 0.0,
        }
    }
}

impl PricingTable {
    /// Parse the embedded pricing JSON. Panics on parse failure (compile-time
    /// data, so any error is a build-time bug).
    pub fn load() -> Self {
        // Only deserialize the fields we need from each provider entry.
        #[derive(Deserialize)]
        struct RawProvider {
            id: String,
            #[serde(default)]
            models: Vec<ModelData>,
        }

        let raw: Vec<RawProvider> =
            serde_json::from_str(PRICING_JSON).expect("genai-prices.json parse failed");

        let providers = raw
            .into_iter()
            .filter(|p| matches!(p.id.as_str(), "anthropic" | "openai" | "google"))
            .map(|p| ProviderData {
                id: p.id,
                models: p.models,
            })
            .collect();

        Self { providers }
    }

    /// Estimate the cost in USD for a single API call.
    /// Returns 0.0 if the provider/model is unknown.
    pub fn estimate_cost(
        &self,
        provider: &str,
        model: Option<&str>,
        input_tokens: Option<u64>,
        output_tokens: Option<u64>,
    ) -> f64 {
        let model_str = match model {
            Some(m) if !m.is_empty() => m,
            _ => return 0.0,
        };

        let input = input_tokens.unwrap_or(0) as f64;
        let output = output_tokens.unwrap_or(0) as f64;

        if input == 0.0 && output == 0.0 {
            return 0.0;
        }

        let prov = match self.providers.iter().find(|p| p.id == provider) {
            Some(p) => p,
            None => return 0.0,
        };

        for m in &prov.models {
            if m.match_rule.matches(model_str) {
                if let Some(price) = m.prices.price() {
                    let input_rate = price.input_mtok.rate();
                    let output_rate = price.output_mtok.rate();
                    return input * input_rate / 1_000_000.0 + output * output_rate / 1_000_000.0;
                }
            }
        }

        0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let cost = table.estimate_cost("anthropic", Some("claude-sonnet-4-20250514"), Some(1000), Some(500));
        assert!(cost > 0.0, "cost should be positive for known model");
    }

    #[test]
    fn estimate_cost_known_google_model() {
        let table = PricingTable::load();
        let cost = table.estimate_cost("google", Some("gemini-2.0-flash"), Some(1000), Some(500));
        assert!(cost > 0.0, "cost should be positive for known model");
    }

    #[test]
    fn estimate_cost_known_openai_model() {
        let table = PricingTable::load();
        let cost = table.estimate_cost("openai", Some("gpt-4o"), Some(1000), Some(500));
        assert!(cost > 0.0, "cost should be positive for known model");
    }

    #[test]
    fn estimate_cost_unknown_model() {
        let table = PricingTable::load();
        let cost = table.estimate_cost("anthropic", Some("nonexistent-model-xyz"), Some(1000), Some(500));
        assert_eq!(cost, 0.0, "unknown model should return 0");
    }

    #[test]
    fn estimate_cost_unknown_provider() {
        let table = PricingTable::load();
        let cost = table.estimate_cost("azure", Some("gpt-4o"), Some(1000), Some(500));
        assert_eq!(cost, 0.0, "unknown provider should return 0");
    }

    #[test]
    fn estimate_cost_no_model() {
        let table = PricingTable::load();
        assert_eq!(table.estimate_cost("anthropic", None, Some(1000), Some(500)), 0.0);
        assert_eq!(table.estimate_cost("anthropic", Some(""), Some(1000), Some(500)), 0.0);
    }

    #[test]
    fn estimate_cost_zero_tokens() {
        let table = PricingTable::load();
        assert_eq!(
            table.estimate_cost("anthropic", Some("claude-sonnet-4-20250514"), Some(0), Some(0)),
            0.0
        );
        assert_eq!(
            table.estimate_cost("anthropic", Some("claude-sonnet-4-20250514"), None, None),
            0.0
        );
    }

    #[test]
    fn match_clause_equals() {
        let mc = MatchClause::Equals { equals: "gpt-4o".to_string() };
        assert!(mc.matches("gpt-4o"));
        assert!(!mc.matches("gpt-4o-mini"));
    }

    #[test]
    fn match_clause_starts_with() {
        let mc = MatchClause::StartsWith { starts_with: "claude-3".to_string() };
        assert!(mc.matches("claude-3-opus"));
        assert!(mc.matches("claude-3-sonnet"));
        assert!(!mc.matches("claude-2"));
    }

    #[test]
    fn match_clause_contains() {
        let mc = MatchClause::Contains { contains: "haiku".to_string() };
        assert!(mc.matches("claude-3-5-haiku-20241022"));
        assert!(!mc.matches("claude-3-sonnet"));
    }

    #[test]
    fn match_clause_or() {
        let mc = MatchClause::Or {
            or: vec![
                MatchClause::Equals { equals: "gpt-4".to_string() },
                MatchClause::StartsWith { starts_with: "gpt-4-".to_string() },
            ],
        };
        assert!(mc.matches("gpt-4"));
        assert!(mc.matches("gpt-4-turbo"));
        assert!(!mc.matches("gpt-3.5"));
    }

    #[test]
    fn tiered_price_uses_base_rate() {
        let table = PricingTable::load();
        // claude-opus-4-6 has tiered pricing; should still return a cost
        let cost = table.estimate_cost("anthropic", Some("claude-opus-4-6"), Some(1000), Some(500));
        assert!(cost > 0.0, "tiered model should still return positive cost");
    }
}
