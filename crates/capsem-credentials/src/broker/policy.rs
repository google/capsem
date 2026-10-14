//! Checked-in operational budgets and canonical request/catalog spelling.
use super::*;
use serde::Deserialize;

impl BrokerLimits {
    pub fn bundled() -> BrokerResult<Self> {
        Self::parse(include_str!("../../../../config/data/credential-broker-policy.json"))
    }
    pub fn parse(source: &str) -> BrokerResult<Self> {
        let limits: Self = serde_json::from_str(source).map_err(|_| api::CredentialErrorCode::ConfigurationRequired)?;
        limits.validate()?;
        Ok(limits)
    }
    pub(super) fn validate(&self) -> BrokerResult<()> {
        let counts = [
            self.max_connections,
            self.max_transactions,
            self.max_grants,
            self.max_profiles,
            self.max_capabilities,
            self.max_scopes,
            self.max_identifier_bytes,
            self.max_label_bytes,
            self.max_scope_bytes,
            self.max_metadata_bytes,
        ];
        let times = [
            self.enrollment_lifetime,
            self.grant_lifetime,
            self.retirement_timeout,
            self.retention_lifetime,
            self.maintenance_interval,
        ];
        if counts.contains(&0)
            || times.iter().any(|duration| {
                duration.is_zero()
                    || Instant::now().checked_add(*duration).is_none()
                    || SystemTime::now().checked_add(*duration).is_none()
            })
        {
            return Err(api::CredentialErrorCode::ConfigurationRequired);
        }
        Ok(())
    }
}
pub(super) fn seconds<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<Duration, D::Error> {
    Ok(Duration::from_secs(u64::deserialize(deserializer)?))
}
pub(super) fn normalize_profile(summary: &mut api::ScopeProfileSummary, limits: BrokerLimits) -> BrokerResult<()> {
    if summary.requested_scopes.len() > limits.max_scopes
        || summary.capabilities.len() > limits.max_capabilities
        || summary.requested_scopes.iter().any(|value| !scope(value, limits))
        || summary.capabilities.iter().any(|capability| {
            capability.required_scopes.len() > limits.max_scopes
                || capability.required_scopes.iter().any(|value| !scope(value, limits))
        })
    {
        return Err(api::CredentialErrorCode::ConfigurationRequired);
    }
    fn scopes(values: &mut Vec<String>) {
        *values = values
            .iter()
            .map(|value| crate::oauth::canonical_scope(value).to_owned())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
    }
    scopes(&mut summary.requested_scopes);
    for capability in &mut summary.capabilities {
        scopes(&mut capability.required_scopes);
    }
    Ok(())
}
pub(super) fn label(value: &str, limits: BrokerLimits) -> bool {
    value.len() <= limits.max_label_bytes && !value.chars().any(char::is_control)
}
pub(super) fn scope(value: &str, limits: BrokerLimits) -> bool {
    !value.is_empty()
        && value.len() <= limits.max_scope_bytes
        && !value.bytes().any(|byte| byte.is_ascii_control() || byte == b' ')
}
pub(super) fn normalize_grant_request(
    request: &mut api::CreateSessionGrantRequest,
    limits: BrokerLimits,
) -> BrokerResult<()> {
    if request.capability_ids.is_empty()
        || request.capability_ids.len() > limits.max_capabilities
        || request
            .capability_ids
            .iter()
            .any(|value| !catalog_id(value) || value.len() > limits.max_identifier_bytes)
    {
        return Err(api::CredentialErrorCode::UnsupportedCapability);
    }
    request.capability_ids.sort();
    if request.capability_ids.windows(2).any(|pair| pair[0] == pair[1]) {
        return Err(api::CredentialErrorCode::UnsupportedCapability);
    }
    if let Some(value) = &mut request.expires_at {
        let instant: SystemTime = time::OffsetDateTime::parse(value, &time::format_description::well_known::Rfc3339)
            .map_err(|_| api::CredentialErrorCode::RequestConflict)?
            .into();
        let mut normalized = String::new();
        std::fmt::write(
            &mut normalized,
            format_args!("{}", humantime::format_rfc3339_nanos(instant)),
        )
        .map_err(|_| api::CredentialErrorCode::RequestConflict)?;
        *value = normalized;
    }
    Ok(())
}
