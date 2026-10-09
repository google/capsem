use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const MAX_CREDENTIAL_VALUE_BYTES: usize = 64 * 1024;
pub const MAX_CREDENTIAL_FIELD_BYTES: usize = 4 * 1024;
pub const MAX_CREDENTIAL_HEADERS: usize = 256;
pub const MAX_CREDENTIAL_ERROR_BYTES: usize = 4 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProxyCredentialProvider {
    Anthropic,
    Google,
    OpenAi,
    Github,
    Mcp,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProxyModelProvider {
    Unknown,
    Anthropic,
    OpenAi,
    Google,
    Ollama,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProxyCredentialObservation {
    pub provider: ProxyCredentialProvider,
    pub raw_value: String,
    pub source: String,
    pub event_type: Option<String>,
    pub trace_id: Option<String>,
    pub context_json: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProxyBrokeredCredential {
    pub provider: ProxyCredentialProvider,
    pub credential_ref: String,
    pub store_account: String,
    pub newly_captured: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProxyHeader {
    pub name: String,
    #[serde(with = "serde_bytes")]
    pub value: Vec<u8>,
}

impl ProxyHeader {
    pub fn new(name: impl Into<String>, value: Vec<u8>) -> Self {
        Self {
            name: name.into(),
            value,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProxyCredentialRequest {
    Capture {
        request_id: u64,
        observation: ProxyCredentialObservation,
    },
    Substitute {
        request_id: u64,
        domain: String,
        ai_provider: Option<ProxyModelProvider>,
        headers: Vec<ProxyHeader>,
        query: Option<String>,
    },
}

impl ProxyCredentialRequest {
    pub const fn request_id(&self) -> u64 {
        match self {
            Self::Capture { request_id, .. } | Self::Substitute { request_id, .. } => *request_id,
        }
    }

    pub fn validate(&self) -> Result<(), ProxyCredentialProtocolError> {
        if self.request_id() == 0 {
            return Err(ProxyCredentialProtocolError::ZeroRequestId);
        }
        match self {
            Self::Capture { observation, .. } => {
                bounded(&observation.raw_value, MAX_CREDENTIAL_VALUE_BYTES)?;
                bounded(&observation.source, MAX_CREDENTIAL_FIELD_BYTES)?;
                bounded_optional(&observation.event_type, MAX_CREDENTIAL_FIELD_BYTES)?;
                bounded_optional(&observation.trace_id, MAX_CREDENTIAL_FIELD_BYTES)?;
                bounded_optional(&observation.context_json, MAX_CREDENTIAL_VALUE_BYTES)
            }
            Self::Substitute {
                domain, headers, query, ..
            } => {
                bounded(domain, MAX_CREDENTIAL_FIELD_BYTES)?;
                if headers.len() > MAX_CREDENTIAL_HEADERS {
                    return Err(ProxyCredentialProtocolError::TooManyHeaders);
                }
                for header in headers {
                    bounded(&header.name, MAX_CREDENTIAL_FIELD_BYTES)?;
                    if header.value.len() > MAX_CREDENTIAL_VALUE_BYTES {
                        return Err(ProxyCredentialProtocolError::FieldTooLong);
                    }
                }
                bounded_optional(query, MAX_CREDENTIAL_VALUE_BYTES)
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProxyCredentialResponse {
    Captured {
        request_id: u64,
        credential: ProxyBrokeredCredential,
    },
    Substituted {
        request_id: u64,
        headers: Vec<ProxyHeader>,
        query: Option<String>,
        credential_ref: Option<String>,
    },
    Rejected {
        request_id: u64,
        error: String,
    },
}

impl ProxyCredentialResponse {
    pub const fn request_id(&self) -> u64 {
        match self {
            Self::Captured { request_id, .. }
            | Self::Substituted { request_id, .. }
            | Self::Rejected { request_id, .. } => *request_id,
        }
    }

    pub fn rejected(request_id: u64, error: impl Into<String>) -> Result<Self, ProxyCredentialProtocolError> {
        if request_id == 0 {
            return Err(ProxyCredentialProtocolError::ZeroRequestId);
        }
        let error = error.into();
        bounded(&error, MAX_CREDENTIAL_ERROR_BYTES)?;
        Ok(Self::Rejected { request_id, error })
    }
}

fn bounded(value: &str, limit: usize) -> Result<(), ProxyCredentialProtocolError> {
    if value.len() > limit {
        Err(ProxyCredentialProtocolError::FieldTooLong)
    } else {
        Ok(())
    }
}

fn bounded_optional(value: &Option<String>, limit: usize) -> Result<(), ProxyCredentialProtocolError> {
    match value {
        Some(value) => bounded(value, limit),
        None => Ok(()),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum ProxyCredentialProtocolError {
    #[error("proxy credential request id must not be zero")]
    ZeroRequestId,
    #[error("proxy credential field exceeds its bound")]
    FieldTooLong,
    #[error("proxy credential request has too many headers")]
    TooManyHeaders,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounded_capture_and_substitution_requests_validate() {
        let capture = ProxyCredentialRequest::Capture {
            request_id: 1,
            observation: ProxyCredentialObservation {
                provider: ProxyCredentialProvider::OpenAi,
                raw_value: "secret".to_string(),
                source: "http.header.authorization".to_string(),
                event_type: Some("http.request".to_string()),
                trace_id: None,
                context_json: None,
            },
        };
        capture.validate().unwrap();

        let substitute = ProxyCredentialRequest::Substitute {
            request_id: 2,
            domain: "api.openai.com".to_string(),
            ai_provider: Some(ProxyModelProvider::OpenAi),
            headers: vec![ProxyHeader::new(
                "authorization",
                b"Bearer credential:blake3:abc".to_vec(),
            )],
            query: Some("key=credential%3Ablake3%3Aabc".to_string()),
        };
        substitute.validate().unwrap();
        assert_eq!(capture.request_id(), 1);
        assert_eq!(substitute.request_id(), 2);
    }

    #[test]
    fn oversized_or_uncorrelated_values_fail_closed() {
        let request = ProxyCredentialRequest::Capture {
            request_id: 0,
            observation: ProxyCredentialObservation {
                provider: ProxyCredentialProvider::Google,
                raw_value: "x".repeat(MAX_CREDENTIAL_VALUE_BYTES + 1),
                source: "test".to_string(),
                event_type: None,
                trace_id: None,
                context_json: None,
            },
        };
        assert!(request.validate().is_err());
        assert!(ProxyCredentialResponse::rejected(0, "no").is_err());
        assert!(ProxyCredentialResponse::rejected(1, "x".repeat(MAX_CREDENTIAL_ERROR_BYTES + 1)).is_err());
    }
}
