//! HTTP dispatch keeps bounded OTLP fixtures out of generic payload logging.
use crate::*;

pub(super) async fn handle_request(
    mut req: Request<Incoming>,
    state: State,
    tls: bool,
) -> Result<Response<RespBody>, Infallible> {
    let method = req.method().clone();
    let path = req.uri().path().to_string();
    if otlp::is_path(&path) {
        return Ok(otlp::handle(req, state.otlp.as_deref()).await);
    }
    if path.starts_with("/ws/")
        || (method == Method::GET
            && path == "/v1/responses"
            && req
                .headers()
                .get(UPGRADE)
                .is_some_and(|value| value.as_bytes().eq_ignore_ascii_case(b"websocket")))
    {
        let response = websocket::upgrade(req, path, state).await;
        return Ok(response);
    }

    let headers = req.headers().clone();
    let query = req.uri().query().map(str::to_owned);
    let request_body = req
        .body_mut()
        .collect()
        .await
        .map(|body| body.to_bytes())
        .unwrap_or_default();
    let response = route(&method, &path, query.as_deref(), &headers, request_body.clone(), tls).await;
    log_request(
        &state,
        &method,
        &path,
        query.as_deref(),
        &headers,
        &request_body,
        &response,
    );
    Ok(response)
}
