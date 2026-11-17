//! Sessionful GET replies: drop a refused stream, report outstanding calls,
//! then reopen only the receive channel. A POST operation is never retried.
use super::*;
use futures::StreamExt;

#[derive(Clone)]
pub(super) struct GetOptions {
    pub uri: Arc<str>,
    pub session_id: Option<Arc<str>>,
    pub last_event_id: Option<String>,
    pub auth_header: Option<String>,
    pub custom_headers: HashMap<HeaderName, HeaderValue>,
    pub limit: usize,
}

impl CapsemMcpHttpClient {
    async fn receive_get(
        &self,
        options: &GetOptions,
    ) -> Result<BoxStream<'static, Result<Sse, SseError>>, StreamableHttpError<std::io::Error>> {
        let mut request_builder = self
            .client
            .get(options.uri.as_ref())
            .header(ACCEPT, [EVENT_STREAM_MIME_TYPE, JSON_MIME_TYPE].join(", "));
        if let Some(session_id) = &options.session_id {
            request_builder = request_builder.header(MCP_HEADER_SESSION_ID, session_id.as_ref());
        }
        if let Some(last_event_id) = &options.last_event_id {
            request_builder = request_builder.header(MCP_HEADER_LAST_EVENT_ID, last_event_id);
        }
        if let Some(auth_header) = &options.auth_header {
            request_builder = request_builder.bearer_auth(auth_header);
        }
        request_builder = apply_mcp_custom_headers(request_builder, options.custom_headers.clone())?;
        let response = self.send(request_builder).await.map_err(StreamableHttpError::Client)?;
        if response.status() == reqwest::StatusCode::METHOD_NOT_ALLOWED {
            return Err(StreamableHttpError::ServerDoesNotSupportSse);
        }
        let response = response
            .error_for_status()
            .map_err(|e| StreamableHttpError::Client(std::io::Error::other(e)))?;
        match response.headers().get(reqwest::header::CONTENT_TYPE) {
            Some(content_type) => {
                if !content_type.as_bytes().starts_with(EVENT_STREAM_MIME_TYPE.as_bytes())
                    && !content_type.as_bytes().starts_with(JSON_MIME_TYPE.as_bytes())
                {
                    return Err(StreamableHttpError::UnexpectedContentType(Some(
                        String::from_utf8_lossy(content_type.as_bytes()).to_string(),
                    )));
                }
            }
            None => return Err(StreamableHttpError::UnexpectedContentType(None)),
        }
        Ok(http_response::sse(response.bytes_stream(), options.limit))
    }

    pub(super) async fn open_get(
        &self,
        options: GetOptions,
    ) -> Result<BoxStream<'static, Result<Sse, SseError>>, StreamableHttpError<std::io::Error>> {
        let initial = self.receive_get(&options).await?;
        let client = self.clone();
        Ok(futures::stream::unfold(
            (Some(initial), std::collections::VecDeque::new(), client, options),
            |(stream, mut replies, client, mut options)| async move {
                if let Some(reply) = replies.pop_front() {
                    return Some((reply, (stream, replies, client, options)));
                }
                let mut stream = match stream {
                    Some(stream) => stream,
                    None => match client.receive_get(&options).await {
                        Ok(stream) => stream,
                        Err(error) => {
                            return Some((
                                Err(SseError::Body(Box::new(error))),
                                (Some(futures::stream::empty().boxed()), replies, client, options),
                            ))
                        }
                    },
                };
                let item = stream.next().await?;
                match item {
                    Err(SseError::Body(error))
                        if error
                            .downcast_ref::<std::io::Error>()
                            .is_some_and(http_response::is_size_refusal) =>
                    {
                        // An excessive event can end before its ID is readable.
                        // Conservatively fail every outstanding response, without
                        // asserting which operation produced that event.
                        replies = client
                            .pending
                            .refuse_all()
                            .into_iter()
                            .map(|id| {
                                let bytes = capsem_proto::framing::encode_json_record(
                                    &http_response::size_error(Some(id)),
                                    capsem_proto::MCP_JSON_RPC_MAX_BYTES,
                                )
                                .and_then(|bytes| String::from_utf8(bytes).map_err(std::io::Error::other));
                                bytes
                                    .map(|data| Sse {
                                        data: Some(data),
                                        ..Sse::default()
                                    })
                                    .map_err(|error| SseError::Body(Box::new(error)))
                            })
                            .collect();
                        let Some(first) = replies.pop_front() else {
                            // Preserve unsolicited refusal diagnostics and stop;
                            // an idle hostile peer must not cause a GET retry loop.
                            return Some((
                                Err(SseError::Body(error)),
                                (Some(futures::stream::empty().boxed()), replies, client, options),
                            ));
                        };
                        // Replaying an unadmitted event cannot repair it. Reopen
                        // the session GET without a Last-Event-ID cursor.
                        options.last_event_id = None;
                        Some((first, (None, replies, client, options)))
                    }
                    item => {
                        if let Ok(event) = &item {
                            client.pending.complete(event);
                        }
                        Some((item, (Some(stream), replies, client, options)))
                    }
                }
            },
        )
        .boxed())
    }
}
