use crate::environment::EnvironmentRuleParameter as Value;
use capsem_proto::{
    ENVIRONMENT_MAX_CONFIGURATION_ENTRIES, ENVIRONMENT_MAX_CONFIGURATION_TEXT_BYTES, ENVIRONMENT_MAX_RULE_BYTES,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum CatalogRuleParameter {
    Boolean {
        default: bool,
    },
    Integer {
        default: i64,
        min: i64,
        max: i64,
    },
    Text {
        default: String,
        max_bytes: usize,
        #[serde(default)]
        allow_empty: bool,
    },
    Choice {
        default: String,
        choices: Vec<String>,
    },
}

fn text(value: &str, max: usize, allow_empty: bool) -> bool {
    (allow_empty || !value.is_empty())
        && value.len() <= max
        && !value
            .chars()
            .any(|ch| ch.is_control() && !matches!(ch, '\n' | '\r' | '\t'))
}
impl CatalogRuleParameter {
    pub fn validate(&self) -> Result<(), &'static str> {
        let valid = match self {
            Self::Boolean { .. } => true,
            Self::Integer { default, min, max } => min <= max && (*min..=*max).contains(default),
            Self::Text {
                default,
                max_bytes,
                allow_empty,
            } => {
                (1..=ENVIRONMENT_MAX_CONFIGURATION_TEXT_BYTES).contains(max_bytes)
                    && text(default, *max_bytes, *allow_empty)
            }
            Self::Choice { default, choices } => {
                let mut unique = BTreeSet::new();
                !choices.is_empty()
                    && choices.len() <= ENVIRONMENT_MAX_CONFIGURATION_ENTRIES
                    && choices.contains(default)
                    && choices.iter().all(|value| {
                        text(value, ENVIRONMENT_MAX_CONFIGURATION_TEXT_BYTES, false) && unique.insert(value)
                    })
            }
        };
        if valid {
            Ok(())
        } else {
            Err("invalid catalog rule parameter descriptor")
        }
    }

    fn literal(&self, value: Option<&Value>) -> Result<String, &'static str> {
        let result = match (self, value) {
            (Self::Boolean { default }, None) => crate::condition_string(&default.to_string()),
            (Self::Boolean { .. }, Some(Value::Boolean(value))) => crate::condition_string(&value.to_string()),
            (Self::Integer { default, .. }, None) => crate::condition_string(&default.to_string()),
            (Self::Integer { min, max, .. }, Some(Value::Integer(value))) if (*min..=*max).contains(value) => {
                crate::condition_string(&value.to_string())
            }
            (Self::Text { default, .. }, None) | (Self::Choice { default, .. }, None) => {
                crate::condition_string(default)
            }
            (
                Self::Text {
                    max_bytes, allow_empty, ..
                },
                Some(Value::Text(value)),
            ) if text(value, *max_bytes, *allow_empty) => crate::condition_string(value),
            (Self::Choice { choices, .. }, Some(Value::Text(value))) if choices.contains(value) => {
                crate::condition_string(value)
            }
            _ => return Err("catalog rule parameter has an invalid type or value"),
        };
        Ok(result)
    }
}

/// Every {{parameter}} occupies a complete literal in the existing condition
/// language. Boolean/numeric facts use that matcher's textual representation.
/// Substitutions are escaped once, never recursively expanded or executed.
pub(super) fn render(
    template: &str,
    parameters: &BTreeMap<String, CatalogRuleParameter>,
    values: &BTreeMap<String, Value>,
) -> Result<String, &'static str> {
    let mut output = String::new();
    let mut remaining = template;
    let mut used = BTreeSet::new();
    while let Some(start) = remaining.find("{{") {
        let (prefix, token) = remaining.split_at(start);
        if !outside_quoted_literal(prefix) {
            return Err("catalog rule parameters must occupy unquoted literal positions");
        }
        let end = token.find("}}").ok_or("unterminated catalog rule parameter")?;
        let name = &token[2..end];
        let parameter = parameters.get(name).ok_or("unknown catalog rule template parameter")?;
        let literal = parameter.literal(values.get(name))?;
        if output.len() + prefix.len() + literal.len() > ENVIRONMENT_MAX_RULE_BYTES {
            return Err("expanded catalog rule exceeds its condition budget");
        }
        output.push_str(prefix);
        output.push_str(&literal);
        used.insert(name);
        remaining = &token[end + 2..];
    }
    if used.len() != parameters.len() {
        return Err("unused catalog rule parameter descriptor");
    }
    if output.len() + remaining.len() > ENVIRONMENT_MAX_RULE_BYTES {
        return Err("expanded catalog rule exceeds its condition budget");
    }
    output.push_str(remaining);
    Ok(output)
}

fn outside_quoted_literal(prefix: &str) -> bool {
    let mut quote = None;
    let mut escaped = false;
    for ch in prefix.chars() {
        if let Some(active) = quote {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == active {
                quote = None;
            }
        } else if matches!(ch, '\'' | '"') {
            quote = Some(ch);
        }
    }
    quote.is_none()
}
