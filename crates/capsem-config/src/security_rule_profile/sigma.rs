use std::collections::BTreeMap;

use serde::Deserialize;

use super::{validate_security_event_field, DetectionLevel, SecurityRule, SecurityRuleAction, SecurityRulePriority};
use crate::validation::validate_identifier;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct SigmaRule {
    title: String,
    #[serde(default)]
    id: Option<String>,
    #[serde(default, rename = "status")]
    _status: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default, rename = "author")]
    _author: Option<String>,
    #[serde(default, rename = "date")]
    _date: Option<String>,
    logsource: SigmaLogsource,
    detection: BTreeMap<String, serde_yaml::Value>,
    #[serde(default, rename = "falsepositives")]
    _falsepositives: Vec<String>,
    level: DetectionLevel,
    #[serde(default)]
    capsem: SigmaCapsem,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SigmaLogsource {
    product: String,
    service: String,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct SigmaCapsem {
    #[serde(default)]
    action: Option<SecurityRuleAction>,
    #[serde(default)]
    reason: Option<String>,
    #[serde(default)]
    priority: Option<SecurityRulePriority>,
    #[serde(default)]
    corp_locked: bool,
}

impl SigmaRule {
    pub(super) fn into_security_rule(self) -> Result<(String, SecurityRule), String> {
        if self.logsource.product != "capsem" || self.logsource.service != "security_event" {
            return Err(format!(
                "Sigma rule '{}' must use logsource product=capsem service=security_event",
                self.title
            ));
        }
        let condition = self
            .detection
            .get("condition")
            .and_then(serde_yaml::Value::as_str)
            .ok_or_else(|| format!("Sigma rule '{}' missing detection.condition", self.title))?;
        let selections = self.selection_clauses()?;
        let condition = sigma_condition_to_security_event_match(condition, &selections)?;
        let rule_key = derive_sigma_rule_key(&self.title)?;
        let rule = SecurityRule {
            name: rule_key.clone(),
            action: self.capsem.action.unwrap_or(SecurityRuleAction::Allow),
            condition,
            enabled: true,
            detection_level: Some(self.level),
            priority: self.capsem.priority,
            corp_locked: self.capsem.corp_locked,
            reason: self
                .capsem
                .reason
                .or(self.description)
                .or_else(|| self.id.map(|id| format!("Sigma rule {id}"))),
            managed: None,
            plugin_config: BTreeMap::new(),
        };
        rule.validate(&format!("profiles.rules.{rule_key}"))?;
        Ok((rule_key, rule))
    }

    fn selection_clauses(&self) -> Result<BTreeMap<String, SigmaSelectionClause>, String> {
        let mut selections = BTreeMap::new();
        for (name, value) in &self.detection {
            if name == "condition" {
                continue;
            }
            validate_identifier("Sigma selection id", name)?;
            let mapping = value
                .as_mapping()
                .ok_or_else(|| format!("Sigma selection '{name}' must be a mapping"))?;
            let mut positive = Vec::new();
            let mut negative = Vec::new();
            for (field, expected) in mapping {
                let field = field
                    .as_str()
                    .ok_or_else(|| format!("Sigma selection '{name}' has a non-string field"))?;
                validate_security_event_field(field)?;
                let clause = sigma_field_clause(field, expected)?;
                positive.push(clause.positive);
                negative.push(clause.negative);
            }
            if positive.is_empty() {
                return Err(format!("Sigma selection '{name}' must not be empty"));
            }
            selections.insert(
                name.clone(),
                SigmaSelectionClause {
                    positive: positive.join(" && "),
                    negative: negative.join(" || "),
                },
            );
        }
        Ok(selections)
    }
}

#[derive(Debug, Clone)]
struct SigmaSelectionClause {
    positive: String,
    negative: String,
}

fn sigma_condition_to_security_event_match(
    condition: &str,
    selections: &BTreeMap<String, SigmaSelectionClause>,
) -> Result<String, String> {
    let tokens = tokenize_sigma_condition(condition)?;
    let mut output = Vec::new();
    let mut negate_next = false;
    for token in tokens {
        match token.as_str() {
            "and" => output.push("&&".to_string()),
            "or" => output.push("||".to_string()),
            "not" => {
                if negate_next {
                    return Err("Sigma condition has repeated 'not'".to_string());
                }
                negate_next = true;
            }
            "(" | ")" => {
                return Err("Sigma condition grouping is not supported yet".to_string());
            }
            name => {
                let clause = selections
                    .get(name)
                    .ok_or_else(|| format!("Sigma condition references unknown selection '{name}'"))?;
                if negate_next {
                    output.push(clause.negative.clone());
                    negate_next = false;
                } else {
                    output.push(clause.positive.clone());
                }
            }
        }
    }
    if negate_next {
        return Err("Sigma condition ends with 'not'".to_string());
    }
    Ok(output.join(" "))
}

fn tokenize_sigma_condition(condition: &str) -> Result<Vec<String>, String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    for ch in condition.chars() {
        match ch {
            '(' | ')' => {
                if !current.is_empty() {
                    tokens.push(std::mem::take(&mut current));
                }
                tokens.push(ch.to_string());
            }
            ch if ch.is_whitespace() => {
                if !current.is_empty() {
                    tokens.push(std::mem::take(&mut current));
                }
            }
            ch if ch == '_' || ch.is_ascii_alphanumeric() => current.push(ch),
            _ => {
                return Err(format!(
                    "unsupported Sigma condition token near '{ch}' in '{condition}'"
                ));
            }
        }
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    if tokens.is_empty() {
        Err("Sigma condition must not be empty".to_string())
    } else {
        Ok(tokens)
    }
}

fn sigma_field_clause(field: &str, expected: &serde_yaml::Value) -> Result<SigmaSelectionClause, String> {
    if let Some(values) = expected.as_sequence() {
        if values.is_empty() {
            return Err(format!("Sigma field '{field}' sequence must not be empty"));
        }
        let mut positive = Vec::new();
        let mut negative = Vec::new();
        for value in values {
            positive.push(sigma_scalar_compare(field, "==", value)?);
            negative.push(sigma_scalar_compare(field, "!=", value)?);
        }
        return Ok(SigmaSelectionClause {
            positive: positive.join(" || "),
            negative: negative.join(" && "),
        });
    }
    Ok(SigmaSelectionClause {
        positive: sigma_scalar_compare(field, "==", expected)?,
        negative: sigma_scalar_compare(field, "!=", expected)?,
    })
}

fn sigma_scalar_compare(field: &str, operator: &str, expected: &serde_yaml::Value) -> Result<String, String> {
    let expected = sigma_scalar_to_string(expected)
        .ok_or_else(|| format!("Sigma field '{field}' value must be a scalar or sequence"))?;
    Ok(format!("{field} {operator} {}", cel_string_literal(&expected)))
}

fn sigma_scalar_to_string(value: &serde_yaml::Value) -> Option<String> {
    match value {
        serde_yaml::Value::String(value) => Some(value.clone()),
        serde_yaml::Value::Number(value) => Some(value.to_string()),
        serde_yaml::Value::Bool(value) => Some(value.to_string()),
        _ => None,
    }
}

fn cel_string_literal(value: &str) -> String {
    serde_json::to_string(value).expect("string literal serialization cannot fail")
}

fn derive_sigma_rule_key(title: &str) -> Result<String, String> {
    let mut output = String::new();
    let mut last_was_sep = true;
    for ch in title.chars() {
        if ch.is_ascii_alphanumeric() {
            output.push(ch.to_ascii_lowercase());
            last_was_sep = false;
        } else if !last_was_sep {
            output.push('_');
            last_was_sep = true;
        }
    }
    while output.ends_with('_') {
        output.pop();
    }
    if output.len() > 64 {
        output.truncate(64);
        while output.ends_with('_') {
            output.pop();
        }
    }
    validate_identifier("Sigma-derived rule id", &output)?;
    Ok(output)
}
