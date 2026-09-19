//! The security ledgers know every event type this build can record.
//!
//! `security_rule_events`, `security_decision_events` and `security_ask_events`
//! each carry a `CHECK (event_type IN (...))`. The list is the build's, so a
//! ledger written before a type existed has a narrower one, and a row of that
//! type cannot be inserted into it at all -- a security decision that was made
//! and cannot be recorded.
//!
//! This module used to fix that by rewriting the table. `network_types::
//! rebuild` renamed the live table to `<name>_before_network_types`, created a
//! new one from `CREATE_SCHEMA`, copied every row across, dropped the old one,
//! and restored the `AUTOINCREMENT` sequence so the ids would look untouched.
//! Three of Capsem's security ledgers, renamed and rewritten in place, on open,
//! with the row ids reset to hide that it happened. That is the shape a
//! forensic ledger must not have, whatever it is used for: the value of these
//! tables is that nobody edited them, and a rail that can rewrite them on the
//! writer's own initiative cannot promise that.
//!
//! Capsem has published no release, so there is no such ledger in the field to
//! rescue. What is left is the detection, which was always the sound half: the
//! same `sql.contains(...)` test, reported instead of acted on. A ledger whose
//! CHECK predates this build is stale, and stale fails by name.
//!
//! The same holds for where the payload lives. These three tables used to
//! carry the matched event inline, in `event_json`; this build archives it.
//! A ledger that still declares the column is refused at open, by table, for
//! the same reason: the alternative is to accept it and fail the first
//! security decision the session tries to record, which is a decision made and
//! lost rather than a ledger turned away.
use super::{table_exists, SECURITY_EVENT_TYPE_CHECK};
use rusqlite::Connection;

/// The three ledgers whose `event_type` is constrained to a known list.
pub(super) const TABLES: &[&str] = &[
    "security_rule_events",
    "security_decision_events",
    "security_ask_events",
];

/// Tables present in `schema` whose `event_type` CHECK is not this build's.
///
/// A table that is absent is not this module's problem -- readiness reports a
/// missing table, and saying it twice in two vocabularies helps nobody.
fn stale(conn: &Connection, schema: &str) -> rusqlite::Result<Vec<&'static str>> {
    let mut found = Vec::new();
    for table in TABLES {
        if !table_exists(conn, schema, table)? {
            continue;
        }
        let sql: Option<String> = conn.query_row(
            &format!("SELECT sql FROM {schema}.sqlite_master WHERE type='table' AND name=?1"),
            [table],
            |row| row.get(0),
        )?;
        if !sql.is_some_and(|sql| sql.contains(SECURITY_EVENT_TYPE_CHECK)) {
            found.push(*table);
        }
    }
    Ok(found)
}

/// Tables present in `schema` that still keep their payload inline.
///
/// Read from the column list rather than the declaration's text, because the
/// question is whether an insert would have to supply it.
fn inline_payloads(conn: &Connection, schema: &str) -> rusqlite::Result<Vec<&'static str>> {
    let mut found = Vec::new();
    for table in TABLES {
        if !table_exists(conn, schema, table)? {
            continue;
        }
        let declares: bool = conn.query_row(
            "SELECT EXISTS (SELECT 1 FROM pragma_table_info(?1, ?2) WHERE name = 'event_json')",
            [*table, schema],
            |row| row.get(0),
        )?;
        if declares {
            found.push(*table);
        }
    }
    Ok(found)
}

/// Every reason `schema` is not this build's security ledger, one clause each.
fn problems(conn: &Connection, schema: &str) -> rusqlite::Result<Vec<String>> {
    let mut problems = Vec::new();
    let stale = stale(conn, schema)?;
    if !stale.is_empty() {
        problems.push(format!("{} constrains event_type to an older list", stale.join(", ")));
    }
    let inline = inline_payloads(conn, schema)?;
    if !inline.is_empty() {
        problems.push(format!(
            "{} keeps its payload inline in event_json, which this build archives",
            inline.join(", ")
        ));
    }
    Ok(problems)
}

/// The writer's check: refuse to open a ledger this build cannot fully record.
pub(super) fn assert_current(conn: &Connection) -> rusqlite::Result<()> {
    let problems = problems(conn, "main")?;
    if problems.is_empty() {
        return Ok(());
    }
    Err(rusqlite::Error::InvalidParameterName(format!(
        "session ledger predates this build's security ledgers: {}",
        problems.join("; ")
    )))
}

/// The reader's check, in readiness's vocabulary.
///
/// Column names cannot catch this: a table with the old CHECK has exactly the
/// columns the new one has. Readiness has to read the declaration itself.
pub(super) fn validate_ready(conn: &Connection, schema: &str) -> Result<(), String> {
    let stale =
        stale(conn, schema).map_err(|error| format!("failed to inspect security event types in {schema}: {error}"))?;
    if !stale.is_empty() {
        return Err(format!(
            "session db table {schema}.{} constrains event_type to a list older than this build",
            stale.join(", ")
        ));
    }
    // Column presence, not absence, is what readiness otherwise checks, so a
    // ledger with the payload still inline would pass it and fail its first
    // security write instead.
    let inline = inline_payloads(conn, schema)
        .map_err(|error| format!("failed to inspect security payload columns in {schema}: {error}"))?;
    if !inline.is_empty() {
        return Err(format!(
            "session db table {schema}.{} keeps its payload inline in event_json, which this build archives",
            inline.join(", ")
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests;
