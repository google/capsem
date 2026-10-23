//! Independent benchmark oracle: original fixture index and verified bytes,
//! never output from the exporter being measured. MessagePack keeps bytes binary.
use super::*;
use serde::{Deserialize, Serialize};

const MAX_RECORDS: usize = 1024;
const MAX_TOTAL_BYTES: usize = capsem_logger::MAX_BODY_BLOB_BYTES;

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct Fact {
    event_id: String,
    source_table: String,
    direction: String,
    content_type: Option<String>,
    original_bytes: u64,
    truncated: bool,
    #[serde(with = "serde_bytes")]
    body: Vec<u8>,
}

fn path() -> PathBuf {
    fixture_path().with_extension("facts.msgpack")
}

fn from_source() -> Vec<Fact> {
    let source = fixture_path();
    let stage = tempfile::tempdir().unwrap();
    let database = stage.path().join("session.db");
    std::fs::copy(&source, &database).unwrap();
    replace_directory(&source.with_extension("bodies"), &database.with_extension("bodies"));
    std::os::unix::fs::OpenOptionsExt::mode(std::fs::OpenOptions::new().write(true).create_new(true), 0o600)
        .open(archive_lock_path(&database))
        .unwrap();
    let index = Connection::open_with_flags(&database, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
    let (rows, bytes): (usize, usize) = index
        .query_row(
            "SELECT COUNT(*), COALESCE(SUM(stored_bytes), 0) FROM event_body_blobs",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert!(rows > 0 && rows <= MAX_RECORDS && bytes > 0 && bytes <= MAX_TOTAL_BYTES);
    let mut statement = index
        .prepare("SELECT DISTINCT event_id FROM event_body_blobs ORDER BY event_id")
        .unwrap();
    let events = statement
        .query_map([], |row| row.get::<_, String>(0))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    let handle = capsem_logger::DbHandle::open_external_reader(&database).unwrap();
    let mut facts = Vec::new();
    for event in events {
        for body in block_on(handle.read_bodies(&event)).unwrap() {
            facts.push(Fact {
                event_id: body.event_id,
                source_table: body.source_table,
                direction: body.direction.as_str().to_owned(),
                content_type: body.content_type,
                original_bytes: body.original_bytes,
                truncated: body.truncated,
                body: body.bytes,
            });
        }
    }
    facts.sort_by(|a, b| {
        (&a.event_id, &a.source_table, &a.direction).cmp(&(&b.event_id, &b.source_table, &b.direction))
    });
    assert_eq!(facts.len(), rows);
    assert_eq!(facts.iter().map(|row| row.body.len()).sum::<usize>(), bytes);
    facts
}

pub(super) fn regenerate() {
    let bytes = rmp_serde::to_vec_named(&from_source()).unwrap();
    assert!(bytes.len() <= MAX_TOTAL_BYTES + MAX_RECORDS * 1024);
    std::fs::write(path(), bytes).unwrap();
    assert_recorded_digests(&[path()]);
}

#[cfg(test)]
#[path = "facts/tests.rs"]
mod tests;
