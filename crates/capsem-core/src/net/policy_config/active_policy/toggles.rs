//! Service toggles (`security.services.*.allow`, `repository.providers.*.allow`)
//! as rules. HTTP is allowed by default, so an enabled toggle needs nothing; one
//! a user or corp turned off blocks its domains: those its `.domains` setting names, or
//! the registry's own when that setting is empty. The rule belongs to whoever
//! set the effective value, so a corp decision is a corp rule the user cannot
//! override, and a user's is a user rule just ahead of the default.

use std::collections::BTreeMap;

use capsem_config::{
    SecurityRule, SecurityRuleAction, SecurityRulePriority, SettingType, SettingValue, CORP_PRIORITY_MAX,
    USER_PRIORITY_MAX,
};

use super::super::types::SettingsFile;

type Rules = BTreeMap<String, SecurityRule>;

/// The block rules of every disabled toggle, as (user rules, corp rules).
pub(super) fn toggle_block_rules(settings: &SettingsFile, corp: &SettingsFile) -> Result<(Rules, Rules), String> {
    let (mut user, mut corp_rules) = (Rules::new(), Rules::new());
    for def in capsem_config::setting_definitions() {
        let Some(prefix) = def.id.strip_suffix(".allow") else {
            continue;
        };
        if def.setting_type != SettingType::Bool || def.metadata.domains.is_empty() {
            continue;
        }
        // Only a toggle someone set is enforced: several registry defaults are
        // off (GitLab, Bing, DuckDuckGo) while HTTP has always reached them,
        // and a default must not start blocking what nobody chose to block.
        let corp_set = corp.settings.get(&def.id).map(|entry| &entry.value);
        let enabled = corp_set.or_else(|| settings.settings.get(&def.id).map(|entry| &entry.value));
        if !matches!(enabled, Some(SettingValue::Bool(false))) {
            continue;
        }
        let domains_id = format!("{prefix}.domains");
        let listed =
            [corp, settings]
                .iter()
                .find_map(|file| match file.settings.get(&domains_id).map(|entry| &entry.value) {
                    Some(SettingValue::Text(text)) if !text.trim().is_empty() => Some(text.clone()),
                    _ => None,
                });
        let domains: Vec<String> = match listed {
            Some(text) => text.split(',').map(|domain| domain.trim().to_string()).collect(),
            None => def.metadata.domains.clone(),
        };
        let condition =
            host_condition(&domains).map_err(|bad| format!("{domains_id}: {bad:?} is not a domain pattern"))?;
        let id = format!("toggle_{}", prefix.replace('.', "_"));
        let (rules, priority) = match corp_set {
            Some(_) => (&mut corp_rules, CORP_PRIORITY_MAX),
            None => (&mut user, USER_PRIORITY_MAX),
        };
        rules.insert(
            id.clone(),
            SecurityRule {
                name: id,
                action: SecurityRuleAction::Block,
                condition,
                enabled: true,
                detection_level: None,
                priority: Some(SecurityRulePriority::Explicit(priority)),
                corp_locked: corp_set.is_some(),
                reason: Some(format!("{} is turned off", def.name)),
                managed: None,
                plugin_config: BTreeMap::new(),
            },
        );
    }
    Ok((user, corp_rules))
}

/// `http.host` matching any of `domains`; `*.example.com` matches its
/// subdomains. Only hostname characters reach the condition, so a domains
/// setting can never become rule syntax.
fn host_condition(domains: &[String]) -> Result<String, String> {
    let mut clauses = Vec::with_capacity(domains.len());
    for domain in domains {
        let (wildcard, host) = match domain.strip_prefix("*.") {
            Some(host) => (true, host),
            None => (false, domain.as_str()),
        };
        let labels: Vec<&str> = host.split('.').collect();
        let valid = labels.len() >= 2
            && labels.iter().all(|label| {
                !label.is_empty()
                    && label.len() <= 63
                    && label
                        .bytes()
                        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
            });
        if !valid {
            return Err(domain.clone());
        }
        clauses.push(if wildcard {
            format!(r#"http.host.endsWith(".{host}")"#)
        } else {
            format!(r#"http.host == "{host}""#)
        });
    }
    Ok(clauses.join(" || "))
}

#[cfg(test)]
mod tests;
