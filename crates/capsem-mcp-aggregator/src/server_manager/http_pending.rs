//! Correlation only: retain IDs while a POST awaits an asynchronous GET reply.
//! No request arguments or result bodies are copied into this bookkeeping.
use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use rmcp::model::{ClientJsonRpcMessage, ClientNotification, JsonRpcMessage, RequestId};
use sse_stream::Sse;

#[derive(Clone, Debug, Default)]
pub(super) struct PendingResponses(Arc<Mutex<HashSet<RequestId>>>);

pub(super) struct PendingRequest {
    pending: PendingResponses,
    id: Option<RequestId>,
}

impl PendingResponses {
    pub(super) fn release_when_done(&self, id: RequestId) -> PendingRequest {
        PendingRequest {
            pending: self.clone(),
            id: Some(id),
        }
    }

    pub(super) fn track(&self, message: &ClientJsonRpcMessage) -> PendingRequest {
        let id = match message {
            JsonRpcMessage::Request(request) => {
                self.0.lock().unwrap().insert(request.id.clone());
                Some(request.id.clone())
            }
            JsonRpcMessage::Notification(notification) => {
                if let ClientNotification::CancelledNotification(cancelled) = &notification.notification {
                    if let Some(id) = &cancelled.params.request_id {
                        self.0.lock().unwrap().remove(id);
                    }
                }
                None
            }
            _ => None,
        };
        PendingRequest {
            pending: self.clone(),
            id,
        }
    }

    pub(super) fn complete(&self, event: &Sse) {
        // External MCP JSON-RPC correlation only. Serde skips payload fields
        // without allocating a second result/argument tree.
        #[derive(serde::Deserialize)]
        struct Identity<'a> {
            id: Option<RequestId>,
            #[serde(borrow)]
            result: Option<&'a serde_json::value::RawValue>,
            #[serde(borrow)]
            error: Option<&'a serde_json::value::RawValue>,
            #[serde(borrow)]
            method: Option<&'a serde_json::value::RawValue>,
        }
        if let Some(data) = &event.data {
            if let Ok(Identity {
                id: Some(id),
                result,
                error,
                method,
            }) = serde_json::from_str::<Identity>(data)
            {
                if method.is_none() && (result.is_some() || error.is_some()) {
                    self.0.lock().unwrap().remove(&id);
                }
            }
        }
    }

    pub(super) fn refuse_all(&self) -> Vec<RequestId> {
        self.0.lock().unwrap().drain().collect()
    }
}

impl PendingRequest {
    pub(super) fn retain(&mut self) {
        self.id = None;
    }
}

impl Drop for PendingRequest {
    fn drop(&mut self) {
        if let Some(id) = &self.id {
            self.pending.0.lock().unwrap().remove(id);
        }
    }
}

#[cfg(test)]
mod tests;
