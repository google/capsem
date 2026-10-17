//! Trusted credential-store operations for one confined proxy capability.

use std::io;
use std::os::unix::net::UnixStream;

use capsem_foundation::ipc_channel;
use capsem_proto::proxy_credentials::{
    ProxyBrokeredCredential, ProxyCredentialObservation, ProxyCredentialProvider, ProxyCredentialRequest,
    ProxyCredentialResponse, ProxyHeader, ProxyModelProvider, MAX_CREDENTIAL_ERROR_BYTES,
};

use capsem_core::credential_broker::{BrokeredCredential, CredentialObservation, CredentialProvider};
use capsem_core::net::ai_traffic::provider::ProviderKind;

pub(crate) async fn serve(stream: UnixStream) -> Result<(), String> {
    let (sender, receiver) =
        ipc_channel::channel_from_std::<ProxyCredentialResponse, ProxyCredentialRequest>(stream).map_err(io_string)?;
    loop {
        let request = match receiver.recv().await {
            Ok(request) => request,
            Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => return Ok(()),
            Err(error) => return Err(format!("receive proxy credential request: {error}")),
        };
        let request_id = request.request_id();
        let response = if let Err(error) = request.validate() {
            rejected(request_id, error.to_string())
        } else {
            match tokio::task::spawn_blocking(move || dispatch(request)).await {
                Ok(Ok(response)) => response,
                Ok(Err(error)) => rejected(request_id, error),
                Err(error) => rejected(request_id, format!("credential broker task failed: {error}")),
            }
        };
        sender
            .send(response)
            .await
            .map_err(|error| format!("send proxy credential response: {error}"))?;
    }
}

fn dispatch(request: ProxyCredentialRequest) -> Result<ProxyCredentialResponse, String> {
    match request {
        ProxyCredentialRequest::Capture {
            request_id,
            observation,
        } => {
            let credential =
                capsem_core::credential_broker::broker_observed_credential(&core_observation(observation))?;
            Ok(ProxyCredentialResponse::Captured {
                request_id,
                credential: wire_credential(credential),
            })
        }
        ProxyCredentialRequest::Substitute {
            request_id,
            domain,
            ai_provider,
            headers,
            query,
        } => {
            let mut headers = core_headers(headers)?;
            let substituted = capsem_core::credential_broker::substitute_brokered_upstream_credentials(
                &domain,
                ai_provider.map(core_model_provider),
                &mut headers,
                query.as_deref(),
            )?;
            Ok(ProxyCredentialResponse::Substituted {
                request_id,
                headers: wire_headers(&headers),
                query: substituted.query,
                credential_ref: substituted.credential_ref,
            })
        }
    }
}

fn core_observation(observation: ProxyCredentialObservation) -> CredentialObservation {
    CredentialObservation {
        provider: core_provider(observation.provider),
        raw_value: observation.raw_value,
        source: observation.source,
        event_type: observation.event_type,
        trace_id: observation.trace_id,
        context_json: observation.context_json,
    }
}

const fn core_provider(provider: ProxyCredentialProvider) -> CredentialProvider {
    match provider {
        ProxyCredentialProvider::Anthropic => CredentialProvider::Anthropic,
        ProxyCredentialProvider::Google => CredentialProvider::Google,
        ProxyCredentialProvider::OpenAi => CredentialProvider::OpenAi,
        ProxyCredentialProvider::Github => CredentialProvider::Github,
        ProxyCredentialProvider::Mcp => CredentialProvider::Mcp,
    }
}

const fn wire_provider(provider: CredentialProvider) -> ProxyCredentialProvider {
    match provider {
        CredentialProvider::Anthropic => ProxyCredentialProvider::Anthropic,
        CredentialProvider::Google => ProxyCredentialProvider::Google,
        CredentialProvider::OpenAi => ProxyCredentialProvider::OpenAi,
        CredentialProvider::Github => ProxyCredentialProvider::Github,
        CredentialProvider::Mcp => ProxyCredentialProvider::Mcp,
    }
}

const fn core_model_provider(provider: ProxyModelProvider) -> ProviderKind {
    match provider {
        ProxyModelProvider::Unknown => ProviderKind::Unknown,
        ProxyModelProvider::Anthropic => ProviderKind::Anthropic,
        ProxyModelProvider::OpenAi => ProviderKind::OpenAi,
        ProxyModelProvider::Google => ProviderKind::Google,
        ProxyModelProvider::Ollama => ProviderKind::Ollama,
    }
}

fn wire_credential(credential: BrokeredCredential) -> ProxyBrokeredCredential {
    ProxyBrokeredCredential {
        provider: wire_provider(credential.provider),
        credential_ref: credential.credential_ref,
        store_account: credential.store_account,
        newly_captured: credential.newly_captured,
    }
}

fn core_headers(headers: Vec<ProxyHeader>) -> Result<http::HeaderMap, String> {
    let mut result = http::HeaderMap::new();
    for header in headers {
        let name = http::header::HeaderName::from_bytes(header.name.as_bytes())
            .map_err(|error| format!("invalid proxy credential header name: {error}"))?;
        let value = http::header::HeaderValue::from_bytes(&header.value)
            .map_err(|error| format!("invalid proxy credential header value: {error}"))?;
        result.append(name, value);
    }
    Ok(result)
}

fn wire_headers(headers: &http::HeaderMap) -> Vec<ProxyHeader> {
    headers
        .iter()
        .map(|(name, value)| ProxyHeader::new(name.as_str(), value.as_bytes().to_vec()))
        .collect()
}

fn rejected(request_id: u64, error: String) -> ProxyCredentialResponse {
    let error = truncate_utf8(error, MAX_CREDENTIAL_ERROR_BYTES);
    ProxyCredentialResponse::rejected(request_id, error).expect("request id and bounded diagnostic were validated")
}

fn truncate_utf8(mut value: String, max_bytes: usize) -> String {
    if value.len() <= max_bytes {
        return value;
    }
    let mut boundary = max_bytes;
    while !value.is_char_boundary(boundary) {
        boundary -= 1;
    }
    value.truncate(boundary);
    value
}

fn io_string(error: io::Error) -> String {
    error.to_string()
}

#[cfg(test)]
mod tests;
