//! Conservative retained JSON allocations, including tiny-value container overhead.
use capsem_proto::mcp_contracts::{JsonRpcMeta, JsonRpcRequest, JsonRpcResponse};
use serde_json::Value;

fn heap(value: &Value) -> usize {
    match value {
        Value::String(value) => value.capacity(),
        Value::Array(values) => values.iter().fold(
            values.capacity().saturating_mul(std::mem::size_of::<Value>()),
            |total, value| total.saturating_add(heap(value)),
        ),
        // Allow a full tree/hash node's overhead per key, in addition to its
        // inline key/value storage. This also bounds sparse cloned maps.
        Value::Object(values) => values.iter().fold(
            values
                .len()
                .saturating_mul(128 + std::mem::size_of::<Value>() + std::mem::size_of::<String>()),
            |total, (key, value)| total.saturating_add(key.capacity()).saturating_add(heap(value)),
        ),
        _ => 0,
    }
}

fn meta(value: Option<&JsonRpcMeta>) -> usize {
    value.map_or(0, |value| {
        value.traceparent.capacity().saturating_add(value.tracestate.capacity())
    })
}

pub(super) fn request(value: &JsonRpcRequest) -> usize {
    std::mem::size_of::<JsonRpcRequest>()
        .saturating_add(value.jsonrpc.capacity())
        .saturating_add(value.method.capacity())
        .saturating_add(value.id.as_ref().map_or(0, heap))
        .saturating_add(value.params.as_ref().map_or(0, heap))
        .saturating_add(meta(value.meta.as_ref()))
        .saturating_mul(4)
}

pub(super) fn response(value: &JsonRpcResponse) -> usize {
    std::mem::size_of::<JsonRpcResponse>()
        .saturating_add(value.jsonrpc.capacity())
        .saturating_add(value.id.as_ref().map_or(0, heap))
        .saturating_add(value.result.as_ref().map_or(0, heap))
        .saturating_add(value.error.as_ref().map_or(0, |error| {
            error
                .message
                .capacity()
                .saturating_add(error.data.as_ref().map_or(0, heap))
        }))
        .saturating_add(meta(value.meta.as_ref()))
        .saturating_mul(4)
}
