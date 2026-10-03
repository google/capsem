//! Reading the host ledger back through the DB object.

use super::*;
use crate::events::{HostEvent, HostEventKind};

const HOST_EVENTS_SQL: &str = "SELECT timestamp_unix_ms, kind, session_id, actor, hex(detail), trace_id
     FROM host_events ORDER BY id";

impl DbHandle {
    /// Every host event in ledger order. Chain verification is
    /// `DbReader::verify_host_chain`; this is the service's replay at startup.
    pub async fn host_events(&self) -> DbResult<Vec<HostEvent>> {
        let json = self.query(HOST_EVENTS_SQL, &[]).await?;
        let table: serde_json::Value =
            serde_json::from_str(&json).map_err(|error| format!("host events query result: {error}"))?;
        let rows = table["rows"]
            .as_array()
            .ok_or("host events query returned no rows array")?;
        rows.iter().map(host_event_from_row).collect()
    }
}

fn host_event_from_row(row: &serde_json::Value) -> DbResult<HostEvent> {
    let text = |index: usize| row[index].as_str().map(str::to_string);
    let kind = text(1).ok_or("host event without a kind")?;
    Ok(HostEvent {
        timestamp_unix_ms: row[0].as_i64().ok_or("host event without a timestamp")?,
        kind: HostEventKind::parse_str(&kind).ok_or_else(|| format!("unknown host event kind {kind}"))?,
        session_id: text(2),
        actor: text(3).ok_or("host event without an actor")?,
        detail: decode_hex(&text(4).unwrap_or_default())?,
        trace_id: text(5),
    })
}

fn decode_hex(hex: &str) -> DbResult<Vec<u8>> {
    if !hex.len().is_multiple_of(2) {
        return Err("host event detail is not whole bytes".to_string());
    }
    (0..hex.len())
        .step_by(2)
        .map(|index| {
            u8::from_str_radix(&hex[index..index + 2], 16).map_err(|error| format!("host event detail: {error}"))
        })
        .collect()
}
