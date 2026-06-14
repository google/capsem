/// Model pricing: estimates cost per API call using bundled pricing data
/// from pydantic/genai-prices. Update via `just update_prices`.
use serde::Deserialize;

/// Embedded pricing data (updated via `just update_prices`).
const PRICING_JSON: &str = include_str!("../../../../../config/data/genai-prices.json");

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
        usage_details: &std::collections::BTreeMap<String, u64>,
    ) -> f64 {
        // Reject oversized model strings before any allocation. Real model
        // names are well under 128 bytes; anything larger is garbage or an
        // attempted DoS via the fuzzy-match `.to_string()` clone below.
        const MAX_MODEL_LEN: usize = 128;

        let model_str = match model {
            Some(m) if !m.is_empty() && m.len() <= MAX_MODEL_LEN => m,
            _ => return 0.0,
        };

        // Subtract cached tokens from input: cached tokens are typically
        // priced much lower (10-25% of input price). We conservatively
        // exclude them from input cost entirely (underestimates slightly).
        let raw_input = input_tokens.unwrap_or(0);
        let cache_read = usage_details.get("cache_read").copied().unwrap_or(0);
        let effective_input = raw_input.saturating_sub(cache_read) as f64;
        let output = output_tokens.unwrap_or(0) as f64;

        if effective_input == 0.0 && output == 0.0 {
            return 0.0;
        }

        let prov = match self.providers.iter().find(|p| p.id == provider) {
            Some(p) => p,
            None => return 0.0,
        };

        // Pass 1: strict match
        if let Some(cost) = Self::try_strict_match(prov, model_str, effective_input, output) {
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
                    if let Some(cost) =
                        Self::try_strict_match(prov, &candidate, effective_input, output)
                    {
                        return cost;
                    }
                }
                _ => break,
            }
        }

        // Pass 3: longest common prefix match (min 8 chars shared)
        if let Some(cost) = Self::try_prefix_match(prov, model_str, effective_input, output) {
            return cost;
        }

        0.0
    }

    /// Try strict match against all models in a provider.
    fn try_strict_match(prov: &ProviderData, model: &str, input: f64, output: f64) -> Option<f64> {
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
    fn try_prefix_match(prov: &ProviderData, model: &str, input: f64, output: f64) -> Option<f64> {
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
                return Some(input * input_rate / 1_000_000.0 + output * output_rate / 1_000_000.0);
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
mod tests;
