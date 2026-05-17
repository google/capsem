//! Corp directives: org-deployed overrides that modify the
//! materialized effective settings after the profile inheritance
//! chain has been merged. Slice 6.4 lands `add` / `remove` /
//! `replace`; `lock` / `forbid` arrive in slice 6.5.
//!
//! Directives source: [`crate::settings_profiles::ServiceSettings::corp_directives`].
//! They are applied by [`apply_corp_directives`] against the
//! merged [`super::Profile`], emitting one trace event per
//! directive into the resolver trace.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::{
    validation_error, AiProviderConfig, CapabilityMode, McpConnectorConfig, Profile, ProfileRule,
    ResolverTrace, ResolverTraceEvent, ResolverTraceOperation, ResolverTraceSourceKind, Result,
    SettingsProfilesError,
};
use super::{RULE_CATCH_ALL_PRIORITY, RULE_CORP_PRIORITY_RANGE};

/// Priority range allowed for rules authored via
/// `corp_directives`. Matches the corp-tier semantics:
/// negative values are corp-exclusive, `0` is the
/// toggle-derived slot which corp can also legitimately use to
/// override system-generated rules. Manual authoring outside
/// this range -- or at the reserved catch-all priority -- is
/// rejected.
const CORP_DIRECTIVE_PRIORITY_RANGE: std::ops::RangeInclusive<i32> = -1000..=0;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct CorpDirective {
    pub operation: CorpDirectiveOperation,
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<toml::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum CorpDirectiveOperation {
    Add,
    Remove,
    Replace,
    /// Set the value at `path` AND stamp the path as
    /// immutable. Any subsequent corp directive that targets
    /// the same path raises [`SettingsProfilesError::ResolverViolation`].
    Lock,
    /// Remove the entry at `path` (if present) AND stamp the
    /// path as forbidden. Any subsequent corp directive that
    /// would restore the entry raises
    /// [`SettingsProfilesError::ResolverViolation`].
    Forbid,
}

impl CorpDirective {
    pub fn validate(&self, path_prefix: &str) -> Result<()> {
        if self.path.trim().is_empty() {
            validation_error(
                &format!("{path_prefix}.path"),
                "corp directive path cannot be empty",
            )?;
        }
        let needs_value = matches!(
            self.operation,
            CorpDirectiveOperation::Add
                | CorpDirectiveOperation::Replace
                | CorpDirectiveOperation::Lock
        );
        if needs_value && self.value.is_none() {
            validation_error(
                &format!("{path_prefix}.value"),
                "add/replace/lock directives require a value",
            )?;
        }
        if !needs_value && self.value.is_some() {
            validation_error(
                &format!("{path_prefix}.value"),
                "remove/forbid directives must not carry a value",
            )?;
        }
        Ok(())
    }
}

/// Per-target-kind record of which keys a corp directive
/// touched. The resolver consults this when building per-rule
/// provenance: a corp-touched rule attributes to source
/// `corp` rather than the chain contributor.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct CorpOverrides {
    /// Rule name -> rule type, for entries the corp directives
    /// added or replaced. Removals do not appear (the rule is
    /// gone from the merged profile).
    pub rules: BTreeMap<String, String>,
    pub connectors: std::collections::BTreeSet<String>,
    pub providers: std::collections::BTreeSet<String>,
    pub capability_fields: std::collections::BTreeSet<String>,
    /// Dotted paths stamped immutable by a `lock` directive.
    /// Any later corp directive targeting one of these paths
    /// raises `SettingsProfilesError::ResolverViolation`.
    pub locked_paths: std::collections::BTreeSet<String>,
    /// Dotted paths stamped denied by a `forbid` directive.
    /// Any later corp directive that would restore the entry
    /// raises `SettingsProfilesError::ResolverViolation`.
    pub forbidden_paths: std::collections::BTreeSet<String>,
}

pub fn apply_corp_directives(
    profile: &mut Profile,
    directives: &[CorpDirective],
    trace: &mut ResolverTrace,
) -> Result<CorpOverrides> {
    let mut overrides = CorpOverrides::default();
    for (idx, directive) in directives.iter().enumerate() {
        apply_corp_directive(profile, directive, trace, &mut overrides, idx)?;
    }
    Ok(overrides)
}

fn apply_corp_directive(
    profile: &mut Profile,
    directive: &CorpDirective,
    trace: &mut ResolverTrace,
    overrides: &mut CorpOverrides,
    directive_index: usize,
) -> Result<()> {
    if overrides.locked_paths.contains(&directive.path) {
        let message = "path is locked by an earlier corp directive";
        emit_reject_event(trace, directive, directive_index, message);
        return Err(violation(directive, directive_index, message));
    }
    let restoring = matches!(
        directive.operation,
        CorpDirectiveOperation::Add
            | CorpDirectiveOperation::Replace
            | CorpDirectiveOperation::Lock
    );
    if restoring && overrides.forbidden_paths.contains(&directive.path) {
        let message = "path is forbidden by an earlier corp directive";
        emit_reject_event(trace, directive, directive_index, message);
        return Err(violation(directive, directive_index, message));
    }
    let segments: Vec<&str> = directive.path.split('.').collect();
    match segments.as_slice() {
        ["security", "rules", rule_type, rule_name] => apply_rule_directive(
            profile,
            rule_type,
            rule_name,
            directive,
            trace,
            overrides,
            directive_index,
        ),
        ["mcp", "connectors", name] => {
            apply_connector_directive(profile, name, directive, trace, overrides, directive_index)
        }
        ["ai", "providers", name] => {
            apply_provider_directive(profile, name, directive, trace, overrides, directive_index)
        }
        ["security", "capabilities", field] => {
            apply_capability_directive(profile, field, directive, trace, overrides, directive_index)
        }
        _ => Err(SettingsProfilesError::Validation {
            path: format!("corp_directives[{directive_index}].path"),
            message: format!(
                "unsupported corp directive path '{}': supported paths are \
                security.rules.<type>.<name>, mcp.connectors.<name>, \
                ai.providers.<name>, security.capabilities.<field>",
                directive.path
            ),
        }),
    }
}

fn apply_rule_directive(
    profile: &mut Profile,
    rule_type: &str,
    rule_name: &str,
    directive: &CorpDirective,
    trace: &mut ResolverTrace,
    overrides: &mut CorpOverrides,
    directive_index: usize,
) -> Result<()> {
    let rules = rules_for_type_mut(profile, rule_type, directive_index)?;
    match directive.operation {
        CorpDirectiveOperation::Add => {
            if rules.contains_key(rule_name) {
                return Err(SettingsProfilesError::Validation {
                    path: format!("corp_directives[{directive_index}].path"),
                    message: format!(
                        "add on existing key '{}'; use replace to override",
                        directive.path
                    ),
                });
            }
            let rule = parse_rule_for_directive(directive, directive_index)?;
            let after = serde_json::to_value(&rule).ok();
            rules.insert(rule_name.to_string(), rule);
            overrides
                .rules
                .insert(rule_name.to_string(), rule_type.to_string());
            push_corp_event(
                trace,
                directive,
                ResolverTraceOperation::Add,
                None,
                after,
                directive_index,
            );
        }
        CorpDirectiveOperation::Replace => {
            let rule = parse_rule_for_directive(directive, directive_index)?;
            let before = rules
                .get(rule_name)
                .and_then(|existing| serde_json::to_value(existing).ok());
            let after = serde_json::to_value(&rule).ok();
            rules.insert(rule_name.to_string(), rule);
            overrides
                .rules
                .insert(rule_name.to_string(), rule_type.to_string());
            push_corp_event(
                trace,
                directive,
                ResolverTraceOperation::Replace,
                before,
                after,
                directive_index,
            );
        }
        CorpDirectiveOperation::Remove => {
            let removed =
                rules
                    .remove(rule_name)
                    .ok_or_else(|| SettingsProfilesError::Validation {
                        path: format!("corp_directives[{directive_index}].path"),
                        message: format!("remove on missing key '{}'", directive.path),
                    })?;
            let before = serde_json::to_value(&removed).ok();
            push_corp_event(
                trace,
                directive,
                ResolverTraceOperation::Remove,
                before,
                None,
                directive_index,
            );
        }
        CorpDirectiveOperation::Lock => {
            let rule = parse_rule_for_directive(directive, directive_index)?;
            let before = rules
                .get(rule_name)
                .and_then(|existing| serde_json::to_value(existing).ok());
            let after = serde_json::to_value(&rule).ok();
            rules.insert(rule_name.to_string(), rule);
            overrides
                .rules
                .insert(rule_name.to_string(), rule_type.to_string());
            overrides.locked_paths.insert(directive.path.clone());
            push_corp_event_locked(
                trace,
                directive,
                ResolverTraceOperation::Lock,
                before,
                after,
                directive_index,
            );
        }
        CorpDirectiveOperation::Forbid => {
            let before = rules
                .remove(rule_name)
                .and_then(|existing| serde_json::to_value(&existing).ok());
            overrides.forbidden_paths.insert(directive.path.clone());
            push_corp_event(
                trace,
                directive,
                ResolverTraceOperation::Forbid,
                before,
                None,
                directive_index,
            );
        }
    }
    Ok(())
}

fn apply_connector_directive(
    profile: &mut Profile,
    name: &str,
    directive: &CorpDirective,
    trace: &mut ResolverTrace,
    overrides: &mut CorpOverrides,
    directive_index: usize,
) -> Result<()> {
    let connectors = &mut profile.mcp.connectors;
    match directive.operation {
        CorpDirectiveOperation::Add => {
            if connectors.contains_key(name) {
                return Err(SettingsProfilesError::Validation {
                    path: format!("corp_directives[{directive_index}].path"),
                    message: format!(
                        "add on existing key '{}'; use replace to override",
                        directive.path
                    ),
                });
            }
            let value =
                parse_value_as::<McpConnectorConfig>(directive, directive_index, "connector")?;
            let after = serde_json::to_value(&value).ok();
            connectors.insert(name.to_string(), value);
            overrides.connectors.insert(name.to_string());
            push_corp_event(
                trace,
                directive,
                ResolverTraceOperation::Add,
                None,
                after,
                directive_index,
            );
        }
        CorpDirectiveOperation::Replace => {
            let value =
                parse_value_as::<McpConnectorConfig>(directive, directive_index, "connector")?;
            let before = connectors
                .get(name)
                .and_then(|existing| serde_json::to_value(existing).ok());
            let after = serde_json::to_value(&value).ok();
            connectors.insert(name.to_string(), value);
            overrides.connectors.insert(name.to_string());
            push_corp_event(
                trace,
                directive,
                ResolverTraceOperation::Replace,
                before,
                after,
                directive_index,
            );
        }
        CorpDirectiveOperation::Remove => {
            let removed =
                connectors
                    .remove(name)
                    .ok_or_else(|| SettingsProfilesError::Validation {
                        path: format!("corp_directives[{directive_index}].path"),
                        message: format!("remove on missing key '{}'", directive.path),
                    })?;
            let before = serde_json::to_value(&removed).ok();
            push_corp_event(
                trace,
                directive,
                ResolverTraceOperation::Remove,
                before,
                None,
                directive_index,
            );
        }
        CorpDirectiveOperation::Lock => {
            let value =
                parse_value_as::<McpConnectorConfig>(directive, directive_index, "connector")?;
            let before = connectors
                .get(name)
                .and_then(|existing| serde_json::to_value(existing).ok());
            let after = serde_json::to_value(&value).ok();
            connectors.insert(name.to_string(), value);
            overrides.connectors.insert(name.to_string());
            overrides.locked_paths.insert(directive.path.clone());
            push_corp_event_locked(
                trace,
                directive,
                ResolverTraceOperation::Lock,
                before,
                after,
                directive_index,
            );
        }
        CorpDirectiveOperation::Forbid => {
            let before = connectors
                .remove(name)
                .and_then(|existing| serde_json::to_value(&existing).ok());
            overrides.forbidden_paths.insert(directive.path.clone());
            push_corp_event(
                trace,
                directive,
                ResolverTraceOperation::Forbid,
                before,
                None,
                directive_index,
            );
        }
    }
    Ok(())
}

fn apply_provider_directive(
    profile: &mut Profile,
    name: &str,
    directive: &CorpDirective,
    trace: &mut ResolverTrace,
    overrides: &mut CorpOverrides,
    directive_index: usize,
) -> Result<()> {
    let providers = &mut profile.ai.providers;
    match directive.operation {
        CorpDirectiveOperation::Add => {
            if providers.contains_key(name) {
                return Err(SettingsProfilesError::Validation {
                    path: format!("corp_directives[{directive_index}].path"),
                    message: format!(
                        "add on existing key '{}'; use replace to override",
                        directive.path
                    ),
                });
            }
            let value = parse_value_as::<AiProviderConfig>(directive, directive_index, "provider")?;
            let after = serde_json::to_value(&value).ok();
            providers.insert(name.to_string(), value);
            overrides.providers.insert(name.to_string());
            push_corp_event(
                trace,
                directive,
                ResolverTraceOperation::Add,
                None,
                after,
                directive_index,
            );
        }
        CorpDirectiveOperation::Replace => {
            let value = parse_value_as::<AiProviderConfig>(directive, directive_index, "provider")?;
            let before = providers
                .get(name)
                .and_then(|existing| serde_json::to_value(existing).ok());
            let after = serde_json::to_value(&value).ok();
            providers.insert(name.to_string(), value);
            overrides.providers.insert(name.to_string());
            push_corp_event(
                trace,
                directive,
                ResolverTraceOperation::Replace,
                before,
                after,
                directive_index,
            );
        }
        CorpDirectiveOperation::Remove => {
            let removed =
                providers
                    .remove(name)
                    .ok_or_else(|| SettingsProfilesError::Validation {
                        path: format!("corp_directives[{directive_index}].path"),
                        message: format!("remove on missing key '{}'", directive.path),
                    })?;
            let before = serde_json::to_value(&removed).ok();
            push_corp_event(
                trace,
                directive,
                ResolverTraceOperation::Remove,
                before,
                None,
                directive_index,
            );
        }
        CorpDirectiveOperation::Lock => {
            let value = parse_value_as::<AiProviderConfig>(directive, directive_index, "provider")?;
            let before = providers
                .get(name)
                .and_then(|existing| serde_json::to_value(existing).ok());
            let after = serde_json::to_value(&value).ok();
            providers.insert(name.to_string(), value);
            overrides.providers.insert(name.to_string());
            overrides.locked_paths.insert(directive.path.clone());
            push_corp_event_locked(
                trace,
                directive,
                ResolverTraceOperation::Lock,
                before,
                after,
                directive_index,
            );
        }
        CorpDirectiveOperation::Forbid => {
            let before = providers
                .remove(name)
                .and_then(|existing| serde_json::to_value(&existing).ok());
            overrides.forbidden_paths.insert(directive.path.clone());
            push_corp_event(
                trace,
                directive,
                ResolverTraceOperation::Forbid,
                before,
                None,
                directive_index,
            );
        }
    }
    Ok(())
}

fn apply_capability_directive(
    profile: &mut Profile,
    field: &str,
    directive: &CorpDirective,
    trace: &mut ResolverTrace,
    overrides: &mut CorpOverrides,
    directive_index: usize,
) -> Result<()> {
    let is_lock = matches!(directive.operation, CorpDirectiveOperation::Lock);
    if !matches!(
        directive.operation,
        CorpDirectiveOperation::Replace | CorpDirectiveOperation::Lock
    ) {
        return Err(SettingsProfilesError::Validation {
            path: format!("corp_directives[{directive_index}].operation"),
            message: format!(
                "security.capabilities.{field} only supports the 'replace' or 'lock' operations"
            ),
        });
    }
    let mode = parse_value_as::<CapabilityMode>(directive, directive_index, "capability mode")?;
    let caps = &mut profile.security.capabilities;
    let before = serde_json::to_value(&*caps).ok();
    let target = match field {
        "credential_brokerage" => &mut caps.credential_brokerage,
        "pii_detection" => &mut caps.pii_detection,
        "mcp_rag" => &mut caps.mcp_rag,
        "mcp_tools" => &mut caps.mcp_tools,
        "network_egress" => &mut caps.network_egress,
        "file_boundaries" => &mut caps.file_boundaries,
        "audit" => &mut caps.audit,
        _ => {
            return Err(SettingsProfilesError::Validation {
                path: format!("corp_directives[{directive_index}].path"),
                message: format!("unknown security.capabilities field '{field}'"),
            });
        }
    };
    *target = mode;
    overrides.capability_fields.insert(field.to_string());
    let after = serde_json::to_value(&profile.security.capabilities).ok();
    if is_lock {
        overrides.locked_paths.insert(directive.path.clone());
        push_corp_event_locked(
            trace,
            directive,
            ResolverTraceOperation::Lock,
            before,
            after,
            directive_index,
        );
    } else {
        push_corp_event(
            trace,
            directive,
            ResolverTraceOperation::Replace,
            before,
            after,
            directive_index,
        );
    }
    Ok(())
}

fn rules_for_type_mut<'a>(
    profile: &'a mut Profile,
    rule_type: &str,
    directive_index: usize,
) -> Result<&'a mut BTreeMap<String, ProfileRule>> {
    match rule_type {
        "mcp" => Ok(&mut profile.security.rules.mcp),
        "http" => Ok(&mut profile.security.rules.http),
        "dns" => Ok(&mut profile.security.rules.dns),
        "model" => Ok(&mut profile.security.rules.model),
        "hook" => Ok(&mut profile.security.rules.hook),
        _ => Err(SettingsProfilesError::Validation {
            path: format!("corp_directives[{directive_index}].path"),
            message: format!("unknown rule type '{rule_type}'"),
        }),
    }
}

fn parse_value_as<T: serde::de::DeserializeOwned>(
    directive: &CorpDirective,
    directive_index: usize,
    kind: &'static str,
) -> Result<T> {
    let value = directive
        .value
        .as_ref()
        .ok_or_else(|| SettingsProfilesError::Validation {
            path: format!("corp_directives[{directive_index}].value"),
            message: format!("missing value for {kind} directive"),
        })?;
    value
        .clone()
        .try_into::<T>()
        .map_err(|source| SettingsProfilesError::Parse {
            kind: "corp directive value",
            details: format!("{kind}: {source}"),
        })
}

/// Parse the directive value as a `ProfileRule`, then enforce
/// the corp-directive contract: rule shape must validate (so
/// `parse_value_as` alone isn't enough -- derived deserialize
/// doesn't run `ProfileRule::validate`), and priority must
/// fall in [`CORP_DIRECTIVE_PRIORITY_RANGE`]. Catch-all priority
/// (`1000`) is the system reservation; allowing corp to author
/// at it would let corp shadow the catch-all and is rejected.
fn parse_rule_for_directive(
    directive: &CorpDirective,
    directive_index: usize,
) -> Result<ProfileRule> {
    let rule = parse_value_as::<ProfileRule>(directive, directive_index, "rule")?;
    let path = format!("corp_directives[{directive_index}].value");
    rule.validate(&path)?;
    if !CORP_DIRECTIVE_PRIORITY_RANGE.contains(&rule.priority) {
        validation_error(
            &format!("corp_directives[{directive_index}].value.priority"),
            &format!(
                "corp directive rule priority must be in [{min}, {max}], got {value}",
                min = *CORP_DIRECTIVE_PRIORITY_RANGE.start(),
                max = *CORP_DIRECTIVE_PRIORITY_RANGE.end(),
                value = rule.priority,
            ),
        )?;
    }
    if rule.priority == RULE_CATCH_ALL_PRIORITY {
        validation_error(
            &format!("corp_directives[{directive_index}].value.priority"),
            &format!(
                "priority {RULE_CATCH_ALL_PRIORITY} is reserved for the system catch-all rule",
            ),
        )?;
    }
    // Reference the corp-exclusive range for symmetry with the
    // profile-side validator -- this constant is exported so
    // downstream surfaces (CLI/UDS validators landing in S07+)
    // share the same authority on the corp priority window.
    let _ = RULE_CORP_PRIORITY_RANGE;
    Ok(rule)
}

fn emit_reject_event(
    trace: &mut ResolverTrace,
    directive: &CorpDirective,
    directive_index: usize,
    message: &str,
) {
    trace.append(ResolverTraceEvent {
        step: 0,
        path: directive.path.clone(),
        operation: ResolverTraceOperation::Reject,
        source_kind: ResolverTraceSourceKind::Corp,
        source_profile_id: None,
        source_label: format!("corp_directives[{directive_index}]"),
        before: None,
        after: None,
        locked: false,
        reason: Some(message.to_string()),
    });
}

fn violation(
    directive: &CorpDirective,
    directive_index: usize,
    message: &str,
) -> SettingsProfilesError {
    SettingsProfilesError::ResolverViolation {
        path: directive.path.clone(),
        source_layer: "corp".to_string(),
        controlling_rule: format!("corp_directives[{directive_index}]"),
        message: message.to_string(),
    }
}

fn push_corp_event(
    trace: &mut ResolverTrace,
    directive: &CorpDirective,
    operation: ResolverTraceOperation,
    before: Option<serde_json::Value>,
    after: Option<serde_json::Value>,
    directive_index: usize,
) {
    push_corp_event_inner(
        trace,
        directive,
        operation,
        before,
        after,
        directive_index,
        false,
    );
}

fn push_corp_event_locked(
    trace: &mut ResolverTrace,
    directive: &CorpDirective,
    operation: ResolverTraceOperation,
    before: Option<serde_json::Value>,
    after: Option<serde_json::Value>,
    directive_index: usize,
) {
    push_corp_event_inner(
        trace,
        directive,
        operation,
        before,
        after,
        directive_index,
        true,
    );
}

fn push_corp_event_inner(
    trace: &mut ResolverTrace,
    directive: &CorpDirective,
    operation: ResolverTraceOperation,
    before: Option<serde_json::Value>,
    after: Option<serde_json::Value>,
    directive_index: usize,
    locked: bool,
) {
    trace.append(ResolverTraceEvent {
        step: 0,
        path: directive.path.clone(),
        operation,
        source_kind: ResolverTraceSourceKind::Corp,
        source_profile_id: None,
        source_label: format!("corp_directives[{directive_index}]"),
        before,
        after,
        locked,
        reason: directive.reason.clone(),
    });
}

#[cfg(test)]
mod tests;
