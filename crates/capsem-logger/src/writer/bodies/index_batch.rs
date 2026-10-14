//! Finite parameter batches reduce SQLite executions without splitting a flush.
use super::*;
use rusqlite::{params_from_iter, ToSql};

// Fifteen fields × 64 rows = 960 parameters, below even SQLite's older 999
// default. Only references are collected; body bytes and transaction ownership
// stay with the existing archive and writer.
const ROWS: usize = 64;
const INSERT: &str = "INSERT OR REPLACE INTO event_body_blobs (
    event_id, event_type, source_table, direction, content_type,
    original_bytes, stored_bytes, truncated, body_hash,
    block_offset, body_offset, body_len, trace_id, turn_id, created_at
) VALUES ";
const VALUES: &str = "(?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)";

fn sql(rows: usize) -> String {
    let mut statement = String::from(INSERT);
    for index in 0..rows {
        if index != 0 {
            statement.push(',');
        }
        statement.push_str(VALUES);
    }
    statement
}
fn values(row: &BodyIndexRow) -> [&dyn ToSql; 15] {
    [
        &row.event_id,
        &row.event_type,
        &row.source_table,
        &row.direction,
        &row.content_type,
        &row.original_bytes,
        &row.stored_bytes,
        &row.truncated,
        &row.body_hash,
        &row.block_offset,
        &row.body_offset,
        &row.stored_bytes,
        &row.trace_id,
        &row.turn_id,
        &row.created_at,
    ]
}
pub(super) fn insert<'a>(conn: &Connection, mut rows: impl Iterator<Item = &'a BodyIndexRow>) -> rusqlite::Result<()> {
    let mut batch = Vec::with_capacity(ROWS);
    let mut full = conn.prepare_cached(&sql(ROWS))?;
    loop {
        batch.clear();
        batch.extend(rows.by_ref().take(ROWS));
        if batch.is_empty() {
            return Ok(());
        }
        let parameters = params_from_iter(batch.iter().flat_map(|row| values(row)));
        if batch.len() == ROWS {
            full.execute(parameters)?;
        } else {
            conn.prepare_cached(&sql(batch.len()))?.execute(parameters)?;
            return Ok(());
        }
    }
}
