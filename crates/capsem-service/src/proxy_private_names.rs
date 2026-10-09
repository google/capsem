//! Session-bound private DNS lookups for one confined proxy worker.

use std::io;
use std::os::unix::net::UnixStream;
use std::sync::Arc;

use capsem_foundation::ipc_channel;
use capsem_proto::proxy_private_names::{
    ProxyPrivateNameRequest, ProxyPrivateNameResponse, MAX_PRIVATE_NAME_ERROR_BYTES,
};

use crate::api::PrivateResolveRequest;
use crate::ServiceState;

pub(crate) async fn serve(state: Arc<ServiceState>, vm_id: String, stream: UnixStream) -> Result<(), String> {
    let (sender, receiver) = ipc_channel::channel_from_std::<ProxyPrivateNameResponse, ProxyPrivateNameRequest>(stream)
        .map_err(|error| error.to_string())?;
    loop {
        let request = match receiver.recv().await {
            Ok(request) => request,
            Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => return Ok(()),
            Err(error) => return Err(format!("receive proxy private-name request: {error}")),
        };
        let request_id = request.request_id();
        let response = if let Err(error) = request.validate() {
            rejected(request_id, error.to_string())
        } else {
            let address_lookup = matches!(&request, ProxyPrivateNameRequest::AddressOf { .. });
            let query = match request {
                ProxyPrivateNameRequest::AddressOf { name, .. } => PrivateResolveRequest {
                    source_vm: vm_id.clone(),
                    name: Some(name),
                    address: None,
                },
                ProxyPrivateNameRequest::NameOf { address, .. } => PrivateResolveRequest {
                    source_vm: vm_id.clone(),
                    name: None,
                    address: Some(address),
                },
            };
            match crate::private_routes::resolve_private(&state, query).await {
                Ok(answer) => {
                    if address_lookup {
                        ProxyPrivateNameResponse::Address {
                            request_id,
                            address: answer.address,
                        }
                    } else {
                        ProxyPrivateNameResponse::Name {
                            request_id,
                            name: answer.name,
                        }
                    }
                }
                Err(error) if error.0 == axum::http::StatusCode::NOT_FOUND => {
                    ProxyPrivateNameResponse::NotFound { request_id }
                }
                Err(error) => rejected(request_id, error.1),
            }
        };
        sender
            .send(response)
            .await
            .map_err(|error| format!("send proxy private-name response: {error}"))?;
    }
}

fn rejected(request_id: u64, error: String) -> ProxyPrivateNameResponse {
    let error = truncate_utf8(error, MAX_PRIVATE_NAME_ERROR_BYTES);
    ProxyPrivateNameResponse::rejected(request_id, error).expect("request id and diagnostic were bounded")
}

fn truncate_utf8(mut value: String, max_bytes: usize) -> String {
    if value.len() <= max_bytes {
        return value;
    }
    let mut boundary = max_bytes;
    while !value.is_char_boundary(boundary) {
        boundary -= 1;
    }
    value.truncate(boundary);
    value
}

#[cfg(test)]
mod tests;
