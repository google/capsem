use std::path::Path;

use rusqlite::{params, Connection, OptionalExtension};

use crate::workspace::{
    WorkspaceCheckpoint, WorkspaceFrame, WorkspaceProjector, WorkspaceRecord, WorkspaceSnapshot,
};

#[derive(Debug, thiserror::Error)]
pub enum WorkspaceStoreError {
    #[error("sqlite workspace store error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("workspace record serialization error: {0}")]
    RecordSerde(#[source] serde_json::Error),
    #[error("workspace checkpoint serialization error: {0}")]
    CheckpointSerde(#[source] serde_json::Error),
    #[error("workspace replay failed at record {seq}: {message}")]
    Replay { seq: u64, message: String },
    #[error("workspace record sequence {seq} is not append-only; expected {expected}")]
    NonMonotonicRecord { seq: u64, expected: u64 },
    #[error(
        "workspace checkpoint sequence {checkpoint_seq} is ahead of persisted record sequence {max_record_seq}"
    )]
    CheckpointAhead {
        checkpoint_seq: u64,
        max_record_seq: u64,
    },
    #[error(
        "workspace checkpoint sequence {checkpoint_seq} is older than latest checkpoint {latest_checkpoint_seq}"
    )]
    StaleCheckpoint {
        checkpoint_seq: u64,
        latest_checkpoint_seq: u64,
    },
    #[error("workspace sequence {0} is too large for sqlite")]
    SequenceTooLarge(u64),
}

pub type WorkspaceStoreResult<T> = Result<T, WorkspaceStoreError>;

#[derive(Debug)]
pub struct WorkspaceRestore {
    pub workspace_id: String,
    pub checkpoint: Option<WorkspaceCheckpoint>,
    pub frames: Vec<WorkspaceFrame>,
    pub projector: WorkspaceProjector,
}

impl WorkspaceRestore {
    pub fn snapshot(&self, created_at: String) -> WorkspaceSnapshot {
        WorkspaceSnapshot {
            workspace_id: self.workspace_id.clone(),
            checkpoint: self
                .checkpoint
                .clone()
                .unwrap_or_else(|| self.projector.checkpoint(&self.workspace_id, created_at)),
            projection: self.projector.projection().clone(),
            tail: self.frames.clone(),
        }
    }
}

#[derive(Debug)]
pub struct SqliteWorkspaceStore {
    conn: Connection,
}

impl SqliteWorkspaceStore {
    pub fn open(path: impl AsRef<Path>) -> WorkspaceStoreResult<Self> {
        let conn = Connection::open(path)?;
        let store = Self { conn };
        store.ensure_schema()?;
        Ok(store)
    }

    pub fn open_in_memory() -> WorkspaceStoreResult<Self> {
        let conn = Connection::open_in_memory()?;
        let store = Self { conn };
        store.ensure_schema()?;
        Ok(store)
    }

    pub fn append_record(
        &self,
        workspace_id: &str,
        record: &WorkspaceRecord,
    ) -> WorkspaceStoreResult<()> {
        let expected = self.max_record_seq(workspace_id)? + 1;
        if record.seq != expected {
            return Err(WorkspaceStoreError::NonMonotonicRecord {
                seq: record.seq,
                expected,
            });
        }
        let record_json =
            serde_json::to_string(record).map_err(WorkspaceStoreError::RecordSerde)?;
        self.conn.execute(
            "INSERT INTO workspace_records (workspace_id, seq, record_json)
             VALUES (?1, ?2, ?3)",
            params![workspace_id, sequence_i64(record.seq)?, record_json],
        )?;
        Ok(())
    }

    pub fn save_checkpoint(
        &self,
        workspace_id: &str,
        checkpoint: &WorkspaceCheckpoint,
    ) -> WorkspaceStoreResult<()> {
        let max_record_seq = self.max_record_seq(workspace_id)?;
        if checkpoint.checkpoint_seq > max_record_seq {
            return Err(WorkspaceStoreError::CheckpointAhead {
                checkpoint_seq: checkpoint.checkpoint_seq,
                max_record_seq,
            });
        }
        let latest_checkpoint_seq = self.latest_checkpoint_seq(workspace_id)?;
        if checkpoint.checkpoint_seq < latest_checkpoint_seq {
            return Err(WorkspaceStoreError::StaleCheckpoint {
                checkpoint_seq: checkpoint.checkpoint_seq,
                latest_checkpoint_seq,
            });
        }
        let checkpoint_json =
            serde_json::to_string(checkpoint).map_err(WorkspaceStoreError::CheckpointSerde)?;
        self.conn.execute(
            "INSERT INTO workspace_checkpoints
                (workspace_id, checkpoint_seq, created_at, checkpoint_json)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(workspace_id, checkpoint_seq) DO UPDATE SET
                created_at = excluded.created_at,
                checkpoint_json = excluded.checkpoint_json",
            params![
                workspace_id,
                sequence_i64(checkpoint.checkpoint_seq)?,
                checkpoint.created_at,
                checkpoint_json
            ],
        )?;
        Ok(())
    }

    pub fn clear_workspace(&self, workspace_id: &str) -> WorkspaceStoreResult<()> {
        self.conn.execute(
            "DELETE FROM workspace_records WHERE workspace_id = ?1",
            params![workspace_id],
        )?;
        self.conn.execute(
            "DELETE FROM workspace_checkpoints WHERE workspace_id = ?1",
            params![workspace_id],
        )?;
        Ok(())
    }

    pub fn latest_checkpoint(
        &self,
        workspace_id: &str,
    ) -> WorkspaceStoreResult<Option<WorkspaceCheckpoint>> {
        let json: Option<String> = self
            .conn
            .query_row(
                "SELECT checkpoint_json
                 FROM workspace_checkpoints
                 WHERE workspace_id = ?1
                 ORDER BY checkpoint_seq DESC
                 LIMIT 1",
                params![workspace_id],
                |row| row.get(0),
            )
            .optional()?;
        json.map(|json| serde_json::from_str(&json).map_err(WorkspaceStoreError::CheckpointSerde))
            .transpose()
    }

    pub fn load_records_after(
        &self,
        workspace_id: &str,
        seq: u64,
    ) -> WorkspaceStoreResult<Vec<WorkspaceRecord>> {
        let mut stmt = self.conn.prepare(
            "SELECT record_json
             FROM workspace_records
             WHERE workspace_id = ?1 AND seq > ?2
             ORDER BY seq ASC",
        )?;
        let rows = stmt.query_map(params![workspace_id, sequence_i64(seq)?], |row| {
            row.get::<_, String>(0)
        })?;
        let mut records = Vec::new();
        for row in rows {
            let json = row?;
            let record = serde_json::from_str(&json).map_err(WorkspaceStoreError::RecordSerde)?;
            records.push(record);
        }
        Ok(records)
    }

    pub fn restore(&self, workspace_id: &str) -> WorkspaceStoreResult<WorkspaceRestore> {
        let checkpoint = self.latest_checkpoint(workspace_id)?;
        let checkpoint_seq = checkpoint
            .as_ref()
            .map(|checkpoint| checkpoint.checkpoint_seq)
            .unwrap_or(0);
        let mut projector = checkpoint
            .clone()
            .map(WorkspaceProjector::from_checkpoint)
            .unwrap_or_else(WorkspaceProjector::new);
        let mut frames = Vec::new();
        for record in self.load_records_after(workspace_id, checkpoint_seq)? {
            let deltas =
                projector
                    .apply(&record)
                    .map_err(|message| WorkspaceStoreError::Replay {
                        seq: record.seq,
                        message,
                    })?;
            frames.push(WorkspaceFrame { record, deltas });
        }
        Ok(WorkspaceRestore {
            workspace_id: workspace_id.to_owned(),
            checkpoint,
            frames,
            projector,
        })
    }

    fn ensure_schema(&self) -> WorkspaceStoreResult<()> {
        self.conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS workspace_records (
                workspace_id TEXT NOT NULL,
                seq INTEGER NOT NULL,
                record_json TEXT NOT NULL,
                PRIMARY KEY (workspace_id, seq)
             );
             CREATE TABLE IF NOT EXISTS workspace_checkpoints (
                workspace_id TEXT NOT NULL,
                checkpoint_seq INTEGER NOT NULL,
                created_at TEXT NOT NULL,
                checkpoint_json TEXT NOT NULL,
                PRIMARY KEY (workspace_id, checkpoint_seq)
             );
             CREATE INDEX IF NOT EXISTS idx_workspace_records_seq
                ON workspace_records(workspace_id, seq);
             CREATE INDEX IF NOT EXISTS idx_workspace_checkpoints_latest
                ON workspace_checkpoints(workspace_id, checkpoint_seq DESC);",
        )?;
        Ok(())
    }

    fn max_record_seq(&self, workspace_id: &str) -> WorkspaceStoreResult<u64> {
        let seq: i64 = self.conn.query_row(
            "SELECT COALESCE(MAX(seq), 0)
             FROM workspace_records
             WHERE workspace_id = ?1",
            params![workspace_id],
            |row| row.get(0),
        )?;
        Ok(seq as u64)
    }

    fn latest_checkpoint_seq(&self, workspace_id: &str) -> WorkspaceStoreResult<u64> {
        let seq: i64 = self.conn.query_row(
            "SELECT COALESCE(MAX(checkpoint_seq), 0)
             FROM workspace_checkpoints
             WHERE workspace_id = ?1",
            params![workspace_id],
            |row| row.get(0),
        )?;
        Ok(seq as u64)
    }
}

fn sequence_i64(seq: u64) -> WorkspaceStoreResult<i64> {
    i64::try_from(seq).map_err(|_| WorkspaceStoreError::SequenceTooLarge(seq))
}
