use std::borrow::Cow;

use capsem_config::PolicySubjectValue;
use serde::Serialize;

use super::SecurityEvent;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ContainerSecurityEvent {
    pub image: String,
    pub registry: String,
    pub digest: Option<String>,
}

impl ContainerSecurityEvent {
    pub(super) fn get(&self, field: &str) -> Option<PolicySubjectValue<'_>> {
        match field {
            "valid" => Some(PolicySubjectValue::Bool(
                !self.image.is_empty() && !self.registry.is_empty(),
            )),
            "image" => Some(PolicySubjectValue::String(Cow::Borrowed(&self.image))),
            "registry" => Some(PolicySubjectValue::String(Cow::Borrowed(&self.registry))),
            "digest" => self
                .digest
                .as_deref()
                .map(|value| PolicySubjectValue::String(Cow::Borrowed(value))),
            _ => None,
        }
    }
}

impl SecurityEvent {
    pub fn with_container(mut self, container: ContainerSecurityEvent) -> Self {
        self.container = Some(container);
        self
    }
}
