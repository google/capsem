//! The pinned xpra-html5 client, served on an Xpra surface's preview origin.
//!
//! The vendored files are served byte for byte, with two Capsem additions
//! loaded around them: `default-settings.txt` (the client's own settings
//! hook) and `capsem-surface.js` (one application, fitted to the viewport).
//! The client page's one inline script is served as `xpra-main.js`, so the
//! page runs under a policy that allows no inline script at all: anything a
//! workload manages to inject into the page's DOM cannot execute.

use std::sync::LazyLock;

/// Where the client lives on a surface's preview origin. `/_capsem/` is the
/// gateway's namespace there; requests under it never reach the workload.
pub(crate) const CLIENT_ROOT: &str = "/_capsem/surface/";

/// `(path, bytes)` for every vendored file, checked against SHA256SUMS by
/// the build script.
static VENDORED: &[(&str, &[u8])] = include!(concat!(env!("OUT_DIR"), "/xpra_html5.rs"));

const CAPSEM_SURFACE: &str = include_str!("assets/capsem-surface.js");
const SETTINGS: &str = include_str!("assets/default-settings.txt");
const DISCONNECTED: &str = include_str!("assets/connect.html");
const EXPIRED: &str = "This app session has expired. Open the app again from Capsem.";

/// The client page with its inline script moved out to `xpra-main.js`.
struct Page {
    index: String,
    main: String,
}

static PAGE: LazyLock<Result<Page, String>> = LazyLock::new(|| {
    let index = vendored("index.html").ok_or("the vendored client has no index.html")?;
    split_index(std::str::from_utf8(index).map_err(|e| e.to_string())?)
});

fn vendored(path: &str) -> Option<&'static [u8]> {
    VENDORED.iter().find(|(name, _)| *name == path).map(|(_, bytes)| *bytes)
}

/// Move the page's single inline `<script>` into `xpra-main.js`, in place,
/// and load `capsem-surface.js` after the client's classes are defined and
/// before that script starts the client.
fn split_index(index: &str) -> Result<Page, String> {
    const INLINE: &str = "<script>";
    const CLOSE: &str = "</script>";
    const HEAD_END: &str = "</head>";
    let one = |needle: &str| -> Result<usize, String> {
        let start = index.find(needle).ok_or(format!("client page has no {needle}"))?;
        if index[start + needle.len()..].contains(needle) {
            return Err(format!("client page has more than one {needle}"));
        }
        Ok(start)
    };
    let (start, head_end) = (one(INLINE)?, one(HEAD_END)?);
    if head_end > start {
        return Err("the client page's inline script is not in its body".into());
    }
    let body = start + INLINE.len();
    let end = body + index[body..].find(CLOSE).ok_or("unterminated inline script")?;
    Ok(Page {
        index: format!(
            "{}<script src=\"capsem-surface.js\"></script>\n  {}<script src=\"xpra-main.js\"></script>{}",
            &index[..head_end],
            &index[head_end..start],
            &index[end + CLOSE.len()..],
        ),
        main: index[body..end].to_owned(),
    })
}

fn content_type(path: &str) -> &'static str {
    match path.rsplit_once('.').map(|(_, extension)| extension) {
        Some("html") => "text/html; charset=utf-8",
        Some("js") => "text/javascript; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("txt") => "text/plain; charset=utf-8",
        Some("map") => "application/json",
        Some("png") => "image/png",
        Some("ico") => "image/x-icon",
        Some("woff") => "font/woff",
        Some("woff2") => "font/woff2",
        Some("ttf") => "font/ttf",
        _ => "application/octet-stream",
    }
}

/// The bytes and media type at `relative`, below [`CLIENT_ROOT`].
fn asset(relative: &str) -> Option<(&'static [u8], &'static str)> {
    let page = PAGE.as_ref().ok();
    let bytes: &'static [u8] = match relative {
        "" | "index.html" => page?.index.as_bytes(),
        "xpra-main.js" => page?.main.as_bytes(),
        "capsem-surface.js" => CAPSEM_SURFACE.as_bytes(),
        "default-settings.txt" => SETTINGS.as_bytes(),
        "connect.html" => DISCONNECTED.as_bytes(),
        other => vendored(other)?,
    };
    let name = if relative.is_empty() { "index.html" } else { relative };
    Some((bytes, content_type(name)))
}

/// The policy every client response carries. Workers take theirs from their
/// own script's response, so scripts carry it too. The websocket may reach
/// this origin only: a link that names another server cannot redirect it.
pub(crate) fn content_security_policy(origin: &str) -> String {
    format!(
        "default-src 'none'; script-src 'self'; worker-src 'self'; connect-src 'self' ws://{origin}; \
         img-src 'self' data: blob:; font-src 'self'; style-src 'self' 'unsafe-inline'; media-src 'self' blob:; \
         form-action 'none'; frame-ancestors 'none'; base-uri 'none'"
    )
}

/// A complete `Connection: close` HTTP/1.1 response for a request under
/// [`CLIENT_ROOT`] on the preview origin `origin` (`label.localhost:port`).
/// `authenticated` says whether it carried a session this gateway issued for
/// that origin.
pub(crate) fn response(method: &str, path: &str, authenticated: bool, origin: &str) -> Vec<u8> {
    let relative = path
        .split(['?', '#'])
        .next()
        .and_then(|path| path.strip_prefix(CLIENT_ROOT));
    let (status, content_type, body): (&str, &str, &[u8]) = match (method, relative) {
        _ if !authenticated => ("401 Unauthorized", "text/plain; charset=utf-8", EXPIRED.as_bytes()),
        ("GET" | "HEAD", Some(relative)) => match asset(relative) {
            Some((bytes, content_type)) => ("200 OK", content_type, bytes),
            None => ("404 Not Found", "text/plain; charset=utf-8", b"Not found."),
        },
        ("GET" | "HEAD", None) => ("404 Not Found", "text/plain; charset=utf-8", b"Not found."),
        _ => (
            "405 Method Not Allowed",
            "text/plain; charset=utf-8",
            b"Method not allowed.",
        ),
    };
    let mut response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\n\
         Content-Security-Policy: {}\r\nX-Content-Type-Options: nosniff\r\nX-Frame-Options: DENY\r\n\
         Referrer-Policy: no-referrer\r\nCross-Origin-Opener-Policy: same-origin\r\nCache-Control: no-store\r\n\
         Connection: close\r\n\r\n",
        body.len(),
        content_security_policy(origin),
    )
    .into_bytes();
    if method != "HEAD" {
        response.extend_from_slice(body);
    }
    response
}

#[cfg(test)]
mod tests;
