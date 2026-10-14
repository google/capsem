//! Fixed, typed OTLP transport granted by the session coordinator.

use std::io;
use std::os::unix::net::UnixStream;

use anyhow::{bail, Context, Result};
use capsem_foundation::ipc_channel;
use capsem_proto::proxy_metrics::{
    ProxyMetricBrokerMessage, ProxyMetricRequest, ProxyMetricResponse, MAX_PROXY_METRIC_BODY_BYTES,
};

pub(crate) struct MetricClient {
    requests: ipc_channel::Sender<ProxyMetricRequest>,
    messages: ipc_channel::Receiver<ProxyMetricBrokerMessage>,
    exchange: tokio::sync::Mutex<()>,
}

impl std::fmt::Debug for MetricClient {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("MetricClient").finish_non_exhaustive()
    }
}

pub(crate) async fn start(stream: UnixStream) -> Result<(MetricClient, String)> {
    let (requests, messages) = ipc_channel::channel_from_std::<ProxyMetricRequest, ProxyMetricBrokerMessage>(stream)
        .context("open proxy metric transport")?;
    let session_id = match messages.recv().await.context("receive proxy metric identity")? {
        ProxyMetricBrokerMessage::Hello { session_id } if !session_id.is_empty() && session_id.len() <= 255 => {
            session_id
        }
        ProxyMetricBrokerMessage::Hello { .. } => bail!("proxy metric identity is invalid"),
        ProxyMetricBrokerMessage::Response(_) => bail!("proxy metric broker responded before identity"),
    };
    Ok((
        MetricClient {
            requests,
            messages,
            exchange: tokio::sync::Mutex::new(()),
        },
        session_id,
    ))
}

#[async_trait::async_trait]
impl opentelemetry_http::HttpClient for MetricClient {
    async fn send_bytes(
        &self,
        request: http::Request<bytes::Bytes>,
    ) -> std::result::Result<http::Response<bytes::Bytes>, opentelemetry_http::HttpError> {
        let body = request.into_body();
        if body.len() > MAX_PROXY_METRIC_BODY_BYTES {
            return Err(metric_error("encoded proxy metrics exceed the broker limit"));
        }
        let _exchange = self.exchange.lock().await;
        self.requests
            .send(ProxyMetricRequest { body: body.to_vec() })
            .await
            .map_err(|error| metric_error(format!("send proxy metrics: {error}")))?;
        match self
            .messages
            .recv()
            .await
            .map_err(|error| metric_error(format!("receive proxy metric result: {error}")))?
        {
            ProxyMetricBrokerMessage::Hello { .. } => Err(metric_error("proxy metric broker repeated its identity")),
            ProxyMetricBrokerMessage::Response(ProxyMetricResponse::Rejected { message }) => {
                Err(metric_error(format!("proxy metric broker rejected export: {message}")))
            }
            ProxyMetricBrokerMessage::Response(ProxyMetricResponse::Relayed {
                status,
                content_type,
                body,
            }) => {
                let mut response = http::Response::builder().status(status);
                if let Some(content_type) = content_type {
                    response = response.header(http::header::CONTENT_TYPE, content_type);
                }
                response
                    .body(bytes::Bytes::from(body))
                    .map_err(|error| metric_error(format!("build proxy metric response: {error}")))
            }
        }
    }
}

fn metric_error(message: impl Into<String>) -> opentelemetry_http::HttpError {
    Box::new(io::Error::other(message.into()))
}

#[cfg(test)]
mod tests;
