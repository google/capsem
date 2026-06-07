use capsem_security_engine::{
    ModelInteractionEvidence, ModelSecuritySubject, SecurityEvent, SecurityEventCommon,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelSecurityEventInput {
    pub provider: String,
    pub model: String,
    pub estimated_input_tokens: Option<u64>,
    pub estimated_output_tokens: Option<u64>,
    pub estimated_cost_micros: Option<u64>,
    pub evidence: Option<ModelInteractionEvidence>,
}

impl ModelSecurityEventInput {
    pub fn from_interaction_evidence(evidence: ModelInteractionEvidence) -> Self {
        Self {
            provider: evidence.provider.as_str().to_owned(),
            model: evidence.model.clone(),
            estimated_input_tokens: evidence.usage.input_tokens,
            estimated_output_tokens: evidence.usage.output_tokens,
            estimated_cost_micros: evidence.usage.estimated_cost_micros,
            evidence: Some(evidence),
        }
    }
}

pub fn build_model_security_event(
    common: SecurityEventCommon,
    input: ModelSecurityEventInput,
) -> SecurityEvent {
    SecurityEvent::model(
        common,
        ModelSecuritySubject {
            provider: input.provider,
            model: input.model,
            estimated_input_tokens: input.estimated_input_tokens,
            estimated_output_tokens: input.estimated_output_tokens,
            estimated_cost_micros: input.estimated_cost_micros,
            evidence: input.evidence.map(Box::new),
        },
    )
}

pub fn build_model_security_event_from_evidence(
    common: SecurityEventCommon,
    evidence: ModelInteractionEvidence,
) -> SecurityEvent {
    build_model_security_event(
        common,
        ModelSecurityEventInput::from_interaction_evidence(evidence),
    )
}

#[cfg(test)]
mod tests;
