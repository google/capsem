//! CORS origin allowlist.
//!
//! A guest can publish its own HTTP server on a host loopback port. Its browser
//! origin is therefore not trusted merely because it names localhost. Only the
//! bundled Tauri webview and the gateway's own fixed origin may read responses.

#[cfg(test)]
mod tests;

/// Returns true iff the Origin header names the bundled Tauri webview or the
/// gateway's own fixed HTTP origin.
///
/// The Origin must be only scheme + host + optional port (no path beyond
/// `""`/`"/"`, no userinfo, no query, no
/// fragment) -- anything else is malformed and rejected.
pub fn is_allowed_origin(value: &http::HeaderValue) -> bool {
    value.to_str().map(is_allowed_origin_str).unwrap_or(false)
}

fn is_allowed_origin_str(s: &str) -> bool {
    let Ok(uri) = http::Uri::try_from(s) else {
        return false;
    };

    // Origin headers per RFC 6454 are scheme + host + optional port, nothing
    // else. Reject anything richer so an attacker cannot smuggle data through
    // path/query/fragment that the parser ignores.
    if !matches!(uri.path(), "" | "/") {
        return false;
    }
    if uri.query().is_some() {
        return false;
    }
    if s.contains('@') || s.contains('#') {
        return false;
    }

    let Some(scheme) = uri.scheme_str() else {
        return false;
    };
    let Some(host) = uri.host() else { return false };

    match scheme {
        "http" => host == "127.0.0.1" && uri.port_u16() == Some(19222),
        "tauri" => host.eq_ignore_ascii_case("localhost") && uri.port().is_none(),
        _ => false,
    }
}

/// True iff `host` names this machine: `localhost`, `127.0.0.1`, or `::1`,
/// with or without IPv6 brackets. Shared by the CORS origin check and the
/// Host header check in `auth`.
pub fn is_loopback_host(host: &str) -> bool {
    // `http::Uri::host` may or may not strip IPv6 brackets depending on input
    // shape; normalize so `[::1]` and `::1` both compare against `::1`.
    let host = host.trim_start_matches('[').trim_end_matches(']');
    host.eq_ignore_ascii_case("localhost") || host == "127.0.0.1" || host == "::1"
}
