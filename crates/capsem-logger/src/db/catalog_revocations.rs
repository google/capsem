//! Exact indexed reads through the existing query-only DB worker; no cache.
use super::*;
use crate::catalog_revocations::{CatalogRevocation, CatalogRevocationKey};
use rusqlite::OptionalExtension;

impl DbHandle {
    pub async fn catalog_revocation(&self, key: CatalogRevocationKey) -> DbResult<Option<CatalogRevocation>> {
        key.validate().map_err(str::to_owned)?;
        let (reply, receiver) = tokio::sync::oneshot::channel();
        self.send_reader(ReadRequest::CatalogRevocation { key, reply })
            .map_err(|_| "catalog revocation reader worker closed")?;
        receiver
            .await
            .map_err(|_| "catalog revocation reader dropped reply")?
            .map(|observed| self.take_observed(observed))
    }
}

pub(super) fn execute(reader: &DbReader, key: &CatalogRevocationKey) -> DbResult<Option<CatalogRevocation>> {
    let started = Instant::now();
    let _deadline = reader.sql_deadline(started + bodies::INTERACTIVE_CAPTURE_DEADLINE);
    let result = reader
        .connection()
        .query_row(
            "SELECT catalog_digest,observed_at_unix_ms FROM environment_catalog_revocations
         WHERE catalog_kind=?1 AND source_repository=?2 AND entry_id=?3 AND version=?4",
            rusqlite::params![key.kind.as_str(), key.source_repository, key.entry_id, key.version],
            |row| {
                let rusqlite::types::ValueRef::Text(bytes) = row.get_ref(0)? else {
                    return Err(rusqlite::Error::InvalidQuery);
                };
                if bytes.len() != 71 {
                    return Err(rusqlite::Error::InvalidQuery);
                }
                let digest = std::str::from_utf8(bytes).map_err(|_| rusqlite::Error::InvalidQuery)?;
                let record = CatalogRevocation {
                    key: key.clone(),
                    catalog_digest: digest.into(),
                    observed_at_unix_ms: row.get(1)?,
                };
                record.validate().map_err(|_| rusqlite::Error::InvalidQuery)?;
                Ok(record)
            },
        )
        .optional()
        .map_err(|error| format!("catalog revocation query: {error}"));
    let status = if result.is_ok() { "ok" } else { "error" };
    ::metrics::counter!(DB_QUERY_TOTAL, "phase" => "catalog_revocation", "status" => status).increment(1);
    ::metrics::histogram!(DB_QUERY_DURATION_MS, "phase" => "catalog_revocation", "status" => status)
        .record(elapsed_ms_f64(started));
    match &result {
        Ok(record) => {
            ::metrics::histogram!(DB_QUERY_RESULT_ROWS, "phase" => "catalog_revocation").record(if record.is_some() {
                1.0
            } else {
                0.0
            });
            tracing::debug!(
                operation = "catalog_revocation",
                found = record.is_some(),
                duration_ms = elapsed_ms(started),
                "catalog revocation read"
            );
        }
        Err(error) => {
            tracing::error!(operation = "catalog_revocation", %error, duration_ms = elapsed_ms(started), "catalog revocation read failed")
        }
    }
    result
}

#[cfg(test)]
mod tests;
