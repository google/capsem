//! CORS origin allowlist.
//!
//! The gateway is bound to 127.0.0.1 but the CORS predicate decides whether a
//! cross-origin browser caller is allowed to read responses (including the
//! gateway token at `/token`, which is otherwise gated only by loopback peer
//! IP -- a check that passes for any browser running on the user's machine).
//! A loose prefix match (e.g. `starts_with("http://localhost")`) approves
//! attacker-controlled hosts like `http://localhostevil.com`, so we parse the
//! Origin and require an exact loopback host with an allowed scheme.

#[cfg(test)]
mod tests;

/// Returns true iff the Origin header value is a same-machine origin we trust:
/// `http`/`https` to `localhost`, `127.0.0.1`, or `::1`, or `tauri://localhost`.
///
/// Any port is accepted on loopback. The Origin must be only scheme + host +
/// optional port (no path beyond `""`/`"/"`, no userinfo, no query, no
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
        "http" | "https" => is_loopback_host(host),
        "tauri" => host.eq_ignore_ascii_case("localhost"),
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
