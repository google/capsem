//! Refuse ledger declarations that cannot enforce canonical integrity metadata.
//! Derive the required checks from the one DDL, without rewriting an old ledger.
use super::{ddl::CREATE_TRANSPORT, CREATE_SCHEMA};
use rusqlite::Connection;

fn normalized(sql: &str) -> String {
    sql.chars()
        .filter(|character| !character.is_ascii_whitespace())
        .collect()
}

/// The host ledger must be able to record every denial this build emits.
/// Compare the kind CHECK against the canonical DDL without rewriting history.
fn validate_host_event_kinds(conn: &Connection) -> rusqlite::Result<()> {
    let required = CREATE_SCHEMA
        .split_once("CREATE TABLE IF NOT EXISTS host_events (")
        .expect("canonical host table")
        .1
        .split_once("CHECK (kind IN (")
        .expect("canonical host kind check")
        .1
        .split_once("))")
        .expect("complete host kind check")
        .0;
    let sql = match conn.query_row(
        "SELECT sql FROM main.sqlite_master WHERE type='table' AND name='host_events'",
        [],
        |row| row.get::<_, String>(0),
    ) {
        Ok(sql) => normalized(&sql),
        Err(rusqlite::Error::QueryReturnedNoRows) => return Ok(()),
        Err(error) => return Err(error),
    };
    let check = format!("CHECK(kindIN({}))", normalized(required));
    if !sql.contains(&check) {
        return Err(rusqlite::Error::InvalidParameterName(
            "session ledger predates current host event kinds: host_events.kind".into(),
        ));
    }
    Ok(())
}

pub(super) fn validate(conn: &Connection) -> rusqlite::Result<()> {
    validate_host_event_kinds(conn)?;
    let mut lookup = conn.prepare_cached("SELECT sql FROM main.sqlite_master WHERE type='table' AND name=?1")?;
    for declaration in [CREATE_SCHEMA, CREATE_TRANSPORT]
        .into_iter()
        .flat_map(|ddl| ddl.split("CREATE TABLE IF NOT EXISTS ").skip(1))
    {
        let table = declaration.split_whitespace().next().expect("canonical table name");
        let mut required = declaration
            .lines()
            .filter(|line| line.contains(" GLOB ") || line.contains(" CHECK (typeof("))
            .peekable();
        if required.peek().is_none() {
            continue;
        }
        let sql = match lookup.query_row([table], |row| row.get::<_, String>(0)) {
            Ok(sql) => normalized(&sql),
            // Missing tables are reported by the owning shape check.
            Err(rusqlite::Error::QueryReturnedNoRows) => continue,
            Err(error) => return Err(error),
        };
        for line in required {
            let line = line.trim().trim_end_matches(',');
            if !sql.contains(&normalized(line)) {
                let column = line.split_whitespace().next().expect("canonical digest column");
                return Err(rusqlite::Error::InvalidParameterName(format!(
                    "session ledger predates canonical integrity checks: {table}.{column}"
                )));
            }
        }
    }
    Ok(())
}
