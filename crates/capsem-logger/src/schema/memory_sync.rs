//! The writer's memory schema: the rows it has accepted and not yet flushed.
//!
//! The writer inserts into `mem` and moves those rows to disk on its flush,
//! deleting them from `mem` in the same transaction. Nothing is copied from
//! disk into `mem`, and no reader reads it: a row is in exactly one of the two
//! places, and what `mem` costs is bounded by one flush interval rather than by
//! the length of the session (#213).

use super::*;

/// Tables that live on disk only and never mirror into the memory schema:
/// the body index and its block table are written straight to disk beside the
/// archive file they point into, the schema markers are not data, and
/// the network registry tables (`network_db`) are small state, not a ledger.
const DISK_ONLY_TABLES: &[&str] = &[
    "archive_state",
    "event_body_blobs",
    "body_blocks",
    "transport_schema",
    "network",
    "network_members",
    "network_schema",
];

pub(crate) fn is_disk_only_table(name: &str) -> bool {
    DISK_ONLY_TABLES.contains(&name)
}

/// Build the writer's memory tables from the disk schema's own declarations.
///
/// A memory table whose columns no longer match its disk table is dropped and
/// rebuilt. This function is intentionally DB-owned: route callers neither
/// inspect nor repair ledger schema.
pub fn reconcile_memory_tables_from_disk(conn: &Connection) -> rusqlite::Result<()> {
    let mut stmt = conn.prepare(
        "SELECT name, sql
         FROM main.sqlite_master
         WHERE type = 'table'
           AND name NOT LIKE 'sqlite_%'
         ORDER BY name",
    )?;
    let tables = stmt
        .query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))?
        .collect::<Result<Vec<_>, _>>()?;

    for table in tables {
        let (name, sql) = table;
        if is_disk_only_table(&name) {
            continue;
        }
        let disk_columns = table_column_names(conn, "main", &name)?;
        let memory_columns = table_column_names(conn, MEMORY_SCHEMA, &name)?;
        if !memory_columns.is_empty() && memory_columns != disk_columns {
            conn.execute_batch(&format!("DROP TABLE {MEMORY_SCHEMA}.{name};"))?;
        }
        // The memory table is derived from the disk table's own declaration,
        // and the disk table has already been refused if its CHECK predates
        // this build -- so the memory table cannot be built from an older list.
        let mem_sql =
            memory_table_sql(&name, &sql).ok_or_else(|| rusqlite::Error::InvalidParameterName(name.clone()))?;
        conn.execute_batch(&mem_sql)?;
    }

    Ok(())
}

pub(crate) fn table_column_names(conn: &Connection, schema: &str, table: &str) -> rusqlite::Result<Vec<String>> {
    let mut stmt = conn.prepare(&format!("PRAGMA {schema}.table_info({table})"))?;
    let columns = stmt
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(columns)
}

/// Start each memory table's AUTOINCREMENT where the disk table's left off.
///
/// A memory row's id is its ledger id: the flush copies it to disk unchanged
/// and `model_calls.id` is already stored in `model_items` and `tool_calls` by
/// then. With nothing copied into `mem` at open, `mem` would otherwise number
/// from 1 and overwrite the session's first rows. The disk's own
/// `sqlite_sequence` is honoured above its `MAX(id)`, because AUTOINCREMENT
/// never hands out an id twice even after the rows that held it are deleted.
pub(crate) fn seed_memory_sequences<'a>(
    conn: &Connection,
    tables: impl IntoIterator<Item = &'a str>,
) -> rusqlite::Result<()> {
    for table in tables {
        if is_disk_only_table(table)
            || !table_exists(conn, "main", table)?
            || !table_exists(conn, MEMORY_SCHEMA, table)?
        {
            continue;
        }
        let next: i64 = conn.query_row(
            &format!(
                "SELECT MAX(
                    COALESCE((SELECT seq FROM main.sqlite_sequence WHERE name = ?1), 0),
                    (SELECT COALESCE(MAX(id), 0) FROM main.{table}),
                    COALESCE((SELECT seq FROM {MEMORY_SCHEMA}.sqlite_sequence WHERE name = ?1), 0),
                    (SELECT COALESCE(MAX(id), 0) FROM {MEMORY_SCHEMA}.{table})
                 )"
            ),
            [table],
            |row| row.get(0),
        )?;
        let updated = conn.execute(
            &format!("UPDATE {MEMORY_SCHEMA}.sqlite_sequence SET seq = ?2 WHERE name = ?1"),
            rusqlite::params![table, next],
        )?;
        if updated == 0 {
            conn.execute(
                &format!("INSERT INTO {MEMORY_SCHEMA}.sqlite_sequence (name, seq) VALUES (?1, ?2)"),
                rusqlite::params![table, next],
            )?;
        }
    }
    Ok(())
}

pub(crate) type MemoryFlushWatermarks = BTreeMap<&'static str, i64>;

pub(crate) fn initial_memory_flush_watermarks<'a>(
    conn: &Connection,
    tables: impl IntoIterator<Item = &'a str>,
) -> rusqlite::Result<MemoryFlushWatermarks> {
    let mut watermarks = MemoryFlushWatermarks::new();
    for table in tables {
        if is_disk_only_table(table) {
            continue;
        }
        if !table_exists(conn, "main", table)? {
            continue;
        }
        let Some(table) = canonical_hot_table(table) else {
            continue;
        };
        let max_id = max_table_id(conn, "main", table)?;
        watermarks.insert(table, max_id);
    }
    Ok(watermarks)
}

/// Hot tables whose rows the writer changes after insert. Such an update
/// cannot assume its row is still in `mem`: the flush may already have moved
/// it, so the writer looks in `mem` first and on disk second.
/// `writer_updates_only_the_updatable_tables` in the reader tests holds the
/// writer to this list.
#[cfg(test)]
pub(crate) const UPDATABLE_HOT_TABLES: &[&str] = &["exec_events"];

/// Copy every unflushed memory row to disk and delete it from memory.
///
/// Both happen on the caller's transaction, so a flush that rolls back leaves
/// the rows in memory for the next one, and one that commits leaves memory
/// empty. The delete is a range on the rowid: it costs the rows it removes,
/// not the size of the ledger.
pub fn flush_memory_tables_to_disk<'a>(
    conn: &Connection,
    tables: impl IntoIterator<Item = &'a str>,
    watermarks: &MemoryFlushWatermarks,
) -> rusqlite::Result<MemoryFlushWatermarks> {
    let mut advanced = MemoryFlushWatermarks::new();
    for table in tables {
        if is_disk_only_table(table) {
            continue;
        }
        if !table_exists(conn, "main", table)? || !table_exists(conn, MEMORY_SCHEMA, table)? {
            continue;
        }
        let Some(table) = canonical_hot_table(table) else {
            continue;
        };
        let last_flushed_id = *watermarks.get(table).unwrap_or(&0);
        let max_memory_id = max_table_id(conn, MEMORY_SCHEMA, table)?;
        if max_memory_id > last_flushed_id {
            if table == "net_events" {
                let columns = non_id_table_columns(conn, "main", table)?;
                let column_list = columns.join(", ");
                conn.execute(
                    &format!(
                        "INSERT INTO main.{table} ({column_list})
                         SELECT {column_list} FROM {MEMORY_SCHEMA}.{table}
                         WHERE id > ?1;"
                    ),
                    [last_flushed_id],
                )?;
            } else if table == "security_rule_events" {
                compact_security_rule_snapshots(conn, last_flushed_id)?;
                conn.execute(
                    "INSERT OR REPLACE INTO main.security_rule_events
                     SELECT event.id, event.timestamp_unix_ms, event.event_id, event.event_type,
                            event.rule_id, event.rule_action, event.detection_level,
                            NULL,
                            COALESCE(event.run_id, (
                                SELECT run.id FROM main.security_rule_runs AS run
                                WHERE run.event_type = event.event_type
                                  AND run.rule_id = event.rule_id
                                  AND run.rule_action = event.rule_action
                                  AND run.detection_level = event.detection_level
                                  AND run.rule_json = event.rule_json
                            )),
                            event.trace_id, event.turn_id, event.credential_ref
                     FROM mem.security_rule_events AS event WHERE event.id > ?1",
                    [last_flushed_id],
                )?;
            } else {
                conn.execute(
                    &format!(
                        "INSERT OR REPLACE INTO main.{table}
                         SELECT * FROM {MEMORY_SCHEMA}.{table}
                         WHERE id > ?1;"
                    ),
                    [last_flushed_id],
                )?;
            }
            advanced.insert(table, max_memory_id);
        }
        if max_memory_id > 0 {
            conn.execute(
                &format!("DELETE FROM {MEMORY_SCHEMA}.{table} WHERE id <= ?1"),
                [max_memory_id],
            )?;
        }
    }
    Ok(advanced)
}

/// Normalize repeated rule snapshots at the same commit boundary as their
/// occurrence rows and archive indexes. The hot memory rows are still ordinary
/// complete events; an interrupted disk flush leaves them available to retry.
fn compact_security_rule_snapshots(conn: &Connection, last_flushed_id: i64) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO main.security_rule_runs (
            event_type, rule_id, rule_action, detection_level, rule_json,
            count, first_timestamp_unix_ms, last_timestamp_unix_ms
         )
         SELECT event_type, rule_id, rule_action, detection_level, rule_json,
                COUNT(*), MIN(timestamp_unix_ms), MAX(timestamp_unix_ms)
         FROM mem.security_rule_events
         WHERE id > ?1 AND rule_json IS NOT NULL
         GROUP BY event_type, rule_id, rule_action, detection_level, rule_json
         ON CONFLICT (event_type, rule_id, rule_action, detection_level, rule_json)
         DO UPDATE SET
            count = count + excluded.count,
            first_timestamp_unix_ms = MIN(first_timestamp_unix_ms, excluded.first_timestamp_unix_ms),
            last_timestamp_unix_ms = MAX(last_timestamp_unix_ms, excluded.last_timestamp_unix_ms)",
        [last_flushed_id],
    )?;
    Ok(())
}

pub(super) fn non_id_table_columns(conn: &Connection, schema: &str, table: &str) -> rusqlite::Result<Vec<String>> {
    let mut stmt = conn.prepare(&format!("PRAGMA {schema}.table_info({table})"))?;
    let columns = stmt
        .query_map([], |row| row.get::<_, String>(1))?
        .filter_map(|result| match result {
            Ok(column) if column != "id" => Some(Ok(column)),
            Ok(_) => None,
            Err(error) => Some(Err(error)),
        })
        .collect();
    columns
}
