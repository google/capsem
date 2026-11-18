//! Durable, monotonic catalog revocations; refresh never deletes or replaces.
use crate::catalog_revocations::CatalogRevocation;
use rusqlite::{params, Connection};

pub(super) fn observe(conn: &Connection, record: &CatalogRevocation) -> rusqlite::Result<()> {
    record
        .validate()
        .map_err(|error| rusqlite::Error::InvalidParameterName(error.into()))?;
    conn.execute(
        "INSERT INTO main.environment_catalog_revocations
         (catalog_kind,source_repository,entry_id,version,catalog_digest,observed_at_unix_ms)
         VALUES(?1,?2,?3,?4,?5,?6)
         ON CONFLICT(catalog_kind,source_repository,entry_id,version) DO NOTHING",
        params![
            record.key.kind.as_str(),
            record.key.source_repository,
            record.key.entry_id,
            record.key.version,
            record.catalog_digest,
            record.observed_at_unix_ms
        ],
    )?;
    Ok(())
}
