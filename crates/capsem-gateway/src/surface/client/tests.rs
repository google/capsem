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
        "<html><head><script src=\"a.js\"></script><link rel=\"stylesheet\" href=\"capsem-surface.css\" />\n  <script src=\"capsem-surface-metrics.js\"></script>\n  <script src=\"capsem-surface.js\"></script>\n  </head><body><script src=\"xpra-main.js\"></script></body></html>"
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
fn display_diagnostics_are_served_as_owned_script_without_changing_vendor_files() {
    let (head, script) = get(&format!("{CLIENT_ROOT}capsem-surface-metrics.js"));
    assert!(head.starts_with("HTTP/1.1 200 OK\r\n"));
    assert!(head.contains("Content-Type: text/javascript; charset=utf-8"));
    assert!(String::from_utf8(script)
        .unwrap()
        .contains("class CapsemSurfaceMetrics"));
    let page = PAGE.as_ref().unwrap();
    let metrics = page.index.find("src=\"capsem-surface-metrics.js\"").unwrap();
    let policy = page.index.find("src=\"capsem-surface.js\"").unwrap();
    assert!(
        metrics < policy,
        "read the trusted parent origin before bootstrap metadata is removed"
    );
}

#[test]
fn graphical_sessions_keep_adaptive_encoding_without_low_quality_or_downscaling() {
    let (_, settings) = get(&format!("{CLIENT_ROOT}default-settings.txt"));
    let settings = String::from_utf8(settings).unwrap();
    let value = |key: &str| {
        settings
            .lines()
            .filter_map(|line| line.split_once('='))
            .find_map(|(name, value)| (name.trim() == key).then(|| value.trim().parse::<u32>().unwrap()))
    };
    // Leave codec and quality selection to Xpra, within a useful quality
    // floor. Fixed lossless quality can stall a high-motion application.
    assert_eq!(value("initial_quality"), None);
    assert_eq!(value("quality"), None);
    assert_eq!(value("min-quality"), Some(80));
    assert!(!settings.lines().any(|line| line.trim_start().starts_with("encoding =")));
    assert_eq!(value("scaling.control"), Some(0));
}

#[test]
fn the_single_app_page_disables_the_unbundled_desktop_background() {
    let (_, page) = get(CLIENT_ROOT);
    let page = String::from_utf8(page).unwrap();
    let vendor = page.find("href=\"css/client.css\"").unwrap();
    let owned = page.find("href=\"capsem-surface.css\"").expect("single-app stylesheet");
    assert!(
        vendor < owned,
        "the single-app policy overrides the vendor desktop theme"
    );
    let (head, css) = get(&format!("{CLIENT_ROOT}capsem-surface.css"));
    assert!(head.starts_with("HTTP/1.1 200 OK\r\n"), "{head}");
    assert!(head.contains("Content-Type: text/css; charset=utf-8\r\n"), "{head}");
    let css = String::from_utf8(css).unwrap();
    assert!(
        css.contains("background-image: none"),
        "no unbundled desktop wallpaper request"
    );
    assert!(!css.contains("url("), "the override needs no external assets");
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
fn client_documents_require_a_session_while_static_code_needs_no_cookie() {
    for path in [
        CLIENT_ROOT.to_owned(),
        format!("{CLIENT_ROOT}index.html"),
        format!("{CLIENT_ROOT}connect.html"),
    ] {
        let (head, body) = split(&response("GET", &path, false, ORIGIN));
        assert!(head.starts_with("HTTP/1.1 401 Unauthorized\r\n"), "{head}");
        assert!(String::from_utf8(body).unwrap().contains("expired"));
    }
    let (head, body) = split(&response("GET", &format!("{CLIENT_ROOT}js/Client.js"), false, ORIGIN));
    assert!(head.starts_with("HTTP/1.1 200 OK\r\n"));
    assert_eq!(body, vendored("js/Client.js").unwrap());
    assert!(!body.windows(15).any(|bytes| bytes == b"scoped-session-"));
}

#[test]
fn the_unused_vendor_wallpaper_is_a_public_empty_image() {
    let (head, body) = split(&response("GET", &format!("{CLIENT_ROOT}background.jpg"), false, ORIGIN));
    assert!(head.starts_with("HTTP/1.1 200 OK\r\n"), "{head}");
    assert!(head.contains("Content-Type: image/svg+xml"));
    assert_eq!(
        body,
        b"<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"1\" height=\"1\"/>\n"
    );
}

#[test]
fn embedded_page_binds_one_trusted_parent_without_inline_script_or_url_credentials() {
    let token = "b".repeat(64);
    let parent = "http://127.0.0.1:5173";
    let (head, body) = split(&embedded_response(&token, ORIGIN, parent).unwrap());
    assert!(head.starts_with("HTTP/1.1 200 OK\r\n"));
    assert!(head.contains(&format!("frame-ancestors {parent};")));
    assert!(!head.contains("X-Frame-Options:") && !head.contains("Set-Cookie:"));
    assert!(head.contains("script-src 'self';") && head.contains("worker-src 'self';"));
    assert!(head.contains("Cache-Control: no-store") && head.contains("Referrer-Policy: no-referrer"));
    assert!(!head.contains(&token));
    let body = String::from_utf8(body).unwrap();
    assert!(
        body.find("<meta name=\"capsem-surface-protocol\"").unwrap()
            < body.find("<script src=\"capsem-surface.js\"").unwrap(),
        "the credential must exist when the transport hook runs during HTML parsing"
    );
    assert!(body.contains(&format!(
        "content=\"{}{token}\"",
        capsem_proto::PREVIEW_WS_PROTOCOL_PREFIX
    )));
    assert!(!body.contains("<script>") && !body.contains(&format!("?{token}")));
    for parent in [
        "null",
        "https://attacker.test",
        "http://localhost.evil.test",
        "http://127.0.0.1:5173/?x=y",
    ] {
        assert!(embedded_response(&token, ORIGIN, parent).is_err());
    }
    for token in ["bad", "\"><script>bad()</script>"] {
        assert!(embedded_response(token, ORIGIN, parent).is_err());
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
