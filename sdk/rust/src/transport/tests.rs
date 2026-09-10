use super::*;
use axum::body::{to_bytes, Body};
use axum::http::{Request as HttpRequest, Response};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

struct Server {
    url: String,
    received: mpsc::UnboundedReceiver<(axum::http::request::Parts, Vec<u8>)>,
    task: JoinHandle<()>,
}

impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl Server {
    async fn reply(status: u16, body: &[u8], redirect: Option<String>) -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let (sent, received) = mpsc::unbounded_channel();
        let bytes = body.to_vec();
        let app = axum::Router::new().fallback(move |incoming: HttpRequest<Body>| {
            let sent = sent.clone();
            let bytes = bytes.clone();
            let redirect = redirect.clone();
            async move {
                let (parts, body) = incoming.into_parts();
                sent.send((parts, to_bytes(body, 1_000_000).await.unwrap().to_vec()))
                    .unwrap();
                let mut response = Response::builder().status(status);
                if let Some(location) = redirect {
                    response = response.header("Location", location);
                }
                response.body(Body::from(bytes)).unwrap()
            }
        });
        let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        Self { url, received, task }
    }

    fn client(&self) -> Transport {
        Transport::new(&self.url, "private-token", Duration::from_secs(2)).unwrap()
    }
}

#[test]
fn invalid_configuration_fails_without_network() {
    for url in [
        "broken",
        "file:///tmp/sock",
        "ftp://host",
        "http://user@host",
        "http://u:p@host",
        "http://host?a=1",
        "http://host#hash",
    ] {
        assert!(matches!(
            Transport::new(url, "token", Duration::from_secs(1)),
            Err(Error::InvalidInput(_))
        ));
    }
    for token in ["", "abc\r\nevil", "abc\0"] {
        assert!(matches!(
            Transport::new("http://localhost", token, Duration::from_secs(1)),
            Err(Error::InvalidInput(_))
        ));
    }
    assert!(Transport::new("http://localhost", "token", Duration::ZERO).is_err());
    let client = Transport::new("https://localhost", "private-token", Duration::from_secs(1)).unwrap();
    assert!(!format!("{client:?}").contains("private-token"));
}

#[tokio::test]
async fn path_query_auth_and_binary_payload_survive_http() {
    let mut server = Server::reply(200, b"\0\xff\r\n", None).await;
    let client = Transport::new(
        &format!("{}/prefix/", server.url),
        "private-token",
        Duration::from_secs(2),
    )
    .unwrap();
    let result = client
        .request(
            Method::POST,
            "/vms/{id}/copy",
            Request {
                parameters: &[("id", "a/b ?#%é")],
                query: &[
                    ("path", "/space here/é".into()),
                    ("layers", "net,model".into()),
                    ("enabled", "false".into()),
                ],
                body: Some(vec![0, 255, 13, 10]),
                content_type: MediaType::Binary,
                accept: MediaType::Binary,
                options: CallOptions {
                    timeout: Some(Duration::from_secs(1)),
                },
            },
        )
        .await
        .unwrap();
    assert_eq!(result, b"\0\xff\r\n");
    let (parts, body) = server.received.recv().await.unwrap();
    assert_eq!(parts.method, Method::POST);
    assert_eq!(parts.uri.path(), "/prefix/vms/a%2Fb%20%3F%23%25%C3%A9/copy");
    assert_eq!(
        parts.uri.query(),
        Some("path=%2Fspace+here%2F%C3%A9&layers=net%2Cmodel&enabled=false")
    );
    assert_eq!(parts.headers[AUTHORIZATION], "Bearer private-token");
    assert_eq!(parts.headers[ACCEPT], "application/octet-stream");
    assert_eq!(parts.headers[CONTENT_TYPE], "application/octet-stream");
    assert_eq!(body, result);
}

#[tokio::test]
async fn json_content_and_no_body_requests() {
    let mut server = Server::reply(200, b"{}", None).await;
    let client = server.client();
    for body in [None, Some(b"{\"confirm\":true}".to_vec())] {
        client
            .request(
                Method::DELETE,
                "/vm",
                Request {
                    body: body.clone(),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        let (parts, actual) = server.received.recv().await.unwrap();
        assert_eq!(parts.headers[ACCEPT], "application/json");
        assert_eq!(parts.headers.contains_key(CONTENT_TYPE), body.is_some());
        assert_eq!(actual, body.unwrap_or_default());
    }
}

#[tokio::test]
async fn invalid_requests_do_not_reach_gateway() {
    let mut server = Server::reply(200, b"{}", None).await;
    let client = server.client();
    for path in [
        "relative",
        "/a?query",
        "/a#hash",
        "/{missing}",
        "/a{bad}",
        "/{bad",
        "/a//b",
        "/../b",
    ] {
        assert!(matches!(
            client.request(Method::GET, path, Request::default()).await,
            Err(Error::InvalidInput(_))
        ));
    }
    for value in ["", ".", ".."] {
        assert!(matches!(
            client
                .request(
                    Method::GET,
                    "/{id}",
                    Request {
                        parameters: &[("id", value)],
                        ..Default::default()
                    }
                )
                .await,
            Err(Error::InvalidInput(_))
        ));
    }
    let request = Request {
        options: CallOptions {
            timeout: Some(Duration::ZERO),
        },
        ..Default::default()
    };
    assert!(matches!(
        client.request(Method::GET, "/vm", request).await,
        Err(Error::InvalidInput(_))
    ));
    assert!(server.received.try_recv().is_err());
}

#[tokio::test]
async fn errors_and_redirects_are_returned_once_without_following() {
    let mut destination = Server::reply(200, b"secret", None).await;
    for status in [301, 302, 303, 307, 308, 401, 403, 404, 409, 429, 500, 503] {
        let mut server = Server::reply(status, b"denied\xff", Some(destination.url.clone())).await;
        let error = server
            .client()
            .request(Method::POST, "/mutate", Request::default())
            .await
            .unwrap_err();
        assert!(matches!(error, Error::Http { status: actual, body } if actual == status && body == b"denied\xff"));
        assert!(server.received.recv().await.is_some());
        assert!(server.received.try_recv().is_err());
        assert!(destination.received.try_recv().is_err());
    }
}

#[tokio::test]
async fn refused_connection_is_typed() {
    let mut server = Server::reply(200, b"{}", None).await;
    let client = server.client();
    server.task.abort();
    let _ = (&mut server.task).await;
    assert!(matches!(
        client.request(Method::GET, "/vm", Request::default()).await,
        Err(Error::Transport(_))
    ));
}

#[tokio::test]
async fn deadlines_cover_headers_and_body_and_dropped_futures_disconnect() {
    for (headers, cancel) in [(false, false), (true, false), (true, true)] {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let client = Transport::new(&url, "token", Duration::from_millis(100)).unwrap();
        let (started, ready) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut buffer = [0; 4096];
            assert!(socket.read(&mut buffer).await.unwrap() > 0);
            if headers {
                socket
                    .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\n\r\na")
                    .await
                    .unwrap();
            }
            started.send(()).unwrap();
            assert_eq!(socket.read(&mut buffer).await.unwrap(), 0);
        });
        let mut request = tokio::spawn(async move { client.request(Method::GET, "/vm", Request::default()).await });
        ready.await.unwrap();
        if cancel {
            request.abort();
            assert!(request.await.unwrap_err().is_cancelled());
        } else {
            let error = tokio::time::timeout(Duration::from_secs(2), &mut request)
                .await
                .unwrap()
                .unwrap()
                .unwrap_err();
            assert!(matches!(error, Error::Transport(error) if error.is_timeout()));
        }
        tokio::time::timeout(Duration::from_secs(2), server)
            .await
            .unwrap()
            .unwrap();
    }
}
