//! Metadata-only retention closure, owned by the existing ledger writer.
use rusqlite::{Connection, Transaction};

pub(super) struct Workset<'a>(pub(super) &'a Connection);

impl Drop for Workset<'_> {
    fn drop(&mut self) {
        // temp_store is MEMORY on the writer. Release this maintenance workset
        // on success and failure rather than retaining a history-sized cache.
        if let Err(error) = self.0.execute_batch(
            "DROP TABLE IF EXISTS temp.retained_environment_nodes;
             DROP TABLE IF EXISTS temp.retained_environment_payloads;",
        ) {
            tracing::warn!(%error, "environment retention workset cleanup failed");
        }
    }
}

pub(super) fn prepare(conn: &Connection, cutoff: &str) -> rusqlite::Result<bool> {
    conn.execute_batch(
        "CREATE TEMP TABLE IF NOT EXISTS retained_environment_nodes (
             kind TEXT NOT NULL, environment_id TEXT NOT NULL, node_id TEXT NOT NULL,
             PRIMARY KEY(kind,environment_id,node_id)
         ) WITHOUT ROWID;
         CREATE TEMP TABLE IF NOT EXISTS retained_environment_payloads (
             source_table TEXT NOT NULL, event_id TEXT NOT NULL,
             PRIMARY KEY(source_table,event_id)
         ) WITHOUT ROWID;
         DELETE FROM retained_environment_nodes;
         DELETE FROM retained_environment_payloads;",
    )?;
    // UNION terminates even for corrupt cyclic lineage. Requests and revisions
    // share the traversal so retained proposals preserve every reconstruction
    // dependency, including a prepared revision and its original proposal.
    conn.execute(
        "WITH RECURSIVE closure(kind,environment_id,node_id) AS (
             SELECT 'revision',environment_id,revision_id FROM environment_revision_references
             UNION
             SELECT 'revision',r.environment_id,r.revision_id FROM environment_revisions r
             WHERE r.state != 'committed' OR EXISTS (
                 SELECT 1 FROM event_body_blobs b WHERE b.source_table='environment_revisions'
                 AND b.event_id=r.payload_event_id AND b.created_at>=?1)
             UNION
             SELECT 'request',q.environment_id,q.request_id FROM environment_requests q
             WHERE q.state='pending' OR (q.state='approved' AND q.result_revision_id IS NULL)
                 OR EXISTS (SELECT 1 FROM event_body_blobs b
                     WHERE b.source_table='environment_requests' AND b.event_id=q.payload_event_id
                     AND b.created_at>=?1)
             UNION
             SELECT 'revision',r.environment_id,r.previous_revision_id
             FROM closure c JOIN environment_revisions r ON c.kind='revision'
                 AND c.environment_id=r.environment_id AND c.node_id=r.revision_id
             WHERE r.previous_revision_id IS NOT NULL
             UNION
             SELECT 'revision',r.fork_parent_environment_id,r.fork_parent_revision_id
             FROM closure c JOIN environment_revisions r ON c.kind='revision'
                 AND c.environment_id=r.environment_id AND c.node_id=r.revision_id
             WHERE r.fork_parent_revision_id IS NOT NULL
             UNION
             SELECT 'request',q.environment_id,q.request_id
             FROM closure c JOIN environment_revisions r ON c.kind='revision'
                 AND c.environment_id=r.environment_id AND c.node_id=r.revision_id
                 JOIN environment_requests q ON q.request_id=r.request_id
             UNION
             SELECT 'revision',q.environment_id,q.base_revision_id
             FROM closure c JOIN environment_requests q ON c.kind='request' AND c.node_id=q.request_id
             WHERE q.base_revision_id IS NOT NULL
             UNION
             SELECT 'revision',q.source_environment_id,q.source_revision_id
             FROM closure c JOIN environment_requests q ON c.kind='request' AND c.node_id=q.request_id
             WHERE q.source_revision_id IS NOT NULL
             UNION
             SELECT 'revision',q.result_environment_id,q.result_revision_id
             FROM closure c JOIN environment_requests q ON c.kind='request' AND c.node_id=q.request_id
             WHERE q.result_revision_id IS NOT NULL
             UNION
             SELECT 'revision',r.environment_id,r.revision_id
             FROM closure c JOIN environment_revisions r ON c.kind='request' AND c.node_id=r.request_id
         ) INSERT INTO retained_environment_nodes SELECT * FROM closure",
        [cutoff],
    )?;
    conn.execute_batch(
        "INSERT INTO retained_environment_payloads
         SELECT 'environment_revisions',r.payload_event_id FROM environment_revisions r
         JOIN retained_environment_nodes c ON c.kind='revision'
             AND c.environment_id=r.environment_id AND c.node_id=r.revision_id
         WHERE r.replayable=1;
         INSERT INTO retained_environment_payloads
         SELECT 'environment_requests',q.payload_event_id FROM environment_requests q
         JOIN retained_environment_nodes c ON c.kind='request' AND c.node_id=q.request_id
         WHERE q.replayable=1;",
    )?;
    let broken: bool = conn.query_row(
        "SELECT EXISTS (SELECT 1 FROM retained_environment_payloads p
         WHERE NOT EXISTS (SELECT 1 FROM event_body_blobs b
             WHERE b.source_table=p.source_table AND b.event_id=p.event_id AND b.direction='payload'))",
        [],
        |row| row.get(0),
    )?;
    if broken {
        return Err(rusqlite::Error::InvalidParameterName(
            "retained environment reconstruction payload is missing".into(),
        ));
    }
    conn.query_row(
        "SELECT EXISTS (
             SELECT 1 FROM environment_revisions r WHERE r.replayable=1 AND NOT EXISTS (
                 SELECT 1 FROM retained_environment_payloads p
                 WHERE p.source_table='environment_revisions' AND p.event_id=r.payload_event_id)
             UNION ALL
             SELECT 1 FROM environment_requests q WHERE q.replayable=1 AND NOT EXISTS (
                 SELECT 1 FROM retained_environment_payloads p
                 WHERE p.source_table='environment_requests' AND p.event_id=q.payload_event_id))",
        [],
        |row| row.get(0),
    )
}

pub(super) fn expire(tx: &Transaction<'_>) -> rusqlite::Result<usize> {
    // Preserve history identity and order. Replayability and payload visibility
    // change in the same transaction as generation publication.
    tx.execute_batch(
        "UPDATE environment_revisions SET replayable=0 WHERE replayable=1 AND NOT EXISTS (
             SELECT 1 FROM retained_environment_payloads p
             WHERE p.source_table='environment_revisions' AND p.event_id=payload_event_id);
         UPDATE environment_requests SET replayable=0 WHERE replayable=1 AND NOT EXISTS (
             SELECT 1 FROM retained_environment_payloads p
             WHERE p.source_table='environment_requests' AND p.event_id=payload_event_id);",
    )?;
    tx.execute(
        "DELETE FROM event_body_blobs WHERE source_table IN ('environment_revisions','environment_requests')
         AND NOT EXISTS (SELECT 1 FROM retained_environment_payloads p
             WHERE p.source_table=event_body_blobs.source_table AND p.event_id=event_body_blobs.event_id)",
        [],
    )
}
