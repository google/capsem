//! Bounded deterministic text and genuinely streamed fixtures for content filters.

use super::{response, response_with_header, LogBody, RespBody};
use bytes::Bytes;
use flate2::{write::GzEncoder, Compression};
use futures::stream;
use http_body_util::{BodyExt, StreamBody};
use hyper::body::Frame;
use hyper::{HeaderMap, Method, Response, StatusCode};
use std::io::Write;
use std::{convert::Infallible, time::Duration};

pub(crate) const ENDPOINTS: &[&str] = &["/content/echo", "/content/text", "/content/stream"];

pub(crate) fn route(
    method: &Method,
    path: &str,
    query: Option<&str>,
    headers: &HeaderMap,
    request: Bytes,
) -> Response<RespBody> {
    if *method == Method::GET && matches!(path, "/chunked" | "/delayed-chunks") {
        return response(
            StatusCode::OK,
            Bytes::from_static(b"chunk-0\nchunk-1\nchunk-2\nchunk-3\n"),
            "text/plain; charset=utf-8",
        );
    }
    if *method == Method::POST && path == "/content/echo" {
        let media = headers
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("application/octet-stream");
        if query == Some("gzip=1") {
            let mut gzip = GzEncoder::new(Vec::new(), Compression::default());
            gzip.write_all(&request).expect("in-memory compression");
            return response_with_header(
                StatusCode::OK,
                Bytes::from(gzip.finish().expect("finish in-memory gzip")),
                media,
                "content-encoding",
                "gzip",
            );
        }
        return response(StatusCode::OK, request, media);
    }
    if *method != Method::GET || !matches!(path, "/content/text" | "/content/stream") {
        return response(StatusCode::NOT_FOUND, Bytes::new(), "text/plain");
    }
    let parameters: Vec<_> = query
        .unwrap_or("")
        .split('&')
        .filter(|s| !s.is_empty())
        .filter_map(|part| part.split_once('='))
        .collect();
    let size = parameter(&parameters, "bytes", 4096, super::limits::MAX_GENERATED_BYTES);
    let delay = parameter(&parameters, "delay_ms", 50, 1000);
    let (Some(size), Some(delay)) = (size, delay) else {
        return response(StatusCode::BAD_REQUEST, Bytes::new(), "text/plain");
    };
    if path == "/content/text" {
        return response(
            StatusCode::OK,
            Bytes::from(vec![b'x'; size]),
            "text/plain; charset=utf-8",
        );
    }
    // First chunk is immediately ready, the final event is deliberately delayed.
    // Both carry no secret or PII; enabled filters must buffer the complete stream.
    let mut generated = Vec::with_capacity(size + 32);
    generated.extend_from_slice(b"data: begin\n\ndata: ");
    generated.resize(generated.len() + size, b'x');
    generated.extend_from_slice(b"\n\ndata: end\n\n");
    let logged = Bytes::from(generated);
    let prefix = logged.slice(..b"data: begin\n\n".len());
    let tail = logged.slice(prefix.len()..);
    let frames = stream::unfold((0, prefix, tail), move |(index, prefix, tail)| async move {
        match index {
            0 => Some((Ok::<_, Infallible>(Frame::data(prefix.clone())), (1, prefix, tail))),
            1 => {
                tokio::time::sleep(Duration::from_millis(delay as u64)).await;
                Some((Ok(Frame::data(tail.clone())), (2, prefix, tail)))
            }
            _ => None,
        }
    });
    let mut response = Response::builder()
        .status(StatusCode::OK)
        .header("content-type", "text/event-stream")
        .body(StreamBody::new(frames).boxed())
        .expect("static response headers");
    response.extensions_mut().insert(LogBody(logged));
    response
}

fn parameter(parameters: &[(&str, &str)], name: &str, default: usize, cap: usize) -> Option<usize> {
    let selected: Vec<_> = parameters.iter().filter(|(key, _)| *key == name).collect();
    match selected.as_slice() {
        [] => Some(default),
        [(_, value)] => value.parse::<usize>().ok().filter(|n| *n <= cap),
        _ => None,
    }
}

#[cfg(test)]
mod tests;
