use crate::credential_broker::redact_observed_credentials_in_bytes;
use crate::net::policy_config::SecurityPluginConfig;
use crate::security_engine::{
    SecurityActionError, SecurityEvent, SecurityPlugin, SecurityPluginResult, SecurityPluginStage,
};

pub(in crate::security_engine) struct LogSanitizerPlugin;

/// Query values are runtime input, not ledger facts. Enforcement already saw
/// the original request; the stored projection records only its presence.
pub(crate) fn redact_query_for_ledger(query: &str) -> String {
    if query.is_empty() {
        String::new()
    } else {
        "[REDACTED]".to_string()
    }
}

impl SecurityPlugin for LogSanitizerPlugin {
    fn id(&self) -> &'static str {
        "log_sanitizer"
    }

    fn stage(&self) -> SecurityPluginStage {
        SecurityPluginStage::Logging
    }

    fn apply(
        &self,
        mut event: SecurityEvent,
        _config: SecurityPluginConfig,
    ) -> Result<SecurityPluginResult, SecurityActionError> {
        let has_http_request = event.http_request.is_some();
        let has_query = event
            .http_request
            .as_ref()
            .is_some_and(|request| request.query.is_some());
        if event.credential_observations.is_empty() && !has_query {
            return Ok(SecurityPluginResult::skipped(event));
        }

        if let Some(request) = event.http_request.as_mut() {
            for value in request.headers.values_mut() {
                let redacted = redact_observed_credentials_in_bytes(value.as_bytes(), &event.credential_observations);
                if redacted != value.as_bytes() {
                    *value = http::HeaderValue::from_bytes(&redacted).map_err(|error| {
                        SecurityActionError::new(format!("log sanitizer produced invalid header value: {error}"))
                    })?;
                }
            }
            if let Some(query) = request.query.as_mut() {
                *query = redact_query_for_ledger(query);
            }
        }

        // Runtime HTTP events have both representations; redact both after
        // enforcement. Telemetry's rule evaluation retains the raw facts and
        // sanitizes its forensic projection separately.
        if has_http_request {
            if let Some(http) = event.http.as_mut() {
                if let Some(query) = http.query.as_mut() {
                    *query = redact_query_for_ledger(query);
                }
            }
        }

        event.credential_observations.clear();
        Ok(SecurityPluginResult::applied(event))
    }
}
