//! One event feed per authenticated gateway epoch, on the sealed HTTP seat.

use super::GatewayChannel;
use crate::{caller_auth::AuthenticatedActor, AppError, ServiceState};
use axum::{
    body::{Body, Bytes},
    extract::State,
    http::{header, StatusCode},
    response::Response,
    routing::get,
    Router,
};
use capsem_core::security_engine::approval::ApprovalBinding;
use capsem_proto::gateway_grant::events::{SessionEvent, CONTENT_TYPE, PATH};
use std::{convert::Infallible, sync::Arc};
use tokio::sync::{broadcast, Semaphore};

#[derive(Clone)]
struct Feed {
    service: Arc<ServiceState>,
    seat: Arc<Semaphore>,
}

struct Subscription {
    changes: broadcast::Receiver<ApprovalBinding>,
    channel: GatewayChannel,
    service: Arc<ServiceState>,
    _seat: tokio::sync::OwnedSemaphorePermit,
}

pub(crate) fn event_routes(service: Arc<ServiceState>) -> Router {
    Router::new()
        .route(PATH, get(subscribe))
        .route_layer(axum::middleware::from_fn_with_state(
            Arc::clone(&service),
            crate::service_peer::restrict_owner_routes,
        ))
        .with_state(Feed {
            service,
            seat: Arc::new(Semaphore::new(1)),
        })
}

async fn subscribe(
    channel: GatewayChannel,
    actor: AuthenticatedActor,
    State(Feed { service, seat }): State<Feed>,
) -> Result<Response, AppError> {
    actor.require_operator()?;
    let seat = seat
        .try_acquire_owned()
        .map_err(|_| AppError::new(StatusCode::CONFLICT, "gateway event feed already open".into()))?;
    let subscription = Subscription {
        changes: service.approvals.broker.subscribe_queue_changes(),
        channel,
        service,
        _seat: seat,
    };
    let stream = futures::stream::unfold(subscription, |mut subscription| async move {
        loop {
            let binding = tokio::select! {
                biased;
                () = subscription.channel.revoked() => return None,
                changed = subscription.changes.recv() => match changed {
                    Ok(binding) => binding,
                    // End a lossy feed so the gateway can invalidate its subscribers.
                    Err(broadcast::error::RecvError::Lagged(_) | broadcast::error::RecvError::Closed) => return None,
                },
            };
            if let Some(event) = approval_event(&subscription.service, &binding) {
                return Some((Ok::<_, Infallible>(Bytes::from_owner(event.encode())), subscription));
            }
        }
    });
    let mut response = Response::new(Body::from_stream(stream));
    response
        .headers_mut()
        .insert(header::CONTENT_TYPE, header::HeaderValue::from_static(CONTENT_TYPE));
    Ok(response)
}

fn approval_event(service: &ServiceState, binding: &ApprovalBinding) -> Option<SessionEvent> {
    let id = uuid::Uuid::parse_str(&binding.session_id).ok()?;
    if id.is_nil() || id.to_string() != binding.session_id {
        return None;
    }
    let instances = service.instances.lock().unwrap();
    let instance = instances.get(&binding.session_id)?;
    (instance.generation == binding.generation && !instance.authority.grant().is_revoked())
        .then_some(SessionEvent::ApprovalPending(*id.as_bytes()))
}

#[cfg(test)]
mod tests;
