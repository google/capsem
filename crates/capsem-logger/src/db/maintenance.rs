//! Ledger maintenance the logger crate owns.
//!
//! Core and session code decide when a ledger is compacted or snapshotted; the
//! SQLite and archive work itself stays behind the DB boundary.

use std::ffi::OsStr;
use std::fs::File;
use std::path::Path;
use std::time::{Duration, Instant};

use anyhow::{bail, Context};
use capsem_archive::{BodyLogReader, BodyLogWriter, FILE_HEADER_BYTES};
use capsem_foundation::unix::contained::{ContainedDir, ContainedOpenOptions};
use capsem_foundation::unix::fs::{durable_sync_directory, durable_sync_file, ensure_private_dir};
use capsem_foundation::unix::lock::{self, LockAttempt, LockMode};
use rusqlite::Connection;

const SESSION_DB_FILE: &str = "session.db";
const SNAPSHOT_TIMEOUT: Duration = Duration::from_secs(5 * 60);
type SnapshotBlock = (u64, u64, u32);

struct SnapshotDatabase {
    connection: Connection,
    state: crate::schema::ArchiveState,
    blocks: Vec<SnapshotBlock>,
}

#[cfg(test)]
struct SnapshotPause {
    reached: std::sync::mpsc::SyncSender<()>,
    resume: std::sync::mpsc::Receiver<()>,
}

#[cfg(test)]
static SNAPSHOT_PAUSES: std::sync::Mutex<Option<std::collections::HashMap<std::path::PathBuf, SnapshotPause>>> =
    std::sync::Mutex::new(None);

#[cfg(test)]
pub(crate) fn pause_next_snapshot_after_vacuum_for_tests(
    db_path: &Path,
) -> (std::sync::mpsc::Receiver<()>, std::sync::mpsc::SyncSender<()>) {
    let (reached_tx, reached_rx) = std::sync::mpsc::sync_channel(0);
    let (resume_tx, resume_rx) = std::sync::mpsc::sync_channel(0);
    SNAPSHOT_PAUSES
        .lock()
        .unwrap()
        .get_or_insert_with(std::collections::HashMap::new)
        .insert(
            db_path.to_path_buf(),
            SnapshotPause {
                reached: reached_tx,
                resume: resume_rx,
            },
        );
    (reached_rx, resume_tx)
}

#[cfg(test)]
fn pause_snapshot_after_vacuum_for_tests(db_path: &Path) {
    let pause = SNAPSHOT_PAUSES
        .lock()
        .unwrap()
        .as_mut()
        .and_then(|pauses| pauses.remove(db_path));
    if let Some(pause) = pause {
        pause.reached.send(()).unwrap();
        pause.resume.recv().unwrap();
    }
}

#[cfg(not(test))]
fn pause_snapshot_after_vacuum_for_tests(_db_path: &Path) {}

/// Clone a coherent session ledger into an unpublished destination directory.
///
/// The source archive's shared publication lock stays held from before
/// `VACUUM INTO` until the destination singleton has selected and pinned its
/// exact source generation. Normal appends may continue, while retention must
/// retry later. The pinned descriptor then supplies exactly `committed_end`
/// bytes to a fresh destination generation; WAL files, locks and orphan
/// generations are never copied.
pub fn snapshot_session_ledger(src_dir: &Path, dst_dir: &Path) -> anyhow::Result<()> {
    let deadline = Instant::now() + SNAPSHOT_TIMEOUT;
    let src_db = src_dir.join(SESSION_DB_FILE);
    let dst_db = dst_dir.join(SESSION_DB_FILE);
    let src_archive = crate::writer::archive_path_for_db(&src_db);
    let dst_archive = crate::writer::archive_path_for_db(&dst_db);
    let dst_lock_path = crate::writer::archive_lock_path_for_db(&dst_db);

    refuse_legacy_archive(&src_archive)?;
    std::fs::create_dir_all(dst_dir).context("create ledger snapshot destination")?;
    refuse_existing_destination(&dst_db, &dst_archive, &dst_lock_path)?;

    let archive_lock = lock::acquire_existing_until(
        &crate::writer::archive_lock_path_for_db(&src_db),
        LockMode::Shared,
        deadline,
    )
    .context("acquire source archive snapshot lock")?;
    let SnapshotDatabase {
        connection: dst_conn,
        state,
        blocks,
    } = snapshot_session_db(&src_db, &dst_db, deadline)?;
    pause_snapshot_after_vacuum_for_tests(&src_db);
    let source_directory = ContainedDir::open_root(&src_archive)
        .and_then(|directory| {
            directory.validate_private()?;
            Ok(directory)
        })
        .with_context(|| format!("open source archive directory {}", src_archive.display()))?;
    let generation_name = state.header.generation_id.file_name();
    let mut source = source_directory
        .open_file(OsStr::new(&generation_name), ContainedOpenOptions::read_only())
        .with_context(|| format!("open source archive generation {generation_name}"))?;
    BodyLogReader::from_descriptor(source.try_clone()?, state.header, state.committed_end)
        .context("validate source archive generation identity")?;
    validate_generation_blocks(&mut source, &blocks, deadline)?;
    drop(archive_lock);

    ensure_private_dir(&dst_archive)
        .with_context(|| format!("create destination archive directory {}", dst_archive.display()))?;
    durable_sync_directory(dst_dir).context("sync destination archive directory entry")?;
    let destination_lock = match lock::try_acquire(&dst_lock_path, LockMode::Exclusive)
        .with_context(|| format!("create destination archive lock {}", dst_lock_path.display()))?
    {
        LockAttempt::Acquired(lock) => lock,
        LockAttempt::Contended => bail!("fresh destination archive lock is unexpectedly contended"),
    };
    let destination_directory = ContainedDir::open_root(&dst_archive)
        .and_then(|directory| {
            directory.validate_private()?;
            Ok(directory)
        })
        .with_context(|| format!("open destination archive directory {}", dst_archive.display()))?;
    let mut writer = BodyLogWriter::create_generation(
        &destination_directory,
        state.header.archive_id,
        state.header.generation_id,
    )
    .context("create destination archive generation")?;
    writer
        .copy_committed_prefix_from(&source, state.committed_end, deadline)
        .context("copy committed archive generation")?;
    writer.sync().context("sync destination archive generation")?;
    drop(writer);
    destination_directory
        .sync()
        .context("sync destination archive directory")?;

    let mut destination = destination_directory
        .open_file(OsStr::new(&generation_name), ContainedOpenOptions::read_only())
        .context("reopen destination archive generation")?;
    BodyLogReader::from_descriptor(destination.try_clone()?, state.header, state.committed_end)
        .context("validate destination archive generation identity")?;
    validate_generation_blocks(&mut destination, &blocks, deadline)?;

    drop(dst_conn);
    let dst_db_file = capsem_foundation::unix::fs::open_regular_file_no_follow(&dst_db)
        .context("reopen destination session database")?;
    durable_sync_file(&dst_db_file).context("sync destination session database")?;
    durable_sync_directory(dst_dir).context("sync complete destination ledger")?;
    drop(destination_lock);
    tracing::debug!(
        src_db_path = %src_db.display(),
        dst_db_path = %dst_db.display(),
        archive_generation = generation_name,
        committed_end = state.committed_end,
        operation = "snapshot",
        "session ledger snapshot completed"
    );
    Ok(())
}

fn refuse_legacy_archive(path: &Path) -> anyhow::Result<()> {
    let metadata =
        std::fs::symlink_metadata(path).with_context(|| format!("inspect source archive {}", path.display()))?;
    if metadata.is_file() {
        bail!(
            "legacy v2 archive {} is a regular file; refusing format v3 snapshot",
            path.display()
        );
    }
    if !metadata.is_dir() {
        bail!("source archive {} is not a directory", path.display());
    }
    Ok(())
}

fn refuse_existing_destination(db: &Path, archive: &Path, lock: &Path) -> anyhow::Result<()> {
    for path in [db, archive, lock] {
        match std::fs::symlink_metadata(path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Ok(_) => bail!("refusing to overwrite destination ledger path {}", path.display()),
            Err(error) => return Err(error).with_context(|| format!("inspect destination path {}", path.display())),
        }
    }
    Ok(())
}

fn snapshot_session_db(src: &Path, dst: &Path, deadline: Instant) -> anyhow::Result<SnapshotDatabase> {
    let src_conn = Connection::open_with_flags(
        src,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .with_context(|| format!("open source session database {}", src.display()))?;
    src_conn.progress_handler(10_000, Some(move || Instant::now() >= deadline));
    let escaped = dst.to_string_lossy().replace('\'', "''");
    let vacuum = src_conn.execute_batch(&format!("VACUUM INTO '{escaped}';"));
    src_conn.progress_handler(0, None::<fn() -> bool>);
    vacuum.with_context(|| format!("VACUUM INTO destination session database {}", dst.display()))?;
    drop(src_conn);

    let dst_conn = Connection::open_with_flags(
        dst,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .with_context(|| format!("open destination session database {}", dst.display()))?;
    dst_conn.progress_handler(10_000, Some(move || Instant::now() >= deadline));
    let quick_check: String = dst_conn
        .pragma_query_value(None, "quick_check", |row| row.get(0))
        .context("quick-check destination session database")?;
    if !quick_check.eq_ignore_ascii_case("ok") {
        bail!("cloned session db failed quick_check: {quick_check}");
    }
    crate::schema::validate_ready_schema(&dst_conn)
        .map_err(anyhow::Error::msg)
        .context("validate destination session schema")?;
    let state = crate::schema::archive_state(&dst_conn).context("read destination archive singleton")?;
    let blocks = snapshot_blocks(&dst_conn, state.committed_end)?;
    validate_body_bounds(&dst_conn)?;
    Ok(SnapshotDatabase {
        connection: dst_conn,
        state,
        blocks,
    })
}

fn snapshot_blocks(conn: &Connection, committed_end: u64) -> anyhow::Result<Vec<SnapshotBlock>> {
    let mut statement =
        conn.prepare("SELECT block_offset, disk_len, raw_len FROM body_blocks ORDER BY block_offset")?;
    let rows = statement.query_map([], |row| {
        Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?, row.get::<_, i64>(2)?))
    })?;
    let mut blocks = Vec::new();
    for row in rows {
        let (block_offset, disk_len, raw_len) = row?;
        let block_offset = u64::try_from(block_offset).context("negative archive block offset")?;
        let disk_len = u64::try_from(disk_len).context("negative archive block length")?;
        let raw_len = u32::try_from(raw_len).context("archive block raw length exceeds u32")?;
        let end = block_offset
            .checked_add(disk_len)
            .context("archive block extent overflow")?;
        if block_offset < FILE_HEADER_BYTES as u64 || end > committed_end {
            bail!("archive block extent {block_offset}..{end} is outside committed_end {committed_end}");
        }
        blocks.push((block_offset, disk_len, raw_len));
    }
    Ok(blocks)
}

fn validate_body_bounds(conn: &Connection) -> anyhow::Result<()> {
    let invalid: i64 = conn.query_row(
        "SELECT COUNT(*) FROM event_body_blobs b
         LEFT JOIN body_blocks k ON k.block_offset = b.block_offset
         WHERE k.block_offset IS NULL OR b.body_offset < 0 OR b.body_len < 0
            OR b.body_offset + b.body_len > k.raw_len",
        [],
        |row| row.get(0),
    )?;
    if invalid != 0 {
        bail!("destination archive index contains {invalid} body rows outside their blocks");
    }
    Ok(())
}

fn validate_generation_blocks(source: &mut File, blocks: &[SnapshotBlock], deadline: Instant) -> anyhow::Result<()> {
    for &(block_offset, disk_len, expected_raw_len) in blocks {
        if Instant::now() >= deadline {
            bail!("ledger snapshot validation timed out");
        }
        let raw_len = capsem_archive::validate_block_extent(source, block_offset, disk_len)
            .with_context(|| format!("validate archive block at {block_offset}"))?;
        if raw_len != expected_raw_len {
            bail!("archive block at {block_offset} has {raw_len} raw bytes; index records {expected_raw_len}");
        }
    }
    Ok(())
}
