use crate::credential_broker::{
    broker_observed_credential, detect_brokered_http_references,
    detect_http_credential_with_provider,
};
use crate::net::policy_config::{PolicyActionId, SecurityPluginConfig, SecurityPluginMode};
use crate::security_engine::{
    security_event_contains_text, SecurityActionError, SecurityEvent, SecurityPlugin,
    SecurityPluginResult, SecurityPluginStage, DUMMY_EICAR_TEST_STRING,
};

pub(in crate::security_engine) struct CredentialBrokerPlugin;

impl SecurityPlugin for CredentialBrokerPlugin {
    fn id(&self) -> &'static str {
        "credential_broker"
    }

    fn stage(&self) -> SecurityPluginStage {
        SecurityPluginStage::Preprocess
    }

    fn apply(
        &self,
        mut event: SecurityEvent,
        _config: SecurityPluginConfig,
    ) -> Result<SecurityPluginResult, SecurityActionError> {
        let trace_id = event.trace_id();
        if let Some(request) = event.http_request.as_ref() {
            let injections = detect_brokered_http_references(
                &request.domain,
                request.ai_provider,
                &request.headers,
                request.query.as_deref(),
                trace_id.clone(),
            );
            for injection in injections {
                if event.credential_ref.is_none() {
                    event.credential_ref = Some(injection.credential_ref.clone());
                }
                event.credential_injections.push(injection);
            }
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

        if event.credential_observations.is_empty() && event.credential_injections.is_empty() {
            return Ok(SecurityPluginResult::skipped(event));
        }

        for observation in &event.credential_observations {
            let brokered =
                broker_observed_credential(observation).map_err(SecurityActionError::new)?;
            if event.credential_ref.is_none() {
                event.credential_ref = Some(brokered.credential_ref);
            }
        }
        if !event.credential_observations.is_empty() {
            event
                .action_trace
                .push(PolicyActionId::CredentialBrokerCapture);
        }
        if !event.credential_injections.is_empty() {
            event
                .action_trace
                .push(PolicyActionId::CredentialBrokerSubstitute);
        }
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

    fn apply(
        &self,
        mut event: SecurityEvent,
        config: SecurityPluginConfig,
    ) -> Result<SecurityPluginResult, SecurityActionError> {
        if !security_event_contains_text(&event, DUMMY_EICAR_TEST_STRING)
            && !security_event_contains_text(&event, "EICAR")
        {
            return Ok(SecurityPluginResult::skipped(event));
        }
        if matches!(config.mode, SecurityPluginMode::Rewrite) {
            rewrite_file_eicar_content(&mut event);
        }
        event
            .action_trace
            .push(PolicyActionId::CredentialBrokerCapture);
        Ok(SecurityPluginResult::applied(event))
    }
}

fn rewrite_file_eicar_content(event: &mut SecurityEvent) {
    const REPLACEMENT: &str = "[capsem-rewritten-eicar]";
    let Some(file) = event.file.as_mut() else {
        return;
    };
    for value in [
        &mut file.content,
        &mut file.import_content,
        &mut file.export_content,
        &mut file.read_content,
        &mut file.create_content,
        &mut file.write_content,
        &mut file.delete_content,
    ] {
        if let Some(content) = value.as_mut() {
            *content = content
                .replace(DUMMY_EICAR_TEST_STRING, REPLACEMENT)
                .replace("EICAR", "CAPSEM_REWRITTEN_EICAR");
        }
    }
}
