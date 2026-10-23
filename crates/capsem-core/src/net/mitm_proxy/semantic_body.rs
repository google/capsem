use std::time::Duration;

use http_body_util::{BodyExt, Full};
use hyper::body::{Body, Bytes};
use tracing::Instrument;

use super::body::ProxyBoxBody;

const BODY_LIMIT: usize = 16 * 1024 * 1024;
const DECOMPRESSED_BODY_LIMIT: usize = 32 * 1024 * 1024;
const BODY_DEADLINE: Duration = Duration::from_secs(60);

pub(super) enum UpstreamRequestBody {
    Replayable(Bytes),
    Streaming(ProxyBoxBody),
}

impl UpstreamRequestBody {
    pub(super) fn into_body(self) -> ProxyBoxBody {
        match self {
            Self::Replayable(body) => Full::new(body)
                .map_err(|never| -> anyhow::Error { match never {} })
                .boxed(),
            Self::Streaming(body) => body,
        }
    }
}

pub(super) async fn collect<B>(body: B, span: tracing::Span) -> Result<Bytes, String>
where
    B: Body<Data = Bytes> + Send + 'static,
    B::Error: Into<Box<dyn std::error::Error + Send + Sync>>,
{
    match tokio::time::timeout(
        BODY_DEADLINE,
        http_body_util::Limited::new(body, BODY_LIMIT)
            .collect()
            .instrument(span),
    )
    .await
    {
        Ok(Ok(collected)) => Ok(collected.to_bytes()),
        Ok(Err(error)) => Err(error.to_string()),
        Err(error) => Err(error.to_string()),
    }
}

pub(super) fn decompress_gzip(body: Bytes, is_gzip: bool) -> anyhow::Result<Bytes> {
    if !is_gzip {
        return Ok(body);
    }
    // A small compressed response can expand into a much larger policy event.
    let decompressed = crate::net::decompress::decompress_gzip_capped(&body, DECOMPRESSED_BODY_LIMIT)?;
    Ok(Bytes::from(decompressed))
}
