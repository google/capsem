use super::*;

const ORIGIN: &str = "0199df26-d0f2-74f2-a304-ef67b79d1217.localhost:19444";

fn split(response: &[u8]) -> (String, Vec<u8>) {
    let end = response.windows(4).position(|w| w == b"\r\n\r\n").unwrap();
    (
        String::from_utf8(response[..end].to_vec()).unwrap(),
        response[end + 4..].to_vec(),
    )
}

fn get(path: &str) -> (String, Vec<u8>) {
    split(&response("GET", path, true, ORIGIN))
}

#[test]
fn the_pinned_client_page_splits_into_a_script_free_page() {
    let page = PAGE.as_ref().expect("the vendored index.html transforms");
    assert!(!page.index.contains("<script>"), "no inline script remains");
    let surface = page.index.find("<script src=\"capsem-surface.js\"></script>").unwrap();
    let main = page.index.find("<script src=\"xpra-main.js\"></script>").unwrap();
    let client = page.index.find("src=\"js/Client.js\"").unwrap();
    let window = page.index.find("src=\"js/Window.js\"").unwrap();
    // capsem-surface.js patches classes the client scripts define, before
    // the inline script starts the client.
    assert!(client < surface && window < surface && surface < main);
    assert!(
        page.main.contains("function init_client()"),
        "the inline script moved whole"
    );
    assert!(!page.main.contains("<script") && !page.main.contains("</script>"));
}

#[test]
fn the_page_split_refuses_a_page_it_cannot_split_exactly() {
    let ok = "<html><head><script src=\"a.js\"></script></head><body><script>go()</script></body></html>";
    let page = split_index(ok).unwrap();
    assert_eq!(page.main, "go()");
    assert_eq!(
        page.index,
        "<html><head><script src=\"a.js\"></script><script src=\"capsem-surface.js\"></script>\n  </head><body><script src=\"xpra-main.js\"></script></body></html>"
    );
    for bad in [
        "<html><head></head><body></body></html>",
        "<html><head></head><body><script>a()</script><script>b()</script></body></html>",
        "<html><head><script>a()</script></head><body></body></html>",
        "<html><body><script>a()</script></body></html>",
        "<html><head></head><body><script>a()</body></html>",
    ] {
        assert!(split_index(bad).is_err(), "{bad}");
    }
}

#[test]
fn vendored_files_are_served_byte_for_byte_with_their_types() {
    for (path, content_type) in [
        ("js/Client.js", "text/javascript; charset=utf-8"),
        ("js/lib/jquery.js", "text/javascript; charset=utf-8"),
        ("css/client.css", "text/css; charset=utf-8"),
        ("icons/materialicons-regular.woff2", "font/woff2"),
        ("favicon.png", "image/png"),
    ] {
        let (head, body) = get(&format!("{CLIENT_ROOT}{path}"));
        assert!(head.starts_with("HTTP/1.1 200 OK\r\n"), "{path}: {head}");
        assert!(
            head.contains(&format!("Content-Type: {content_type}\r\n")),
            "{path}: {head}"
        );
        assert_eq!(body, vendored(path).unwrap(), "{path}");
    }
    // The page itself, its root and the moved script.
    let (_, index) = get(CLIENT_ROOT);
    assert_eq!(index, PAGE.as_ref().unwrap().index.as_bytes());
    let (_, named) = get(&format!("{CLIENT_ROOT}index.html"));
    assert_eq!(index, named);
    let (_, main) = get(&format!("{CLIENT_ROOT}xpra-main.js"));
    assert_eq!(main, PAGE.as_ref().unwrap().main.as_bytes());
}

#[test]
fn capsem_serves_its_own_settings_and_disconnect_page() {
    let (_, settings) = get(&format!("{CLIENT_ROOT}default-settings.txt"));
    let settings = String::from_utf8(settings).unwrap();
    for line in [
        "path = /",
        "floating_menu = false",
        "keyboard = false",
        "sound = false",
        "file_transfer = false",
        "clipboard_poll = false",
    ] {
        assert!(settings.lines().any(|l| l == line), "settings lack {line:?}");
    }
    let (_, disconnected) = get(&format!("{CLIENT_ROOT}connect.html"));
    let disconnected = String::from_utf8(disconnected).unwrap();
    assert!(disconnected.contains("Capsem") && !disconnected.contains("<script"));
    let (_, surface) = get(&format!("{CLIENT_ROOT}capsem-surface.js"));
    assert!(String::from_utf8(surface).unwrap().contains("system_tray = false"));
}

#[test]
fn every_client_response_carries_a_policy_that_allows_no_inline_script() {
    let (head, _) = get(CLIENT_ROOT);
    let policy = head
        .lines()
        .find_map(|line| line.strip_prefix("Content-Security-Policy: "))
        .expect("CSP header");
    // Exactly 'self': no inline script, no hashes, no eval.
    assert!(policy.contains("script-src 'self';"), "{policy}");
    assert!(policy.contains("worker-src 'self';"), "{policy}");
    assert!(!policy.contains("unsafe-eval"), "{policy}");
    assert!(
        policy.contains(&format!("connect-src 'self' ws://{ORIGIN};")),
        "{policy}"
    );
    assert!(policy.contains("frame-ancestors 'none'"), "{policy}");
    for header in [
        "X-Frame-Options: DENY",
        "X-Content-Type-Options: nosniff",
        "Referrer-Policy: no-referrer",
        "Cache-Control: no-store",
        "Connection: close",
    ] {
        assert!(head.contains(header), "{header} missing: {head}");
    }
    let (worker, _) = get(&format!("{CLIENT_ROOT}js/Protocol.js"));
    assert!(
        worker.contains("Content-Security-Policy: "),
        "workers take their own policy"
    );
}

#[test]
fn the_client_is_served_only_to_a_session_this_gateway_issued() {
    for path in [CLIENT_ROOT.to_owned(), format!("{CLIENT_ROOT}js/Client.js")] {
        let (head, body) = split(&response("GET", &path, false, ORIGIN));
        assert!(head.starts_with("HTTP/1.1 401 Unauthorized\r\n"), "{head}");
        assert!(String::from_utf8(body).unwrap().contains("expired"));
    }
}

#[test]
fn unknown_paths_methods_and_traversal_are_refused() {
    for path in [
        "js/nope.js",
        "../index.html",
        "js/../index.html",
        "connect.html.gz",
        "sw.js",
        "mitm.html",
        "default-settings.txt.gz",
    ] {
        let (head, _) = get(&format!("{CLIENT_ROOT}{path}"));
        assert!(head.starts_with("HTTP/1.1 404 Not Found\r\n"), "{path}: {head}");
    }
    let (head, _) = split(&response("POST", CLIENT_ROOT, true, ORIGIN));
    assert!(head.starts_with("HTTP/1.1 405 Method Not Allowed\r\n"), "{head}");
    let head_only = response("HEAD", CLIENT_ROOT, true, ORIGIN);
    let (head, body) = split(&head_only);
    assert!(head.starts_with("HTTP/1.1 200 OK\r\n") && body.is_empty());
    // A query string names nothing on its own.
    let (head, body) = get(&format!("{CLIENT_ROOT}?server=evil.example&port=443"));
    assert!(head.starts_with("HTTP/1.1 200 OK\r\n"));
    assert_eq!(body, PAGE.as_ref().unwrap().index.as_bytes());
}
