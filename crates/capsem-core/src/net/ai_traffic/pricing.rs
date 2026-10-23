/// Model pricing: estimates cost per API call using bundled pricing data
/// from pydantic/genai-prices. Update via `just update-prices`.
use serde::Deserialize;

/// Embedded pricing data (updated via `just update-prices`).
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
    #[serde(default)]
    cache_read_mtok: PriceValue,
    #[serde(default)]
    cache_write_mtok: PriceValue,
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
    fn rate(&self) -> Option<f64> {
        let value = match self {
            Self::Flat(v) => *v,
            Self::Tiered { base } => *base,
            Self::Missing => return None,
        };
        (value.is_finite() && value >= 0.0).then_some(value)
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

        let raw: Vec<RawProvider> = serde_json::from_str(PRICING_JSON).expect("genai-prices.json parse failed");

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
    /// Unknown models, missing usage and unsupported cache rates remain unknown.
    ///
    /// Uses only the upstream `match` clauses from pydantic/genai-prices. If
    /// the bundled ledger does not claim the model, Capsem does not guess.
    pub fn estimate_cost(
        &self,
        provider: &str,
        model: Option<&str>,
        input_tokens: Option<u64>,
        output_tokens: Option<u64>,
        usage_details: &std::collections::BTreeMap<String, u64>,
    ) -> Option<f64> {
        const MAX_MODEL_LEN: usize = 128;
        let model = model.filter(|m| !m.is_empty() && m.len() <= MAX_MODEL_LEN)?;
        let input = input_tokens?;
        let output = output_tokens?;
        let read = usage_details.get("cache_read").copied().unwrap_or(0);
        let write = usage_details.get("cache_write").copied().unwrap_or(0);
        // The bundled table has no duration-specific write rate. Never guess it.
        if usage_details.get("cache_write_1h").copied().unwrap_or(0) > 0 {
            return None;
        }
        let provider = self.providers.iter().find(|p| p.id == provider)?;
        let price = provider
            .models
            .iter()
            .find(|m| m.match_rule.matches(model))?
            .prices
            .price()?;
        let base = input.checked_sub(read)?.checked_sub(write)?;
        let component = |tokens: u64, rate: &PriceValue| -> Option<f64> {
            if tokens == 0 {
                Some(0.0)
            } else {
                Some(tokens as f64 * rate.rate()? / 1_000_000.0)
            }
        };
        Some(
            component(base, &price.input_mtok)?
                + component(output, &price.output_mtok)?
                + component(read, &price.cache_read_mtok)?
                + component(write, &price.cache_write_mtok)?,
        )
    }
}

#[cfg(test)]
mod tests;
