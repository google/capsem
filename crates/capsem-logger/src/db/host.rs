//! Reading the host ledger back through the DB object.

use super::*;
use crate::events::{HostEvent, HostEventKind};

const HOST_EVENTS_SQL: &str = "SELECT timestamp_unix_ms, kind, session_id, actor, hex(detail), trace_id
     FROM host_events ORDER BY id";

impl DbHandle {
    /// A bounded page in durable insertion order. The cursor is exclusive;
    /// timestamps never participate in ordering or pagination.
    pub async fn host_event_page(
        &self,
        kind: HostEventKind,
        after_id: i64,
        limit: usize,
    ) -> DbResult<Vec<(i64, HostEvent)>> {
        if after_id < 0 || limit == 0 || limit > crate::ledger_protocol::MAX_LEDGER_QUERY_ROWS {
            return Err("invalid host event page bounds".into());
        }
        let json = self
            .query(
                "SELECT timestamp_unix_ms,kind,session_id,actor,hex(detail),trace_id,id
             FROM host_events WHERE kind=?1 AND id>?2 ORDER BY id LIMIT ?3",
                &[kind.as_str().into(), after_id.into(), limit.into()],
            )
            .await?;
        let table: serde_json::Value =
            serde_json::from_str(&json).map_err(|error| format!("host event page result: {error}"))?;
        let rows = table["rows"]
            .as_array()
            .ok_or("host event page returned no rows array")?;
        rows.iter()
            .map(|row| {
                let id = row[6]
                    .as_i64()
                    .filter(|id| *id > after_id)
                    .ok_or("host event page without a valid cursor")?;
                Ok((id, host_event_from_row(row)?))
            })
            .collect()
    }

    /// Every host event in ledger order for the service startup replay.
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
