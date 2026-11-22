//! Registry reads selected from the built-in package service definitions.
use capsem_config::{
    environment::{EnvironmentAccessRule, EnvironmentPackages},
    setting_definitions, CompiledSecurityRule, SecurityRuleAction, SettingDef, SettingType,
    MANAGED_PERMISSION_PRIORITY,
};
use std::collections::BTreeSet;

pub(super) fn selected(packages: &EnvironmentPackages) -> &[String] {
    if packages.allows_package_manager {
        &packages.registries
    } else {
        &[]
    }
}

fn reviewed(registries: &[String]) -> Result<Vec<(&str, SettingDef)>, String> {
    if registries.is_empty() {
        return Ok(Vec::new());
    }
    let definitions = setting_definitions();
    let mut seen = BTreeSet::new();
    registries
        .iter()
        .map(|id| {
            if !seen.insert(id) {
                return Err("duplicate environment package registry".into());
            }
            let key = format!("security.services.registry.{id}.allow");
            let definition = definitions
                .iter()
                .find(|definition| definition.id == key && definition.setting_type == SettingType::Bool)
                .ok_or_else(|| format!("unknown environment package registry: {id}"))?;
            let permissions = definition
                .metadata
                .rules
                .get("default")
                .ok_or("package registry lacks a reviewed read rule")?;
            if !permissions.get || permissions.path.is_some() || definition.metadata.domains.is_empty() {
                return Err("package registry lacks a reviewed read rule".into());
            }
            Ok((id.as_str(), definition.clone()))
        })
        .collect()
}

pub(super) fn validate(packages: &EnvironmentPackages) -> Result<(), String> {
    reviewed(selected(packages)).map(|_| ())
}

pub(super) fn compile(registries: &[String]) -> Result<Vec<CompiledSecurityRule>, String> {
    reviewed(registries)?
        .into_iter()
        .map(|(id, definition)| {
            let permissions = &definition.metadata.rules["default"];
            let domains = if permissions.domains.is_empty() {
                &definition.metadata.domains
            } else {
                &permissions.domains
            };
            let hosts = crate::net::policy_config::host_condition(domains)
                .map_err(|_| "invalid reviewed package registry domains")?;
            let mut rule = CompiledSecurityRule::from_environment_access(
                id,
                &EnvironmentAccessRule {
                    name: definition.name,
                    action: SecurityRuleAction::Allow,
                    condition: format!(r#"http.method == "GET" && ({hosts})"#),
                    enabled: true,
                    priority: Some(MANAGED_PERMISSION_PRIORITY),
                },
            )?;
            rule.rule_id = format!("environment.packages.{id}");
            rule.namespace = "environment.packages".into();
            Ok(rule)
        })
        .collect()
}
