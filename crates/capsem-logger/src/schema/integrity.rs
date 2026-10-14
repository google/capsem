//! Refuse ledger declarations that cannot enforce canonical digest metadata.
//! Derive the required checks from the one DDL, without rewriting an old ledger.
use super::CREATE_SCHEMA;
use rusqlite::Connection;

fn normalized(sql: &str) -> String {
    sql.chars()
        .filter(|character| !character.is_ascii_whitespace())
        .collect()
}

pub(super) fn validate(conn: &Connection) -> rusqlite::Result<()> {
    let mut lookup = conn.prepare_cached("SELECT sql FROM main.sqlite_master WHERE type='table' AND name=?1")?;
    for declaration in CREATE_SCHEMA.split("CREATE TABLE IF NOT EXISTS ").skip(1) {
        let table = declaration.split_whitespace().next().expect("canonical table name");
        let mut required = declaration
            .lines()
            .filter(|line| line.contains(" GLOB 'blake3:") || line.contains(" GLOB 'credential:blake3:"))
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
                    "session ledger predates canonical digest checks: {table}.{column}"
                )));
            }
        }
    }
    Ok(())
}
