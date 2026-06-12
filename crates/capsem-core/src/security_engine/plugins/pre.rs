use crate::credential_broker::{broker_observed_credential, detect_http_credential_with_provider};
use crate::net::policy_config::PolicyActionId;
use crate::security_engine::{
    security_event_contains_text, SecurityActionError, SecurityDecisionKind, SecurityEvent,
    SecurityPlugin, SecurityPluginResult, SecurityPluginStage, DUMMY_EICAR_TEST_STRING,
};

pub(in crate::security_engine) struct CredentialBrokerPlugin;

impl SecurityPlugin for CredentialBrokerPlugin {
    fn id(&self) -> &'static str {
        "credential_broker"
    }

    fn stage(&self) -> SecurityPluginStage {
        SecurityPluginStage::Preprocess
    }

    fn apply(&self, mut event: SecurityEvent) -> Result<SecurityPluginResult, SecurityActionError> {
        let trace_id = event.trace_id();
        if let Some(request) = event.http_request.as_ref() {
            for (name, value) in request.headers.iter() {
                if let Some(mut observation) = detect_http_credential_with_provider(
                    &request.domain,
                    request.ai_provider,
                    name.as_str(),
                    value.as_bytes(),
                ) {
                    if observation.trace_id.is_none() {
                        observation.trace_id = trace_id.clone();
                    }
                    event.credential_observations.push(observation);
                }
            }
        }

        if event.credential_observations.is_empty() {
            return Ok(SecurityPluginResult::skipped(event));
        }

        for observation in &event.credential_observations {
            let brokered =
                broker_observed_credential(observation).map_err(SecurityActionError::new)?;
            if event.credential_ref.is_none() {
                event.credential_ref = Some(brokered.credential_ref);
            }
        }
        event
            .action_trace
            .push(PolicyActionId::CredentialBrokerCapture);
        Ok(SecurityPluginResult::applied(event))
    }
}

pub(in crate::security_engine) struct DummyPreEicarPlugin;

impl SecurityPlugin for DummyPreEicarPlugin {
    fn id(&self) -> &'static str {
        "dummy_pre_eicar"
    }

    fn stage(&self) -> SecurityPluginStage {
        SecurityPluginStage::Preprocess
    }

    fn apply(&self, mut event: SecurityEvent) -> Result<SecurityPluginResult, SecurityActionError> {
        if !security_event_contains_text(&event, DUMMY_EICAR_TEST_STRING)
            && !security_event_contains_text(&event, "EICAR")
        {
            return Ok(SecurityPluginResult::skipped(event));
        }
        event.request_decision(SecurityDecisionKind::Block);
        event
            .action_trace
            .push(PolicyActionId::CredentialBrokerCapture);
        Ok(SecurityPluginResult::applied(event))
    }
}
