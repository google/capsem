use super::*;
use hyper::header::{AUTHORIZATION, COOKIE, PROXY_AUTHORIZATION};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[test]
fn control_credentials_are_removed_before_guest_http() {
    let mut headers = hyper::HeaderMap::new();
    headers.insert(AUTHORIZATION, "Bearer administrator".parse().unwrap());
    headers.insert(PROXY_AUTHORIZATION, "Basic c2VjcmV0".parse().unwrap());
    headers.insert(
        COOKIE,
        "theme=dark; capsem_preview=owner-secret; app=value".parse().unwrap(),
    );
    strip_control_headers(&mut headers);
    assert!(!headers.contains_key(AUTHORIZATION));
    assert!(!headers.contains_key(PROXY_AUTHORIZATION));
    assert_eq!(headers.get(COOKIE).unwrap(), "theme=dark; app=value");
}

#[test]
fn an_only_control_cookie_leaves_no_cookie_header() {
    let mut headers = hyper::HeaderMap::new();
    headers.insert(COOKIE, "capsem_preview=owner-secret".parse().unwrap());
    strip_control_headers(&mut headers);
    assert!(!headers.contains_key(COOKIE));
}

#[test]
fn workload_cookies_survive_but_the_reserved_session_cookie_does_not() {
    let mut headers = hyper::HeaderMap::new();
    headers.append(SET_COOKIE, "app_session=guest; HttpOnly".parse().unwrap());
    headers.append(SET_COOKIE, "capsem_preview=forged; Path=/".parse().unwrap());
    strip_control_set_cookies(&mut headers);
    let values = headers
        .get_all(SET_COOKIE)
        .iter()
        .map(|value| value.to_str().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(values, ["app_session=guest; HttpOnly"]);
}

async fn read_framed_http(stream: &mut UnixStream) -> Vec<u8> {
    let mut request = Vec::new();
    loop {
        let length = stream.read_u32().await.unwrap();
        if length == 0 {
            return request;
        }
        let mut payload = vec![0; length as usize];
        stream.read_exact(&mut payload).await.unwrap();
        request.extend_from_slice(&payload);
        if let Some(headers) = request.windows(4).position(|window| window == b"\r\n\r\n") {
            let header_end = headers + 4;
            let text = std::str::from_utf8(&request[..header_end]).unwrap();
            let content_length = text
                .lines()
                .find_map(|line| line.strip_prefix("content-length: "))
                .and_then(|value| value.parse::<usize>().ok())
                .unwrap_or(0);
            if request.len() >= header_end + content_length {
                return request;
            }
        }
    }
}

async fn write_frame(stream: &mut UnixStream, bytes: &[u8]) {
    stream.write_u32(bytes.len() as u32).await.unwrap();
    stream.write_all(bytes).await.unwrap();
}

#[tokio::test]
async fn confined_preview_streams_http_and_filters_both_control_directions() {
    let (mut browser, mut source) = UnixStream::pair().unwrap();
    let (mut destination, mut guest) = UnixStream::pair().unwrap();
    let relay = tokio::spawn(async move { relay(&mut source, &mut destination, std::future::pending()).await });
    let guest_task = tokio::spawn(async move {
        let request = read_framed_http(&mut guest).await;
        let request = String::from_utf8(request).unwrap();
        assert!(request.starts_with("POST /submit HTTP/1.1\r\n"), "{request}");
        assert!(request.contains("cookie: theme=dark\r\n"), "{request}");
        assert!(!request.to_ascii_lowercase().contains("authorization:"), "{request}");
        assert!(!request.contains("capsem_preview"), "{request}");
        assert!(request.ends_with("\r\n\r\nname=capsem"), "{request}");

        write_frame(
            &mut guest,
            b"HTTP/1.1 302 Found\r\nLocation: /next\r\nSet-Cookie: guest=secret; HttpOnly\r\nSet-Cookie: capsem_preview=forged\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n6\r\nstream\r\n",
        )
        .await;
        write_frame(&mut guest, b"3\r\ned!\r\n0\r\n\r\n").await;
        guest.write_u32(0).await.unwrap();
    });

    browser
        .write_all(
            b"POST /submit HTTP/1.1\r\nHost: preview.localhost\r\nAuthorization: Bearer administrator\r\nProxy-Authorization: Basic c2VjcmV0\r\nCookie: theme=dark; capsem_preview=owner-secret\r\nContent-Length: 11\r\nConnection: close\r\n\r\nname=capsem",
        )
        .await
        .unwrap();
    let mut response = Vec::new();
    tokio::time::timeout(std::time::Duration::from_secs(2), browser.read_to_end(&mut response))
        .await
        .unwrap()
        .unwrap();
    let response = String::from_utf8(response).unwrap();
    assert!(response.starts_with("HTTP/1.1 302 Found\r\n"), "{response}");
    assert!(response.contains("location: /next\r\n"), "{response}");
    assert!(
        response.contains("set-cookie: guest=secret; HttpOnly\r\n"),
        "{response}"
    );
    assert!(!response.contains("capsem_preview"), "{response}");
    assert!(response.ends_with("6\r\nstream\r\n3\r\ned!\r\n0\r\n\r\n"), "{response}");
    guest_task.await.unwrap();
    let outcome = tokio::time::timeout(std::time::Duration::from_secs(2), relay)
        .await
        .unwrap()
        .unwrap();
    assert!(
        matches!(
            outcome.reason,
            capsem_foundation::unix::router_stream::CloseReason::Complete
                | capsem_foundation::unix::router_stream::CloseReason::Reset
        ),
        "the delivered response may end by orderly close or browser reset: {outcome:?}"
    );
}

#[tokio::test]
async fn confined_preview_carries_websocket_upgrades_after_filtering_credentials() {
    let (mut browser, mut source) = UnixStream::pair().unwrap();
    let (mut destination, mut guest) = UnixStream::pair().unwrap();
    let relay = tokio::spawn(async move { relay(&mut source, &mut destination, std::future::pending()).await });
    let guest_task = tokio::spawn(async move {
        let request = String::from_utf8(read_framed_http(&mut guest).await).unwrap();
        assert!(request.starts_with("GET /live HTTP/1.1\r\n"), "{request}");
        assert!(!request.to_ascii_lowercase().contains("authorization:"), "{request}");
        assert!(!request.contains("capsem_preview"), "{request}");
        write_frame(
            &mut guest,
            b"HTTP/1.1 101 Switching Protocols\r\nConnection: upgrade\r\nUpgrade: websocket\r\nSec-WebSocket-Accept: s3pPLMBiTxaQ9kYGzzhZRbK+xOo=\r\n\r\n",
        )
        .await;

        let length = guest.read_u32().await.unwrap();
        let mut frame = vec![0; length as usize];
        guest.read_exact(&mut frame).await.unwrap();
        assert_eq!(frame, [0x81, 0x82, 1, 2, 3, 4, 0x6e, 0x69]);
        write_frame(&mut guest, &[0x81, 0x02, b'h', b'i']).await;
        guest.write_u32(0).await.unwrap();
    });

    browser
        .write_all(
            b"GET /live HTTP/1.1\r\nHost: preview.localhost\r\nConnection: upgrade\r\nUpgrade: websocket\r\nSec-WebSocket-Version: 13\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nAuthorization: Bearer administrator\r\nCookie: capsem_preview=owner-secret\r\n\r\n",
        )
        .await
        .unwrap();
    let mut response = Vec::new();
    loop {
        response.push(browser.read_u8().await.unwrap());
        if response.ends_with(b"\r\n\r\n") {
            break;
        }
    }
    let response = String::from_utf8(response).unwrap();
    assert!(
        response.starts_with("HTTP/1.1 101 Switching Protocols\r\n"),
        "{response}"
    );
    browser.write_all(&[0x81, 0x82, 1, 2, 3, 4, 0x6e, 0x69]).await.unwrap();
    let mut returned = [0; 4];
    tokio::time::timeout(std::time::Duration::from_secs(2), browser.read_exact(&mut returned))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(returned, [0x81, 0x02, b'h', b'i']);
    drop(browser);
    guest_task.await.unwrap();
    let _ = tokio::time::timeout(std::time::Duration::from_secs(2), relay)
        .await
        .unwrap()
        .unwrap();
}
