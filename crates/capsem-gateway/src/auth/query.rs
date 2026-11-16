//! Browser query credentials are operator-only and are removed before dispatch.
use axum::extract::{OriginalUri, Request};
use http::{header, HeaderValue, Method, Uri};

fn token_field(pair: &str) -> Option<(std::borrow::Cow<'_, str>, std::borrow::Cow<'_, str>)> {
    let key = pair.split('=').next()?;
    let (name, _) = url::form_urlencoded::parse(key.as_bytes()).next()?;
    if !name.eq_ignore_ascii_case("token") {
        return None;
    }
    url::form_urlencoded::parse(pair.as_bytes()).next()
}

fn websocket(request: &Request) -> bool {
    request.method() == Method::GET
        && request
            .headers()
            .get(header::UPGRADE)
            .is_some_and(|value| value.as_bytes().eq_ignore_ascii_case(b"websocket"))
        && request.headers().get_all(header::CONNECTION).iter().any(|value| {
            value
                .to_str()
                .is_ok_and(|value| value.split(',').any(|part| part.trim().eq_ignore_ascii_case("upgrade")))
        })
        && request
            .headers()
            .get(header::SEC_WEBSOCKET_VERSION)
            .is_some_and(|value| value == "13")
        && request.headers().contains_key(header::SEC_WEBSOCKET_KEY)
        && (request.uri().path() == "/events" || super::is_stream_path(request.uri().path()))
}

/// Requests without a token field keep their original URI. A supplied token is
/// stripped even when duplicate, invalid or competing with a header credential.
pub(super) fn canonicalize(request: &mut Request, operator: &str) -> Result<(), ()> {
    let Some(query) = request.uri().query() else {
        return Ok(());
    };
    let mut count = 0;
    let mut valid = false;
    for pair in query.split('&') {
        if let Some((name, value)) = token_field(pair) {
            count += 1;
            valid = name == "token" && !value.is_empty() && super::token_matches(&value, operator);
        }
    }
    if count == 0 {
        return Ok(());
    }
    let admitted = count == 1 && valid && !request.headers().contains_key(header::AUTHORIZATION) && websocket(request);
    // Only credential-bearing requests take the second pass; retain every
    // other query pair's original bytes in one final URI buffer.
    let mut path = String::with_capacity(request.uri().path().len() + query.len() + 1);
    path.push_str(request.uri().path());
    let mut retained = false;
    for pair in query.split('&') {
        if token_field(pair).is_some() {
            continue;
        }
        path.push(if retained { '&' } else { '?' });
        path.push_str(pair);
        retained = true;
    }
    let mut parts = request.uri().clone().into_parts();
    parts.path_and_query = Some(http::uri::PathAndQuery::try_from(path).map_err(|_| ())?);
    let uri = Uri::from_parts(parts).map_err(|_| ())?;
    if let Some(original) = request.extensions_mut().get_mut::<OriginalUri>() {
        original.0 = uri.clone();
    }
    *request.uri_mut() = uri;
    if !admitted {
        return Err(());
    }
    let mut value = HeaderValue::from_str(&format!("Bearer {operator}")).map_err(|_| ())?;
    value.set_sensitive(true);
    request.headers_mut().insert(header::AUTHORIZATION, value);
    Ok(())
}

#[cfg(test)]
mod tests;
