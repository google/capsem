//! A network's audit history, read from its own database in pages.
//!
//! Rows are the `transport_events` ledger, ordered by their autoincrement
//! id, so a cursor is simply the last id a caller saw: appends never land
//! below it and a page never repeats or skips a row. The cursor also carries
//! the network and the filters it was cut with, so a cursor from another
//! network or another filter set is refused rather than silently misread.
use super::{database, NetworkError, NetworkRegistry};
use capsem_foundation::paths::network_db_path_in;
use capsem_logger::DbHandle;
use std::sync::Arc;
use uuid::Uuid;

pub const DEFAULT_LOG_LIMIT: usize = 100;
pub const MAX_LOG_LIMIT: usize = 1000;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LogQuery {
    pub cursor: Option<String>,
    pub limit: Option<usize>,
    /// Either endpoint's VM id on a flow, or the VM of a lifecycle event.
    pub vm: Option<String>,
    pub connection: Option<String>,
    pub event_type: Option<String>,
    /// The effective decision recorded on the event: allow, ask or block.
    pub decision: Option<String>,
    pub since_unix_ms: Option<i64>,
    pub until_unix_ms: Option<i64>,
}

impl LogQuery {
    /// The filters, in a fixed order, as the cursor remembers them.
    fn fingerprint(&self) -> u64 {
        let text = format!(
            "{:?}|{:?}|{:?}|{:?}|{:?}|{:?}",
            self.vm, self.connection, self.event_type, self.decision, self.since_unix_ms, self.until_unix_ms
        );
        // FNV-1a: stable across processes and versions, which the default
        // hasher is not.
        text.bytes().fold(0xcbf2_9ce4_8422_2325u64, |hash, byte| {
            (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3)
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogEvent {
    pub sequence: i64,
    pub event_id: String,
    pub timestamp_unix_ms: i64,
    pub event_type: String,
    pub connection_id: Option<String>,
    pub event_json: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogPage {
    pub events: Vec<LogEvent>,
    /// Present when the page was full; absent means the caller has seen
    /// everything written so far and may poll again with the same cursor.
    pub next_cursor: Option<String>,
    /// The cursor to continue from either way.
    pub cursor: String,
}

fn encode_cursor(network: Uuid, fingerprint: u64, sequence: i64) -> String {
    format!("{network}.{fingerprint:016x}.{sequence}")
}

fn decode_cursor(text: &str, network: Uuid, fingerprint: u64) -> Result<i64, NetworkError> {
    let refuse = |reason: &str| NetworkError::Cursor(format!("{reason}: {text:?}"));
    let mut parts = text.split('.');
    let (Some(id), Some(filters), Some(sequence), None) = (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return Err(refuse("malformed cursor"));
    };
    if Uuid::parse_str(id).ok() != Some(network) {
        return Err(refuse("cursor belongs to another network"));
    }
    if u64::from_str_radix(filters, 16).ok() != Some(fingerprint) {
        return Err(refuse("cursor was cut with different filters"));
    }
    sequence
        .parse::<i64>()
        .ok()
        .filter(|sequence| *sequence >= 0)
        .ok_or_else(|| refuse("malformed cursor sequence"))
}

impl NetworkRegistry {
    /// The database of an active or retired network. A retired network's
    /// reader is opened on first use and kept; the file stays for the
    /// retention window, so its history outlives the network.
    pub fn reader(&mut self, id: Uuid) -> Result<Arc<DbHandle>, NetworkError> {
        if let Some(entry) = self.networks.get(&id) {
            return Ok(Arc::clone(&entry.handle));
        }
        if let Some(handle) = self.retired_readers.get(&id) {
            return Ok(Arc::clone(handle));
        }
        let path = network_db_path_in(&self.root, &id.to_string());
        if !path.exists() {
            return Err(NetworkError::NotFound(id));
        }
        let handle =
            Arc::new(DbHandle::open_external_reader(&path).map_err(|error| database(&path, error.to_string()))?);
        self.retired_readers.insert(id, Arc::clone(&handle));
        Ok(handle)
    }

    /// One page of a network's audit history, oldest first from the cursor.
    pub async fn logs(&mut self, id: Uuid, query: &LogQuery) -> Result<LogPage, NetworkError> {
        let limit = query.limit.unwrap_or(DEFAULT_LOG_LIMIT);
        if limit == 0 || limit > MAX_LOG_LIMIT {
            return Err(NetworkError::Cursor(format!(
                "limit must be 1..={MAX_LOG_LIMIT}, got {limit}"
            )));
        }
        let fingerprint = query.fingerprint();
        let after = match &query.cursor {
            Some(cursor) => decode_cursor(cursor, id, fingerprint)?,
            None => 0,
        };
        let handle = self.reader(id)?;
        let path = network_db_path_in(&self.root, &id.to_string());
        let mut sql = String::from(
            "SELECT id, event_id, timestamp_unix_ms, event_type, connection_id, event_json \
             FROM transport_events WHERE network_id = ?1 AND id > ?2",
        );
        let mut params: Vec<serde_json::Value> = vec![id.to_string().into(), after.into()];
        let mut bind = |clause: &str, value: serde_json::Value| {
            params.push(value);
            sql.push_str(&clause.replace("?N", &format!("?{}", params.len())));
        };
        if let Some(vm) = &query.vm {
            bind(
                " AND (json_extract(event_json, '$.network.source.vm.id') = ?N \
                 OR json_extract(event_json, '$.network.destination.vm.id') = ?N \
                 OR json_extract(event_json, '$.network.vm.id') = ?N)",
                vm.clone().into(),
            );
        }
        if let Some(connection) = &query.connection {
            bind(" AND connection_id = ?N", connection.clone().into());
        }
        if let Some(event_type) = &query.event_type {
            bind(" AND event_type = ?N", event_type.clone().into());
        }
        if let Some(decision) = &query.decision {
            bind(
                " AND json_extract(event_json, '$.decision.effective') = ?N",
                decision.clone().into(),
            );
        }
        if let Some(since) = query.since_unix_ms {
            bind(" AND timestamp_unix_ms >= ?N", since.into());
        }
        if let Some(until) = query.until_unix_ms {
            bind(" AND timestamp_unix_ms <= ?N", until.into());
        }
        sql.push_str(&format!(" ORDER BY id LIMIT {limit}"));
        handle.ready().await.map_err(|error| database(&path, error))?;
        let json = handle
            .query(&sql, &params)
            .await
            .map_err(|error| database(&path, error))?;
        let mut events = Vec::new();
        for row in super::rows(&path, &json)? {
            events.push(LogEvent {
                sequence: super::integer(&path, &row[0])?,
                event_id: super::text(&path, &row[1])?,
                timestamp_unix_ms: super::integer(&path, &row[2])?,
                event_type: super::text(&path, &row[3])?,
                connection_id: row[4].as_str().map(str::to_string),
                event_json: super::text(&path, &row[5])?,
            });
        }
        let last = events.last().map_or(after, |event| event.sequence);
        let cursor = encode_cursor(id, fingerprint, last);
        let next_cursor = (events.len() == limit).then(|| cursor.clone());
        Ok(LogPage {
            events,
            next_cursor,
            cursor,
        })
    }
}
