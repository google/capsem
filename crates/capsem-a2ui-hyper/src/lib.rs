use std::{convert::Infallible, net::SocketAddr};

use bytes::Bytes;
use capsem_ui_catalog::{
    ui::validate_messages,
    ui_tools::{run_tool_program, UiToolObservation, UiToolProgram, UiToolSurfaceSnapshot},
};
use http_body_util::{BodyExt, Full};
use hyper::{
    body::Incoming,
    header::{HeaderValue, CONTENT_TYPE},
    http::StatusCode,
    service::service_fn,
    Method, Request, Response,
};
use hyper_util::rt::TokioIo;
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::{net::TcpListener, sync::oneshot};

const MAX_BODY_BYTES: usize = 1024 * 1024;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct A2uiRenderResponse {
    pub ok: bool,
    pub conformance: A2uiConformance,
    pub observations: Vec<UiToolObservation>,
    pub surfaces: Vec<UiToolSurfaceSnapshot>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct A2uiConformance {
    pub ok: bool,
    pub errors: Vec<String>,
}

pub async fn serve(
    listener: TcpListener,
    mut shutdown: oneshot::Receiver<()>,
) -> anyhow::Result<()> {
    loop {
        tokio::select! {
            biased;
            _ = &mut shutdown => return Ok(()),
            accepted = listener.accept() => {
                let (stream, _peer) = accepted?;
                tokio::spawn(async move {
                    let io = TokioIo::new(stream);
                    if let Err(error) = hyper::server::conn::http1::Builder::new()
                        .serve_connection(io, service_fn(handle_request))
                        .await
                    {
                        eprintln!("hyper a2ui connection error: {error}");
                    }
                });
            }
        }
    }
}

pub async fn bind_and_serve(
    addr: SocketAddr,
    shutdown: oneshot::Receiver<()>,
) -> anyhow::Result<()> {
    let listener = TcpListener::bind(addr).await?;
    serve(listener, shutdown).await
}

pub async fn handle_request(
    request: Request<Incoming>,
) -> Result<Response<Full<Bytes>>, Infallible> {
    Ok(match (request.method(), request.uri().path()) {
        (&Method::GET, "/health") => json_response(
            StatusCode::OK,
            &json!({
                "ok": true,
                "service": "capsem-a2ui-hyper"
            }),
        ),
        (&Method::POST, "/a2ui/render") => render(request).await,
        (&Method::GET | &Method::POST, _) => json_error(StatusCode::NOT_FOUND, "not found"),
        _ => json_error(StatusCode::METHOD_NOT_ALLOWED, "method not allowed"),
    })
}

async fn render(request: Request<Incoming>) -> Response<Full<Bytes>> {
    let body = match request.into_body().collect().await {
        Ok(body) => body.to_bytes(),
        Err(error) => {
            return json_error(StatusCode::BAD_REQUEST, &format!("invalid body: {error}"))
        }
    };
    if body.len() > MAX_BODY_BYTES {
        return json_error(StatusCode::PAYLOAD_TOO_LARGE, "request body too large");
    }

    let program: UiToolProgram = match serde_json::from_slice(&body) {
        Ok(program) => program,
        Err(error) => {
            return json_error(StatusCode::BAD_REQUEST, &format!("invalid json: {error}"))
        }
    };

    let result = run_tool_program(program);
    let conformance = conformance_for(&result.surfaces);
    let ok = result.ok && conformance.ok;
    let response = A2uiRenderResponse {
        ok,
        conformance,
        observations: result.observations,
        surfaces: result.surfaces,
    };
    let status = if ok {
        StatusCode::OK
    } else {
        StatusCode::UNPROCESSABLE_ENTITY
    };
    json_response(status, &response)
}

pub fn conformance_for(surfaces: &[UiToolSurfaceSnapshot]) -> A2uiConformance {
    let mut errors = Vec::new();
    if surfaces.is_empty() {
        errors.push("render output must contain at least one surface".to_owned());
    }
    for surface in surfaces {
        if let Err(error) = validate_messages(&surface.messages) {
            errors.push(format!("{}: {error}", surface.surface_id));
        }
    }
    A2uiConformance {
        ok: errors.is_empty(),
        errors,
    }
}

fn json_response<T: Serialize>(status: StatusCode, value: &T) -> Response<Full<Bytes>> {
    let body = serde_json::to_vec(value).expect("json response serializes");
    let mut response = Response::new(Full::new(Bytes::from(body)));
    *response.status_mut() = status;
    response
        .headers_mut()
        .insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    response
}

fn json_error(status: StatusCode, message: &str) -> Response<Full<Bytes>> {
    json_response(
        status,
        &json!({
            "ok": false,
            "error": message,
        }),
    )
}
