//! External JSON-RPC bytes: content and correlation have independent bounds.
use std::io::{self, Write};

use serde::{ser::SerializeMap, Deserialize, Serialize};
use serde_json::value::RawValue;

use super::{is_valid_mcp_request_id, JsonRpcResponse};
use crate::{framing::FinalBuffer, MCP_JSON_RPC_MAX_BYTES, MCP_PAYLOAD_MAX_BYTES};

fn too_large() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        "MCP content exceeds the shared transport payload limit",
    )
}

#[derive(Deserialize)]
struct Identity<'a> {
    #[serde(borrow, rename = "jsonrpc")]
    _jsonrpc: &'a RawValue,
    #[serde(borrow, default, deserialize_with = "present_id")]
    id: Option<&'a RawValue>,
}

fn present_id<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<Option<&'de RawValue>, D::Error> {
    Deserialize::deserialize(deserializer).map(Some)
}

/// Admit raw protocol bytes before parsing content trees. Small messages need
/// no second parse. Larger messages may use only their bounded ID field's
/// allowance; whitespace and other fields still consume the content budget.
/// The protocol owner remains responsible for ordinary JSON-RPC validation.
pub fn validate_size(bytes: &[u8]) -> io::Result<()> {
    if bytes.len() <= MCP_PAYLOAD_MAX_BYTES {
        return Ok(());
    }
    if bytes.len() > MCP_JSON_RPC_MAX_BYTES {
        return Err(too_large());
    }
    let identity: Identity<'_> = serde_json::from_slice(bytes).map_err(|_| too_large())?;
    let raw = identity.id.ok_or_else(too_large)?;
    if raw.get().len() > crate::MCP_JSON_RPC_ID_MAX_BYTES {
        return Err(too_large());
    }
    let id: serde_json::Value = serde_json::from_str(raw.get()).map_err(|_| too_large())?;
    if !id.is_null() && !is_valid_mcp_request_id(&id) {
        return Err(too_large());
    }
    if bytes.len() - raw.get().len() - b",\"id\":".len() > MCP_PAYLOAD_MAX_BYTES {
        return Err(too_large());
    }
    Ok(())
}

struct Content<'a>(&'a JsonRpcResponse);
impl Serialize for Content<'_> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(None)?;
        map.serialize_entry("jsonrpc", &self.0.jsonrpc)?;
        if let Some(result) = &self.0.result {
            map.serialize_entry("result", result)?;
        }
        if let Some(error) = &self.0.error {
            map.serialize_entry("error", error)?;
        }
        if let Some(meta) = &self.0.meta {
            map.serialize_entry("_meta", meta)?;
        }
        map.end()
    }
}

/// Encode content once into bounded final storage, then append correlation in
/// its derived envelope allowance. Remapping a short upstream ID to a maximally
/// escaped guest ID cannot shrink the usable content ceiling. No sizing pass,
/// content clone or payload-to-frame copy is involved.
pub fn encode_response(response: &JsonRpcResponse) -> io::Result<Vec<u8>> {
    if response.id.as_ref().is_some_and(|id| !is_valid_mcp_request_id(id)) {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "invalid MCP response ID"));
    }
    let mut bytes = crate::framing::encode_json_record(&Content(response), MCP_PAYLOAD_MAX_BYTES)?;
    if response.id.is_some() {
        bytes.pop(); // Replace the final object delimiter, retaining its storage.
        let mut out = FinalBuffer::new(&mut bytes, MCP_JSON_RPC_MAX_BYTES);
        out.write_all(b",\"id\":")?;
        serde_json::to_writer(&mut out, &response.id).map_err(io::Error::other)?;
        out.write_all(b"}")?;
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests;
