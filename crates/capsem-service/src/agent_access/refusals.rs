//! Bounded audit facts contain no caller body, raw URI or credential material.
use axum::{
    extract::{MatchedPath, Request},
    http::{Method, StatusCode},
};
use capsem_api::CallerActor;
use serde::Serialize;

use crate::{AppError, ServiceState};

#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Reason {
    ForeignPeer,
    OwnerIdentityDenied,
    OwnerRouteDenied,
    GatewayEpochEnded,
    CredentialDenied,
    ScopeRequired,
    GatewayTransportRequired,
}
impl Reason {
    pub(crate) fn status(self) -> StatusCode {
        if matches!(self, Self::CredentialDenied) {
            StatusCode::UNAUTHORIZED
        } else {
            StatusCode::FORBIDDEN
        }
    }
    pub(crate) fn message(self) -> &'static str {
        match self {
            Self::ForeignPeer => "foreign service peer denied",
            Self::OwnerIdentityDenied => "VM owner service identity denied",
            Self::OwnerRouteDenied => "VM owner service route denied",
            Self::GatewayEpochEnded => "gateway control epoch ended",
            Self::CredentialDenied => "service credential denied",
            Self::ScopeRequired => "delegated service operation requires an authorized scope",
            Self::GatewayTransportRequired => "service API requires authenticated gateway transport",
        }
    }
}

#[derive(Serialize)]
struct Detail<'a> {
    method: &'static str,
    route: Option<&'a str>,
    reason: Reason,
}

pub(crate) async fn record(
    state: &ServiceState,
    actor: Option<CallerActor>,
    request: Request,
    reason: Reason,
) -> Result<(), AppError> {
    let method = match *request.method() {
        Method::GET => "GET",
        Method::HEAD => "HEAD",
        Method::POST => "POST",
        Method::PUT => "PUT",
        Method::PATCH => "PATCH",
        Method::DELETE => "DELETE",
        Method::OPTIONS => "OPTIONS",
        Method::CONNECT => "CONNECT",
        Method::TRACE => "TRACE",
        _ => "OTHER",
    };
    let route = request.extensions().get::<MatchedPath>().map(MatchedPath::as_str);
    let detail = rmp_serde::to_vec_named(&Detail { method, route, reason }).map_err(super::unavailable)?;
    let actor = actor.map_or_else(|| "unauthenticated".to_owned(), |actor| actor.ledger_id());
    super::audit(state, actor, capsem_logger::HostEventKind::AccessDenied, detail).await
}
