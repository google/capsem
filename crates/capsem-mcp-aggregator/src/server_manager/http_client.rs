//! The Streamable HTTP adapter for the manager's rmcp peer.
use super::*;

impl StreamableHttpClient for CapsemMcpHttpClient {
    type Error = std::io::Error;

    async fn get_stream(
        &self,
        uri: Arc<str>,
        session_id: Option<Arc<str>>,
        last_event_id: Option<String>,
        auth_header: Option<String>,
        custom_headers: HashMap<HeaderName, HeaderValue>,
    ) -> Result<BoxStream<'static, Result<Sse, SseError>>, StreamableHttpError<Self::Error>> {
        self.get_stream_with_max_sse_event_size(
            uri,
            session_id,
            last_event_id,
            auth_header,
            custom_headers,
            capsem_proto::MCP_JSON_RPC_MAX_BYTES,
        )
        .await
    }

    async fn get_stream_with_max_sse_event_size(
        &self,
        uri: Arc<str>,
        session_id: Option<Arc<str>>,
        last_event_id: Option<String>,
        auth_header: Option<String>,
        custom_headers: HashMap<HeaderName, HeaderValue>,
        max_sse_event_size: usize,
    ) -> Result<BoxStream<'static, Result<Sse, SseError>>, StreamableHttpError<Self::Error>> {
        self.open_get(http_get::GetOptions {
            uri,
            session_id,
            last_event_id,
            auth_header,
            custom_headers,
            limit: max_sse_event_size.min(capsem_proto::MCP_JSON_RPC_MAX_BYTES),
        })
        .await
    }

    async fn delete_session(
        &self,
        uri: Arc<str>,
        session_id: Arc<str>,
        auth_header: Option<String>,
        custom_headers: HashMap<HeaderName, HeaderValue>,
    ) -> Result<(), StreamableHttpError<Self::Error>> {
        let mut request_builder = self
            .client
            .delete(uri.as_ref())
            .header(MCP_HEADER_SESSION_ID, session_id.as_ref());
        if let Some(auth_header) = auth_header {
            request_builder = request_builder.bearer_auth(auth_header);
        }
        request_builder = apply_mcp_custom_headers(request_builder, custom_headers)?;
        let response = self.send(request_builder).await.map_err(StreamableHttpError::Client)?;
        if response.status() == reqwest::StatusCode::METHOD_NOT_ALLOWED {
            tracing::debug!("this server doesn't support deleting session");
            return Ok(());
        }
        response
            .error_for_status()
            .map_err(|e| StreamableHttpError::Client(std::io::Error::other(e)))?;
        Ok(())
    }

    async fn post_message(
        &self,
        uri: Arc<str>,
        message: ClientJsonRpcMessage,
        session_id: Option<Arc<str>>,
        auth_header: Option<String>,
        custom_headers: HashMap<HeaderName, HeaderValue>,
    ) -> Result<StreamableHttpPostResponse, StreamableHttpError<Self::Error>> {
        self.post_message_with_max_sse_event_size(
            uri,
            message,
            session_id,
            auth_header,
            custom_headers,
            capsem_proto::MCP_JSON_RPC_MAX_BYTES,
        )
        .await
    }

    async fn post_message_with_max_sse_event_size(
        &self,
        uri: Arc<str>,
        message: ClientJsonRpcMessage,
        session_id: Option<Arc<str>>,
        auth_header: Option<String>,
        custom_headers: HashMap<HeaderName, HeaderValue>,
        max_sse_event_size: usize,
    ) -> Result<StreamableHttpPostResponse, StreamableHttpError<Self::Error>> {
        let mut request = self
            .client
            .post(uri.as_ref())
            .header(ACCEPT, [EVENT_STREAM_MIME_TYPE, JSON_MIME_TYPE].join(", "));
        if let Some(auth_header) = auth_header {
            request = request.bearer_auth(auth_header);
        }
        request = apply_mcp_custom_headers(request, custom_headers)?;
        let session_was_attached = session_id.is_some();
        if let Some(session_id) = session_id {
            request = request.header(MCP_HEADER_SESSION_ID, session_id.as_ref());
        }
        // External JSON-RPC is encoded once into bounded final storage. The
        // request builder takes ownership instead of allocating via `.json()`.
        let bytes = capsem_proto::framing::encode_json_record(&message, capsem_proto::MCP_JSON_RPC_MAX_BYTES)
            .map_err(StreamableHttpError::Client)?;
        capsem_proto::mcp_contracts::json_rpc_wire::validate_size(&bytes).map_err(StreamableHttpError::Client)?;
        let response_id = match &message {
            JsonRpcMessage::Request(request) => Some(request.id.clone()),
            _ => None,
        };
        let mut pending = self.pending.track(&message);
        let response = self
            .send(
                request
                    .header(reqwest::header::CONTENT_TYPE, JSON_MIME_TYPE)
                    .body(bytes),
            )
            .await
            .map_err(StreamableHttpError::Client)?;
        if response.status() == reqwest::StatusCode::UNAUTHORIZED {
            if let Some(header) = response.headers().get(WWW_AUTHENTICATE) {
                let header = header
                    .to_str()
                    .map_err(|_| {
                        StreamableHttpError::UnexpectedServerResponse(Cow::from(
                            "invalid www-authenticate header value",
                        ))
                    })?
                    .to_string();
                return Err(StreamableHttpError::AuthRequired(AuthRequiredError::new(header)));
            }
        }
        if response.status() == reqwest::StatusCode::FORBIDDEN {
            if let Some(header) = response.headers().get(WWW_AUTHENTICATE) {
                let header_str = header.to_str().map_err(|_| {
                    StreamableHttpError::UnexpectedServerResponse(Cow::from("invalid www-authenticate header value"))
                })?;
                return Err(StreamableHttpError::InsufficientScope(InsufficientScopeError::new(
                    header_str.to_string(),
                    extract_mcp_scope_from_header(header_str),
                )));
            }
        }

        let status = response.status();
        if matches!(status, reqwest::StatusCode::ACCEPTED | reqwest::StatusCode::NO_CONTENT) {
            pending.retain();
            return Ok(StreamableHttpPostResponse::Accepted);
        }
        if status == reqwest::StatusCode::NOT_FOUND && session_was_attached {
            return Err(StreamableHttpError::SessionExpired);
        }
        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .map(|ct| String::from_utf8_lossy(ct.as_bytes()).to_string());
        let session_id = response
            .headers()
            .get(MCP_HEADER_SESSION_ID)
            .and_then(|value| value.to_str().ok())
            .map(|value| value.to_string());

        if !status.is_success() {
            let bytes = match http_response::json_body(response).await {
                Ok(bytes) => bytes,
                Err(error) if http_response::is_size_refusal(&error) => {
                    return Ok(StreamableHttpPostResponse::Json(
                        http_response::size_error(response_id),
                        session_id,
                    ));
                }
                Err(error) => return Err(StreamableHttpError::Client(error)),
            };
            let body = String::from_utf8_lossy(&bytes);
            if content_type
                .as_deref()
                .is_some_and(|ct| ct.as_bytes().starts_with(JSON_MIME_TYPE.as_bytes()))
            {
                if let Some(message) = parse_mcp_json_rpc_error(&body) {
                    return Ok(StreamableHttpPostResponse::Json(message, session_id));
                }
                tracing::warn!("HTTP {status}: could not parse JSON body as a JSON-RPC error");
            }
            return Err(StreamableHttpError::UnexpectedServerResponse(Cow::Owned(format!(
                "HTTP {status}: {body}"
            ))));
        }

        match content_type.as_deref() {
            Some(ct) if ct.as_bytes().starts_with(EVENT_STREAM_MIME_TYPE.as_bytes()) => {
                pending.retain();
                let pending = self.pending.clone();
                Ok(StreamableHttpPostResponse::Sse(
                    http_response::post_sse(
                        response.bytes_stream(),
                        max_sse_event_size.min(capsem_proto::MCP_JSON_RPC_MAX_BYTES),
                        response_id,
                    )
                    .inspect(move |item| {
                        if let Ok(event) = item {
                            pending.complete(event);
                        }
                    })
                    .boxed(),
                    session_id,
                ))
            }
            Some(ct) if ct.as_bytes().starts_with(JSON_MIME_TYPE.as_bytes()) => {
                let bytes = match http_response::json_body(response).await {
                    Ok(bytes) => bytes,
                    Err(error) if http_response::is_size_refusal(&error) => {
                        return Ok(StreamableHttpPostResponse::Json(
                            http_response::size_error(response_id),
                            session_id,
                        ));
                    }
                    Err(error) => return Err(StreamableHttpError::Client(error)),
                };
                match serde_json::from_slice::<ServerJsonRpcMessage>(&bytes) {
                    Ok(message) => Ok(StreamableHttpPostResponse::Json(message, session_id)),
                    Err(error) => {
                        tracing::warn!(
                            "could not parse JSON response as ServerJsonRpcMessage, treating as accepted: {error}"
                        );
                        Ok(StreamableHttpPostResponse::Accepted)
                    }
                }
            }
            _ => {
                tracing::error!("unexpected content type: {:?}", content_type);
                Err(StreamableHttpError::UnexpectedContentType(content_type))
            }
        }
    }
}
