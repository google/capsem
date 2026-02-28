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
    ///
    /// Uses a three-pass strategy:
    /// 1. Strict match against all provider model rules
    /// 2. Progressive suffix stripping (remove trailing `-segment`s and retry)
    /// 3. Longest common prefix match against model IDs (min 8 chars)
    pub fn estimate_cost(
        &self,
        provider: &str,
        model: Option<&str>,
        input_tokens: Option<u64>,
        output_tokens: Option<u64>,
    ) -> f64 {
        // Reject oversized model strings before any allocation. Real model
        // names are well under 128 bytes; anything larger is garbage or an
        // attempted DoS via the fuzzy-match `.to_string()` clone below.
        const MAX_MODEL_LEN: usize = 128;

        let model_str = match model {
            Some(m) if !m.is_empty() && m.len() <= MAX_MODEL_LEN => m,
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

        // Pass 1: strict match
        if let Some(cost) = Self::try_strict_match(prov, model_str, input, output) {
            return cost;
        }

        // Pass 2: progressive suffix stripping (max 4 strips, min 4 chars remaining)
        const MAX_STRIP_DEPTH: usize = 4;
        const MIN_STRIP_LEN: usize = 4;
        let mut candidate = model_str.to_string();
        for _ in 0..MAX_STRIP_DEPTH {
            match candidate.rfind('-') {
                Some(pos) if pos >= MIN_STRIP_LEN => {
                    candidate.truncate(pos);
                    if let Some(cost) = Self::try_strict_match(prov, &candidate, input, output) {
                        return cost;
                    }
                }
                _ => break,
            }
        }

        // Pass 3: longest common prefix match (min 8 chars shared)
        if let Some(cost) = Self::try_prefix_match(prov, model_str, input, output) {
            return cost;
        }

        0.0
    }

    /// Try strict match against all models in a provider.
    fn try_strict_match(
        prov: &ProviderData,
        model: &str,
        input: f64,
        output: f64,
    ) -> Option<f64> {
        for m in &prov.models {
            if m.match_rule.matches(model) {
                if let Some(price) = m.prices.price() {
                    let input_rate = price.input_mtok.rate();
                    let output_rate = price.output_mtok.rate();
                    return Some(
                        input * input_rate / 1_000_000.0 + output * output_rate / 1_000_000.0,
                    );
                }
            }
        }
        None
    }

    /// Find the model whose ID shares the longest common prefix with the input.
    /// Requires at least `MIN_PREFIX_LEN` chars of shared prefix.
    /// Ties broken by closest version number (higher version preferred).
    fn try_prefix_match(
        prov: &ProviderData,
        model: &str,
        input: f64,
        output: f64,
    ) -> Option<f64> {
        const MIN_PREFIX_LEN: usize = 8;

        let mut best_len: usize = 0;
        let mut best_idx: Option<usize> = None;
        let mut best_version: Option<u64> = None;

        for (i, m) in prov.models.iter().enumerate() {
            let prefix_len = common_prefix_len(model, &m.id);
            if prefix_len < MIN_PREFIX_LEN {
                continue;
            }
            if prefix_len > best_len
                || (prefix_len == best_len && Self::version_closer(model, &m.id, best_version))
            {
                best_len = prefix_len;
                best_idx = Some(i);
                best_version = extract_trailing_version(&m.id);
            }
        }

        if let Some(idx) = best_idx {
            if let Some(price) = prov.models[idx].prices.price() {
                let input_rate = price.input_mtok.rate();
                let output_rate = price.output_mtok.rate();
                return Some(
                    input * input_rate / 1_000_000.0 + output * output_rate / 1_000_000.0,
                );
            }
        }
        None
    }

    /// Returns true if the candidate model's version is a better tiebreaker
    /// than the current best. Prefers higher version numbers (latest model).
    fn version_closer(_query: &str, candidate_id: &str, current_best: Option<u64>) -> bool {
        match (extract_trailing_version(candidate_id), current_best) {
            (Some(v), Some(best)) => v > best,
            (Some(_), None) => true,
            _ => false,
        }
    }
}

/// Length of the longest common prefix between two strings.
fn common_prefix_len(a: &str, b: &str) -> usize {
    a.bytes().zip(b.bytes()).take_while(|(x, y)| x == y).count()
}

/// Extract a trailing numeric version from a model ID.
/// E.g. "claude-opus-4-6" -> Some(6), "claude-opus-4-0" -> Some(0).
fn extract_trailing_version(id: &str) -> Option<u64> {
    let last_seg = id.rsplit('-').next()?;
    last_seg.parse::<u64>().ok()
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

    // --- Fuzzy matching tests ---

    #[test]
    fn fuzzy_suffix_strip() {
        let table = PricingTable::load();
        // gemini-3.1-pro-preview has starts_with rule; adding -customtools should
        // still match after one round of suffix stripping
        let exact = table.estimate_cost(
            "google",
            Some("gemini-3.1-pro-preview"),
            Some(1000),
            Some(500),
        );
        let fuzzy = table.estimate_cost(
            "google",
            Some("gemini-3.1-pro-preview-customtools"),
            Some(1000),
            Some(500),
        );
        assert!(exact > 0.0, "exact match should have a cost");
        // The suffixed variant already matches the starts_with rule directly,
        // so it should match at the same price regardless of fuzzy logic
        assert_eq!(fuzzy, exact, "suffixed variant should match same price");
    }

    #[test]
    fn fuzzy_date_stamp_strip() {
        let table = PricingTable::load();
        // gpt-4o uses equals:"gpt-4o" -- a date-stamped variant like
        // gpt-4o-2025-01-15 won't match strictly but should match after
        // stripping -15, -01, -2025 segments
        let base_cost =
            table.estimate_cost("openai", Some("gpt-4o"), Some(1_000_000), Some(500_000));
        let dated_cost = table.estimate_cost(
            "openai",
            Some("gpt-4o-2025-01-15"),
            Some(1_000_000),
            Some(500_000),
        );
        assert!(base_cost > 0.0, "gpt-4o should have a cost");
        assert_eq!(
            dated_cost, base_cost,
            "date-stamped gpt-4o should match base gpt-4o price via suffix stripping"
        );
    }

    #[test]
    fn fuzzy_version_closest() {
        let table = PricingTable::load();
        // claude-opus-4-99 doesn't match any strict rule and stripping -99
        // leaves claude-opus-4 which matches equals:"claude-opus-4" (strict).
        // But test with a model that needs prefix fallback: use a name that
        // won't match after stripping either, like "claude-sonnet-4.future"
        // (dot, not dash, so no stripping). Should fall through to prefix match
        // against claude-sonnet-4-* model IDs (prefix >= 15 chars).
        let cost = table.estimate_cost(
            "anthropic",
            Some("claude-sonnet-4.future"),
            Some(1000),
            Some(500),
        );
        let known_cost = table.estimate_cost(
            "anthropic",
            Some("claude-sonnet-4-20250514"),
            Some(1000),
            Some(500),
        );
        assert!(known_cost > 0.0, "known sonnet should have cost");
        assert_eq!(
            cost, known_cost,
            "prefix-matched model should use the same pricing"
        );
    }

    #[test]
    fn fuzzy_no_nonsense_match() {
        let table = PricingTable::load();
        // A completely unrelated model name should not fuzzy-match anything.
        // "totally-unknown-model" shares no meaningful prefix with any model.
        let cost = table.estimate_cost(
            "anthropic",
            Some("totally-unknown-model"),
            Some(1000),
            Some(500),
        );
        assert_eq!(
            cost, 0.0,
            "unrelated model should not fuzzy-match (prefix too short)"
        );
    }

    #[test]
    fn fuzzy_strip_depth_limit() {
        let table = PricingTable::load();
        // "gpt-4o-z-z-z-z-z" needs 5 strips to reach "gpt-4o" but budget is 4.
        // After 4 strips: "gpt-4o-z" -- no strict match.
        // Prefix fallback: longest common prefix with any model is 7 chars
        // ("gpt-4o-" with gpt-4o-audio-preview) which is below the 8-char minimum.
        let cost = table.estimate_cost(
            "openai",
            Some("gpt-4o-z-z-z-z-z"),
            Some(1_000_000),
            Some(500_000),
        );
        assert_eq!(
            cost, 0.0,
            "too many segments should exhaust strip budget and prefix too short"
        );
    }

    #[test]
    fn common_prefix_len_basic() {
        assert_eq!(common_prefix_len("abc", "abd"), 2);
        assert_eq!(common_prefix_len("abc", "abc"), 3);
        assert_eq!(common_prefix_len("abc", "xyz"), 0);
        assert_eq!(common_prefix_len("", "abc"), 0);
    }

    #[test]
    fn extract_trailing_version_basic() {
        assert_eq!(extract_trailing_version("claude-opus-4-6"), Some(6));
        assert_eq!(extract_trailing_version("claude-opus-4-0"), Some(0));
        assert_eq!(extract_trailing_version("gpt-4o"), None);
        assert_eq!(extract_trailing_version("model"), None);
    }

    #[test]
    fn oversized_model_string_rejected() {
        let table = PricingTable::load();
        // A multi-KB model string must be rejected before any allocation.
        let huge = "claude-sonnet-4-".to_string() + &"x".repeat(8192);
        let cost = table.estimate_cost("anthropic", Some(&huge), Some(1000), Some(500));
        assert_eq!(cost, 0.0, "oversized model string should be rejected");
    }
}
