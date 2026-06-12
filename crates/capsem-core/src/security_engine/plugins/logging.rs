use crate::credential_broker::redact_observed_credentials_in_bytes;
use crate::security_engine::{
    SecurityActionError, SecurityEvent, SecurityPlugin, SecurityPluginResult, SecurityPluginStage,
};

pub(in crate::security_engine) struct LogSanitizerPlugin;

impl SecurityPlugin for LogSanitizerPlugin {
    fn id(&self) -> &'static str {
        "log_sanitizer"
    }

    fn stage(&self) -> SecurityPluginStage {
        SecurityPluginStage::Logging
    }

    fn apply(&self, mut event: SecurityEvent) -> Result<SecurityPluginResult, SecurityActionError> {
        if event.credential_observations.is_empty() {
            return Ok(SecurityPluginResult::skipped(event));
        }

        if let Some(request) = event.http_request.as_mut() {
            for value in request.headers.values_mut() {
                let redacted = redact_observed_credentials_in_bytes(
                    value.as_bytes(),
                    &event.credential_observations,
                );
                if redacted != value.as_bytes() {
                    *value = http::HeaderValue::from_bytes(&redacted).map_err(|error| {
                        SecurityActionError::new(format!(
                            "log sanitizer produced invalid header value: {error}"
                        ))
                    })?;
                }
            }
            if let Some(query) = request.query.as_mut() {
                let redacted = redact_observed_credentials_in_bytes(
                    query.as_bytes(),
                    &event.credential_observations,
                );
                if redacted != query.as_bytes() {
                    *query = String::from_utf8(redacted).map_err(|error| {
                        SecurityActionError::new(format!(
                            "log sanitizer produced invalid query text: {error}"
                        ))
                    })?;
                }
            }
        }

        event.credential_observations.clear();
        Ok(SecurityPluginResult::applied(event))
    }
}
