/// Model pricing: estimates cost per API call using bundled pricing data
/// from pydantic/genai-prices. See config/data/README.md for its producer.
use serde::Deserialize;

/// Embedded canonical pricing data.
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
    Equals {
        equals: String,
    },
    StartsWith {
        starts_with: String,
    },
    EndsWith {
        ends_with: String,
    },
    Contains {
        contains: String,
    },
    Regex {
        #[serde(deserialize_with = "read_regex")]
        regex: regex::Regex,
    },
    Or {
        or: Vec<MatchClause>,
    },
}

impl MatchClause {
    fn matches(&self, model: &str) -> bool {
        match self {
            MatchClause::Equals { equals } => model.eq_ignore_ascii_case(equals),
            MatchClause::StartsWith { starts_with } => model
                .get(..starts_with.len())
                .is_some_and(|prefix| prefix.eq_ignore_ascii_case(starts_with)),
            MatchClause::EndsWith { ends_with } => model
                .get(model.len().saturating_sub(ends_with.len())..)
                .is_some_and(|suffix| suffix.eq_ignore_ascii_case(ends_with)),
            MatchClause::Contains { contains } => {
                contains.is_empty()
                    || model
                        .as_bytes()
                        .windows(contains.len())
                        .any(|part| part.eq_ignore_ascii_case(contains.as_bytes()))
            }
            MatchClause::Regex { regex } => regex.is_match(model),
            MatchClause::Or { or } => or.iter().any(|c| c.matches(model)),
        }
    }
}

/// Price spec: either a flat `{ input_mtok, output_mtok }` or a conditional
/// list of date-constrained overrides. The last applicable entry takes precedence.
#[derive(Deserialize)]
#[serde(untagged)]
enum PriceSpec {
    Direct(ModelPrice),
    Conditional(Vec<ConditionalPrice>),
}

impl PriceSpec {
    fn price(&self, date: time::Date) -> Option<&ModelPrice> {
        match self {
            PriceSpec::Direct(p) => Some(p),
            PriceSpec::Conditional(list) => list
                .iter()
                .rev()
                .find(|entry| {
                    entry
                        .constraint
                        .as_ref()
                        .is_none_or(|constraint| constraint.start_date.0 <= date)
                })
                .map(|entry| &entry.prices),
        }
    }
}

#[derive(Deserialize)]
struct ConditionalPrice {
    #[serde(default)]
    constraint: Option<PriceConstraint>,
    prices: ModelPrice,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PriceConstraint {
    start_date: PriceDate,
}

struct PriceDate(time::Date);

impl<'de> Deserialize<'de> for PriceDate {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        use serde::de::Error;
        let text = String::deserialize(deserializer)?;
        if text.len() != 10
            || !text.bytes().enumerate().all(|(index, byte)| match index {
                4 | 7 => byte == b'-',
                _ => byte.is_ascii_digit(),
            })
        {
            return Err(D::Error::custom("expected YYYY-MM-DD pricing date"));
        }
        let year = text[..4].parse().map_err(D::Error::custom)?;
        let month = text[5..7]
            .parse::<u8>()
            .map_err(D::Error::custom)?
            .try_into()
            .map_err(D::Error::custom)?;
        let day = text[8..].parse().map_err(D::Error::custom)?;
        time::Date::from_calendar_date(year, month, day)
            .map(Self)
            .map_err(D::Error::custom)
    }
}

fn read_regex<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<regex::Regex, D::Error> {
    use serde::de::Error;
    regex::RegexBuilder::new(&String::deserialize(deserializer)?)
        .case_insensitive(true)
        .build()
        .map_err(D::Error::custom)
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
        tiers: Vec<PriceTier>,
    },
    #[default]
    Missing,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PriceTier {
    start: u64,
    price: f64,
}

impl PriceValue {
    fn rate(&self, input_tokens: u64) -> Option<f64> {
        let value = match self {
            Self::Flat(v) => *v,
            Self::Tiered { base, tiers } => tiers
                .iter()
                .filter(|tier| input_tokens > tier.start)
                .max_by_key(|tier| tier.start)
                .map_or(*base, |tier| tier.price),
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
        self.estimate_cost_at(
            provider,
            model,
            input_tokens,
            output_tokens,
            usage_details,
            std::time::SystemTime::now(),
        )
    }

    /// Price at the recorded event's UTC date; historical calls never borrow today's rates.
    pub fn estimate_cost_at(
        &self,
        provider: &str,
        model: Option<&str>,
        input_tokens: Option<u64>,
        output_tokens: Option<u64>,
        usage_details: &std::collections::BTreeMap<String, u64>,
        timestamp: std::time::SystemTime,
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
            .price(time::OffsetDateTime::from(timestamp).date())?;
        let base = input.checked_sub(read)?.checked_sub(write)?;
        let component = |tokens: u64, rate: &PriceValue| -> Option<f64> {
            if tokens == 0 {
                Some(0.0)
            } else {
                Some(tokens as f64 * rate.rate(input)? / 1_000_000.0)
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
