//! One archive staging path, with lossless guarantees for approved configuration.
use super::*;

pub(in crate::writer) const ENVIRONMENT_REVISION_SOURCE: &str = "environment_revisions";
pub(in crate::writer) const ENVIRONMENT_REQUEST_SOURCE: &str = "environment_requests";

impl BodyArchive {
    /// Stage one body: its bytes into the open block, its index row beside
    /// them. Empty and absent bodies produce neither.
    pub(in crate::writer) fn stage(&mut self, conn: &Connection, blob: EventBodyBlob<'_>) {
        // Only the owned environment row writers select these static sources.
        // They require complete, durable payloads instead of optional excerpts.
        let required = matches!(
            blob.source_table,
            ENVIRONMENT_REVISION_SOURCE | ENVIRONMENT_REQUEST_SOURCE
        );
        if required && self.required_failure.is_some() {
            return;
        }
        if self.writer.is_none() {
            if required {
                self.required_failure = Some("archive_unavailable");
            }
            return;
        }
        let Some(bytes) = blob.body.filter(|body| !body.is_empty()) else {
            if required {
                self.required_failure = Some("missing_payload");
            }
            return;
        };
        if required
            && (bytes.len() > MAX_BODY_BLOB_BYTES
                || blob
                    .original_bytes
                    .is_some_and(|original| original > bytes.len() as u64))
        {
            self.required_failure = Some("incomplete_payload");
            return;
        }
        let stored_len = bytes.len().min(MAX_BODY_BLOB_BYTES);
        // What the producer sent may already be an excerpt -- guest exec
        // output is capped at the vsock boundary -- and then the row must
        // report the size it was cut from, not the size that arrived. A
        // producer that reports less than it sent is not believed: the floor
        // is what actually arrived, so under-reporting cannot also erase the
        // truncation this writer did on top of it.
        let original_bytes = blob.original_bytes.unwrap_or(0).max(bytes.len() as u64);
        let stored = &bytes[..stored_len];
        // Over the stored bytes, not the buffer they were cut from: a hash of
        // something the archive does not hold cannot be checked against
        // anything, and a reader that verifies what it read is how a corrupted
        // index row stops being a body. It is also what finds a repeat.
        let body_hash = content_digest(stored);
        let Some((reference, committed)) = self.stage_bytes(conn, stored, &body_hash, required) else {
            if required {
                self.required_failure = Some("stage");
            }
            return;
        };
        let seq = self.next_seq;
        self.next_seq += 1;
        // A row naming bytes already committed and synced waits for no
        // segment: it joins the next transaction as it is.
        let pending = if committed { &mut self.reused } else { &mut self.staged };
        pending.push(BodyIndexRow {
            seq,
            required,
            event_id: blob.event_id.to_string(),
            event_type: blob.event_type,
            source_table: blob.source_table,
            direction: blob.direction,
            content_type: blob.content_type.map(str::to_string),
            original_bytes: original_bytes as i64,
            stored_bytes: stored_len as i64,
            // Per row, even when the span is shared: two producers can cut the
            // same stored bytes from bodies of different sizes.
            truncated: original_bytes > stored_len as u64,
            body_hash,
            block_offset: reference.block_offset as i64,
            body_offset: i64::from(reference.offset),
            trace_id: blob.trace_id.map(str::to_string),
            turn_id: blob.turn_id.map(str::to_string),
            created_at: format_timestamp((self.now)()),
        });
    }

    /// Stage bytes into the open block -- or, when the open block already
    /// holds these exact bytes, hand back their span and add nothing.
    ///
    /// Keyed by the full XXH3-128 fingerprint of the stored bytes: a length or
    /// a prefix match would index one body against another's bytes, and the
    /// reader's hash check would then refuse it forever.
    ///
    /// A block open longer than `MAX_BLOCK_AGE` is closed first, so this body
    /// starts a new one. `BlockFull` is the archive's close-and-retry
    /// contract, not a failure: the open block is within 16 MiB of its ceiling
    /// and this body does not fit beside what is already there. Closing
    /// empties it, and a body capped at `MAX_BODY_BLOB_BYTES` (10 MiB) always
    /// fits an empty one, so the retry is the last step and not a loop.
    ///
    /// Every other error skips the body and logs. A poisoned writer takes the
    /// archive out of service here, because the rows staged beside it would
    /// name bytes no file received. In no case may a body take down the
    /// writer thread that owns the whole session ledger.
    fn stage_bytes(
        &mut self,
        conn: &Connection,
        bytes: &[u8],
        body_hash: &str,
        required: bool,
    ) -> Option<(BodyRef, bool)> {
        if self.block_is_stale() {
            self.close_block();
        }
        // Required snapshots never substitute bytes using a fast fingerprint.
        if !required {
            if let Some(reference) = self.block_spans.get(body_hash) {
                ::metrics::counter!(DB_ARCHIVE_BODIES_DEDUPLICATED_TOTAL, "scope" => "block").increment(1);
                return Some((*reference, false));
            }
            if let Some(reference) = committed_span(conn, bytes.len(), body_hash) {
                ::metrics::counter!(DB_ARCHIVE_BODIES_DEDUPLICATED_TOTAL, "scope" => "archive").increment(1);
                return Some((reference, true));
            }
        }
        match self.stage_into_open_block(bytes, body_hash, !required)? {
            Ok(reference) => return Some((reference, false)),
            Err(ArchiveError::BlockFull) => {}
            Err(error) => {
                self.refuse_body(&error, bytes.len(), "stage");
                return None;
            }
        }
        self.close_block();
        match self.stage_into_open_block(bytes, body_hash, !required) {
            Some(Ok(reference)) => Some((reference, false)),
            Some(Err(error)) => {
                self.refuse_body(&error, bytes.len(), "stage_after_close");
                None
            }
            None => {
                // `close_block` failed its write and retired the writer. The
                // count it took covered the rows it was holding; this body has
                // no row yet, so it was not among them. Without this line it is
                // the one body the counter never sees.
                warn!(body_bytes = bytes.len(), "body not archived after close");
                self.drop_bodies(1, "stage_after_close");
                None
            }
        }
    }

    /// One attempt at staging: `None` when there is no writer.
    fn stage_into_open_block(
        &mut self,
        bytes: &[u8],
        body_hash: &str,
        cache_span: bool,
    ) -> Option<Result<BodyRef, ArchiveError>> {
        let writer = self.writer.as_mut()?;
        let opening = writer.open_block_raw_len().is_none();
        let staged = writer.stage(bytes);
        if let Ok(reference) = &staged {
            if opening {
                self.block_opened_at = Some((self.now)());
            }
            if cache_span {
                self.block_spans.insert(body_hash.to_string(), *reference);
            }
        }
        Some(staged)
    }

    pub(in crate::writer) fn required_reader(
        &self,
        conn: &Connection,
    ) -> rusqlite::Result<capsem_archive::BodyLogReader> {
        let state = crate::schema::archive_state(conn)?;
        let directory = self
            .directory
            .as_ref()
            .ok_or_else(|| archive_contract_error("required snapshot archive is unavailable"))?;
        let reader = capsem_archive::BodyLogReader::open_generation_with_codecs(
            directory,
            state.header,
            state.committed_end,
            self.codecs.clone(),
        )
        .map_err(|_| archive_contract_error("required snapshot archive cannot be opened"))?;
        Ok(reader)
    }

    pub(in crate::writer) fn read_required_bytes(
        &self,
        conn: &Connection,
        event_id: &str,
        source: &str,
    ) -> rusqlite::Result<Vec<u8>> {
        let reader = self.required_reader(conn)?;
        Self::read_required_bytes_with_reader(conn, &reader, event_id, source)
    }

    pub(in crate::writer) fn read_required_bytes_with_reader(
        conn: &Connection,
        reader: &capsem_archive::BodyLogReader,
        event_id: &str,
        source: &str,
    ) -> rusqlite::Result<Vec<u8>> {
        let (block, offset, len, truncated, hash): (i64,i64,i64,bool,String) = conn.query_row(
            "SELECT block_offset,body_offset,body_len,truncated,body_hash FROM main.event_body_blobs WHERE event_id=?1 AND source_table=?2 AND direction='payload'",
            rusqlite::params![event_id,source], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?,row.get(4)?)))?;
        if truncated {
            return Err(archive_contract_error("required snapshot is truncated"));
        }
        let reference = BodyRef {
            block_offset: block
                .try_into()
                .map_err(|_| archive_contract_error("invalid snapshot block"))?,
            offset: offset
                .try_into()
                .map_err(|_| archive_contract_error("invalid snapshot offset"))?,
            len: len
                .try_into()
                .map_err(|_| archive_contract_error("invalid snapshot length"))?,
        };
        let bytes = reader
            .read(reference)
            .map_err(|_| archive_contract_error("required snapshot archive cannot be read"))?;
        if content_digest(&bytes) != hash {
            return Err(archive_contract_error(
                "required snapshot failed stored-content verification",
            ));
        }
        Ok(bytes)
    }

    pub(in crate::writer) fn required_status(&self) -> rusqlite::Result<()> {
        match self.required_failure {
            None => Ok(()),
            Some(reason) => Err(archive_contract_error(format!(
                "required archive payload was not preserved: {reason}"
            ))),
        }
    }

    pub(super) fn note_required_loss(&mut self, reason: &'static str) {
        let required = self.staged.iter().chain(&self.reused).any(|row| row.required)
            || self
                .appended
                .iter()
                .chain(&self.synced)
                .chain(self.syncing.iter().flatten())
                .any(|segment| segment.rows.iter().any(|row| row.required));
        if required && self.required_failure.is_none() {
            self.required_failure = Some(reason);
        }
    }
}
