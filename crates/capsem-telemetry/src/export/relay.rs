//! Trusted service-owned delivery; a confined sender never chooses a URL/header.
//!
//! Only admitted normalized HTTP metrics can enter this transport. It returns
//! no collector body, follows no redirect and never includes config/payload
//! values in errors. Environment semantics match the SDK's metrics overrides;
//! unsupported protocol/compression fails closed rather than changing encoding.
use bytes::Bytes;
use http::{HeaderMap, HeaderName, HeaderValue};

use super::{http_admission, Destination, EXPORT_TIMEOUT};

#[cfg(test)]
mod tests;

/// Payload admitted without copying its original shared buffer.
pub struct AdmittedBody(Bytes);

impl AdmittedBody {
    pub fn admit(body: Bytes) -> Result<Self, CollectorError> {
        http_admission::validate(&body).map_err(|_| CollectorError)?;
        Ok(Self(body))
    }
}

#[derive(Clone)]
pub struct Collector {
    endpoint: reqwest::Url,
    headers: HeaderMap,
    client: reqwest::Client,
}

impl std::fmt::Debug for Collector {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Collector").finish_non_exhaustive()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CollectorError;
impl std::fmt::Display for CollectorError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("metric collector configuration or delivery failed")
    }
}
impl std::error::Error for CollectorError {}

impl Collector {
    /// Called only by the trusted service. No child-supplied config is accepted.
    pub fn resolve(
        corp_endpoint: Option<&str>,
        env: impl Fn(&str) -> Option<String>,
    ) -> Result<Option<Self>, CollectorError> {
        let Some(destination) = Destination::resolve(corp_endpoint, &env) else {
            return Ok(None);
        };
        if preferred(&env, "PROTOCOL").is_some_and(|value| !value.trim().is_empty() && value != "http/protobuf")
            || preferred(&env, "COMPRESSION").is_some_and(|value| !value.trim().is_empty())
        {
            return Err(CollectorError);
        }
        let endpoint = match destination {
            Destination::Corp(base) => {
                check_endpoint(&base)?;
                format!("{base}/v1/metrics")
            }
            Destination::Environment => {
                if let Some(signal) =
                    env("OTEL_EXPORTER_OTLP_METRICS_ENDPOINT").filter(|value| !value.trim().is_empty())
                {
                    signal
                } else {
                    let base = env("OTEL_EXPORTER_OTLP_ENDPOINT").ok_or(CollectorError)?;
                    check_endpoint(&base)?;
                    format!("{}/v1/metrics", base.trim_end_matches('/'))
                }
            }
        };
        let endpoint = check_endpoint(&endpoint)?;
        let mut headers = HeaderMap::new();
        if let Some(input) = preferred(&env, "HEADERS") {
            if input.len() > 16 * 1024 {
                return Err(CollectorError);
            }
            for (index, entry) in input.split_terminator(',').enumerate() {
                if index >= 64 {
                    return Err(CollectorError);
                }
                let (key, value) = entry.trim().split_once('=').ok_or(CollectorError)?;
                let key = HeaderName::from_bytes(key.trim().as_bytes()).map_err(|_| CollectorError)?;
                let decoded = decode_header(value.trim()).ok_or(CollectorError)?;
                let mut value = HeaderValue::from_str(&decoded).map_err(|_| CollectorError)?;
                value.set_sensitive(true);
                headers.insert(key, value);
            }
        }
        let client = reqwest::Client::builder()
            .timeout(EXPORT_TIMEOUT)
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|_| CollectorError)?;
        Ok(Some(Self {
            endpoint,
            headers,
            client,
        }))
    }

    /// Receipt means collector HTTP success, not a persistence/measurement claim.
    /// Drop the collector response immediately; no response bytes enter a worker.
    pub async fn send(&self, body: AdmittedBody) -> Result<(), CollectorError> {
        let response = self
            .client
            .post(self.endpoint.clone())
            .headers(self.headers.clone())
            .header(http::header::CONTENT_TYPE, "application/x-protobuf")
            .body(body.0)
            .send()
            .await
            .map_err(|_| CollectorError)?;
        if response.status().is_success() {
            Ok(())
        } else {
            Err(CollectorError)
        }
    }
}

fn preferred(env: &impl Fn(&str) -> Option<String>, suffix: &str) -> Option<String> {
    env(&format!("OTEL_EXPORTER_OTLP_METRICS_{suffix}")).or_else(|| env(&format!("OTEL_EXPORTER_OTLP_{suffix}")))
}

fn decode_header(input: &str) -> Option<String> {
    let mut output = Vec::with_capacity(input.len());
    let mut bytes = input.bytes();
    while let Some(byte) = bytes.next() {
        if byte == b'%' {
            let high = char::from(bytes.next()?).to_digit(16)?;
            let low = char::from(bytes.next()?).to_digit(16)?;
            output.push((high * 16 + low) as u8);
        } else {
            output.push(byte);
        }
    }
    String::from_utf8(output).ok()
}

fn check_endpoint(input: &str) -> Result<reqwest::Url, CollectorError> {
    if input.len() > 2048 || !(input.starts_with("http://") || input.starts_with("https://")) {
        return Err(CollectorError);
    }
    let endpoint = reqwest::Url::parse(input).map_err(|_| CollectorError)?;
    if endpoint.host_str().is_none() || endpoint.fragment().is_some() {
        return Err(CollectorError);
    }
    Ok(endpoint)
}
