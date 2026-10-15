//! One-way model/file correlation hints for the workspace-owning process.

use std::io;
use std::os::fd::AsFd as _;
use std::os::unix::net::UnixStream;
use std::sync::Arc;
use std::time::Duration;

use capsem_core::net::ai_traffic::{TraceHintFuture, TraceHintSink};
use capsem_proto::proxy_trace_hints::{encode_proxy_trace_hint, ProxyTraceHint, PROXY_TRACE_HINT_ACK};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

const WRITE_TIMEOUT: Duration = Duration::from_secs(1);

pub(super) struct TraceHintClient {
    stream: tokio::sync::Mutex<tokio::net::UnixStream>,
}

impl TraceHintClient {
    pub(super) fn start(stream: UnixStream) -> io::Result<Arc<Self>> {
        capsem_foundation::unix::fd::set_nonblocking(stream.as_fd(), true)?;
        Ok(Arc::new(Self {
            stream: tokio::sync::Mutex::new(tokio::net::UnixStream::from_std(stream)?),
        }))
    }
}

impl TraceHintSink for TraceHintClient {
    fn register<'a>(&'a self, trace_id: &'a str, relative_paths: &'a [String]) -> TraceHintFuture<'a> {
        Box::pin(async move {
            let mut stream = self.stream.lock().await;
            for relative_path in relative_paths {
                let frame = encode_proxy_trace_hint(&ProxyTraceHint {
                    trace_id: trace_id.to_string(),
                    relative_path: relative_path.clone(),
                })
                .map_err(|error| error.to_string())?;
                tokio::time::timeout(WRITE_TIMEOUT, stream.write_all(&frame))
                    .await
                    .map_err(|_| "proxy trace-hint write timed out".to_string())?
                    .map_err(|error| format!("write proxy trace hint: {error}"))?;
                let mut acknowledgement = [0; PROXY_TRACE_HINT_ACK.len()];
                tokio::time::timeout(WRITE_TIMEOUT, stream.read_exact(&mut acknowledgement))
                    .await
                    .map_err(|_| "proxy trace-hint acknowledgement timed out".to_string())?
                    .map_err(|error| format!("read proxy trace-hint acknowledgement: {error}"))?;
                if acknowledgement != PROXY_TRACE_HINT_ACK {
                    return Err("proxy trace-hint owner returned an invalid acknowledgement".to_string());
                }
            }
            drop(stream);
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests;
